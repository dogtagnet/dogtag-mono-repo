//! `ChainClient` trait abstracting the ROAX (chainId 135) on-chain surface the CENTRAL/admin backend
//! needs: the `IssuerRegistry` whitelist (`whitelistFor` / `delistFor` / `isWhitelistedFor`) written by
//! the WHITELIST_ADMIN signer, plus `DogTagSBT.mint(to,dogTagId,root)` written by the ISSUER signer.
//! An Alloy-backed implementation broadcasts real transactions; an in-memory `MemChain` emulates the
//! whitelist set + the SBT mint/ownerOf so the full HTTP flow is testable without a live node.
//!
//! Signing (impl §1.8): EIP-1559 with a legacy `gas_price` fallback; chainId pinned to 135.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, Bytes, FixedBytes, B256, U256};
use alloy::sol;
use async_trait::async_trait;

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IIssuerRegistry {
        function whitelistFor(bytes32 recordType, address signer) external;
        function delistFor(bytes32 recordType, address signer) external;
        function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
    }

    #[sol(rpc)]
    contract IDogTagSBT {
        function mint(address to, uint256 id, bytes32 root) external;
        function ownerOf(uint256 id) external view returns (address);
        // AccessControl surface — the DEFAULT_ADMIN holder grants ISSUER_ROLE so a vet can mint.
        function grantRole(bytes32 role, address account) external;
        function hasRole(bytes32 role, address account) external view returns (bool);
    }
}

/// `DogTagSBT.ISSUER_ROLE = keccak256("ISSUER")` — the role that gates `mint`.
pub fn issuer_role_key() -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(b"ISSUER");
    format!("0x{}", hex::encode(h.as_slice()))
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

/// Result of a broadcast: the tx hash.
#[derive(Clone, Debug)]
pub struct SentTx {
    pub tx_hash: String,
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

fn parse_u256_dec_or_hex(s: &str) -> U256 {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        U256::from_str_radix(h, 16).unwrap_or(U256::ZERO)
    } else {
        U256::from_str_radix(t, 10).unwrap_or(U256::ZERO)
    }
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Register the admin signer (32-byte secp256k1 private key) for an account index, with its
    /// derived address. The Alloy impl keeps the key for broadcasting; MemChain keeps only the address.
    async fn register_signer(&self, index: u32, private_key: [u8; 32], address: String);

    /// IssuerRegistry.whitelistFor(recordType, signer) — admin-only write. `record_type` is the
    /// keccak256 bytes32 key (NOT the human label).
    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError>;

    /// IssuerRegistry.delistFor(recordType, signer) — admin-only write.
    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError>;

    /// IssuerRegistry.isWhitelistedFor(recordType, signer).
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError>;

    /// DogTagSBT.mint(to, dogTagId, root) — the admin holds the ISSUER role.
    async fn mint(
        &self,
        account_index: u32,
        sbt_addr: &str,
        to: &str,
        dog_tag_id: &str,
        root: &str,
    ) -> Result<SentTx, ChainError>;

    /// DogTagSBT.ownerOf(dogTagId) (lowercase 0x.. address; Err(NotFound) if unminted).
    async fn owner_of(&self, sbt_addr: &str, dog_tag_id: &str) -> Result<String, ChainError>;

    /// DogTagSBT.grantRole(ISSUER_ROLE, grantee) — broadcast by the admin signer (which holds
    /// DEFAULT_ADMIN_ROLE), granting the mint capability so `grantee` can call `DogTagSBT.mint`.
    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError>;

    /// DogTagSBT.hasRole(ISSUER_ROLE, account) — read so approve can skip an already-granted role.
    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError>;
}

// --------------------------------------------------------------------------------------------
// MemChain — in-memory emulation of the whitelist set + SBT mint/ownerOf.
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemChainInner {
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    /// (sbt_addr, dog_tag_id) -> owner address.
    owners: HashMap<(String, String), String>,
    /// (sbt_addr, account) holding DogTagSBT.ISSUER_ROLE.
    issuer_roles: std::collections::HashSet<(String, String)>,
    /// admin signer addresses by account index.
    signers: HashMap<u32, String>,
    nonce: u64,
}

#[derive(Clone, Default)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register an admin signer address for an account index (test harness wires this from custody).
    pub fn set_signer(&self, index: u32, address: &str) {
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }
    fn next_tx(g: &mut MemChainInner) -> String {
        g.nonce += 1;
        format!("0x{:064x}", g.nonce)
    }
}

