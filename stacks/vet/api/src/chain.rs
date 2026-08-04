//! `ChainClient` trait abstracting the ROAX (chainId 135) on-chain surface the backend needs, plus an
//! Alloy-backed implementation and an in-memory `MemChain` stub (emulates issue / isValid / RootIssued)
//! so the full HTTP flow is testable without a live node.
//!
//! Signing (impl §1.8): EIP-1559 with a legacy `gas_price` fallback; chainId pinned to 135.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, Bytes, FixedBytes, B256, U256};
use alloy::rpc::types::{Filter, Log};
use alloy::sol;
use alloy::sol_types::SolEvent;
use async_trait::async_trait;

// LAYER 2 of the two-layer issuance requirement. The shaping and the honesty rules live there; this
// file only gathers the reads.
use crate::issuance_allowed::{build_roster, normalize_addr, IssuanceRoster};
// Owner-hidden consent public-signal indices.
use dogtag_standard::public_signals::level_b as PB;
// The issuer-whitelist pillar's historical question and its ordering/fold rule live in the SDK, so
// every Rust surface that asks it shares ONE definition rather than three that can drift.
pub use dogtag_standard::verify::{grant_in_force_at, GrantAtIssuance, GrantEvent, LogPoint};

/// Where a mined log sits, as the ordering key the fold compares.
///
/// `None` for a log carrying no position. Callers must treat that as UNDETERMINED rather than
/// silently dropping it: a grant whose place in the sequence is unknown could flip the answer, and
/// answering anyway would be exactly the could-not-check-rendered-as-a-verdict this pillar refuses.
fn log_point(l: &Log) -> Option<LogPoint> {
    Some(LogPoint {
        block_number: l.block_number?,
        log_index: l.log_index?,
    })
}

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
        function issuedBy(bytes32 r) external view returns (address);
        function recordType() external view returns (bytes32);
        /// The ONE registry whose `_wl` mapping `onlyWhitelisted` consults. Written once in
        /// `initialize` from the factory's own `immutable registry`, with no setter — so for a
        /// factory-resolved clone this is unforgeable, and it is the only authority whose grant log
        /// can answer for this contract's issuances.
        function registry() external view returns (address);

        /// LAYER 2 of the two-layer issuance requirement — this contract's OWN list.
        ///
        /// The event is how you learn WHICH addresses the list has an opinion about (there is no
        /// enumeration), and the getter is how you learn what that opinion currently IS. Never read
        /// the event's `allowed` word as the answer — see `crate::issuance_allowed`.
        ///
        /// `initialize` emits this same event for the creation seed rather than a second topic
        /// (`DogTagIssuer.sol:262-269`), deliberately, so ONE filter reconstructs the whole set.
        event IssuanceAllowedSet(address indexed signer, bool allowed, address indexed setBy);
        function issuanceAllowed(address signer) external view returns (bool);
        /// The only address that may ADMIT to the list above.
        function owner() external view returns (address);
    }

    /// The clone factory. Its write-once `rootIssuer[R]` index (`DogTagIssuerFactory.sol:19`, written
    /// from inside a clone's `issue()` and gated on `isClone[msg.sender]`) is the ONLY authoritative
    /// answer to "which contract issued this root". The document's own `issuer.documentStore` sits
    /// outside the Merkle root and is therefore just a claim.
    #[sol(rpc)]
    contract IDogTagIssuerFactory {
        function rootIssuer(bytes32 root) external view returns (address);
        function isClone(address a) external view returns (bool);
    }

    /// The provider authority (`contracts/src/ProviderRegistry.sol`), reached off the resolved
    /// clone's own `registry()` and never from this deployment's configuration.
    ///
    /// Only the reads this backend actually puts to it are declared.
    ///
    /// `canIssue` is what `DogTagIssuer`'s `onlyIssuanceCapable` modifier itself calls, so a
    /// preflight built on it refuses exactly what the write would refuse — deliberately NOT the
    /// wider `isRecognizedIssuer`, which passes where the write reverts.
    ///
    /// `isWhitelistedFor` is the VERIFY axis only. At a plain `eth_call`'s zero `msg.sender` the
    /// authority answers off `_verifierCapabilities[key][signer]`, and `verify_key_from_purpose_word`
    /// reproduces its `verificationKey` byte-for-byte — so for a VERIFY key this is the same question
    /// for the same inputs. It must never be handed a RECORD-TYPE key: that key is never a
    /// `verificationKey` output, so the answer would be a confident `false` about every genuine
    /// issuer signer. Record-type callers use `canIssue` / the grant log below.
    #[sol(rpc)]
    contract IProviderAuthority {
        /// The grant history for ONE ADDRESS. `account` is indexed, so one filtered `eth_getLogs`
        /// reconstructs the whole sequence. `setRights` refuses a no-op with `NoChange()` and
        /// otherwise emits unconditionally, so the log is complete.
        ///
        /// `rights` is the address's COMPLETE settable mask after the write, not a delta, so the last
        /// event at or before a block IS the mask in force then and the fold needs no prior state.
        /// A reader interested in one right masks the word.
        ///
        /// NOTE there is NO service in this log, and that is the re-keying rather than an omission in
        /// this declaration: a grant is on the address alone. So the history a verifier folds is the
        /// signer's, and a root anchored on another provider's clone by a granted signer folds to
        /// AUTHORIZED. The separate check that the DOCUMENT's claimed record type matches
        /// `clone.recordType()` still stays at the caller and still refuses a relabelled credential.
        event RightsSet(address indexed account, uint256 rights);
        function canIssue(address service, address signer) external view returns (bool);
        function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
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

/// May this signer anchor a NEW root on this clone, right now?
///
/// A PRESENT-TENSE question, and deliberately a different one from
/// [`dogtag_standard::verify::GrantAtIssuance`], which asks whether a signer was authorised at the
/// moment an EXISTING root was anchored. Those are the two halves of the split-by-question rule in
/// `docs/CLIENT_REPOINT.md`: a pre-issue gate wants the present, a verifier wants the past, and one
/// answer must never be handed to the other caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuanceCapability {
    /// The clone's own governing authority says yes.
    Authorized,
    /// The clone's own governing authority says no. Evidence about the signer.
    NotAuthorized,
    /// The authority could not be established, or answered in no vocabulary this build recognises.
    /// Never a pass and never a definite refusal — the caller reports its inability instead.
    Undetermined,
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
    /// `DogTagIssuerFactory.rootIssuer(root)` — the clone that registered this root, or `None` when
    /// the factory has no record of it (the on-chain zero address).
    ///
    /// This is the anchor every verdict-deciding read hangs off. `issuer.documentStore` is outside the
    /// Merkle root, so a forged document can name any contract it likes; `rootIssuer` is write-once
    /// and only a `isClone` contract can write it, so it names the clone that issued THIS root and
    /// cannot be pointed at an attacker's contract.
    ///
    /// `Ok(None)` ("we asked, the chain has no record") and `Err` ("we could not ask") are kept
    /// distinct on purpose — the first is evidence about the credential, the second is not.
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
    ) -> Result<Option<String>, ChainError>;
    /// `DogTagIssuer.issuedBy(root)` — the H-1 originator that actually called `issue(root)` on this
    /// clone, or `None` when the clone never issued it (the on-chain zero address).
    ///
    /// This is what lets the issuer-whitelist pillar resolve its own signer instead of asking an
    /// operator to type one in, and therefore what lets that pillar be MANDATORY. `issue()` is
    /// `onlyWhitelisted` (`DogTagIssuer.sol:40,52`), so a genuinely issued root's originator was
    /// whitelisted for its record type at issuance by construction.
    async fn issued_by(&self, issuer_addr: &str, root: &str) -> Result<Option<String>, ChainError>;
    /// `DogTagIssuer.recordType()` — the clone's own immutable record-type key (set by the factory's
    /// `createIssuer` at KYC time), or `None` when the contract reports the zero word
    /// (uninitialized / not a clone). Read from the RESOLVED clone so the whitelist question is asked
    /// about the record type the CHAIN says this root belongs to, never the one the envelope claims.
    async fn issuer_record_type(&self, issuer_addr: &str) -> Result<Option<String>, ChainError>;
    /// IssuerRegistry.isWhitelistedFor(recordType, signer) — the CURRENT-state getter.
    ///
    /// Still the right read for a forward-looking question ("may this relayer verify this purpose
    /// now?"), which is what the consent path asks. It is NOT the read the issuer-whitelist pillar
    /// makes: see [`ChainClient::whitelisted_at_issuance`].
    ///
    /// **Every surviving caller passes a VERIFY key**, never a record-type key, and that restriction
    /// is what keeps `ISSUER_REGISTRY_ADDR` repointable. `ProviderRegistry` implements this exact
    /// selector but branches on `msg.sender`; a plain `eth_call` sends none, so the successor always
    /// takes its `_verifierCapabilities[key][signer]` branch. For a VERIFY key that is the same
    /// question for the same inputs — `verify_key_from_purpose_word` reproduces
    /// `ProviderRegistry.verificationKey` byte-for-byte (pinned by
    /// `verify_key_matches_provider_registry_verification_key`). For a RECORD-TYPE key it is not:
    /// that key is never a `verificationKey` output, so the successor answers a confident `false`
    /// about every genuine issuer signer. Record-type callers use
    /// [`ChainClient::issuance_capability`] instead.
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError>;
    /// May `signer` anchor a NEW root on `issuer_addr` right now? The record-type-keyed issuance
    /// preflight, asked service-scoped so it survives the generation-2 cutover.
    ///
    /// Like [`ChainClient::whitelisted_at_issuance`] it takes NO registry address — the authority is
    /// the resolved clone's own `registry()`. That is the whole point: `ISSUER_REGISTRY_ADDR` is one
    /// value read by both key shapes in one process, so there was nothing to split at the config
    /// layer while a record-type caller still read it. Sourcing the authority from the clone removes
    /// the record-type key shape from that variable entirely, and each clone then answers in its own
    /// generation's vocabulary with no operator-asserted generation flag to get wrong.
    ///
    /// `record_type` is passed for the generation-1 arm only, whose grant is keyed by it. The
    /// generation-2 arm is service-scoped and needs no key.
    ///
    /// The generation-2 target is `canIssue(service, signer)` and deliberately NOT
    /// `isRecognizedIssuer(service, signer)`: the ladder is `isRecognizedIssuer` ⊇ `canRevoke` ⊇
    /// `canIssue`, `DogTagIssuerV2.issue` is gated by `onlyIssuanceCapable` == `canIssue`, and a
    /// preflight built on a wider rung passes where the write reverts — which is the one thing a
    /// preflight exists to prevent.
    async fn issuance_capability(
        &self,
        issuer_addr: &str,
        signer: &str,
    ) -> Result<IssuanceCapability, ChainError>;
    /// Was `signer` authorised for `record_type` at the moment `root` was anchored on `issuer_addr`?
    ///
    /// The issuer-whitelist pillar's read. It takes NO registry address: the authority comes off the
    /// resolved clone's own `registry()`, which is unforgeable for a factory-resolved clone and is the
    /// only instance whose `_wl` mapping gated this contract's `issue()`. Answering from this client's
    /// separately-configured registry would ask a different contract's mapping and, on a mis-paired
    /// deployment, refuse a genuine credential over our own configuration.
    ///
    /// Delisting is forward-only (`DogTagIssuer.sol:82`), so the current-state getter above cannot
    /// answer this: it refuses every credential a since-rotated signer ever issued.
    async fn whitelisted_at_issuance(
        &self,
        issuer_addr: &str,
        signer: &str,
        root: &str,
    ) -> Result<GrantAtIssuance, ChainError>;
    /// LAYER 2 as a roster: this clone's `owner()` plus every address its own issuance list has an
    /// opinion about, each carrying its CURRENT storage value.
    ///
    /// `also` is an extra address to include even when the log never named it — this deployment's own
    /// custody signer, so the address the provider needs admitted always has a row rather than being
    /// invisible in exactly the state they are trying to diagnose.
    ///
    /// The log supplies only WHICH addresses to ask about; every value is a fresh
    /// `issuanceAllowed(address)` read, so a pending write can never read as a completed grant and no
    /// sequencing is required. See `crate::issuance_allowed` for why that inversion is the whole
    /// point.
    ///
    /// Reads THROW rather than answering an empty roster: "this contract admits nobody" and "we could
    /// not ask" are different claims, and only the caller can render them apart.
    async fn issuance_allowed_roster(
        &self,
        issuer_addr: &str,
        also: Option<&str>,
    ) -> Result<IssuanceRoster, ChainError>;
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

