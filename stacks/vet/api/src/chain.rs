//! `ChainClient` trait abstracting the ROAX (chainId 135) on-chain surface the backend needs, plus an
//! Alloy-backed implementation and an in-memory `MemChain` stub (emulates issue / isValid / RootIssued)
//! so the full HTTP flow is testable without a live node.
//!
//! Signing (impl §1.8): EIP-1559 with a legacy `gas_price` fallback; chainId pinned to 135.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, Bytes, FixedBytes, B256, U256};
use alloy::sol;
use alloy::sol_types::SolEvent;
use async_trait::async_trait;

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IDogTagIssuer {
        event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
        event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);
        function issue(bytes32 r) external;
        function revoke(bytes32 r) external;
        function isValid(bytes32 r) external view returns (bool);
        function issuedAt(bytes32 r) external view returns (uint256);
    }

    #[sol(rpc)]
    contract IIssuerRegistry {
        function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Other(String),
}

/// Result of a broadcast: the tx hash plus the fields confirm must bind against.
#[derive(Clone, Debug)]
pub struct SentTx {
    pub tx_hash: String,
}

/// The fields of a mined transaction that `confirm` binds to the prepared draft (impl §11.6).
#[derive(Clone, Debug)]
pub struct TxView {
    pub to: String,
    pub input: String, // hex calldata, 0x-prefixed
    pub value: U256,
    pub chain_id: Option<u64>,
    pub from: String,
    pub success: bool,
    /// RootIssued logs emitted by `issuer_addr` in this tx: (root_hex, by_addr).
    pub root_issued_logs: Vec<(String, String)>,
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Register the backend signer (32-byte secp256k1 private key) for an account index, with its
    /// derived address. Called by the unlock handler after custody decrypts the seed. The Alloy impl
    /// keeps the key for broadcasting; MemChain keeps only the address.
    async fn register_signer(&self, index: u32, private_key: [u8; 32], address: String);
    /// DogTagIssuer.isValid(root).
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError>;
    /// DogTagIssuer.issuedAt(root) (0 == not issued).
    async fn issued_at(&self, issuer_addr: &str, root: &str) -> Result<U256, ChainError>;
    /// IssuerRegistry.isWhitelistedFor(recordType, signer).
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError>;
    /// Sign+broadcast a tx FROM the backend signer at `account_index` to `to` with `calldata`.
    /// Returns the tx hash. EIP-1559 with legacy fallback.
    async fn sign_and_send(
        &self,
        account_index: u32,
        to: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError>;
    /// Fetch a mined tx's bound view (tx fields + RootIssued logs from `issuer_addr`),
    /// waiting up to `confirmations` blocks. Err(NotFound) if unknown/unmined.
    async fn get_tx_view(
        &self,
        tx_hash: &str,
        issuer_addr: &str,
        confirmations: u64,
    ) -> Result<TxView, ChainError>;
    /// Encode issue(bytes32) calldata for `root`.
    fn encode_issue(&self, root: &str) -> String {
        issue_calldata(root)
    }
    /// Encode revoke(bytes32) calldata for `root`.
    fn encode_revoke(&self, root: &str) -> String {
        revoke_calldata(root)
    }
}

fn parse_b256(h: &str) -> B256 {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let mut out = [0u8; 32];
    if let Ok(b) = hex::decode(s) {
        if b.len() == 32 {
            out.copy_from_slice(&b);
        }
    }
    B256::from(out)
}

fn parse_addr(h: &str) -> Address {
    h.parse::<Address>().unwrap_or(Address::ZERO)
}

/// Exact typed calldata encoders (canonical selectors).
pub fn issue_calldata(root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuer::issueCall { r: parse_b256(root) };
    format!("0x{}", hex::encode(call.abi_encode()))
}
pub fn revoke_calldata(root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuer::revokeCall { r: parse_b256(root) };
    format!("0x{}", hex::encode(call.abi_encode()))
}

// --------------------------------------------------------------------------------------------
// MemChain — in-memory emulation of issue / isValid / issuedAt / RootIssued + whitelist.
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemChainInner {
    /// (issuer_addr, root) -> issuedAt timestamp (0 == not issued/revoked-cleared).
    issued: HashMap<(String, String), u64>,
    revoked: HashMap<(String, String), u64>,
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    /// txHash -> TxView (recorded at sign_and_send time).
    txs: HashMap<String, TxView>,
    /// backend signer addresses by account index.
    signers: HashMap<u32, String>,
    nonce: u64,
    clock: u64,
}

#[derive(Clone, Default)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register a backend signer address for an account index (test harness wires this from custody).
    pub fn set_signer(&self, index: u32, address: &str) {
        self.inner.lock().unwrap().signers.insert(index, address.to_lowercase());
    }
    /// Whitelist a signer for a (registry, recordType, signer) tuple.
    pub fn whitelist(&self, registry: &str, record_type: &str, signer: &str) {
        self.inner.lock().unwrap().whitelist.insert(
            (registry.to_lowercase(), record_type.to_lowercase(), signer.to_lowercase()),
            true,
        );
    }
    /// Decode an issue(bytes32)/revoke(bytes32) calldata into (is_issue, root_hex).
    fn decode_b32_call(calldata: &str) -> Option<(bool, String)> {
        let s = calldata.strip_prefix("0x").unwrap_or(calldata);
        let bytes = hex::decode(s).ok()?;
        if bytes.len() != 36 {
            return None;
        }
        let selector = &bytes[0..4];
        let root = format!("0x{}", hex::encode(&bytes[4..36]));
        // canonical selectors from the sol! ABI.
        use alloy::sol_types::SolCall;
        if selector == IDogTagIssuer::issueCall::SELECTOR {
            Some((true, root))
        } else if selector == IDogTagIssuer::revokeCall::SELECTOR {
            Some((false, root))
        } else {
            None
        }
    }
}

