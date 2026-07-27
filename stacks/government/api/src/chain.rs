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

/// Sentinel `chain_id()` for a simulated backend. A `MemChain` is NOT on any real network, so it must
/// never answer with a real EIP-155 id — reporting `135` was the whole substance of the "simulated but
/// indistinguishable from live" bug: an operator reading `/health` saw `chainId:135` and reasonably
/// concluded the backend was ROAX. Callers surface `backend()`/`is_simulated()` and map this to `null`.
pub const SIMULATED_CHAIN_ID: u64 = 0;

/// Which chain surface a `ChainClient` actually talks to. This is deliberately a first-class part of
/// the trait rather than something inferred from a `demo` config flag: the operator-facing question
/// ("are these reads and writes REAL?") is a property of the client in use, not of how it was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainBackend {
    /// A real JSON-RPC node. Reads reflect chain state; writes cost gas and are irreversible.
    Live,
    /// In-process emulation. Nothing is broadcast, nothing is persisted beyond this process.
    Simulated,
}

impl ChainBackend {
    /// Stable machine-readable token for `/health` and the portal status strip.
    pub fn as_str(self) -> &'static str {
        match self {
            ChainBackend::Live => "live",
            ChainBackend::Simulated => "simulated",
        }
    }
    pub fn is_simulated(self) -> bool {
        matches!(self, ChainBackend::Simulated)
    }
}