/// A contract the factory never deployed, whose reads return whatever its operator chose. This is the
/// forgery the issuer pillar exists to catch: a real attacker deploys exactly this and points a
/// relabelled document's `issuer.documentStore` at it.
///
/// It is a first-class part of the emulation rather than a test-only shim because the honest `issue()`
/// path CANNOT express it — that path stamps `issued_by = <the backend signer>` and registers the root
/// in the factory index, so it can only ever produce answers a genuine clone would give. Without this,
/// a test aimed at the attack silently exercises an honest clone instead and passes for the wrong
/// reason.
#[derive(Clone)]
struct HostileClone {
    is_valid: bool,
    issued_at: u64,
    issued_by: String,
    record_type: String,
}

#[derive(Default)]
struct MemChainInner {
    /// (issuer_addr, root) -> issuedAt timestamp (0 == not issued/revoked-cleared).
    issued: HashMap<(String, String), u64>,
    /// (issuer_addr, root) -> the signer that issued it, mirroring the real clone's
    /// `issuedBy[r] = msg.sender`. Absent == never issued here, which the whitelist pillar reads as
    /// indeterminate rather than as a failure.
    issued_by: HashMap<(String, String), String>,
    revoked: HashMap<(String, String), u64>,
    /// (factory_addr, root) -> the clone that registered it. Mirrors `DogTagIssuerFactory.rootIssuer`,
    /// which a clone writes from inside `issue()` and which is strictly write-once.
    root_issuers: HashMap<(String, String), String>,
    /// issuer_addr -> the clone's immutable `recordType()` key, as the factory's `createIssuer` fixes
    /// it on a real clone.
    record_types: HashMap<String, String>,
    /// Contracts the factory never deployed, answering with attacker-chosen values.
    hostile: HashMap<String, HostileClone>,
    /// (registry_addr, verify_key, signer) -> may that relayer verify that purpose NOW.
    ///
    /// The VERIFY axis only. `isWhitelistedFor` is never handed a record-type key: the authority
    /// answers that off `_verifierCapabilities`, so a record-type key would be a confident `false`
    /// about every genuine issuer signer.
    whitelist: HashMap<(String, String, String), bool>,
    /// (registry_addr, service_addr, signer) -> that pair's issuance grant history IN THAT
    /// AUTHORITY, oldest first — the `IssuanceCapabilitySet` log.
    ///
    /// Separate from `issuance_capabilities` because the two answer different questions and the
    /// whole point is that they can disagree: a signer whose capability has since been withdrawn is
    /// `false` there and still authorised-at-issuance here.
    /// (registry_addr, signer) -> that SIGNER's rights history in that authority, oldest first —
    /// the `RightsSet` log. There is no service in the key, exactly as there is none in the event:
    /// a grant is on the address alone, so one history answers for every service.
    grants: HashMap<(String, String), Vec<GrantEvent>>,
    /// (clone_addr, root) -> where that clone's anchoring `RootIssued` sits in the log.
    root_issued_at: HashMap<(String, String), LogPoint>,
    /// clone_addr -> the authority whose capability mapping gates it (`DogTagIssuer.registry()`).
    /// Per contract, because the real mapping is; a single global set could not model a mis-paired
    /// client.
    governing_registry: HashMap<String, String>,
    /// The authority every clone answers for unless `governing_registry` overrides it. Adopted from
    /// the first grant call, which is the matched factory/authority pair every honest test already
    /// wires — so this models the real `initialize` binding without a second setter.
    default_registry: String,
    /// (registry_addr, service_addr, signer) -> `canIssue` NOW.
    ///
    /// Stored rather than derived from `grants`, because the real `canIssue` folds live-lifecycle
    /// terms the grant log does not carry (provider standing, service standing, the current
    /// pointer). A fake that computed one from the other could not model a superseded clone whose
    /// past issuances stay genuine while it may anchor nothing new — precisely the case that catches
    /// a pillar wired to a current-state predicate.
    issuance_capabilities: HashMap<(String, String, String), bool>,
    /// Authorities that ANSWER NOTHING: an address whose selectors revert, or no contract at all.
    /// The node executed the call; what it established is that this build knows none of the
    /// contract's language.
    unanswerable_registries: HashSet<String>,
    /// clone_addr -> `owner()`. The only address a real clone lets ADMIT to the list below.
    clone_owners: HashMap<String, String>,
    /// (clone_addr, signer) -> `issuanceAllowed(signer)` NOW — LAYER 2's storage.
    issuance_allowed: HashMap<(String, String), bool>,
    /// clone_addr -> every address an `IssuanceAllowedSet` has named on that clone, oldest first.
    ///
    /// Kept SEPARATE from the storage map above, exactly as the chain keeps the log separate from the
    /// mapping, so a withdrawn address stays named by the log after its storage has gone back to
    /// `false` — which is the whole distinction between "withdrawn" and "never admitted".
    issuance_allowed_named: HashMap<String, Vec<String>>,
    /// Fault injection for the LOG read alone.
    ///
    /// Its own switch rather than a shared one, following `set_failing_approval_log_reads` beside
    /// `set_failing_provider_reads`: the realistic failure is a range-capping or rate-limiting peer
    /// refusing `eth_getLogs` while every `eth_call` beside it answers, and a shared switch would
    /// fail the owner read first and collapse the route into a 502 — so the `unavailable` arm this
    /// surface exists to render would never be built and the headline claim would go unpinned.
    failing_issuance_allowed_log_reads: bool,
    /// Monotone synthetic log position. Every emulated event — a registry grant, an anchoring — takes
    /// the next one, so CALL ORDER IS LOG ORDER: a test expresses "delisted after issuance" by
    /// delisting after it issued, and "before" by delisting before. Without an ordering this fake
    /// could not tell the two apart, and the tests that matter could not fail.
    log_seq: u64,
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

impl MemChainInner {
    /// The next synthetic log position. One event per block keeps it simple; the same-block ordering
    /// the real fold has to handle is pinned directly on `grant_in_force_at` in the SDK.
    fn next_log_point(&mut self) -> LogPoint {
        self.log_seq += 1;
        LogPoint {
            block_number: self.log_seq,
            log_index: 0,
        }
    }
}

#[derive(Clone, Default)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
    /// Factory that issuances register under; empty = this fake models a chain whose factory has no
    /// record of what it issued.
    factory: String,
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register issuances under this factory, mirroring the `registerRoot` every real `issue()`
    /// performs (`DogTagIssuer.sol:56`). Left UNSET by default on purpose: "issued, but the factory has
    /// no record of it" is a distinct and testable state, and a fake that collapsed it into "resolved"
    /// would make the unresolvable-root case impossible to model.
    pub fn with_factory(mut self, factory_addr: &str) -> Self {
        self.factory = factory_addr.to_lowercase();
        self
    }
    /// Seed `rootIssuer[R]` directly — which clone the factory says registered a given root.
    pub fn set_root_issuer(&self, factory_addr: &str, root: &str, clone_addr: &str) {
        self.inner.lock().unwrap().root_issuers.insert(
            (factory_addr.to_lowercase(), root.to_lowercase()),
            clone_addr.to_lowercase(),
        );
    }
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
    /// Install a HOSTILE, non-factory contract: it answers `isValid`/`isRevoked`/`issuedAt`/
    /// `issuedBy`/`recordType` with attacker-chosen values while the factory's `rootIssuer` index
    /// deliberately does NOT point at it.
    ///
    /// This exists because a forged-clone test built on `issue()` cannot fail: that path forces
    /// `issued_by` to a signer the emulation controls and registers the root in the factory index, so
    /// the emulated attacker would be compelled to answer honestly. A fake that can only produce
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
    /// Declare a clone's `owner()` — the only address a real clone lets ADMIT to its issuance list.
    ///
    /// Left UNSET by default: a clone whose owner this fake was never told about answers the zero
    /// address, which is what a real `owner()` on an uninitialized contract would give and is a state
    /// the page must render rather than crash on.
    pub fn set_clone_owner(&self, clone_addr: &str, owner: &str) {
        self.inner
            .lock()
            .unwrap()
            .clone_owners
            .insert(clone_addr.to_lowercase(), owner.to_lowercase());
    }
    /// Mirror `DogTagIssuer.setIssuanceAllowed` — write the storage AND name the address in the log,
    /// because the real function does both in one call and a fake that wrote only the storage could
    /// not produce a withdrawn entry at all.
    ///
    /// Deliberately NOT gated on `owner()`: this is a seeder for tests, and the authority asymmetry it
    /// would be modelling is pinned where it is enforced — on the contract, in
    /// `CustodialIssuance.t.sol::test_the_clones_own_list_is_what_admits_a_signer_and_only_its_owner_writes_it`.
    /// Emulating the gate here would be a second implementation of a rule this backend never applies,
    /// since it never sends this transaction.
    pub fn set_issuance_allowed(&self, clone_addr: &str, signer: &str, allowed: bool) {
        let mut g = self.inner.lock().unwrap();
        let clone = clone_addr.to_lowercase();
        let who = signer.to_lowercase();
        g.issuance_allowed
            .insert((clone.clone(), who.clone()), allowed);
        g.issuance_allowed_named.entry(clone).or_default().push(who);
    }
    /// Make the `IssuanceAllowedSet` log read fail while every `eth_call` beside it still answers.
    /// Default off. See `failing_issuance_allowed_log_reads` for why it is its own switch.
    pub fn set_failing_issuance_allowed_log_reads(&self, failing: bool) {
        self.inner
            .lock()
            .unwrap()
            .failing_issuance_allowed_log_reads = failing;
    }
    /// Register a backend signer address for an account index (test harness wires this from custody).
    pub fn set_signer(&self, index: u32, address: &str) {
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }
    /// Grant a relayer a VERIFY purpose — `ProviderRegistry.setVerifierCapability`.
    ///
    /// The verify axis is a plain current-state mapping: nothing asks a historical question about
    /// it, so unlike the issuance axis it records no positioned event.
    pub fn whitelist(&self, registry: &str, verify_key: &str, signer: &str) {
        self.set_verifier_capability(registry, verify_key, signer, true);
    }
    /// Withdraw a relayer's VERIFY purpose — `ProviderRegistry.setVerifierCapability(.., false)`.
    pub fn delist(&self, registry: &str, verify_key: &str, signer: &str) {
        self.set_verifier_capability(registry, verify_key, signer, false);
    }
    /// Shared body of [`MemChain::whitelist`]/[`MemChain::delist`].
    pub fn set_verifier_capability(
        &self,
        registry: &str,
        verify_key: &str,
        signer: &str,
        allowed: bool,
    ) {
        let mut g = self.inner.lock().unwrap();
        if g.default_registry.is_empty() {
            g.default_registry = registry.to_lowercase();
        }
        g.whitelist.insert(
            (
                registry.to_lowercase(),
                verify_key.to_lowercase(),
                signer.to_lowercase(),
            ),
            allowed,
        );
    }
    /// Where a clone's anchoring `RootIssued` for `root` sits, so a test can position a grant
    /// RELATIVE to it rather than guessing at this fake's internal clock.
    pub fn root_issued_at(&self, clone_addr: &str, root: &str) -> Option<LogPoint> {
        self.inner
            .lock()
            .unwrap()
            .root_issued_at
            .get(&(clone_addr.to_lowercase(), root.to_lowercase()))
            .copied()
    }
    /// Overwrite a `(registry, service, signer)` issuance grant history outright.
    ///
    /// Needed for the same reason [`MemChain::with_hostile_clone`] is: the honest path CANNOT express
    /// "withdrawn before this root was anchored", because issuance is gated on the capability, so a
    /// test driving a withdrawal then `issue` through the backend is refused at the preflight and
    /// never reaches the pillar. Without a way to seed it, the delisted-BEFORE half of the forward-only
    /// rule would be untestable at the route level and the delisted-AFTER half would be a check that
    /// only ever passes — which is exactly the shape of a test that cannot fail.
    pub fn set_grant_history(
        &self,
        registry: &str,
        service: &str,
        signer: &str,
        history: Vec<GrantEvent>,
    ) {
        let mut g = self.inner.lock().unwrap();
        if g.default_registry.is_empty() {
            g.default_registry = registry.to_lowercase();
        }
        // `canIssue` stays SERVICE-scoped — it folds live-lifecycle terms this fake models per
        // service — while the history is keyed on the SIGNER alone, exactly as `RightsSet` is. The
        // `service` argument therefore seeds only the former.
        g.issuance_capabilities.insert(
            (
                registry.to_lowercase(),
                service.to_lowercase(),
                signer.to_lowercase(),
            ),
            history.last().map(|e| e.granted).unwrap_or(false),
        );
        g.grants
            .insert((registry.to_lowercase(), signer.to_lowercase()), history);
    }
    /// Declare the registry every clone answers `registry()` with, without recording any grant.
    ///
    /// Needed because a real clone answers `registry()` whether or not anyone was ever whitelisted,
    /// while this fake otherwise adopts that address from the first `whitelist`/`delist` call. So a
    /// test that seeds NO grant leaves the fake with no authority at all, and every authority read
    /// then reports "could not determine" where a real chain reports an authority whose mapping is
    /// simply empty. Those are different answers — `Undetermined` versus `NotAuthorized` — and the
    /// second is the one a never-whitelisted signer deserves.
    ///
    /// Mirrors `government-api`'s `MemChain::with_registry`, which exists for exactly this reason.
    pub fn with_registry(self, registry: &str) -> Self {
        self.inner.lock().unwrap().default_registry = registry.to_lowercase();
        self
    }
    /// Point a clone at an authority OTHER than the one its grants were recorded under — the
    /// mis-paired client. Without an override every clone answers `default_registry`, which is the
    /// matched pair `initialize` produces on a real chain.
    pub fn set_governing_registry(&self, clone_addr: &str, registry: &str) {
        self.inner
            .lock()
            .unwrap()
            .governing_registry
            .insert(clone_addr.to_lowercase(), registry.to_lowercase());
    }
    /// Seed `canIssue` for `(service, signer)` on `registry`, AND record the signer's grant.
    ///
    /// The two halves are keyed differently on purpose, because the contract keys them differently:
    /// `canIssue` folds live-lifecycle terms and stays per service, while the grant is on the
    /// signer's ADDRESS and reaches every service. So granting here for one service makes the
    /// at-issuance pillar answer AUTHORIZED for that signer on ANY service in this registry — which
    /// is the real behaviour, and a fake that kept a per-service history could not model it.
    pub fn set_issuance_capability(
        &self,
        registry: &str,
        service: &str,
        signer: &str,
        can_issue: bool,
    ) {
        let mut g = self.inner.lock().unwrap();
        if g.default_registry.is_empty() {
            g.default_registry = registry.to_lowercase();
        }
        g.issuance_capabilities.insert(
            (
                registry.to_lowercase(),
                service.to_lowercase(),
                signer.to_lowercase(),
            ),
            can_issue,
        );
        // The real `setRights` flips the mapping AND emits `RightsSet` in one call, so this fake does
        // both: a test that grants and then issues produces the honest ordering, and the pillar can
        // answer historically without any extra seeding.
        let at = g.next_log_point();
        g.grants
            .entry((registry.to_lowercase(), signer.to_lowercase()))
            .or_default()
            .push(GrantEvent {
                at,
                granted: can_issue,
            });
    }
    /// Declare `registry` unable to ANSWER at all — an address whose selectors revert, or one
    /// carrying no contract.
    ///
    /// Fault injection, default-off, for the same reason `set_grant_history` exists: "could not
    /// determine" is a required state that the honest path cannot reach, and a state that cannot be
    /// driven is a state whose renderer is untested.
    pub fn set_registry_unanswerable(&self, registry: &str) {
        self.inner
            .lock()
            .unwrap()
            .unanswerable_registries
            .insert(registry.to_lowercase());
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
        if let Some(h) = g.hostile.get(&issuer_addr.to_lowercase()) {
            return Ok(h.is_valid);
        }
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        let issued = g.issued.get(&key).copied().unwrap_or(0) != 0;
        let revoked = g.revoked.get(&key).copied().unwrap_or(0) != 0;
        Ok(issued && !revoked)
    }
    async fn is_revoked(&self, issuer_addr: &str, root: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        if g.hostile.contains_key(&issuer_addr.to_lowercase()) {
            // An attacker's contract never admits to a revocation.
            return Ok(false);
        }
        let key = (issuer_addr.to_lowercase(), root.to_lowercase());
        Ok(g.revoked.get(&key).copied().unwrap_or(0) != 0)
    }
    async fn issued_at(&self, issuer_addr: &str, root: &str) -> Result<U256, ChainError> {
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
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
    ) -> Result<Option<String>, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.root_issuers
            .get(&(factory_addr.to_lowercase(), root.to_lowercase()))
            .cloned())
    }
    async fn issued_by(&self, issuer_addr: &str, root: &str) -> Result<Option<String>, ChainError> {
        let g = self.inner.lock().unwrap();
        let addr = issuer_addr.to_lowercase();
        if let Some(h) = g.hostile.get(&addr) {
            // The zero address is what a real contract returns for "never issued"; the trait maps it
            // to `None`, so the fake must too rather than reporting a literal zero string.
            let is_zero = h
                .issued_by
                .trim_start_matches("0x")
                .chars()
                .all(|c| c == '0');
            return Ok((!is_zero).then(|| h.issued_by.clone()));
        }
        Ok(g.issued_by.get(&(addr, root.to_lowercase())).cloned())
    }
    async fn issuer_record_type(&self, issuer_addr: &str) -> Result<Option<String>, ChainError> {
        let g = self.inner.lock().unwrap();
        let addr = issuer_addr.to_lowercase();
        if let Some(h) = g.hostile.get(&addr) {
            return Ok(Some(h.record_type.clone()));
        }
        Ok(g.record_types.get(&addr).cloned())
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
    async fn issuance_capability(
        &self,
        issuer_addr: &str,
        signer: &str,
    ) -> Result<IssuanceCapability, ChainError> {
        let g = self.inner.lock().unwrap();
        let clone = issuer_addr.to_lowercase();
        let governing = g
            .governing_registry
            .get(&clone)
            .cloned()
            .unwrap_or_else(|| g.default_registry.clone());
        if governing.is_empty() || g.unanswerable_registries.contains(&governing) {
            return Ok(IssuanceCapability::Undetermined);
        }
        let can_issue = g
            .issuance_capabilities
            .get(&(governing, clone, signer.to_lowercase()))
            .copied()
            .unwrap_or(false);
        Ok(if can_issue {
            IssuanceCapability::Authorized
        } else {
            IssuanceCapability::NotAuthorized
        })
    }
    /// Composed from the same reads the Alloy implementation makes, in the same order, so this fake
    /// models the real answer rather than short-circuiting to one.
    async fn whitelisted_at_issuance(
        &self,
        issuer_addr: &str,
        signer: &str,
        root: &str,
    ) -> Result<GrantAtIssuance, ChainError> {
        let g = self.inner.lock().unwrap();
        let clone = issuer_addr.to_lowercase();
        let governing = g
            .governing_registry
            .get(&clone)
            .cloned()
            .unwrap_or_else(|| g.default_registry.clone());
        if governing.is_empty() || g.unanswerable_registries.contains(&governing) {
            // No authority to ask — the fake's counterpart of an initialized clone answering zero.
            return Ok(GrantAtIssuance::Undetermined);
        }
        let Some(anchored_at) = g
            .root_issued_at
            .get(&(clone.clone(), root.to_lowercase()))
            .copied()
        else {
            return Ok(GrantAtIssuance::Undetermined);
        };
        // Keyed on the SIGNER alone, exactly as `RightsSet`'s indexed topic is. `clone` selected the
        // governing AUTHORITY above and no longer narrows the history.
        let _ = &clone;
        let history = g
            .grants
            .get(&(governing, signer.to_lowercase()))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        // An EMPTY history is a DEFINITE refusal here, and that is not a shortcut: `issue()` is
        // `onlyIssuanceCapable`, so an honest clone cannot have anchored without a grant. A read
        // that FAILED returned above rather than arriving as an empty one.
        Ok(grant_in_force_at(history, anchored_at))
    }
    async fn issuance_allowed_roster(
        &self,
        issuer_addr: &str,
        also: Option<&str>,
    ) -> Result<IssuanceRoster, ChainError> {
        let g = self.inner.lock().unwrap();
        let clone = issuer_addr.to_lowercase();
        // The LOG read, and only it, is what the switch fails — the owner `eth_call` below still
        // answers, which is the asymmetry a real range-capping peer produces.
        if g.failing_issuance_allowed_log_reads {
            return Err(ChainError::Rpc("issuance-allowed log read failed".into()));
        }
        let owner = g.clone_owners.get(&clone).cloned().unwrap_or_default();
        let named = g
            .issuance_allowed_named
            .get(&clone)
            .cloned()
            .unwrap_or_default();
        // STORAGE supplies every value, exactly as the live implementation does. Built for the union
        // of log-named addresses and `also`, so the roster shaper never has to invent one.
        let mut values = std::collections::BTreeMap::new();
        for who in named.iter().cloned().chain(also.map(normalize_addr)) {
            let v = g
                .issuance_allowed
                .get(&(clone.clone(), who.clone()))
                .copied()
                .unwrap_or(false);
            values.insert(who, v);
        }
        Ok(IssuanceRoster {
            owner,
            entries: build_roster(&named, also, &values),
        })
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
            // `rootIndex.registerRoot(r)` (`DogTagIssuer.sol:56`), write-once and `isClone`-gated. Only
            // recorded when this fake models a chain that HAS a factory: "issued, but the factory has
            // no record of it" is a state a verifier must be able to see, so it stays reachable.
            //
            // STRICTLY write-once, and it REVERTS on a second claimant rather than quietly keeping the
            // first (`DogTagIssuerFactory.sol:52`, `require(rootIssuer[root] == address(0), "root
            // taken")`). Keeping the first would let this fake model a chain where two clones both
            // report a nonzero `issuedAt` for one root — a state the real chain forbids — so a forgery
            // test built on it could pass for the wrong reason. Checked BEFORE any mutation, because a
            // real revert leaves no partial state behind.
            let root_idx =
                (!self.factory.is_empty()).then(|| (self.factory.clone(), root.to_lowercase()));
            if root_idx
                .as_ref()
                .is_some_and(|i| g.root_issuers.contains_key(i))
            {
                return Err(ChainError::Other("root taken".into()));
            }
            g.issued.insert(key.clone(), ts);
            // `emit RootIssued(...)` (`DogTagIssuer.sol:57`) — positioned in the SAME synthetic log
            // stream the registry's grants take their places from, which is what makes "granted, then
            // anchored, then delisted" an orderable sequence in this fake rather than three booleans.
            let at = g.next_log_point();
            g.root_issued_at.insert(key.clone(), at);
            // `issuedBy[r] = msg.sender` (`DogTagIssuer.sol:55`) — the H-1 originator the mandatory
            // issuer-whitelist pillar resolves instead of asking an operator to type a signer in.
            g.issued_by.insert(key, signer.clone());
            if let Some(idx) = root_idx {
                g.root_issuers.insert(idx, to_l.clone());
            }
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
    async fn root_issuer(
        &self,
        factory_addr: &str,
        root: &str,
    ) -> Result<Option<String>, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let r = c
            .rootIssuer(parse_b256(root))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // The zero address is a SUCCESSFUL read saying "this factory has no record of this root".
        // Returned as `None` so a caller cannot confuse it with a read that never happened.
        Ok((!r._0.is_zero()).then(|| r._0.to_string().to_lowercase()))
    }
    async fn issued_by(&self, issuer_addr: &str, root: &str) -> Result<Option<String>, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let r = c
            .issuedBy(parse_b256(root))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // Zero == "this clone never issued this root". Returned as `None` so callers treat it as
        // indeterminate rather than asking the registry whether the ZERO address is whitelisted — a
        // definite `false` for entirely the wrong reason.
        Ok((!r._0.is_zero()).then(|| r._0.to_string().to_lowercase()))
    }
    async fn issuer_record_type(&self, issuer_addr: &str) -> Result<Option<String>, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuer::new(parse_addr(issuer_addr), provider);
        let r = c
            .recordType()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // An all-zero word means uninitialized / not a clone: indeterminate, never a record type.
        Ok((!r._0.is_zero()).then(|| format!("0x{}", hex::encode(r._0.as_slice()))))
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
        let c = IProviderAuthority::new(parse_addr(registry_addr), provider);
        let r = c
            .isWhitelistedFor(parse_b256(record_type), parse_addr(signer))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }
    async fn issuance_capability(
        &self,
        issuer_addr: &str,
        signer: &str,
    ) -> Result<IssuanceCapability, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        // (1) WHICH authority answers — off the clone, never off this deployment's config. That is
        // what makes this read generation-correct with no operator-asserted generation flag, and it
        // is what removes the record-type key shape from `ISSUER_REGISTRY_ADDR`.
        let governing = IDogTagIssuer::new(parse_addr(issuer_addr), provider.clone())
            .registry()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            ._0;
        if governing.is_zero() {
            return Ok(IssuanceCapability::Undetermined);
        }

        // (2) MAY THIS SIGNER ANCHOR NOW? `canIssue`, NOT the wider `isRecognizedIssuer`:
        // `DogTagIssuer.issue` is gated by `onlyIssuanceCapable`, which is literally
        // `registry.canIssue(address(this), msg.sender)`. A preflight built on a wider rung passes
        // where the write reverts, which is the one thing a preflight exists to prevent.
        //
        // A failed call is UNDETERMINED and never a refusal. There is deliberately no value
        // fallback: inventing `false` here would render a transport failure as "not approved for
        // this recordType yet" — could-not-check as a definite answer, on the path that decides
        // whether an operator can issue at all.
        match IProviderAuthority::new(governing, provider)
            .canIssue(parse_addr(issuer_addr), parse_addr(signer))
            .call()
            .await
        {
            Ok(r) => Ok(if r._0 {
                IssuanceCapability::Authorized
            } else {
                IssuanceCapability::NotAuthorized
            }),
            Err(_) => Ok(IssuanceCapability::Undetermined),
        }
    }
    async fn whitelisted_at_issuance(
        &self,
        issuer_addr: &str,
        signer: &str,
        root: &str,
    ) -> Result<GrantAtIssuance, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        // (1) WHICH authority answers. Off the clone, never off this deployment's config.
        let governing = IDogTagIssuer::new(parse_addr(issuer_addr), provider.clone())
            .registry()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            ._0;
        if governing.is_zero() {
            // An initialized clone never answers zero here (`initialize` rejects it), so there is no
            // authority to ask. Undetermined, not an accusation.
            return Ok(GrantAtIssuance::Undetermined);
        }

        // (2) WHEN this root was anchored, as a log point. `issuedAt` is a unix TIMESTAMP and cannot
        // be compared against a log's height without a timestamp->block search; the anchoring event
        // carries the height directly.
        let anchoring = provider
            .get_logs(
                &Filter::new()
                    .address(parse_addr(issuer_addr))
                    .event_signature(IDogTagIssuer::RootIssued::SIGNATURE_HASH)
                    .topic1(parse_b256(root))
                    .from_block(0u64),
            )
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // Write-once `issuedAt` makes a second `RootIssued` impossible on an honest clone; take the
        // FIRST regardless, so a clone that somehow emitted twice cannot move the anchoring later.
        let Some(anchored_at) = anchoring.iter().filter_map(log_point).min() else {
            return Ok(GrantAtIssuance::Undetermined);
        };

        // (3) That authority's grant history for this SIGNER. `account` is indexed, so one filtered
        // `eth_getLogs` reconstructs the whole sequence in log order.
        //
        // There is no service in this filter, because the grant carries none: rights are keyed on the
        // address alone. `issuer_addr` still selected the AUTHORITY above (step 2 resolved it from the
        // clone's own `registry()`), so a mis-paired client still cannot answer from the wrong
        // registry's log — what it no longer does is narrow the history to one service. The separate
        // check that the DOCUMENT's claimed record type matches `clone.recordType()` stays at the
        // caller, where a relabelled credential is still refused.
        let grants = provider
            .get_logs(
                &Filter::new()
                    .address(governing)
                    .event_signature(IProviderAuthority::RightsSet::SIGNATURE_HASH)
                    .topic1(parse_addr(signer).into_word())
                    .from_block(0u64),
            )
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let mut history: Vec<GrantEvent> = Vec::with_capacity(grants.len());
        for l in &grants {
            // A grant whose position is unknown cannot be sequenced, and dropping it could turn a
            // withdrawn-before into an authorised. Refuse to answer instead.
            let Some(at) = log_point(l) else {
                return Ok(GrantAtIssuance::Undetermined);
            };
            // `rights` is the one NON-indexed argument, so it is the single data word — the address's
            // whole settable mask after that write. A body that does not decode is a malformed log
            // rather than a fact about the credential.
            let Ok(decoded) = IProviderAuthority::RightsSet::decode_log_data(l.data(), true) else {
                return Ok(GrantAtIssuance::Undetermined);
            };
            history.push(GrantEvent {
                at,
                // Masked in the FULL 256-bit width. Truncating to a u64 first is harmless for every
                // mask this contract can emit today, which is precisely why the habit would survive
                // review until a right is allocated above bit 63.
                granted: (decoded.rights & U256::from(dogtag_standard::verify::RIGHT_ISSUE))
                    != U256::ZERO,
            });
        }

        // (4) An EMPTY history is a DEFINITE refusal, and that is evidence about the credential
        // rather than about us: `issue()` is `onlyIssuanceCapable`, so an honest clone cannot have
        // anchored this root without the registrar having granted this signer the capability. A read
        // that FAILED never reaches here — it returned above.
        //
        // The pillar folds the registrar's raw capability grant and deliberately NOT `canIssue`,
        // which additionally folds live-lifecycle terms (provider standing, service standing, the
        // current pointer). Those can change after issuance, so folding them would turn an ordinary
        // repoint or suspension into a forgery verdict against credentials that were genuinely
        // issued — the current-state-getter mistake this pillar exists to avoid.
        Ok(grant_in_force_at(&history, anchored_at))
    }
    async fn issuance_allowed_roster(
        &self,
        issuer_addr: &str,
        also: Option<&str>,
    ) -> Result<IssuanceRoster, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let clone = parse_addr(issuer_addr);
        let issuer = IDogTagIssuer::new(clone, provider.clone());

        // (1) WHO may admit. `Ownable2Step`, so this is the accepted owner rather than a pending one.
        let owner = issuer
            .owner()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            ._0;

        // (2) WHICH addresses this list has an opinion about. `signer` is indexed, so one filtered
        // `eth_getLogs` names the whole set — including the creation seed, which `initialize` emits
        // through this same event rather than a second topic.
        let logs = provider
            .get_logs(
                &Filter::new()
                    .address(clone)
                    .event_signature(IDogTagIssuer::IssuanceAllowedSet::SIGNATURE_HASH)
                    .from_block(0u64),
            )
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let mut named: Vec<String> = Vec::with_capacity(logs.len());
        for l in &logs {
            // A log carrying no position is a log the node has not mined. Nothing here SEQUENCES, so
            // it could be tolerated — the value comes from storage either way, and a pending write
            // would correctly still read `false`. It is refused anyway, for one reason that is about
            // honesty rather than safety: `ever_named` is what separates "withdrawn" from "never
            // admitted", and an unmined log would report a merely-pending admit as a withdrawal.
            // Refusing keeps this reader's posture identical to every other log reader in the repo
            // rather than making it the one that reasons about positions differently.
            if log_point(l).is_none() {
                return Err(ChainError::Rpc(
                    "an IssuanceAllowedSet log carried no block position".into(),
                ));
            }
            // ONLY the indexed address is taken. The event's own `allowed` word is deliberately never
            // read — see step 3.
            let Some(topic) = l.topics().get(1) else {
                return Err(ChainError::Rpc(
                    "an IssuanceAllowedSet log carried no signer topic".into(),
                ));
            };
            named.push(format!("{:#x}", Address::from_word(*topic)));
        }

        // (3) WHAT each of them may do NOW, from STORAGE. This is the inversion the whole module is
        // built on: the log is an index of addresses to ask about, never a source of answers. A
        // pending write therefore cannot read as a completed grant, and no ordering is needed because
        // the chain has already folded the history into this mapping.
        let mut values = std::collections::BTreeMap::new();
        for who in named.iter().cloned().chain(also.map(normalize_addr)) {
            if values.contains_key(&who) {
                continue;
            }
            let allowed = issuer
                .issuanceAllowed(parse_addr(&who))
                .call()
                .await
                .map_err(|e| ChainError::Rpc(e.to_string()))?
                ._0;
            values.insert(who, allowed);
        }

        Ok(IssuanceRoster {
            owner: format!("{owner:#x}"),
            entries: build_roster(&named, also, &values),
        })
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

    /// `DogTagIssuerFactory.registerRoot` is STRICTLY write-once and REVERTS on a second claimant
    /// (`DogTagIssuerFactory.sol:52`). The fake must revert too rather than quietly keeping the
    /// first: `issued` is keyed per-clone, so a silent keep would let two clones both report a
    /// nonzero `issuedAt` for one root — a state the real chain cannot reach, and one a forgery test
    /// could then pass against for entirely the wrong reason.
    #[tokio::test]
    async fn a_second_clone_cannot_claim_a_root_the_factory_already_indexed() {
        const FACTORY: &str = "0x00000000000000000000000000000000000000fa";
        const CLONE_A: &str = "0x00000000000000000000000000000000000000a1";
        const CLONE_B: &str = "0x00000000000000000000000000000000000000b1";
        let root = format!("0x{:064x}", 7u8);

        let mem = MemChain::new().with_factory(FACTORY);
        mem.set_signer(0, "0x00000000000000000000000000000000000000c0");

        mem.sign_and_send(0, CLONE_A, &issue_calldata(&root))
            .await
            .expect("the first clone registers the root");

        let e = mem
            .sign_and_send(0, CLONE_B, &issue_calldata(&root))
            .await
            .expect_err("a second clone must not be able to claim the same root");
        assert!(format!("{e}").contains("root taken"), "{e}");

        // The rejected issue left NO partial state: the second clone must not report the root as
        // issued, or the fake still models the impossible world just one field over.
        assert!(mem.issued_at(CLONE_B, &root).await.unwrap().is_zero());
        assert_eq!(
            mem.root_issuer(FACTORY, &root).await.unwrap().as_deref(),
            Some(CLONE_A)
        );
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
