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

// Owner-hidden consent public-signal indices.
use dogtag_standard::public_signals::level_b as PB;

pub const ROAX_CHAIN_ID: u64 = 135;

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

    #[sol(rpc)]
    contract IDogTagSBT {
        function mintCustodial(uint256 id, bytes32 root) external;
        function profileRoot(uint256 id) external view returns (bytes32);
    }

    /// Owner-hidden verification registry. The event and calldata contain no owner/subject address.
    #[sol(rpc)]
    contract IVerificationRegistryConsent {
        event Verified(
            uint256 indexed dogTagId,
            address indexed relayer,
            bytes32 purpose,
            bytes32 nullifier,
            uint256 deadline,
            uint256 ts
        );

        function recordVerificationZK(
            uint256[2] a,
            uint256[2][2] b,
            uint256[2] c,
            uint256[7] pub
        ) external;
        function consumed(bytes32 nf) external view returns (bool);
    }

}

/// An owner-hidden `Verified` event read from a `VerificationRegistryConsent` receipt.
#[derive(Clone, Debug)]
pub struct ConsentVerifiedEvent {
    pub dog_tag_id: U256,
    pub relayer: String,
    pub purpose: String,
    pub nullifier: String,
    pub deadline: U256,
    pub ts: U256,
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
    /// The block number the tx mined into (the immutable on-chain proof stored against the record).
    pub block_number: Option<u64>,
    /// RootIssued logs emitted by `issuer_addr` in this tx: (root_hex, by_addr).
    pub root_issued_logs: Vec<(String, String)>,
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// The EIP-155 chain id this client signs/validates against (config-driven via `CHAIN_ID`;
    /// default `ROAX_CHAIN_ID` = 135). Used to stamp the wallet-mode `unsignedTx.chainId` and to
    /// bind the confirm path's `tx.chainId` check — so a chain swap stays config-only.
    fn chain_id(&self) -> u64 {
        ROAX_CHAIN_ID
    }
    /// Register the backend signer (32-byte secp256k1 private key) for an account index, with its
    /// derived address. Called by the unlock handler after custody decrypts the seed. The Alloy impl
    /// keeps the key for broadcasting; MemChain keeps only the address.
    async fn register_signer(&self, index: u32, private_key: [u8; 32], address: String);
    /// DogTagIssuer.isValid(root).
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError>;
    /// DogTagIssuer.isRevoked(root).
    async fn is_revoked(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError>;
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
    /// Broadcast the owner-hidden 4-arg `recordVerificationZK(a,b,c,pub)` to a
    /// `VerificationRegistryConsent` at `registry_addr`, FROM the backend signer at `account_index`.
    ///
    /// The relayer must be the broadcaster: the contract requires
    /// `address(uint160(pub[level_b::RELAYER])) == msg.sender`, and `relayer` is bound into both the
    /// signed consent message and the nullifier — so the owner consents to ONE submitter and no other
    /// address can carry this proof.
    async fn record_verification_zk_consent(
        &self,
        account_index: u32,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError>;
    /// Read the owner-hidden `Verified(dogTagId,relayer,purpose,nullifier,deadline,ts)` event
    /// emitted by `registry_addr` in the given tx's receipt. Err(NotFound) if absent/unmined.
    async fn get_consent_verified_event(
        &self,
        tx_hash: &str,
        registry_addr: &str,
    ) -> Result<ConsentVerifiedEvent, ChainError>;
    /// VerificationRegistry.consumed(nullifier).
    async fn consumed(&self, registry_addr: &str, nullifier: &str) -> Result<bool, ChainError>;
    /// `DogTagIssuer.issue(R)` on `issuer_addr` FROM the signer at `account_index` (which must be
    /// whitelisted for the clone's recordType). Anchors `R` so `rootIssuer[R]` resolves to this clone.
    ///
    /// On its own this does not mint anything; paired with
    /// [`ChainClient::mint_custodial`] it satisfies the second of the two independent on-chain
    /// conditions an owner-hidden verify checks (`VerificationRegistryConsent.sol:188-192`).
    async fn issue(
        &self,
        account_index: u32,
        issuer_addr: &str,
        root: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = issue_calldata(root);
        self.sign_and_send(account_index, issuer_addr, &calldata)
            .await
    }
    /// `DogTagSBTConsent.mintCustodial(dogTagId, R)` FROM the signer at `account_index` (must hold
    /// ISSUER_ROLE on the owner-hidden SBT). Seals `R` as `profileRoot[dogTagId]`, write-once.
    ///
    /// **Call [`ChainClient::issue`] FIRST.** The mint is irreversible and the id is single-use
    /// forever (`profileRoot[id] != 0` rejects any re-mint, even after a burn), so minting before the
    /// root is anchored risks a permanently bricked tag that reverts `unknown root` on every verify -
    /// the failure mode pinned by `contracts/test/CustodialIssuance.t.sol:367`. The contract states
    /// the ordering itself (`DogTagSBTConsent.sol:139-143`): "Issue the root, then mint."
    ///
    /// There is deliberately no recipient parameter - see the `sol!` note on `mintCustodial`.
    async fn mint_custodial(
        &self,
        account_index: u32,
        sbt_addr: &str,
        dog_tag_id: &str,
        root: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = mint_custodial_calldata(dog_tag_id, root);
        self.sign_and_send(account_index, sbt_addr, &calldata).await
    }
    /// DogTagSBT.profileRoot(dogTagId) (0x.. bytes32 hex; 0x0..0 if unminted).
    async fn profile_root_of(
        &self,
        _sbt_addr: &str,
        _dog_tag_id: &str,
    ) -> Result<String, ChainError> {
        Err(ChainError::NotFound)
    }
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
    let call = IDogTagIssuer::issueCall {
        r: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}
pub fn revoke_calldata(root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuer::revokeCall {
        r: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}
/// The block-explorer link for a tx hash, per the ROAX explorer scheme (`/tx/<hash>`). Persisted
/// against a record so the operator has a ready-to-click on-chain proof link.
pub fn explorer_tx_url(tx_hash: &str) -> String {
    format!("https://explorer.roax.net/tx/{tx_hash}")
}
/// ABI-encode the owner-blind `DogTagSBTConsent.mintCustodial(dogTagId, root)`.
///
/// `dog_tag_id` MUST already be the CANONICAL field `field_of_value(Integer(handle))`, never the raw
/// operator handle: the same field is the KDF binding input the device folded into `R`, so a raw
/// handle here yields `R != profileRoot(id)` and every verify fails closed
/// (`crates/dogtag-standard-rs/src/profile_tree.rs` fixture warning). Callers go through
/// [`crate::routes::onchain_dog_tag_id`].
pub fn mint_custodial_calldata(dog_tag_id: &str, root: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::mintCustodialCall {
        id: parse_u256_dec_or_hex(dog_tag_id),
        root: parse_b256(root),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}
/// Normalize a dogTagId (decimal or hex) into a canonical decimal string so MemChain keys collide
/// regardless of input radix.
fn normalize_id(dog_tag_id: &str) -> String {
    parse_u256_dec_or_hex(dog_tag_id).to_string()
}

fn parse_u256_dec_or_hex(s: &str) -> U256 {
    let t = s.trim();
    if let Some(h) = t.strip_prefix("0x") {
        U256::from_str_radix(h, 16).unwrap_or(U256::ZERO)
    } else {
        U256::from_str_radix(t, 10).unwrap_or(U256::ZERO)
    }
}

/// ABI-encode `recordVerificationZK(a,b,c,pub)` — four args, from decimal-or-hex string
/// proof components.
///
/// No `recordType`/`deadline` parameters: both are public
/// signals (`pub[5]`/`pub[6]`) bound to the proof, so the relayer cannot choose or widen them. A
/// relayer that wants a longer freshness window cannot get one by re-encoding — it has to ask the
/// device for a fresh proof.
pub fn record_verification_zk_consent_calldata(
    a: &[String; 2],
    b: &[[String; 2]; 2],
    c: &[String; 2],
    pub_signals: &[String; 7],
) -> String {
    use alloy::sol_types::SolCall;
    let g = |s: &str| parse_u256_dec_or_hex(s);
    let a_arr = [g(&a[0]), g(&a[1])];
    let b_arr = [[g(&b[0][0]), g(&b[0][1])], [g(&b[1][0]), g(&b[1][1])]];
    let c_arr = [g(&c[0]), g(&c[1])];
    let mut pub_arr: [U256; 7] = [U256::ZERO; 7];
    for (slot, s) in pub_arr.iter_mut().zip(pub_signals.iter()) {
        *slot = g(s);
    }
    let call = IVerificationRegistryConsent::recordVerificationZKCall {
        a: a_arr,
        b: b_arr,
        c: c_arr,
        r#pub: pub_arr,
    };
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
    /// (registry_addr, nullifier) consumed by a recordVerification(ZK).
    consumed: HashMap<(String, String), bool>,
    /// txHash -> owner-hidden Verified event emitted by the consent registry.
    consent_verified: HashMap<String, ConsentVerifiedEvent>,
    /// (sbt_addr, dog_tag_id) -> profileRoot (DogTagSBT.profileRoot).
    sbt_roots: HashMap<(String, String), String>,
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
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }
    /// Whitelist a signer for a (registry, recordType, signer) tuple.
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
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }
    async fn is_valid(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        let issued = g.issued.get(&key).copied().unwrap_or(0) != 0;
        let revoked = g.revoked.get(&key).copied().unwrap_or(0) != 0;
        Ok(issued && !revoked)
    }
    async fn is_revoked(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        Ok(g.revoked.get(&key).copied().unwrap_or(0) != 0)
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
            // Emulate a monotonic block height so the persisted on-chain proof carries a block number.
            block_number: Some(1_000 + g.nonce),
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
    async fn record_verification_zk_consent(
        &self,
        account_index: u32,
        registry_addr: &str,
        _a: &[String; 2],
        _b: &[[String; 2]; 2],
        _c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError> {
        // The nullifier is pub[3]; pub[4] is the profile root.
        let nf = format!(
            "0x{}",
            hex::encode(parse_b256_dec_or_hex(&pub_signals[PB::NULLIFIER]))
        );
        let mut g = self.inner.lock().unwrap();
        let _from = g
            .signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no signer for index".into()))?;
        let reg = registry_addr.to_lowercase();
        // Mirrors the contract's `require(!consumed[nf], "replayed")`.
        if g.consumed
            .get(&(reg.clone(), nf.clone()))
            .copied()
            .unwrap_or(false)
        {
            return Err(ChainError::Other("replayed".into()));
        }
        g.consumed.insert((reg.clone(), nf.clone()), true);
        g.clock += 12;
        let ts = U256::from(g.clock);
        g.nonce += 1;
        let tx_hash = format!("0x{:064x}", g.nonce);
        // OWNER-BLIND: no `subject` is derived, because no public signal carries one.
        let ev = ConsentVerifiedEvent {
            dog_tag_id: parse_u256_dec_or_hex(&pub_signals[PB::DOG_TAG_ID]),
            relayer: format!(
                "0x{:040x}",
                parse_u256_dec_or_hex(&pub_signals[PB::RELAYER])
            ),
            purpose: format!(
                "0x{}",
                hex::encode(parse_b256_dec_or_hex(&pub_signals[PB::PURPOSE]))
            ),
            nullifier: nf,
            deadline: parse_u256_dec_or_hex(&pub_signals[PB::DEADLINE]),
            ts,
        };
        g.consent_verified.insert(tx_hash.clone(), ev);
        Ok(SentTx { tx_hash })
    }
    async fn get_consent_verified_event(
        &self,
        tx_hash: &str,
        _registry_addr: &str,
    ) -> Result<ConsentVerifiedEvent, ChainError> {
        let g = self.inner.lock().unwrap();
        g.consent_verified
            .get(tx_hash)
            .cloned()
            .ok_or(ChainError::NotFound)
    }
    async fn consumed(&self, registry_addr: &str, nullifier: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.consumed
            .get(&(registry_addr.to_lowercase(), nullifier.to_lowercase()))
            .copied()
            .unwrap_or(false))
    }
    async fn mint_custodial(
        &self,
        account_index: u32,
        sbt_addr: &str,
        dog_tag_id: &str,
        root: &str,
    ) -> Result<SentTx, ChainError> {
        // Emulate DogTagSBTConsent.mintCustodial(id, root). No per-tag owner is modelled; write-once is
        // keyed by profileRoot, mirroring `profileRoot[id] != 0` in the contract.
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no issuer signer for index".into()))?;
        if parse_b256(root) == B256::ZERO {
            return Err(ChainError::Other("BadRoot: zero root".into()));
        }
        let key = (sbt_addr.to_lowercase(), normalize_id(dog_tag_id));
        if g.sbt_roots.contains_key(&key) {
            return Err(ChainError::Other("BadRoot: dogTagId already minted".into()));
        }
        g.sbt_roots.insert(key, root.to_lowercase());
        g.nonce += 1;
        let tx_hash = format!("0x{:064x}", g.nonce);
        Ok(SentTx { tx_hash })
    }
    async fn profile_root_of(
        &self,
        sbt_addr: &str,
        dog_tag_id: &str,
    ) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        g.sbt_roots
            .get(&(sbt_addr.to_lowercase(), normalize_id(dog_tag_id)))
            .cloned()
            .ok_or(ChainError::NotFound)
    }
}

/// Big-endian 32-byte word from a decimal-or-hex string (for MemChain nullifier emulation).
fn parse_b256_dec_or_hex(s: &str) -> B256 {
    B256::from(parse_u256_dec_or_hex(s).to_be_bytes::<32>())
}

// --------------------------------------------------------------------------------------------
// AlloyChain — real ROAX/anvil-backed client using a MnemonicBuilder-derived wallet set.
// --------------------------------------------------------------------------------------------

/// A funded, unlocked Alloy chain client. Holds derived signers (by account index) and the RPC url.
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
}

#[async_trait]
impl ChainClient for AlloyChain {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }
    async fn register_signer(&self, index: u32, private_key: [u8; 32], _address: String) {
        if let Ok(s) = alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from(private_key))
        {
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
    async fn is_revoked(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let r = c
            .isRevoked(parse_b256(root))
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
        // Wait for the tx to be MINED before returning, so the immediate confirm step's
        // get_tx_view / issuedAt reads (and any cast isValid read) reflect the on-chain effect.
        // Returning at broadcast time made confirm race the mempool and fail with tx NotFound.
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
                    let head = provider
                        .get_block_number()
                        .await
                        .map_err(|e| ChainError::Rpc(e.to_string()))?;
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
            block_number: receipt.block_number,
            root_issued_logs: logs,
        })
    }
    async fn record_verification_zk_consent(
        &self,
        account_index: u32,
        registry_addr: &str,
        a: &[String; 2],
        b: &[[String; 2]; 2],
        c: &[String; 2],
        pub_signals: &[String; 7],
    ) -> Result<SentTx, ChainError> {
        let calldata = record_verification_zk_consent_calldata(a, b, c, pub_signals);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }
    async fn get_consent_verified_event(
        &self,
        tx_hash: &str,
        registry_addr: &str,
    ) -> Result<ConsentVerifiedEvent, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::sol_types::SolEvent;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let hash: B256 = parse_b256(tx_hash);
        let receipt = provider
            .get_transaction_receipt(hash)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            .ok_or(ChainError::NotFound)?;
        if !receipt.status() {
            return Err(ChainError::Other("tx reverted".into()));
        }
        let reg = parse_addr(registry_addr);
        for log in receipt.inner.logs() {
            // Address-gate before decoding the owner-hidden event.
            if log.address() != reg {
                continue;
            }
            if let Ok(ev) = IVerificationRegistryConsent::Verified::decode_log(log.as_ref(), true) {
                return Ok(ConsentVerifiedEvent {
                    dog_tag_id: ev.dogTagId,
                    relayer: format!("{:#x}", ev.relayer),
                    purpose: format!("0x{}", hex::encode(ev.purpose.as_slice())),
                    nullifier: format!("0x{}", hex::encode(ev.nullifier.as_slice())),
                    deadline: ev.deadline,
                    ts: ev.ts,
                });
            }
        }
        Err(ChainError::NotFound)
    }
    async fn consumed(&self, registry_addr: &str, nullifier: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IVerificationRegistryConsent::new(parse_addr(registry_addr), provider);
        let r = c
            .consumed(parse_b256(nullifier))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn profile_root_of(
        &self,
        sbt_addr: &str,
        dog_tag_id: &str,
    ) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagSBT::new(parse_addr(sbt_addr), provider);
        let r = c
            .profileRoot(parse_u256_dec_or_hex(dog_tag_id))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("0x{}", hex::encode(r._0.as_slice())))
    }
}