sol! {
    #[sol(rpc)]
    contract IDogTagIssuer {
        event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
        event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);
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

    /// The unified owner-hidden verification registry. The government records a
    /// proof-of-verification here as any other VERIFIER does — same contract, same four-arg
    /// `recordVerificationZK`, same `VERIFY:<purpose>` gating. Mirror of the `sol!` block in
    /// `stacks/vet/api/src/chain.rs`; the ABI must stay byte-identical or the calldata drifts.
    #[sol(rpc)]
    contract IVerificationRegistryConsent {
        function recordVerificationZK(
            uint256[2] a,
            uint256[2][2] b,
            uint256[2] c,
            uint256[7] pub
        ) external;
        function consumed(bytes32 nf) external view returns (bool);
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

/// Result of a broadcast: the tx hash the caller records against the credential, plus the block it
/// mined into (the immutable on-chain proof surfaced back to the operator alongside the explorer link).
#[derive(Clone, Debug)]
pub struct SentTx {
    pub tx_hash: String,
    /// The block number the tx mined into (`None` when the receipt did not carry one; MemChain
    /// emulates a monotonic height).
    pub block_number: Option<u64>,
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// The EIP-155 chain id this client signs against (config-driven via `CHAIN_ID`; default 135).
    /// A simulated backend MUST return [`SIMULATED_CHAIN_ID`] rather than a real network id.
    fn chain_id(&self) -> u64 {
        ROAX_CHAIN_ID
    }
    /// Which chain surface this client actually talks to. Required (no default) so a new
    /// implementation cannot silently inherit "live" and misrepresent itself to operators.
    fn backend(&self) -> ChainBackend;
    /// `true` when this client is an in-process emulation rather than a real node.
    fn is_simulated(&self) -> bool {
        self.backend().is_simulated()
    }
    /// `true` when the client can sign+broadcast (a signer key is loaded). Reads work regardless.
    ///
    /// NOTE: on a simulated backend this reports the EMULATED capability — it is `true` while nothing
    /// is ever broadcast. Anything operator-facing must therefore pair it with [`Self::backend`];
    /// see `can_broadcast_real_tx` for the "will a real tx land on a real chain?" question.
    fn can_sign(&self) -> bool;
    /// `true` only when a real tx signed by a real key would actually land on a real chain. This is the
    /// predicate operator-facing surfaces want: it is `false` for every simulated backend.
    fn can_broadcast_real_tx(&self) -> bool {
        !self.is_simulated() && self.can_sign()
    }
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
    /// Sign+broadcast `revoke(root)` FROM the government signer to the `issuer_addr` clone (the
    /// on-chain soft-invalidation: the root stays issued-history but `isValid` flips to false). Awaits
    /// the receipt so a subsequent `is_valid` read reflects the revocation. Returns the tx hash.
    async fn revoke(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError>;
    /// `VerificationRegistryConsent.recordVerificationZK(a,b,c,pub)` FROM the government signer — the
    /// authority recording an owner-hidden proof-of-verification. The registry itself re-checks the
    /// Groth16 proof, `relayer == msg.sender`, the `VERIFY:<purpose>` whitelist, the
    /// `R == profileRoot(dogTagId)` binding and the nullifier, so this is a plain signed send.
    async fn record_verification_zk_consent(
        &self,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError>;
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
    let call = IDogTagIssuer::issueCall {
        r: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `revoke(bytes32)` calldata — the on-chain invalidation path (mirrors the vet stack).
pub fn revoke_calldata(root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuer::revokeCall {
        r: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// The block-explorer link for a tx hash, per the ROAX explorer scheme (`/tx/<hash>`).
pub fn explorer_tx_url(tx_hash: &str) -> String {
    format!("https://explorer.roax.net/tx/{tx_hash}")
}

/// Parse a proof scalar that may arrive as a decimal string (snarkjs) or `0x` hex. Unparseable input
/// folds to zero — the registry's own verifier rejects it, and panicking on a client-supplied field
/// would turn a bad request into a 500. Mirror of vet-api's `parse_u256_dec_or_hex`.
fn parse_u256_dec_or_hex(s: &str) -> U256 {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        U256::from_str_radix(h, 16).unwrap_or(U256::ZERO)
    } else {
        U256::from_str_radix(t, 10).unwrap_or(U256::ZERO)
    }
}

/// ABI-encode `recordVerificationZK(a,b,c,pub)` from decimal-or-hex proof components.
///
/// No `recordType`/`deadline` parameters: both are public signals inside `pub`, proof-bound. The
/// four-arg shape and the argument order are pinned by a test against the selector the vet stack
/// sends, so the two verifiers can never drift apart on the same registry.
pub fn record_verification_zk_consent_calldata(
    a: &[String; 2],
    b: &[[String; 2]; 2],
    c: &[String; 2],
    pub_signals: &[String; 7],
) -> String {
    use alloy::sol_types::SolCall;
    let g = |s: &str| parse_u256_dec_or_hex(s);
    let mut pub_arr: [U256; 7] = [U256::ZERO; 7];
    for (slot, s) in pub_arr.iter_mut().zip(pub_signals.iter()) {
        *slot = g(s);
    }
    let call = IVerificationRegistryConsent::recordVerificationZKCall {
        a: [g(&a[0]), g(&a[1])],
        b: [[g(&b[0][0]), g(&b[0][1])], [g(&b[1][0]), g(&b[1][1])]],
        c: [g(&c[0]), g(&c[1])],
        r#pub: pub_arr,
    };
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
        let bytes =
            hex::decode(s).map_err(|e| ChainError::Other(format!("bad signer key hex: {e}")))?;
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
    fn backend(&self) -> ChainBackend {
        ChainBackend::Live
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
        self.send_call(issuer_addr, &issue_calldata(root)).await
    }
    async fn revoke(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError> {
        self.send_call(issuer_addr, &revoke_calldata(root)).await
    }
    async fn record_verification_zk_consent(
        &self,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError> {
        let calldata = record_verification_zk_consent_calldata(a, b, c, pub_signals);
        self.send_call(registry_addr, &calldata).await
    }
}

impl AlloyChain {
    /// Sign+broadcast arbitrary calldata FROM the government signer to `to`, await the receipt, and
    /// return the tx hash + mined block number. Shared by `issue`/`revoke`.
    async fn send_call(&self, to: &str, calldata: &str) -> Result<SentTx, ChainError> {
        use alloy::network::{EthereumWallet, TransactionBuilder};
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;

        let signer = self.signer.clone().ok_or_else(|| {
            ChainError::Other("no government signer configured (set GOV_SIGNER_KEY)".into())
        })?;
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let data = Bytes::from(
            hex::decode(calldata.strip_prefix("0x").unwrap_or_default())
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
            .with_to(parse_addr(to))
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
        Ok(SentTx {
            tx_hash,
            block_number: receipt.block_number,
        })
    }
}

// --------------------------------------------------------------------------------------------
// MemChain — in-memory emulation for demo/local + tests (no live node).
// --------------------------------------------------------------------------------------------

/// Fixed base for the MemChain emulated block clock (2026-07-01T00:00:00Z). The demo has no real
/// node, so `issue()` derives its `issuedAt` timestamp from this monotonic clock; seeding it to a
/// realistic recent instant (rather than 0 = 1970) makes the derived "Date of issuance" on the
/// receipt + public status page read as a plausible present-day date in demo mode.
const MEMCHAIN_CLOCK_BASE: u64 = 1_782_864_000;

struct MemChainInner {
    /// (issuer_addr, root) -> issuedAt timestamp (0 == not issued).
    issued: HashMap<(String, String), u64>,
    revoked: HashMap<(String, String), u64>,
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    /// (registry_addr, nullifier) consumed by a recordVerificationZK — the emulated replay guard.
    consumed: HashMap<(String, String), bool>,
    nonce: u64,
    clock: u64,
}

impl Default for MemChainInner {
    fn default() -> Self {
        MemChainInner {
            issued: HashMap::new(),
            revoked: HashMap::new(),
            whitelist: HashMap::new(),
            consumed: HashMap::new(),
            nonce: 0,
            clock: MEMCHAIN_CLOCK_BASE,
        }
    }
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
    /// Has a nullifier already been consumed on the emulated registry? (test harness — the replay
    /// guard the real `VerificationRegistryConsent` enforces on-chain.)
    pub fn is_consumed(&self, registry: &str, nullifier: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .consumed
            .get(&(registry.to_lowercase(), nullifier.to_lowercase()))
            .copied()
            .unwrap_or(false)
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
    /// Never a real network id — see [`SIMULATED_CHAIN_ID`].
    fn chain_id(&self) -> u64 {
        SIMULATED_CHAIN_ID
    }
    fn backend(&self) -> ChainBackend {
        ChainBackend::Simulated
    }
    /// `true` because the emulation accepts `issue`/`revoke`; `can_broadcast_real_tx()` stays `false`.
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
        // Emulate a monotonic block height so the persisted on-chain proof carries a block number.
        let block_number = Some(1_000 + g.nonce);
        Ok(SentTx {
            tx_hash,
            block_number,
        })
    }
    async fn revoke(&self, issuer_addr: &str, root: &str) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        if g.issued.get(&key).copied().unwrap_or(0) == 0 {
            return Err(ChainError::Other("BadRoot: not issued".into()));
        }
        g.clock += 12;
        let ts = g.clock;
        g.revoked.insert(key, ts);
        g.nonce += 1;
        let tx_hash = format!("0x{:064x}", g.nonce);
        let block_number = Some(1_000 + g.nonce);
        Ok(SentTx {
            tx_hash,
            block_number,
        })
    }
    /// Emulates the registry's nullifier consume/replay guard only. The Groth16 verification itself has
    /// no in-memory equivalent, so MemChain accepts any well-formed proof: the ZK soundness of this
    /// path is exercised against the real contract (`contracts/test`), never here.
    async fn record_verification_zk_consent(
        &self,
        registry_addr: &str,
        _a: &[String; 2],
        _b: &[[String; 2]; 2],
        _c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError> {
        use dogtag_standard::public_signals::level_b as P;
        let nullifier = format!(
            "0x{}",
            hex::encode(parse_u256_dec_or_hex(&pub_signals[P::NULLIFIER]).to_be_bytes::<32>())
        );
        let mut g = self.inner.lock().unwrap();
        let key = (registry_addr.to_lowercase(), nullifier);
        if g.consumed.get(&key).copied().unwrap_or(false) {
            return Err(ChainError::Other("NullifierUsed".into()));
        }
        g.consumed.insert(key, true);
        g.clock += 12;
        g.nonce += 1;
        Ok(SentTx {
            tx_hash: format!("0x{:064x}", g.nonce),
            block_number: Some(1_000 + g.nonce),
        })
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
        assert!(tx.block_number.is_some(), "issue surfaces a block number");
        assert!(c.is_valid(issuer, root).await.unwrap());
        // double-issue rejected
        assert!(c.issue(issuer, root).await.is_err());
    }

    #[tokio::test]
    async fn memchain_revoke_flips_valid_but_keeps_history() {
        let c = MemChain::new();
        let issuer = "0x1111111111111111111111111111111111111111";
        let root = "0x3333333333333333333333333333333333333333333333333333333333333333";
        // can't revoke an unissued root.
        assert!(c.revoke(issuer, root).await.is_err());
        c.issue(issuer, root).await.unwrap();
        assert!(c.is_valid(issuer, root).await.unwrap());
        let tx = c.revoke(issuer, root).await.unwrap();
        assert!(tx.tx_hash.starts_with("0x"));
        assert!(tx.block_number.is_some());
        // isValid flips to false, but issuedAt (the historical anchor) stays intact.
        assert!(!c.is_valid(issuer, root).await.unwrap());
        assert!(!c.issued_at(issuer, root).await.unwrap().is_zero());
    }

    /// The selector the vet/groomer stacks send for the same registry call. Pinned literally: if the
    /// `sol!` ABI here ever drifts from theirs, the government's verifications would revert on the
    /// shared `VerificationRegistryConsent` and this test fails first.
    #[test]
    fn record_verification_zk_selector_matches_the_shared_registry_abi() {
        let a = ["1".to_string(), "2".to_string()];
        let b = [
            ["3".to_string(), "4".to_string()],
            ["5".to_string(), "6".to_string()],
        ];
        let c = ["7".to_string(), "8".to_string()];
        let pubs: [String; 7] = std::array::from_fn(|i| i.to_string());
        let cd = record_verification_zk_consent_calldata(&a, &b, &c, &pubs);
        // keccak("recordVerificationZK(uint256[2],uint256[2][2],uint256[2],uint256[7])")[0..4]
        assert_eq!(&cd[2..10], "dd080593", "calldata: {cd}");
        // 4-byte selector + 8 static proof words + 7 public signals, all inline (no dynamic tails).
        assert_eq!(cd.len(), 2 + 8 + (8 + 7) * 64, "calldata: {cd}");
    }

    #[tokio::test]
    async fn memchain_consent_record_consumes_the_nullifier_once() {
        let c = MemChain::new();
        let registry = "0x4444444444444444444444444444444444444444";
        let a = ["1".to_string(), "2".to_string()];
        let b = [
            ["3".to_string(), "4".to_string()],
            ["5".to_string(), "6".to_string()],
        ];
        let cc = ["7".to_string(), "8".to_string()];
        // pub[3] is the nullifier (frozen output order).
        let pubs: [String; 7] = std::array::from_fn(|i| if i == 3 { "99".into() } else { "1".into() });
        assert!(!c.is_consumed(registry, &format!("0x{:064x}", 99)));
        let tx = c
            .record_verification_zk_consent(registry, &a, &b, &cc, &pubs)
            .await
            .unwrap();
        assert!(tx.tx_hash.starts_with("0x"));
        assert!(c.is_consumed(registry, &format!("0x{:064x}", 99)));
        // replay of the same nullifier is refused, as the registry refuses it on-chain.
        assert!(c
            .record_verification_zk_consent(registry, &a, &b, &cc, &pubs)
            .await
            .is_err());
    }

    #[test]
    fn memchain_never_claims_a_real_chain_id_or_real_signing() {
        // The bug this pins: MemChain used the trait-default chain_id() (135) and can_sign()==true, so
        // /health was indistinguishable from a live ROAX backend with a funded signer.
        let c = MemChain::new();
        assert_eq!(c.chain_id(), SIMULATED_CHAIN_ID);
        assert_ne!(c.chain_id(), ROAX_CHAIN_ID);
        assert_eq!(c.backend(), ChainBackend::Simulated);
        assert!(c.is_simulated());
        // It emulates signing, but it can never broadcast a real tx.
        assert!(c.can_sign());
        assert!(!c.can_broadcast_real_tx());
    }

    #[test]
    fn alloychain_reports_live_and_gates_real_broadcast_on_a_signer() {
        let live = AlloyChain::new("https://devrpc.roax.net".to_string()).with_chain_id(ROAX_CHAIN_ID);
        assert_eq!(live.backend(), ChainBackend::Live);
        assert!(!live.is_simulated());
        assert_eq!(live.chain_id(), ROAX_CHAIN_ID);
        // No signer loaded -> reads only, and it must not claim real broadcast capability.
        assert!(!live.can_sign());
        assert!(!live.can_broadcast_real_tx());

        // With a signer loaded it can broadcast for real (test-only key, not a funded account).
        let signed = AlloyChain::new("https://devrpc.roax.net".to_string())
            .with_signer_hex("0x1111111111111111111111111111111111111111111111111111111111111111")
            .expect("valid 32-byte key");
        assert!(signed.can_sign());
        assert!(signed.can_broadcast_real_tx());
    }

    #[test]
    fn explorer_url_uses_roax_scheme() {
        assert_eq!(
            explorer_tx_url("0xabc"),
            "https://explorer.roax.net/tx/0xabc"
        );
    }
}
