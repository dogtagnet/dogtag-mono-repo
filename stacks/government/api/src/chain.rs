//! `ChainClient` — the ROAX (chainId 135) on-chain surface the government authority backend needs:
//! read a credential root's issuance/revocation status (`DogTagIssuer.isValid`), read the issuer
//! whitelist (`IssuerRegistry.isWhitelistedFor`), and (for issuance) sign+broadcast an
//! `issue(bytes32)` anchoring tx from the government signer.
//!
//! Two implementations mirror the vet stack so the full HTTP flow is testable without a live node:
//!   - `AlloyChain` — real ROAX RPC + legacy-priced signing (the node mines at `eth_gasPrice`).
//!   - `MemChain`   — in-memory emulation of issue / isValid / isWhitelistedFor.
//!
//! This is a deliberately trimmed copy of `stacks/vet/api/src/chain.rs` (the government authority
//! only anchors/verifies roots — it has no SBT mint, consent-key, or ZK-verification surface). A
//! future refactor could extract a shared `crates/dogtag-chain-rs`; see `docs/ROLE_APPS.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, Bytes, B256, U256};
use alloy::sol;
use async_trait::async_trait;

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IDogTagIssuer {
        event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
        function issue(bytes32 r) external;
        function revoke(bytes32 r) external;
        function isValid(bytes32 r) external view returns (bool);
        function isRevoked(bytes32 r) external view returns (bool);
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

/// Result of a broadcast: the tx hash the caller records against the credential.
#[derive(Clone, Debug)]
pub struct SentTx {
    pub tx_hash: String,
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// The EIP-155 chain id this client signs against (config-driven via `CHAIN_ID`; default 135).
    fn chain_id(&self) -> u64 {
        ROAX_CHAIN_ID
    }
    /// `true` when the client can sign+broadcast (a signer key is loaded). Reads work regardless.
    fn can_sign(&self) -> bool;
    /// The government signer's `0x..` address, if a signer is loaded.
    fn signer_address(&self) -> Option<String>;
    /// `DogTagIssuer.isValid(root)` — issued && !revoked.
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError>;
    /// `DogTagIssuer.issuedAt(root)` (0 == not issued).
    async fn issued_at(&self, issuer_addr: &str, root: &str) -> Result<U256, ChainError>;
    /// `IssuerRegistry.isWhitelistedFor(recordType, signer)`.
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError>;
    /// Sign+broadcast `issue(root)` FROM the government signer to the `issuer_addr` clone. Awaits the
    /// receipt so a subsequent `is_valid` read reflects the anchor. Returns the tx hash.
    async fn issue(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError>;
}

// --------------------------------------------------------------------------------------------
// calldata
// --------------------------------------------------------------------------------------------

fn parse_b256(h: &str) -> B256 {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let mut buf = [0u8; 32];
    if let Ok(bytes) = hex::decode(s) {
        let n = bytes.len().min(32);
        // right-align (a short value like "0x00" becomes 0x000..00).
        buf[32 - n..].copy_from_slice(&bytes[bytes.len() - n..]);
    }
    B256::from(buf)
}

fn parse_addr(h: &str) -> Address {
    h.parse::<Address>().unwrap_or(Address::ZERO)
}

/// `issue(bytes32)` calldata (selector 0f75e81f) — pinned by the parity test.
pub fn issue_calldata(root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuer::issueCall { r: parse_b256(root) };
    format!("0x{}", hex::encode(call.abi_encode()))
}

// --------------------------------------------------------------------------------------------
// AlloyChain — real ROAX RPC.
// --------------------------------------------------------------------------------------------

pub struct AlloyChain {
    pub rpc_url: String,
    pub chain_id: u64,
    signer: Option<alloy::signers::local::PrivateKeySigner>,
}

impl AlloyChain {
    pub fn new(rpc_url: String) -> Self {
        AlloyChain {
            rpc_url,
            chain_id: ROAX_CHAIN_ID,
            signer: None,
        }
    }
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }
    /// Load the government signer from a 32-byte secp256k1 private key (0x-optional hex). A malformed
    /// key is a hard config error (the whole point of this signer is to anchor real txs).
    pub fn with_signer_hex(mut self, key_hex: &str) -> Result<Self, ChainError> {
        let s = key_hex.strip_prefix("0x").unwrap_or(key_hex);
        let bytes = hex::decode(s).map_err(|e| ChainError::Other(format!("bad signer key hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(ChainError::Other(format!(
                "signer key must be 32 bytes (got {})",
                bytes.len()
            )));
        }
        let signer = alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from_slice(&bytes))
            .map_err(|e| ChainError::Other(format!("bad signer key: {e}")))?;
        self.signer = Some(signer);
        Ok(self)
    }
}

