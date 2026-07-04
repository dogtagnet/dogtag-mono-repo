//! The canonical, **non-PII** indexed-event model.
//!
//! Every row the indexer stores is one on-chain log flattened into a uniform envelope (block / tx /
//! log-index / timestamp / emitting contract) plus the event-specific, non-personal payload. Nothing
//! here is or derives from personal data: roots are salted Poseidon commitments, `dogTagId` is a
//! non-personal SBT id, and every address is a public on-chain signer/clone — exactly the
//! "chain = shared non-PII substrate" doctrine (arch §4, `dogtag-govarch-r8` Part 4).
//!
//! The `id` (`{txHash}:{logIndex}`) is the natural idempotency key: re-scanning a block range (resume
//! or a chunk overlap) re-derives the same id, so an upsert is a no-op rather than a duplicate.

use serde::{Deserialize, Serialize};

/// The seven oversight event kinds the arch enumerates (govarch §4.3 / adminportal §3.2). Serialized
/// as a stable lowerCamel string so the query API + any consumer filters on a fixed vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventType {
    /// `DogTagIssuerFactory.IssuerCreated(clone, recordType, name)` — a new issuer clone deployed.
    IssuerCreated,
    /// `DogTagIssuerFactory.RootRegistered(root, clone)` — write-once root→clone binding.
    RootRegistered,
    /// `IssuerRegistry.Whitelisted(recordType, signer)` — a signer gained issue/verify capability.
    Whitelisted,
    /// `IssuerRegistry.Delisted(recordType, signer)` — a signer lost that capability.
    Delisted,
    /// `DogTagIssuer.RootIssued(root, by, ts)` — a credential root was anchored.
    RootIssued,
    /// `DogTagIssuer.RootRevoked(root, by, ts)` — a credential root was invalidated.
    RootRevoked,
    /// `VerificationRegistry.Verified(dogTagId, relayer, subject, purpose, nullifier, ts)`.
    Verified,
}

/// The finality lifecycle of an indexed event (captain-directed finality-aware model). ROAX is an
/// EVM/PoS chain with a finality gadget, so a **finalized** block can never reorg — its events are
/// immutable and never rewound. A **pending** block (height > the finalized watermark) is still
/// reorg-able and is the ONLY range the reorg logic ever touches. A government oversight feed must
/// distinguish the two so it never treats a pre-finality, reorg-able issuance as authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Finality {
    /// block height > finalized watermark — reorg-able, in-flight.
    Pending,
    /// block height <= finalized watermark — immutable, settled.
    Finalized,
}

impl Finality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Finality::Pending => "pending",
            Finality::Finalized => "finalized",
        }
    }
    pub fn parse(s: &str) -> Option<Finality> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Finality::Pending),
            "finalized" => Some(Finality::Finalized),
            _ => None,
        }
    }
}

impl EventType {
    /// The stable wire token used in JSON + the `?type=` query filter.
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::IssuerCreated => "issuerCreated",
            EventType::RootRegistered => "rootRegistered",
            EventType::Whitelisted => "whitelisted",
            EventType::Delisted => "delisted",
            EventType::RootIssued => "rootIssued",
            EventType::RootRevoked => "rootRevoked",
            EventType::Verified => "verified",
        }
    }

    /// Parse a `?type=` filter token (case-insensitive) back to an `EventType`.
    pub fn parse(s: &str) -> Option<EventType> {
        match s.trim().to_ascii_lowercase().as_str() {
            "issuercreated" => Some(EventType::IssuerCreated),
            "rootregistered" => Some(EventType::RootRegistered),
            "whitelisted" => Some(EventType::Whitelisted),
            "delisted" => Some(EventType::Delisted),
            "rootissued" => Some(EventType::RootIssued),
            "rootrevoked" => Some(EventType::RootRevoked),
            "verified" => Some(EventType::Verified),
            _ => None,
        }
    }
}

/// One flattened, non-PII on-chain event. All hex fields are lowercase `0x…`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexedEvent {
    /// `{txHash}:{logIndex}` — the idempotency / primary key (stable across re-scans).
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: EventType,

    // ---- provenance (uniform envelope) --------------------------------------------------------
    /// The contract that emitted the log: the factory, the registry, the VerificationRegistry, or a
    /// specific `DogTagIssuer` clone. Load-bearing for clone-scoping + anti-spoof gating.
    #[serde(rename = "contract")]
    pub contract: String,
    #[serde(rename = "blockNumber")]
    pub block_number: u64,
    #[serde(rename = "blockHash")]
    pub block_hash: String,
    #[serde(rename = "txHash")]
    pub tx_hash: String,
    #[serde(rename = "logIndex")]
    pub log_index: u64,
    /// Block timestamp (Unix seconds) resolved from the block header — always present so the feed is
    /// time-filterable/sortable regardless of whether the event carries its own `ts`.
    #[serde(rename = "blockTimestamp")]
    pub block_timestamp: u64,
    /// Finality lifecycle state — `finalized` (immutable, settled) vs `pending` (still reorg-able).
    /// The scanner assigns this from the finalized watermark; the reorg logic only touches `pending`.
    #[serde(rename = "finality")]
    pub finality: Finality,

    // ---- scoping keys (server-side enforcement) -----------------------------------------------
    /// The signer that performed the action — `by` (RootIssued/Revoked), `signer`
    /// (Whitelisted/Delisted), or `relayer` (Verified). `None` for factory events. This is the
    /// per-signer scope key: a vet/groomer sees an event iff `actor ∈ its signers` OR the event's
    /// clone ∈ its clones.
    #[serde(rename = "actor", skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The `DogTagIssuer` clone an event pertains to (the emitting clone for RootIssued/Revoked, the
    /// created clone for IssuerCreated/RootRegistered). `None` for registry / VerificationRegistry
    /// events, which are not clone-scoped. This is the per-clone scope key.
    #[serde(rename = "clone", skip_serializing_if = "Option::is_none")]
    pub clone: Option<String>,

    // ---- event-specific payload (all non-PII; absent fields omitted) ---------------------------
    /// keccak256(recordType) — the on-chain record-type key (IssuerCreated/Whitelisted/Delisted).
    #[serde(rename = "recordType", skip_serializing_if = "Option::is_none")]
    pub record_type: Option<String>,
    /// Human issuer name from `IssuerCreated`.
    #[serde(rename = "name", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The anchored Poseidon root `R` (RootIssued/RootRevoked/RootRegistered).
    #[serde(rename = "root", skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The non-personal SBT id (Verified).
    #[serde(rename = "dogTagId", skip_serializing_if = "Option::is_none")]
    pub dog_tag_id: Option<String>,
    /// The credential subject wallet address (Verified) — a public on-chain address, not PII.
    #[serde(rename = "subject", skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// keccak/BN254 purpose key (Verified).
    #[serde(rename = "purpose", skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// The one-time verification nullifier (Verified) — a Poseidon image, unlinkable.
    #[serde(rename = "nullifier", skip_serializing_if = "Option::is_none")]
    pub nullifier: Option<String>,
    /// The on-chain `ts` field carried by RootIssued/RootRevoked/Verified (== emit block timestamp).
    #[serde(rename = "onchainTs", skip_serializing_if = "Option::is_none")]
    pub onchain_ts: Option<u64>,
}

impl IndexedEvent {
    /// Deterministic id for a log: `{txHash}:{logIndex}` (lowercase). Re-scanning yields the same id.
    pub fn make_id(tx_hash: &str, log_index: u64) -> String {
        format!("{}:{}", tx_hash.to_ascii_lowercase(), log_index)
    }
}
