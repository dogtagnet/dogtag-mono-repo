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
use alloy::providers::RootProvider;
use alloy::sol;
use alloy::transports::BoxTransport;
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
        /// The clone's own name, written by the factory at `createIssuer` — which is `onlyOwner` on the
        /// factory, i.e. the protocol multisig at KYC time. This is an AUTHORITATIVE issuer name in
        /// exactly the way the document's `issuer.name` is not (that one is outside the Merkle root).
        function name() external view returns (string);
        function isRevoked(bytes32 r) external view returns (bool);
        function issuedAt(bytes32 r) external view returns (uint256);
        function issuedBy(bytes32 r) external view returns (address);
        function recordType() external view returns (bytes32);
    }

    /// The clone factory. `isClone` is the FIRST link of the issuer↔domain chain: it proves the
    /// contract was deployed by the DogTag factory, i.e. that it passed through KYC-gated
    /// `createIssuer` (`onlyOwner`, the protocol multisig). Without it the binding is worthless —
    /// anyone can deploy their own contract, claim a domain for it and publish a matching TXT record.
    #[sol(rpc)]
    contract IDogTagIssuerFactory {
        function isClone(address candidate) external view returns (bool);
        /// Write-once `R -> issuing clone`. THE authoritative answer to "which contract issued this
        /// credential" — the document's `issuer.documentStore` is only a claim, and pointing it at a
        /// contract you control is the sharper form of the relabelling attack.
        function rootIssuer(bytes32 root) external view returns (address);
    }

    #[sol(rpc)]
    contract IIssuerRegistry {
        function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
    }

    /// The ADDITIVE issuer↔domain claim registry. `getBinding` deliberately does NOT revert on an
    /// unknown clone — "this issuer has claimed no domain" is a normal day-one state — so the caller
    /// discriminates on `updatedAt != 0` rather than on a revert.
    #[sol(rpc)]
    contract IIssuerDomainRegistry {
        struct Binding { string domain; uint64 updatedAt; uint64 updatedAtBlock; address setBy; }
        function getBinding(address clone) external view returns (Binding memory);
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

/// A clone's on-chain domain claim, with the block anchor that makes it reproducible.
///
/// `updated_at_block` is the block the CLAIM was written at (from the contract). It is distinct from the
/// block the claim was READ at, which the caller records separately: the first says when the issuer last
/// changed its mind, the second says which chain state this answer came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainClaim {
    pub domain: String,
    pub updated_at: u64,
    pub updated_at_block: u64,
    pub set_by: String,
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
    /// `DogTagIssuer.recordType()` — the clone's own immutable record type key, or `None` when the
    /// contract reports the zero word (uninitialized / not a clone). Read from the RESOLVED clone so
    /// the whitelist question is asked about the record type the chain says this root belongs to,
    /// never the one the envelope claims.
    async fn issuer_record_type(
        &self,
        issuer_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError>;
    /// `DogTagIssuer.isValid(root)` — issued && !revoked.
    ///
    /// Takes `at_block` because this is a VERDICT-DECIDING read: a response that reports a block anchor
    /// while answering this one at `latest` is anchored in name only — a revoke landing between the head
    /// read and this call yields a verdict that cannot be reproduced at the block printed beside it.
    /// `None` means the caller has no anchor and the answer is reported as unanchored.
    async fn is_valid(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError>;
    /// `DogTagIssuer.issuedAt(root)` (0 == not issued). Pinned for the same reason as `is_valid`, so a
    /// caller that has an anchor cannot silently read this one at a different height.
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<U256, ChainError>;
    /// `DogTagIssuerFactory.isClone(clone)` — LINK 1 of the issuer↔domain chain.
    ///
    /// Re-checked at every verification rather than inherited from the fact that the domain registry
    /// once enforced it: a stored binding is a CLAIM, and an app must not inherit trust it did not
    /// verify itself. `Err` means the read failed, which is emphatically not the same as `Ok(false)`
    /// ("this is not a DogTag issuer") and must never be rendered as it.
    async fn is_factory_clone(
        &self,
        factory_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError>;
    /// `DogTagIssuer.name()` — the clone's on-chain name.
    ///
    /// The ONLY issuer name a surface may present. The document's `issuer.name` is outside the Merkle
    /// root, so relabelling it alone passes both integrity AND the `data.issuer` DID assertion (the DID
    /// carries a DOMAIN, not a name) — leaving a fabricated authority rendered beside a green check.
    /// Reading the name from the clone closes that.
    async fn issuer_onchain_name(
        &self,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<String, ChainError>;
    /// `DogTagIssuerFactory.rootIssuer(root)` — the clone that ISSUED this root, or `None` when the
    /// factory has no record of it.
    ///
    /// Old credentials must resolve against the clone that issued them, not against whatever the current
    /// clone for that record type happens to be. If a clone is superseded, a credential issued by the
    /// old one is still legitimate, and verification must not silently re-point at a successor.
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError>;
    /// The current chain head. Every binding read is pinned to ONE block so the whole verification is a
    /// consistent, reproducible snapshot rather than a smear across several heights.
    async fn block_number(&self) -> Result<u64, ChainError>;
    /// `IssuerDomainRegistry.getBinding(clone).domain` — the domain the ISSUER CLONE claims on chain.
    ///
    /// `Ok(None)` means the clone has claimed no domain (a normal state), which is NOT the same as a
    /// read failure (`Err`). Callers must keep those apart: an unread claim cannot be displayed as
    /// "no claim".
    ///
    /// This is the ONLY domain a surface may attribute to an issuer. The document's `issuer.domain` is
    /// outside the Merkle root and can be relabelled at will (audit-m9 finding 4).
    async fn issuer_claimed_domain(
        &self,
        domain_registry_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<DomainClaim>, ChainError>;
    /// `DogTagIssuer.issuedBy(root)` — the H-1 originator that actually called `issue(root)` on this
    /// clone, or `None` when the clone never issued it (the on-chain zero address).
    ///
    /// This is what lets the issuer-whitelist pillar resolve its own signer instead of asking an
    /// operator to type one in, and therefore what lets that pillar be MANDATORY. `issue()` is
    /// `onlyWhitelisted`, so a genuinely issued root's originator was whitelisted for its record type
    /// at issuance by construction.
    ///
    /// Pinned to `at_block` for the same reason as `is_valid`: every read behind one verdict must come
    /// from ONE height, or the verdict cannot be reproduced at the block printed beside it.
    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError>;
    /// `IssuerRegistry.isWhitelistedFor(recordType, signer)`.
    ///
    /// Pinned like the other verdict-deciding reads: when this pillar is reported under a block anchor it
    /// must be reproducible at that block, or a delist landing mid-verification makes the printed anchor
    /// a false claim. `None` for the live preflights, which gate a broadcast rather than report a
    /// verdict and therefore want `latest`.
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
        at_block: Option<u64>,
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
    /// ONE connection, shared by every gasless read.
    ///
    /// Each read used to build its own provider, so a single `POST /v1/verify` — an UNAUTHENTICATED
    /// route that now makes up to seven reads — cost seven connect+call round trips against our own
    /// node, and an attacker varying the caller-supplied `documentStore` amortised none of it.
    ///
    /// `OnceCell` rather than eager construction so a node that is down at boot does not stop the
    /// service, and a failed connect leaves the cell EMPTY (the next call retries) rather than poisoning
    /// the client for the process's lifetime.
    ///
    /// Deliberately read-only: `send_call` keeps building its own wallet+fillers provider, because that
    /// one carries a signer and a nonce filler whose state must not be shared with the read path.
    read_provider: tokio::sync::OnceCell<RootProvider<BoxTransport>>,
}

impl AlloyChain {
    pub fn new(rpc_url: String) -> Self {
        AlloyChain {
            rpc_url,
            chain_id: ROAX_CHAIN_ID,
            signer: None,
            read_provider: tokio::sync::OnceCell::new(),
        }
    }

    /// The shared read provider, connected on first use.
    async fn provider(&self) -> Result<RootProvider<BoxTransport>, ChainError> {
        self.read_provider
            .get_or_try_init(|| async {
                RootProvider::<BoxTransport>::connect_builtin(&self.rpc_url)
                    .await
                    .map_err(|e| ChainError::Rpc(e.to_string()))
            })
            .await
            .cloned()
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
    async fn is_valid(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let mut call = c.isValid(parse_b256(root));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<U256, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let mut call = c.issuedAt(parse_b256(root));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn block_number(&self) -> Result<u64, ChainError> {
        use alloy::providers::Provider;
        let provider = self.provider().await?;
        provider
            .get_block_number()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))
    }
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let mut call = c.rootIssuer(parse_b256(root));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        if r._0 == Address::ZERO {
            return Ok(None);
        }
        Ok(Some(format!("{:#x}", r._0)))
    }
    async fn is_factory_clone(
        &self,
        factory_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let mut call = c.isClone(parse_addr(clone_addr));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn issuer_onchain_name(
        &self,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<String, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuer::new(parse_addr(clone_addr), provider);
        let mut call = c.name();
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn issuer_claimed_domain(
        &self,
        domain_registry_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<DomainClaim>, ChainError> {
        let provider = self.provider().await?;
        let c = IIssuerDomainRegistry::new(parse_addr(domain_registry_addr), provider);
        let mut call = c.getBinding(parse_addr(clone_addr));
        if let Some(blk) = at_block {
            call = call.block(blk.into());
        }
        let b = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // updatedAt == 0 is the contract's documented "no binding published" discriminator; an empty
        // domain is unrepresentable on-chain, so both checks agree.
        if b._0.updatedAt == 0 || b._0.domain.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(DomainClaim {
            domain: b._0.domain,
            updated_at: b._0.updatedAt,
            updated_at_block: b._0.updatedAtBlock,
            set_by: format!("{:#x}", b._0.setBy),
        }))
    }
    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let mut call = c.issuedBy(parse_b256(root));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // The zero address is a SUCCESSFUL read saying "this clone never issued this root". Return it
        // as `None` so callers treat it as indeterminate rather than asking the registry whether the
        // zero address is whitelisted (a definite `false` for the wrong reason).
        Ok((!r._0.is_zero()).then(|| r._0.to_string()))
    }
    async fn issuer_record_type(
        &self,
        issuer_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        let provider = self.provider().await?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let mut call = c.recordType();
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // An all-zero word means uninitialized / not a clone: indeterminate, never a record type.
        Ok((!r._0.is_zero()).then(|| format!("{:#x}", r._0)))
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        let provider = self.provider().await?;
        let c = IIssuerRegistry::new(parse_addr(registry_addr), provider);
        let mut call = c.isWhitelistedFor(parse_b256(record_type), parse_addr(signer));
        if let Some(b) = at_block {
            call = call.block(b.into());
        }
        let r = call
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

/// A contract the factory never deployed, whose reads return whatever its operator chose. This is the
/// forgery the issuer pillar exists to catch: a real attacker deploys exactly this and points a
/// relabelled document's `issuer.documentStore` at it.
///
/// It is a first-class part of the emulation rather than a test-only shim because the honest
/// `issue()` path CANNOT express it - that path stamps `issued_by = self.signer` and registers the
/// root in the factory index, so it can only ever produce answers a genuine clone would give. Without
/// this, a test aimed at the attack silently exercises an honest clone instead and passes for the
/// wrong reason.
#[derive(Clone)]
struct HostileClone {
    is_valid: bool,
    issued_at: u64,
    issued_by: String,
    record_type: String,
}

struct MemChainInner {
    /// (issuer_addr, root) -> issuedAt timestamp (0 == not issued).
    issued: HashMap<(String, String), u64>,
    /// (issuer_addr, root) -> the signer that issued it, mirroring the real clone's `issuedBy[r] =
    /// msg.sender`. Absent == never issued here, which the whitelist pillar reads as indeterminate.
    issued_by: HashMap<(String, String), String>,
    revoked: HashMap<(String, String), u64>,
    /// root -> the clone that registered it. Mirrors `DogTagIssuerFactory.rootIssuer`, which is
    /// protocol-global and strictly write-once, so this is keyed by root ALONE (the emulation has one
    /// factory) and `issue` refuses a root another clone already claimed.
    root_issuer: HashMap<String, String>,
    /// issuer_addr -> the clone's immutable `recordType()` key.
    record_types: HashMap<String, String>,
    /// Contracts the factory never deployed, answering with attacker-chosen values.
    hostile: HashMap<String, HostileClone>,
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    /// (registry_addr, nullifier) consumed by a recordVerificationZK — the emulated replay guard.
    consumed: HashMap<(String, String), bool>,
    /// (domain_registry_addr, clone_addr) -> the clone's claimed domain (absent == no claim).
    claimed_domains: HashMap<(String, String), DomainClaim>,
    /// (factory_addr, root) -> the clone that issued it.
    root_issuers: HashMap<(String, String), String>,
    /// clone_addr -> the clone's on-chain `name()`.
    onchain_names: HashMap<String, String>,
    /// (factory_addr, clone_addr) -> whether the factory deployed it. Absent == not a clone.
    factory_clones: HashMap<(String, String), bool>,
    /// read name -> the `at_block` that read was LAST asked for.
    ///
    /// The emulation ignores the anchor (there is no history to read), so without recording it no
    /// hermetic test can tell a pinned read from an unpinned one — and "the response's `blockNumber` is
    /// true for every read behind it" is exactly the claim that silently rots when a new read is added
    /// without the parameter, or an existing one is passed `None`. Keyed by name rather than appended,
    /// so a long-running demo backend (`GOV_CHAIN_BACKEND=mem`) cannot grow this without bound.
    at_blocks: HashMap<&'static str, Option<u64>>,
    nonce: u64,
    clock: u64,
}

impl Default for MemChainInner {
    fn default() -> Self {
        MemChainInner {
            issued: HashMap::new(),
            issued_by: HashMap::new(),
            revoked: HashMap::new(),
            root_issuer: HashMap::new(),
            record_types: HashMap::new(),
            hostile: HashMap::new(),
            whitelist: HashMap::new(),
            consumed: HashMap::new(),
            claimed_domains: HashMap::new(),
            onchain_names: HashMap::new(),
            factory_clones: HashMap::new(),
            root_issuers: HashMap::new(),
            at_blocks: HashMap::new(),
            nonce: 0,
            clock: MEMCHAIN_CLOCK_BASE,
        }
    }
}

#[derive(Clone)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
    signer: String,
    /// Factory that issuances register under; empty = this fake models a chain whose factory has no
    /// record of what it issued.
    factory: String,
}

impl Default for MemChain {
    fn default() -> Self {
        // A deterministic demo signer address so `signer_address()`/whitelist wiring is stable.
        MemChain {
            inner: Arc::new(Mutex::new(MemChainInner::default())),
            signer: "0x0000000000000000000000000000000000600760".to_lowercase(),
            factory: String::new(),
        }
    }
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Override the emulated signer address (test harness).
    /// Register issuances under this factory, mirroring the `registerRoot` every real `issue()`
    /// performs. Left UNSET by default on purpose: "issued, but the factory has no record of it" is a
    /// distinct and testable state, and a fake that collapsed it into "resolved" would make the
    /// unresolvable-root case impossible to model.
    pub fn with_factory(mut self, factory_addr: &str) -> Self {
        self.factory = factory_addr.to_lowercase();
        self
    }
    pub fn with_signer(mut self, addr: &str) -> Self {
        self.signer = addr.to_lowercase();
        self
    }
    /// Seed factory provenance for a clone (test harness). Not seeding it emulates a contract that was
    /// NOT deployed by the DogTag factory — the attack link 1 exists to catch.
    pub fn set_factory_clone(&self, factory_addr: &str, clone_addr: &str, is_clone: bool) {
        let mut g = self.inner.lock().unwrap();
        g.factory_clones.insert(
            (factory_addr.to_lowercase(), clone_addr.to_lowercase()),
            is_clone,
        );
    }
    /// Seed a clone's on-chain `name()` (test harness).
    pub fn set_onchain_name(&self, clone_addr: &str, name: &str) {
        let mut g = self.inner.lock().unwrap();
        g.onchain_names
            .insert(clone_addr.to_lowercase(), name.to_string());
    }
    /// Seed a clone's on-chain domain claim (test harness). Absence of a seeded value emulates the
    /// normal "this issuer has claimed no domain" state.
    pub fn set_claimed_domain(&self, domain_registry_addr: &str, clone_addr: &str, domain: &str) {
        let mut g = self.inner.lock().unwrap();
        let block = g.clock.max(1);
        let ts = g.clock;
        g.claimed_domains.insert(
            (
                domain_registry_addr.to_lowercase(),
                clone_addr.to_lowercase(),
            ),
            DomainClaim {
                domain: domain.to_string(),
                updated_at: ts,
                updated_at_block: block,
                set_by: "0x00000000000000000000000000000000000000ad".to_string(),
            },
        );
    }
    /// Seed `rootIssuer[R]` (test harness) — which clone issued a given root.
    /// Declare a clone's own immutable `recordType()`, mirroring what the factory's `createIssuer`
    /// fixes on a real clone. The issuer pillar reads the record type from the RESOLVED clone rather
    /// than from the document, so an undeclared clone leaves that pillar indeterminate.
    pub fn set_record_type(&self, clone_addr: &str, record_type_key: &str) {
        self.inner
            .lock()
            .unwrap()
            .record_types
            .insert(clone_addr.to_lowercase(), record_type_key.to_lowercase());
    }

    /// Install a HOSTILE, non-factory contract: it answers `isValid`/`issuedAt`/`issuedBy`/
    /// `recordType` with attacker-chosen values while the factory's `rootIssuer` index deliberately
    /// does NOT point at it.
    ///
    /// This exists because the earlier version of the forged-clone test could not fail: `issue()`
    /// forced `issued_by = self.signer`, so the emulated attacker was compelled to report its own
    /// non-whitelisted address and the test passed for the wrong reason. A fake that can only produce
    /// honest answers cannot exercise a dishonest contract.
    pub fn with_hostile_clone(
        &self,
        clone_addr: &str,
        is_valid: bool,
        issued_at: u64,
        issued_by: &str,
        record_type_key: &str,
    ) {
        self.inner.lock().unwrap().hostile.insert(
            clone_addr.to_lowercase(),
            HostileClone {
                is_valid,
                issued_at,
                issued_by: issued_by.to_lowercase(),
                record_type: record_type_key.to_lowercase(),
            },
        );
    }

    pub fn set_root_issuer(&self, factory_addr: &str, root: &str, clone_addr: &str) {
        let mut g = self.inner.lock().unwrap();
        g.root_issuers.insert(
            (factory_addr.to_lowercase(), root.to_lowercase()),
            clone_addr.to_lowercase(),
        );
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
    /// Record the anchor a pinned read was asked for. See [`MemChainInner::at_blocks`].
    fn record_at_block(&self, what: &'static str, at_block: Option<u64>) {
        self.inner
            .lock()
            .unwrap()
            .at_blocks
            .insert(what, at_block);
    }

    /// Every pinned read this client has served, and the `at_block` each was LAST asked for.
    ///
    /// The property worth asserting is that ONE verification is ONE consistent snapshot: every entry is
    /// `Some(b)` for the same `b`, and that `b` is the `blockNumber` the response prints. An entry of
    /// `None` is a read taken at `latest`, which makes that printed anchor a false claim.
    pub fn recorded_at_blocks(&self) -> HashMap<&'static str, Option<u64>> {
        self.inner.lock().unwrap().at_blocks.clone()
    }

    /// Forget the recorded anchors, so a test can scope its assertion to one request.
    pub fn clear_recorded_at_blocks(&self) {
        self.inner.lock().unwrap().at_blocks.clear();
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
    async fn is_valid(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        self.record_at_block("isValid", at_block);
        let g = self.inner.lock().unwrap();
        if let Some(h) = g.hostile.get(&issuer_addr.to_lowercase()) {
            return Ok(h.is_valid);
        }
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        let issued = g.issued.get(&key).copied().unwrap_or(0) != 0;
        let revoked = g.revoked.get(&key).copied().unwrap_or(0) != 0;
        Ok(issued && !revoked)
    }
    async fn issued_at(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<U256, ChainError> {
        self.record_at_block("issuedAt", at_block);
        let g = self.inner.lock().unwrap();
        if let Some(h) = g.hostile.get(&issuer_addr.to_lowercase()) {
            return Ok(U256::from(h.issued_at));
        }
        let v = g
            .issued
            .get(&(issuer_addr.to_lowercase(), root.to_lowercase()))
            .copied()
            .unwrap_or(0);
        Ok(U256::from(v))
    }
    async fn block_number(&self) -> Result<u64, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.clock.max(1))
    }
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        self.record_at_block("rootIssuer", at_block);
        let g = self.inner.lock().unwrap();
        Ok(g.root_issuers
            .get(&(factory_addr.to_lowercase(), root.to_lowercase()))
            .cloned())
    }
    /// Link 1, and the ONE read where an unseeded pair must NOT default to `false`.
    ///
    /// `Ok(false)` here is a DEFINITE "the factory says it did not deploy this contract" — a verdict
    /// pillar, and a categorically strong claim. A simulated chain has no knowledge of a real deployed
    /// clone, so answering `false` for an address it was never told about manufactures that claim out of
    /// nothing: pointing a `GOV_CHAIN_BACKEND=mem` stack at the real `FACTORY_ADDR` would fail every
    /// legitimate credential. Same doctrine as [`SIMULATED_CHAIN_ID`] — the emulation may not assert real
    /// facts it cannot have.
    ///
    /// A test that wants the definite negative seeds it: `set_factory_clone(factory, clone, false)`.
    async fn is_factory_clone(
        &self,
        factory_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        self.record_at_block("isClone", at_block);
        let g = self.inner.lock().unwrap();
        g.factory_clones
            .get(&(factory_addr.to_lowercase(), clone_addr.to_lowercase()))
            .copied()
            .ok_or_else(|| {
                ChainError::Other(
                    "simulated chain has no factory-provenance record for this contract".into(),
                )
            })
    }
    async fn issuer_onchain_name(
        &self,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<String, ChainError> {
        self.record_at_block("name", at_block);
        let g = self.inner.lock().unwrap();
        Ok(g.onchain_names
            .get(&clone_addr.to_lowercase())
            .cloned()
            .unwrap_or_default())
    }
    async fn issuer_claimed_domain(
        &self,
        domain_registry_addr: &str,
        clone_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<DomainClaim>, ChainError> {
        self.record_at_block("domainOf", at_block);
        let g = self.inner.lock().unwrap();
        Ok(g.claimed_domains
            .get(&(
                domain_registry_addr.to_lowercase(),
                clone_addr.to_lowercase(),
            ))
            .cloned())
    }
    async fn issued_by(
        &self,
        issuer_addr: &str,
        root: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        self.record_at_block("issuedBy", at_block);
        let g = self.inner.lock().unwrap();
        let addr = issuer_addr.to_lowercase();
        if let Some(h) = g.hostile.get(&addr) {
            return Ok((!h.issued_by.is_empty()).then(|| h.issued_by.clone()));
        }
        Ok(g.issued_by.get(&(addr, root.to_lowercase())).cloned())
    }
    async fn issuer_record_type(
        &self,
        issuer_addr: &str,
        at_block: Option<u64>,
    ) -> Result<Option<String>, ChainError> {
        self.record_at_block("recordType", at_block);
        let g = self.inner.lock().unwrap();
        let addr = issuer_addr.to_lowercase();
        if let Some(h) = g.hostile.get(&addr) {
            return Ok((!h.record_type.is_empty()).then(|| h.record_type.clone()));
        }
        Ok(g.record_types.get(&addr).cloned())
    }
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
        at_block: Option<u64>,
    ) -> Result<bool, ChainError> {
        self.record_at_block("isWhitelistedFor", at_block);
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
        let addr = issuer_addr.to_lowercase();
        let root_key = root.to_lowercase();
        let key = (addr.clone(), root_key.clone());
        if g.issued.get(&key).copied().unwrap_or(0) != 0 {
            return Err(ChainError::Other("BadRoot: already issued".into()));
        }
        // Mirror `DogTagIssuerFactory.registerRoot`, which every real `issue()` calls: the root->clone
        // index is protocol-global and strictly write-once, so a second clone cannot claim a root that
        // is already someone else's. Without this the emulation would let an "attacker" re-anchor a
        // genuine root under their own address - something the real chain reverts.
        if let Some(existing) = g.root_issuer.get(&root_key) {
            if existing != &addr {
                return Err(ChainError::Other("root taken".into()));
            }
        }
        g.clock += 12;
        let ts = g.clock;
        g.issued.insert(key.clone(), ts);
        // Mirror the real clone's `issuedBy[r] = msg.sender` so the whitelist pillar resolves against
        // this fake exactly as it does on chain — otherwise every MemChain-backed record looks like
        // one nobody issued.
        g.issued_by.insert(key, self.signer.clone());
        g.root_issuer.insert(root_key.clone(), addr.clone());
        if !self.factory.is_empty() {
            g.root_issuers.insert((self.factory.clone(), root_key), addr);
        }
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
        assert!(!c.is_valid(issuer, root, None).await.unwrap());
        let tx = c.issue(issuer, root).await.unwrap();
        assert!(tx.tx_hash.starts_with("0x"));
        assert!(tx.block_number.is_some(), "issue surfaces a block number");
        assert!(c.is_valid(issuer, root, None).await.unwrap());
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
        assert!(c.is_valid(issuer, root, None).await.unwrap());
        let tx = c.revoke(issuer, root).await.unwrap();
        assert!(tx.tx_hash.starts_with("0x"));
        assert!(tx.block_number.is_some());
        // isValid flips to false, but issuedAt (the historical anchor) stays intact.
        assert!(!c.is_valid(issuer, root, None).await.unwrap());
        assert!(!c.issued_at(issuer, root, None).await.unwrap().is_zero());
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
        let pubs: [String; 7] =
            std::array::from_fn(|i| if i == 3 { "99".into() } else { "1".into() });
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
        let live =
            AlloyChain::new("https://devrpc.roax.net".to_string()).with_chain_id(ROAX_CHAIN_ID);
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

    /// The emulation may not assert a real fact it cannot have. `Ok(false)` from `isClone` is a DEFINITE
    /// "the factory did not deploy this" and is a verdict pillar, so an address the simulation was never
    /// told about must come back as a failed read, not as that claim — otherwise a `GOV_CHAIN_BACKEND=mem`
    /// stack pointed at the real `FACTORY_ADDR` fails every legitimate credential.
    #[tokio::test]
    async fn memchain_never_invents_a_definite_provenance_answer() {
        let c = MemChain::new();
        let factory = "0x00000000000000000000000000000000000000fa";
        let clone = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f";
        assert!(
            c.is_factory_clone(factory, clone, None).await.is_err(),
            "an unseeded pair is 'we were never told', not 'the factory said no'"
        );
        // A test that WANTS the definite negative seeds it, and gets it.
        c.set_factory_clone(factory, clone, false);
        assert!(!c.is_factory_clone(factory, clone, None).await.unwrap());
        c.set_factory_clone(factory, clone, true);
        assert!(c.is_factory_clone(factory, clone, None).await.unwrap());
    }

    /// The anchor every pinned read was asked for is recorded, so "one verification is one consistent
    /// snapshot" is assertable rather than merely stated. Keyed by read name, so a long-running demo
    /// backend cannot grow it without bound.
    #[tokio::test]
    async fn memchain_records_the_anchor_each_pinned_read_was_asked_for() {
        let c = MemChain::new();
        let issuer = "0x1111111111111111111111111111111111111111";
        let root = "0x2222222222222222222222222222222222222222222222222222222222222222";
        assert!(c.recorded_at_blocks().is_empty());

        let _ = c.is_valid(issuer, root, Some(42)).await;
        let _ = c.issued_at(issuer, root, Some(42)).await;
        let _ = c.is_whitelisted_for(issuer, "0x00", issuer, Some(42)).await;
        let reads = c.recorded_at_blocks();
        assert_eq!(reads.get("isValid"), Some(&Some(42)));
        assert_eq!(reads.get("issuedAt"), Some(&Some(42)));
        assert_eq!(reads.get("isWhitelistedFor"), Some(&Some(42)));

        // An UNPINNED read is recorded as such — that is the regression the recording exists to catch.
        let _ = c.is_valid(issuer, root, None).await;
        assert_eq!(c.recorded_at_blocks().get("isValid"), Some(&None));

        c.clear_recorded_at_blocks();
        assert!(c.recorded_at_blocks().is_empty());
    }

    #[test]
    fn explorer_url_uses_roax_scheme() {
        assert_eq!(
            explorer_tx_url("0xabc"),
            "https://explorer.roax.net/tx/0xabc"
        );
    }
}