#[async_trait]
impl ChainClient for AlloyChain {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }
    fn can_sign(&self) -> bool {
        self.signer.is_some()
    }
    fn signer_address(&self) -> Option<String> {
        self.signer.as_ref().map(|s| format!("{:#x}", s.address()))
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
    async fn issue(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError> {
        use alloy::network::{EthereumWallet, TransactionBuilder};
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;

        let signer = self
            .signer
            .clone()
            .ok_or_else(|| ChainError::Other("no government signer configured (set GOV_SIGNER_KEY)".into()))?;
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let data = Bytes::from(
            hex::decode(issue_calldata(root).strip_prefix("0x").unwrap_or_default())
                .map_err(|e| ChainError::Other(format!("bad calldata: {e}")))?,
        );
        // LEGACY pricing on ROAX (mirrors vet-api chain.rs): the node's base fee is ~7 wei but its
        // mempool only mines at the ~1 gwei eth_gasPrice, so an EIP-1559 tx derived from the base fee
        // is accepted-but-never-mined. Read eth_gasPrice and send a legacy tx.
        let gp = provider
            .get_gas_price()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let tx = TransactionRequest::default()
            .with_to(parse_addr(issuer_addr))
            .with_input(data)
            .with_value(U256::ZERO)
            .with_chain_id(self.chain_id)
            .with_gas_price(gp);

        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let tx_hash = format!("{:#x}", receipt.transaction_hash);
        if !receipt.status() {
            return Err(ChainError::Other(format!("tx reverted: {tx_hash}")));
        }
        Ok(SentTx { tx_hash })
    }
}

// --------------------------------------------------------------------------------------------
// MemChain — in-memory emulation for demo/local + tests (no live node).
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemChainInner {
    /// (issuer_addr, root) -> issuedAt timestamp (0 == not issued).
    issued: HashMap<(String, String), u64>,
    revoked: HashMap<(String, String), u64>,
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    nonce: u64,
    clock: u64,
}

#[derive(Clone)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
    signer: String,
}

impl Default for MemChain {
    fn default() -> Self {
        // A deterministic demo signer address so `signer_address()`/whitelist wiring is stable.
        MemChain {
            inner: Arc::new(Mutex::new(MemChainInner::default())),
            signer: "0x0000000000000000000000000000000000600760".to_lowercase(),
        }
    }
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Override the emulated signer address (test harness).
    pub fn with_signer(mut self, addr: &str) -> Self {
        self.signer = addr.to_lowercase();
        self
    }
    /// Whitelist a signer for a (registry, recordType, signer) tuple (test harness / demo bootstrap).
    pub fn whitelist(&self, registry: &str, record_type: &str, signer: &str) {
        self.inner.lock().unwrap().whitelist.insert(
            (
                registry.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ),
            true,
        );
    }
}

#[async_trait]
impl ChainClient for MemChain {
    fn can_sign(&self) -> bool {
        true
    }
    fn signer_address(&self) -> Option<String> {
        Some(self.signer.clone())
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
        let v = g
            .issued
            .get(&(issuer_addr.to_lowercase(), root.to_lowercase()))
            .copied()
            .unwrap_or(0);
        Ok(U256::from(v))
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.whitelist
            .get(&(
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ))
            .copied()
            .unwrap_or(false))
    }
    async fn issue(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        if g.issued.get(&key).copied().unwrap_or(0) != 0 {
            return Err(ChainError::Other("BadRoot: already issued".into()));
        }
        g.clock += 12;
        let ts = g.clock;
        g.issued.insert(key, ts);
        g.nonce += 1;
        let tx_hash = format!("0x{:064x}", g.nonce);
        Ok(SentTx { tx_hash })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_calldata_selector_is_pinned() {
        // issue(bytes32) canonical selector (matches vet-api's parity test).
        let cd = issue_calldata("0x00");
        assert_eq!(&cd[2..10], "0f75e81f");
    }

    #[tokio::test]
    async fn memchain_issue_then_valid() {
        let c = MemChain::new();
        let issuer = "0x1111111111111111111111111111111111111111";
        let root = "0x2222222222222222222222222222222222222222222222222222222222222222";
        assert!(!c.is_valid(issuer, root).await.unwrap());
        let tx = c.issue(issuer, root).await.unwrap();
        assert!(tx.tx_hash.starts_with("0x"));
        assert!(c.is_valid(issuer, root).await.unwrap());
        // double-issue rejected
        assert!(c.issue(issuer, root).await.is_err());
    }
}