#[async_trait]
impl ChainClient for MemChain {
    async fn register_signer(&self, index: u32, _private_key: [u8; 32], address: String) {
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }

    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        // emulate onlyRole(WHITELIST_ADMIN): require a registered admin signer at this index.
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.whitelist.insert(
            (
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ),
            true,
        );
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.whitelist.insert(
            (
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ),
            false,
        );
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
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

    async fn mint(
        &self,
        account_index: u32,
        sbt_addr: &str,
        to: &str,
        dog_tag_id: &str,
        _root: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no issuer signer for index".into()))?;
        let key = (sbt_addr.to_lowercase(), normalize_id(dog_tag_id));
        if g.owners.contains_key(&key) {
            return Err(ChainError::Other("ERC721: token already minted".into()));
        }
        g.owners.insert(key, to.to_lowercase());
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn owner_of(&self, sbt_addr: &str, dog_tag_id: &str) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        g.owners
            .get(&(sbt_addr.to_lowercase(), normalize_id(dog_tag_id)))
            .cloned()
            .ok_or(ChainError::NotFound)
    }

    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        // emulate onlyRole(DEFAULT_ADMIN_ROLE): require a registered admin signer at this index.
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.issuer_roles
            .insert((sbt_addr.to_lowercase(), grantee.to_lowercase()));
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.issuer_roles
            .contains(&(sbt_addr.to_lowercase(), account.to_lowercase())))
    }
}

/// Normalize a dogTagId (decimal or hex) into a canonical decimal string so MemChain keys collide
/// regardless of input radix.
fn normalize_id(dog_tag_id: &str) -> String {
    parse_u256_dec_or_hex(dog_tag_id).to_string()
}

// --------------------------------------------------------------------------------------------
// Calldata encoders (canonical typed ABI).
// --------------------------------------------------------------------------------------------