#[async_trait]
impl ChainClient for MemChain {
    async fn register_signer(&self, index: u32, _private_key: [u8; 32], address: String) {
        self.inner.lock().unwrap().signers.insert(index, address.to_lowercase());
    }
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        let issued = g.issued.get(&key).copied().unwrap_or(0) != 0;
        let revoked = g.revoked.get(&key).copied().unwrap_or(0) != 0;
        Ok(issued && !revoked)
    }
    async fn issued_at(&self, issuer_addr: &str, root: &str) -> Result<U256, ChainError> {
        let g = self.inner.lock().unwrap();
        let v = g.issued.get(&(issuer_addr.to_lowercase(), root.to_lowercase())).copied().unwrap_or(0);
        Ok(U256::from(v))
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g
            .whitelist
            .get(&(registry_addr.to_lowercase(), record_type.to_lowercase(), signer.to_lowercase()))
            .copied()
            .unwrap_or(false))
    }
    async fn sign_and_send(
        &self,
        account_index: u32,
        to: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        let signer = g
            .signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no signer for index".into()))?;
        let (is_issue, root) = Self::decode_b32_call(calldata)
            .ok_or_else(|| ChainError::Other("undecodable calldata".into()))?;
        let to_l = to.to_lowercase();
        let key = (to_l.clone(), root.to_lowercase());
        g.clock += 12;
        let ts = g.clock;
        let mut logs = Vec::new();
        if is_issue {
            // emulate DogTagIssuer.issue: require whitelisted? The MemChain whitelist is enforced by
            // the backend preflight; here we emulate the on-chain effect of a successful issue.
            if g.issued.get(&key).copied().unwrap_or(0) != 0 {
                return Err(ChainError::Other("BadRoot: already issued".into()));
            }
            g.issued.insert(key, ts);
            logs.push((root.to_lowercase(), signer.clone()));
        } else {
            if g.issued.get(&key).copied().unwrap_or(0) == 0 {
                return Err(ChainError::Other("BadRoot: not issued".into()));
            }
            g.revoked.insert(key, ts);
        }
        g.nonce += 1;
        let tx_hash = format!("0x{:064x}", g.nonce);
        let view = TxView {
            to: to_l,
            input: calldata.to_lowercase(),
            value: U256::ZERO,
            chain_id: Some(ROAX_CHAIN_ID),
            from: signer,
            success: true,
            root_issued_logs: logs,
        };
        g.txs.insert(tx_hash.clone(), view);
        Ok(SentTx { tx_hash })
    }
    async fn get_tx_view(
        &self,
        tx_hash: &str,
        issuer_addr: &str,
        _confirmations: u64,
    ) -> Result<TxView, ChainError> {
        let g = self.inner.lock().unwrap();
        let mut view = g.txs.get(tx_hash).cloned().ok_or(ChainError::NotFound)?;
        // only return RootIssued logs from the pinned issuer.
        if view.to != issuer_addr.to_lowercase() {
            view.root_issued_logs.clear();
        }
        Ok(view)
    }
}

// --------------------------------------------------------------------------------------------
// AlloyChain — real ROAX/anvil-backed client using a MnemonicBuilder-derived wallet set.
// --------------------------------------------------------------------------------------------