/// Helper: normalize a record-type string into its keccak256 bytes32 (the whitelist / issuer key).
pub fn record_type_key(record_type: &str) -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(record_type.as_bytes());
    format!("0x{}", hex::encode(h.as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The first 4 bytes of any calldata are the canonical function selector
    // (keccak256(signature)[..4]). These are pinned independently (via `cast sig`)
    // so a drift in the sol! ABI or an accidental signature change breaks the build.
    fn selector(calldata: &str) -> String {
        let s = calldata.strip_prefix("0x").unwrap();
        s[..8].to_string()
    }

    #[test]
    fn calldata_encoders_use_canonical_selectors() {
        // issue(bytes32) / revoke(bytes32)
        assert_eq!(selector(&issue_calldata("0x00")), "0f75e81f");
        assert_eq!(selector(&revoke_calldata("0x00")), "b75c7dc6");
        // mintCustodial(uint256,bytes32) has no address word that could carry an owner.
        assert_eq!(selector(&mint_custodial_calldata("1", "0x00")), "de49152b");
        let a = ["0".to_string(), "0".to_string()];
        let b = [
            ["0".to_string(), "0".to_string()],
            ["0".to_string(), "0".to_string()],
        ];
        let c = ["0".to_string(), "0".to_string()];
        let pubs = [
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
        ];
        // recordVerificationZK(uint256[2],uint256[2][2],uint256[2],uint256[7]) — 4-arg.
        assert_eq!(
            selector(&record_verification_zk_consent_calldata(&a, &b, &c, &pubs)),
            "dd080593"
        );
    }

    #[test]
    fn issue_calldata_is_selector_plus_one_word_and_deterministic() {
        let root = "0x1111111111111111111111111111111111111111111111111111111111111111";
        let cd = issue_calldata(root);
        // 0x + 4-byte selector + one 32-byte word = 2 + 8 + 64 hex chars.
        assert_eq!(cd.len(), 2 + 8 + 64);
        assert!(cd.ends_with(&"1".repeat(64)));
        assert_eq!(cd, issue_calldata(root));
        // The 0x prefix is optional on the root and yields identical encoding.
        let bare = &root[2..];
        assert_eq!(issue_calldata(bare), cd);
    }

    #[test]
    fn parse_b256_tolerates_bad_and_short_input() {
        let valid = "0x2222222222222222222222222222222222222222222222222222222222222222";
        assert_eq!(parse_b256(valid), parse_b256(&valid[2..])); // prefix optional
        assert_ne!(parse_b256(valid), B256::ZERO);
        // Non-hex, wrong-length, and empty all collapse to the zero word (never panic).
        assert_eq!(parse_b256("0xzz"), B256::ZERO);
        assert_eq!(parse_b256("0x1234"), B256::ZERO); // not 32 bytes
        assert_eq!(parse_b256(""), B256::ZERO);
    }

    #[test]
    fn parse_addr_falls_back_to_zero_on_garbage() {
        let a = "0x00000000000000000000000000000000000000aB";
        assert_ne!(parse_addr(a), Address::ZERO);
        assert_eq!(parse_addr("not-an-address"), Address::ZERO);
        assert_eq!(parse_addr(""), Address::ZERO);
    }

    #[test]
    fn parse_u256_dec_or_hex_handles_radix_and_fallback() {
        assert_eq!(parse_u256_dec_or_hex("255"), U256::from(255u64));
        assert_eq!(parse_u256_dec_or_hex("0xff"), U256::from(255u64));
        assert_eq!(parse_u256_dec_or_hex("  42  "), U256::from(42u64)); // trims
                                                                        // Unparseable input falls back to zero rather than panicking.
        assert_eq!(parse_u256_dec_or_hex("0xnothex"), U256::ZERO);
        assert_eq!(parse_u256_dec_or_hex("notdec"), U256::ZERO);
    }

    #[test]
    fn normalize_id_collapses_radix_to_canonical_decimal() {
        assert_eq!(normalize_id("0x10"), "16");
        assert_eq!(normalize_id("16"), "16");
        assert_eq!(normalize_id("0x10"), normalize_id("16"));
        assert_eq!(normalize_id("garbage"), "0"); // fallback
    }

    #[test]
    fn record_type_key_anchors_keccak_of_empty_string() {
        // keccak256("") — the canonical empty-input digest, mirroring the admin stack.
        assert_eq!(
            record_type_key(""),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        // Distinct labels hash distinctly; output is always a 0x + 64-hex bytes32.
        let k = record_type_key("boarding_intake");
        assert_eq!(k.len(), 66);
        assert_ne!(k, record_type_key(""));
    }
}