pub fn whitelist_for_calldata(record_type: &str, signer: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IIssuerRegistry::whitelistForCall {
        recordType: parse_b256(record_type),
        signer: parse_addr(signer),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

pub fn delist_for_calldata(record_type: &str, signer: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IIssuerRegistry::delistForCall {
        recordType: parse_b256(record_type),
        signer: parse_addr(signer),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

pub fn grant_issuer_role_calldata(grantee: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::grantRoleCall {
        role: parse_b256(&issuer_role_key()),
        account: parse_addr(grantee),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

pub fn mint_calldata(to: &str, dog_tag_id: &str, root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::mintCall {
        to: parse_addr(to),
        id: parse_u256_dec_or_hex(dog_tag_id),
        root: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

// --------------------------------------------------------------------------------------------
// AlloyChain — real ROAX/anvil-backed client using a derived signer set.
// --------------------------------------------------------------------------------------------

pub struct AlloyChain {
    pub rpc_url: String,
    /// EIP-155 chain id used when signing legacy txs (default `ROAX_CHAIN_ID`; overridable via `CHAIN_ID`).
    pub chain_id: u64,
    /// account index -> alloy local signer (registered at unlock time).
    signers: Mutex<HashMap<u32, alloy::signers::local::PrivateKeySigner>>,
}

impl AlloyChain {
    pub fn new(rpc_url: String) -> Self {
        AlloyChain {
            rpc_url,
            chain_id: ROAX_CHAIN_ID,
            signers: Mutex::new(HashMap::new()),
        }
    }
    /// Override the EIP-155 chain id (config-only chain swap; default stays `ROAX_CHAIN_ID` = 135).
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }
    fn signer(&self, index: u32) -> Option<alloy::signers::local::PrivateKeySigner> {
        self.signers.lock().unwrap().get(&index).cloned()
    }

    /// Sign+broadcast a tx FROM the signer at `account_index` to `to` with `calldata`. EIP-1559 with
    /// a legacy gas_price fallback; chainId pinned to 135.
    async fn sign_and_send(
        &self,
        account_index: u32,
        to: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        use alloy::network::EthereumWallet;
        use alloy::network::TransactionBuilder;
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;

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
        // LEGACY pricing on ROAX: the node's base fee is ~7 wei but its mempool only mines txs at the
        // ~1 gwei eth_gasPrice. Alloy's EIP-1559 filler derives maxFeePerGas from the (tiny) base fee,
        // producing an underpriced tx that the node ACCEPTS but never mines (stuck forever). Read
        // eth_gasPrice and send a legacy tx (mirrors the working `cast send --legacy`).
        let gp = provider
            .get_gas_price()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let tx = TransactionRequest::default()
            .with_to(parse_addr(to))
            .with_input(data)
            .with_value(U256::ZERO)
            .with_chain_id(self.chain_id)
            .with_gas_price(gp);

        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // wait for the tx to be mined so on-chain reads (isWhitelistedFor/ownerOf) reflect it.
        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        if !receipt.status() {
            return Err(ChainError::Other("tx reverted".into()));
        }
        let tx_hash = format!("{:#x}", receipt.transaction_hash);
        Ok(SentTx { tx_hash })
    }
}

#[async_trait]
impl ChainClient for AlloyChain {
    async fn register_signer(&self, index: u32, private_key: [u8; 32], _address: String) {
        if let Ok(s) = alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from(private_key))
        {
            self.signers.lock().unwrap().insert(index, s);
        }
    }

    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = whitelist_for_calldata(record_type, signer);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }

    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = delist_for_calldata(record_type, signer);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
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

    async fn mint(
        &self,
        account_index: u32,
        sbt_addr: &str,
        to: &str,
        dog_tag_id: &str,
        root: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = mint_calldata(to, dog_tag_id, root);
        self.sign_and_send(account_index, sbt_addr, &calldata).await
    }

    async fn owner_of(&self, sbt_addr: &str, dog_tag_id: &str) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagSBT::new(parse_addr(sbt_addr), provider);
        match c.ownerOf(parse_u256_dec_or_hex(dog_tag_id)).call().await {
            Ok(r) => Ok(format!("{:#x}", r._0)),
            Err(e) => {
                let s = e.to_string();
                if s.contains("nonexistent") || s.contains("ERC721") || s.contains("revert") {
                    Err(ChainError::NotFound)
                } else {
                    Err(ChainError::Rpc(s))
                }
            }
        }
    }

    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = grant_issuer_role_calldata(grantee);
        self.sign_and_send(account_index, sbt_addr, &calldata).await
    }

    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagSBT::new(parse_addr(sbt_addr), provider);
        let r = c
            .hasRole(parse_b256(&issuer_role_key()), parse_addr(account))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
}

/// Helper: normalize a record-type string into its keccak256 bytes32 (the whitelist / issuer key).
pub fn record_type_key(record_type: &str) -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(record_type.as_bytes());
    format!("0x{}", hex::encode(h.as_slice()))
}

/// The purpose label reduced to the registry's bytes32 `purpose` field: keccak256(label) reduced mod
/// the BN254 scalar field r (a field element, distinct from recordType). MUST byte-match the vet
/// stack's `verify::purpose_key` and the on-chain `_verifyKey` input. (Mirrors stacks/vet/api verify.rs.)
pub fn purpose_key(label: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    // BN254 r.
    let r = U256::from_str_radix(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
        10,
    )
    .unwrap();
    let full = U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0);
    let reduced = full % r;
    format!("0x{}", hex::encode(reduced.to_be_bytes::<32>()))
}

/// The IssuerRegistry whitelist key the VerificationRegistry checks for the relayer on a given purpose:
/// `keccak256(abi.encode("VERIFY:", purpose))` where `purpose` is the bytes32 from `purpose_key(label)`
/// (Solidity `abi.encode(string,bytes32)` = head[offset=0x40] ++ purpose ++ len(7) ++ "VERIFY:" padded).
/// MUST byte-match the on-chain `VerificationRegistry._verifyKey` + the vet stack's `verify::verify_key`.
/// (Mirrors stacks/vet/api verify.rs ~47-68.)
pub fn verify_key(label: &str) -> String {
    use alloy::primitives::keccak256;
    let purpose_hex = purpose_key(label);
    let purpose = hex::decode(purpose_hex.trim_start_matches("0x")).unwrap_or_default();
    // abi.encode(string "VERIFY:", bytes32 purpose)
    let mut buf = Vec::with_capacity(160);
    // [0] offset to the string data = 0x40 (after the two head words).
    let mut off = [0u8; 32];
    off[31] = 0x40;
    buf.extend_from_slice(&off);
    // [1] the bytes32 purpose word.
    buf.extend_from_slice(&purpose);
    // [2] string length = 7 ("VERIFY:").
    let mut len = [0u8; 32];
    len[31] = 7;
    buf.extend_from_slice(&len);
    // [3] string bytes, right-padded to 32.
    let mut data = [0u8; 32];
    data[..7].copy_from_slice(b"VERIFY:");
    buf.extend_from_slice(&data);
    format!("0x{}", hex::encode(keccak256(&buf).as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `verify_key` must byte-match the on-chain `_verifyKey` + the demo-bootstrap value the vet stack
    /// produces for "boarding_intake" — the verifier-onboarding whitelist parity guard (plan A3).
    #[test]
    fn verify_key_parity_boarding_intake() {
        assert_eq!(
            verify_key("boarding_intake"),
            "0x9f894293e0cbaa46eca3cc026ad45e5012c10c4d3217ede0488ca0d2b5eaf764"
        );
    }
}