/// A funded, unlocked Alloy chain client. Holds derived signers (by account index) and the RPC url.
pub struct AlloyChain {
    pub rpc_url: String,
    /// account index -> alloy local signer (registered at unlock time).
    signers: Mutex<HashMap<u32, alloy::signers::local::PrivateKeySigner>>,
}

impl AlloyChain {
    pub fn new(rpc_url: String) -> Self {
        AlloyChain { rpc_url, signers: Mutex::new(HashMap::new()) }
    }
    fn signer(&self, index: u32) -> Option<alloy::signers::local::PrivateKeySigner> {
        self.signers.lock().unwrap().get(&index).cloned()
    }
}

#[async_trait]
impl ChainClient for AlloyChain {
    async fn register_signer(&self, index: u32, private_key: [u8; 32], _address: String) {
        if let Ok(s) = alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from(private_key)) {
            self.signers.lock().unwrap().insert(index, s);
        }
    }
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let r = c
            .isValid(parse_b256(root))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn issued_at(&self, issuer_addr: &str, root: &str) -> Result<U256, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let r = c
            .issuedAt(parse_b256(root))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IIssuerRegistry::new(parse_addr(registry_addr), provider);
        let r = c
            .isWhitelistedFor(parse_b256(record_type), parse_addr(signer))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn sign_and_send(
        &self,
        account_index: u32,
        to: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        use alloy::network::EthereumWallet;
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;
        use alloy::network::TransactionBuilder;

        let signer = self
            .signer(account_index)
            .ok_or_else(|| ChainError::Other("no signer for index (unlocked?)".into()))?;
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let data = Bytes::from(
            hex::decode(calldata.strip_prefix("0x").unwrap_or(calldata))
                .map_err(|e| ChainError::Other(format!("bad calldata: {e}")))?,
        );
        let tx = TransactionRequest::default()
            .with_to(parse_addr(to))
            .with_input(data)
            .with_value(U256::ZERO)
            .with_chain_id(ROAX_CHAIN_ID);

        // EIP-1559 first; if the node rejects 1559 fields, fall back to legacy gas_price.
        let pending = match provider.send_transaction(tx.clone()).await {
            Ok(p) => p,
            Err(_) => {
                let gp = provider.get_gas_price().await.map_err(|e| ChainError::Rpc(e.to_string()))?;
                let legacy = tx.with_gas_price(gp);
                provider
                    .send_transaction(legacy)
                    .await
                    .map_err(|e| ChainError::Rpc(e.to_string()))?
            }
        };
        let tx_hash = format!("{:#x}", *pending.tx_hash());
        Ok(SentTx { tx_hash })
    }
    async fn get_tx_view(
        &self,
        tx_hash: &str,
        issuer_addr: &str,
        confirmations: u64,
    ) -> Result<TxView, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let hash: B256 = parse_b256(tx_hash);

        let tx = provider
            .get_transaction_by_hash(hash)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            .ok_or(ChainError::NotFound)?;
        let receipt = provider
            .get_transaction_receipt(hash)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            .ok_or(ChainError::NotFound)?;

        // wait for N confirmations (reorg-safe).
        if confirmations > 1 {
            if let Some(bn) = receipt.block_number {
                loop {
                    let head = provider.get_block_number().await.map_err(|e| ChainError::Rpc(e.to_string()))?;
                    if head.saturating_sub(bn) + 1 >= confirmations {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }

        let issuer = parse_addr(issuer_addr);
        let mut logs = Vec::new();
        for log in receipt.inner.logs() {
            if log.address() != issuer {
                continue;
            }
            if let Ok(ev) = IDogTagIssuer::RootIssued::decode_log(log.as_ref(), true) {
                logs.push((
                    format!("0x{}", hex::encode(ev.root.as_slice())),
                    format!("{:#x}", ev.by),
                ));
            }
        }

        use alloy::consensus::Transaction as _ConsensusTx;
        let to = tx.inner.to().map(|a| format!("{a:#x}")).unwrap_or_default();
        Ok(TxView {
            to,
            input: format!("0x{}", hex::encode(tx.inner.input())),
            value: tx.inner.value(),
            chain_id: tx.inner.chain_id(),
            from: format!("{:#x}", tx.from),
            success: receipt.status(),
            root_issued_logs: logs,
        })
    }
}

/// Helper: normalize a record-type string into its keccak256 bytes32 (the whitelist / issuer key).
pub fn record_type_key(record_type: &str) -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(record_type.as_bytes());
    format!("0x{}", hex::encode(h.as_slice()))
}
