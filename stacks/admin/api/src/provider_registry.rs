//! The `ProviderRegistry` REGISTRAR surface - the admin-side half of the generation-2 provider
//! journey (registry plan C-2, the part `docs/CUTOVER_REHEARSAL.md` records as still outstanding).
//!
//! `registerProvider` and `setServiceCreationApproval` were callable only from contracts and tests:
//! no portal called either, so `providerCount()` was 0 and every provider self-service action
//! refused. This module is the pure core of the surface that closes that - the value types, the
//! calldata builders, and the approval fold - with the chain I/O in `chain.rs` and the HTTP routes
//! in `routes.rs`.
//!
//! ## Three registrar steps, not two
//!
//! `registerProvider` writes `standing: Standing.PENDING` (`ProviderRegistry.sol:300`), and
//! `canWriteProvider` returns false for anything but `ACTIVE` (`:458`) - which `canCreateService`
//! folds (`:834`). So registering and approving a record type is NOT sufficient: without
//! `setProviderStanding(providerId, ACTIVE)` the provider's own deploy preflight reads
//! `provider-standing: fail` forever and no clone can be created. The standing transition is
//! therefore part of this surface rather than a later slice; a registrar screen offering only the
//! two named calls would hand the captain a provider that can never act.
//!
//! ## The approval bit has NO getter, and that shapes the read surface
//!
//! `_serviceCreationApprovals` is `private` with no accessor (`ProviderRegistry.sol:137`). The only
//! on-chain read is `canCreateService`, which is an AGGREGATE of four terms - a registered factory
//! as `msg.sender`, an active generation, the approval bit, and `canWriteProvider` - so a `false`
//! from it is unattributable and is emphatically NOT "this record type is not approved". The raw bit
//! is therefore reconstructed from `ServiceCreationApprovalSet` logs, where both `providerId` and
//! `recordType` are indexed. A log read that FAILS is `ApprovalsRead::Unavailable`, never an empty
//! approval set: "we could not ask" and "nothing is approved" send an admin to different places.

use serde::Serialize;
use std::collections::BTreeMap;

/// `ProviderRegistry.Standing` (`ProviderRegistry.sol:58-64`), shared by providers and services.
///
/// `NONE` is also what a zero-filled struct reads back as for an unregistered id, so standing alone
/// cannot distinguish "never registered" from "registered as NONE" - existence is `controller != 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    None,
    Pending,
    Active,
    Suspended,
    Retired,
}

impl Standing {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Standing::Pending,
            2 => Standing::Active,
            3 => Standing::Suspended,
            4 => Standing::Retired,
            _ => Standing::None,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Standing::None => 0,
            Standing::Pending => 1,
            Standing::Active => 2,
            Standing::Suspended => 3,
            Standing::Retired => 4,
        }
    }

    /// The wire spelling accepted by the standing route.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Some(Standing::Active),
            "suspended" => Some(Standing::Suspended),
            "retired" => Some(Standing::Retired),
            _ => None,
        }
    }

    /// `_requireSettableStanding` (`ProviderRegistry.sol:1102-1106`) admits only these three;
    /// `NONE` and `PENDING` revert `InvalidStanding()`, so a screen must not offer them.
    pub fn is_settable(self) -> bool {
        matches!(self, Standing::Active | Standing::Suspended | Standing::Retired)
    }
}

/// A provider record as the registrar screen needs it (`ProviderRegistry.Provider`, `:71-80`).
#[derive(Clone, Debug, Serialize)]
pub struct ProviderRecord {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub controller: String,
    #[serde(rename = "pendingController")]
    pub pending_controller: String,
    #[serde(rename = "controllerEpoch")]
    pub controller_epoch: u64,
    pub standing: Standing,
    /// `controller != 0` is the existence sentinel - `provider()` does not revert for an unknown id.
    pub registered: bool,
}

/// The registrar-written identity anchor (`ProviderRegistry.PublicIdentityAnchor`, `:84-92`).
///
/// Deliberately NOT the same thing as `ProviderDirectory`'s `ProfileAnchor`: that one is
/// PROVIDER-written content (contacts, logo, address text), this one is the REGISTRAR's own
/// assertion about who it cleared. They live on different contracts under different schema
/// namespaces and must never be conflated - see `PROVIDER_IDENTITY_SCHEMA`.
#[derive(Clone, Debug, Serialize)]
pub struct IdentityAnchor {
    pub digest: String,
    pub schema: u32,
    pub codec: u16,
    #[serde(rename = "hashAlgorithm")]
    pub hash_algorithm: u8,
    pub revision: u64,
    #[serde(rename = "updatedAtBlock")]
    pub updated_at_block: u64,
}

/// Schema id of the registrar's KYC identity statement, version 1.
///
/// This is the `ProviderRegistry.publicIdentityAnchor` schema namespace, which is SEPARATE from
/// `ProviderDirectory`'s `ProfileAnchor` schema namespace (where `2` already means
/// `dogtag/provider-profile/2`, S-17). Two different contracts, two different mappings, two
/// different authors - a registrar assertion must never be published under a provider-written
/// schema, because a consumer would then read the registrar's clearance as the provider's own claim.
/// An anchor is only ever obtained by calling one of those two contracts, so the namespaces cannot
/// be reached ambiguously; the distinct name and value are what keep them from being conflated by
/// hand.
pub const PROVIDER_IDENTITY_SCHEMA: u32 = 1;
pub const PROVIDER_IDENTITY_SCHEMA_ID: &str = "dogtag/provider-identity/1";

/// keccak-256's multicodec code - the repo's existing spelling for "this digest is a keccak256".
/// Naming sha2-256 (`0x12`) because it is the commoner constant would make a verifier recompute the
/// wrong hash and call a genuine statement altered.
pub const HASH_ALGORITHM_KECCAK256: u8 = 0x1b;

/// The registrar publishes no content for the identity statement, so there is nothing to address.
/// Same reasoning as S-17's empty `contenthash`: an identifier for bytes nobody serves is a claim
/// that cannot be checked.
pub const IDENTITY_CODEC_NONE: u16 = 0;

/// One `(recordType, allowed)` pair as last written by the registrar.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ApprovalEntry {
    /// The `bytes32` record-type key exactly as it appears in the log topic.
    #[serde(rename = "recordTypeKey")]
    pub record_type_key: String,
    /// The human label when the key is one of the record types this deployment knows, else `null`.
    /// keccak is one-way, so an unknown key genuinely has no recoverable label - never invent one.
    #[serde(rename = "recordType")]
    pub record_type: Option<String>,
    pub allowed: bool,
}

/// The result of reading a provider's service-creation approvals.
///
/// Two members, and collapsing them is the defect this type exists to prevent: an empty
/// `Resolved` says the registrar has approved nothing, while `Unavailable` says we could not ask.
/// Rendering the second as the first tells an admin a provider is unapproved on the strength of a
/// read that never happened.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ApprovalsRead {
    Resolved { entries: Vec<ApprovalEntry> },
    Unavailable { reason: String },
}

impl ApprovalsRead {
    /// The entries when the read resolved. `None` when it did not - callers must branch rather than
    /// defaulting to empty.
    pub fn entries(&self) -> Option<&[ApprovalEntry]> {
        match self {
            ApprovalsRead::Resolved { entries } => Some(entries),
            ApprovalsRead::Unavailable { .. } => None,
        }
    }

    /// Whether `record_type_key` is currently approved, as a tri-state: `Some(true)`/`Some(false)`
    /// when the log resolved, `None` when it did not.
    pub fn approved(&self, record_type_key: &str) -> Option<bool> {
        let entries = self.entries()?;
        Some(
            entries
                .iter()
                .find(|e| e.record_type_key.eq_ignore_ascii_case(record_type_key))
                .map(|e| e.allowed)
                // A record type the log never mentions has never been approved. That IS an answer -
                // the log resolved and contains no grant - so it is a definite `false`, not `None`.
                .unwrap_or(false),
        )
    }
}

/// Fold an ordered `ServiceCreationApprovalSet` log into the current approval set.
///
/// `events` MUST be in ascending `(blockNumber, logIndex)` order; the LAST write for a record type
/// wins, which is what makes this the current state rather than a history. An explicit `false` is
/// KEPT rather than dropped: "the registrar approved this and then withdrew it" is a different fact
/// from "the registrar never approved it", and the screen shows both.
pub fn fold_approvals(
    events: &[(String, bool)],
    label_for: &dyn Fn(&str) -> Option<String>,
) -> Vec<ApprovalEntry> {
    // BTreeMap keeps the output deterministic (keys are fixed-width hex), so two reads of the same
    // chain state render identically rather than in log order.
    let mut latest: BTreeMap<String, bool> = BTreeMap::new();
    for (key, allowed) in events {
        latest.insert(key.to_ascii_lowercase(), *allowed);
    }
    latest
        .into_iter()
        .map(|(record_type_key, allowed)| ApprovalEntry {
            record_type: label_for(&record_type_key),
            record_type_key,
            allowed,
        })
        .collect()
}

/// The record-type labels this deployment can name. keccak is one-way, so a key outside this list
/// is rendered as the raw key - never as a guessed label.
pub const KNOWN_RECORD_TYPES: &[&str] = &[
    "DOG_PROFILE",
    "VACCINATION",
    "TRAVEL_CLEARANCE",
    "EU_HEALTH_CERT",
    "GROOMING",
    "BOARDING",
];

/// Reverse the keccak of a record-type key by trying the labels this deployment knows.
pub fn record_type_label(key: &str) -> Option<String> {
    KNOWN_RECORD_TYPES
        .iter()
        .find(|label| crate::chain::record_type_key(label).eq_ignore_ascii_case(key))
        .map(|label| (*label).to_string())
}

// -------------------------------------------------------------------------------------------------
// Services, capabilities and typed resolvers - the rest of the journey past "the provider deployed".
// -------------------------------------------------------------------------------------------------

/// A service (an attached provider-deployed issuer contract) as the registrar screen needs it.
///
/// `attachService` resolves `recordType`, `factoryGeneration` and `confirmedOwner` off the clone and
/// the pinned factory - none of them is registrar-supplied - so every field here except the standing
/// is a fact about the contract rather than a claim about it.
#[derive(Clone, Debug, Serialize)]
pub struct ServiceRecord {
    #[serde(rename = "serviceAddress")]
    pub service_address: String,
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "factoryGeneration")]
    pub factory_generation: String,
    #[serde(rename = "recordTypeKey")]
    pub record_type_key: String,
    /// The human label when the key is one this deployment knows, else `null` - never a guess.
    #[serde(rename = "recordType")]
    pub record_type: Option<String>,
    #[serde(rename = "confirmedOwner")]
    pub confirmed_owner: String,
    #[serde(rename = "domainResolver")]
    pub domain_resolver: String,
    #[serde(rename = "ownerEpoch")]
    pub owner_epoch: u64,
    pub standing: Standing,
    /// `providerId != 0` is the existence sentinel: `service()` answers a zero-filled struct for an
    /// address it has never seen rather than reverting, exactly like `provider()`.
    pub attached: bool,
}

/// The five lifecycle terms `canIssue` folds, reported APART.
///
/// Never pre-AND them into one bool. Each has a different remedy - a suspended provider is the
/// registrar's to lift, an unconfirmed owner needs `confirmServiceOwner`, an inactive factory
/// generation is terminal - so one bool would tell an admin that something is wrong while
/// withholding the only thing that says what to do about it.
///
/// `has_active_issuer` is the one term that is NOT independent of its neighbours: the contract
/// answers `_issueRightHolders != 0 && _serviceIssuanceEligible(..)`, which re-folds the owner and
/// the standings AND adds the provider's current pointer. So it can be false with all four of the
/// others true, and the remedy in that state is the PROVIDER's `repointService` - never another
/// registrar grant.
///
/// The counter is REGISTRY-WIDE since rights moved onto the address (`_issueRightHolders`, not the
/// removed per-service `_activeIssuerCount`): a grant names no service, so "some signer can issue
/// here" is a property of the whole registry folded against THIS service's lifecycle. Practical
/// consequence for the remedy an admin acts on: on any registry where at least one address already
/// holds `RIGHT_ISSUE`, a false here can ONLY come from the lifecycle half, so a remedy naming a
/// grant would send them to a step that changes nothing.
#[derive(Clone, Debug, Serialize)]
pub struct ServiceEffective {
    #[serde(rename = "providerStanding")]
    pub provider_standing: Standing,
    #[serde(rename = "serviceStanding")]
    pub service_standing: Standing,
    #[serde(rename = "factoryActive")]
    pub factory_active: bool,
    #[serde(rename = "ownerConfirmed")]
    pub owner_confirmed: bool,
    #[serde(rename = "hasActiveIssuer")]
    pub has_active_issuer: bool,
}

/// One `(holder, allowed)` pair as last written by the registrar, for any capability whose raw
/// mapping is private and therefore only knowable from its event log.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct CapabilityEntry {
    /// The address the capability is held by - an issuance signer, or a verifier relayer.
    pub holder: String,
    pub allowed: bool,
}

/// The result of reading a capability log. Same three-state discipline, and same reason, as
/// [`ApprovalsRead`]: an empty `Resolved` says nobody holds this capability, `Unavailable` says we
/// could not ask, and rendering the second as the first states a fact about a service on the
/// strength of a read that never happened.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum CapabilitiesRead {
    Resolved { entries: Vec<CapabilityEntry> },
    Unavailable { reason: String },
}

impl CapabilitiesRead {
    pub fn entries(&self) -> Option<&[CapabilityEntry]> {
        match self {
            CapabilitiesRead::Resolved { entries } => Some(entries),
            CapabilitiesRead::Unavailable { .. } => None,
        }
    }

    /// Whether `holder` currently holds this capability, tri-state: `None` when the log did not
    /// resolve, so a caller must branch rather than defaulting to "not held".
    pub fn allowed(&self, holder: &str) -> Option<bool> {
        let entries = self.entries()?;
        Some(
            entries
                .iter()
                .find(|e| e.holder.eq_ignore_ascii_case(holder))
                .map(|e| e.allowed)
                // A holder the resolved log never mentions has never been granted. That IS an
                // answer, so it is a definite `false` rather than a could-not-check.
                .unwrap_or(false),
        )
    }
}

/// Fold an ordered capability log into the current holder set.
///
/// Identical rule to [`fold_approvals`] and identical precondition: `events` MUST be in ascending
/// `(blockNumber, logIndex)` order, and a withdrawal is KEPT as an explicit `false` because
/// "granted then withdrawn" and "never granted" are different facts about what the registrar did.
pub fn fold_capabilities(events: &[(String, bool)]) -> Vec<CapabilityEntry> {
    let mut latest: BTreeMap<String, bool> = BTreeMap::new();
    for (holder, allowed) in events {
        latest.insert(holder.to_ascii_lowercase(), *allowed);
    }
    latest
        .into_iter()
        .map(|(holder, allowed)| CapabilityEntry { holder, allowed })
        .collect()
}

/// `ProviderRegistry.ResolverKind` (`ProviderRegistry.sol:66-69`).
///
/// A typed resolver answers NOTHING until the registrar approves it here AND the provider selects
/// it, so both halves must be visible - the core never clears a stored selection when a resolver is
/// deapproved, which is exactly why the approval is a separate fleet-wide lever.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolverKind {
    Directory,
    Domain,
}

impl ResolverKind {
    pub fn as_u8(self) -> u8 {
        match self {
            ResolverKind::Directory => 0,
            ResolverKind::Domain => 1,
        }
    }

    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(ResolverKind::Directory),
            1 => Some(ResolverKind::Domain),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "directory" => Some(ResolverKind::Directory),
            "domain" => Some(ResolverKind::Domain),
            _ => None,
        }
    }
}

/// The verify purposes this deployment can name, for the orthogonal verifier axis.
///
/// `setVerifierCapability` is keyed by PURPOSE and not by service - it is the verify axis, which is
/// orthogonal to issuance - so it is never rendered as a property of a service row.
///
/// HAND-MIRRORED from `PURPOSE_LABELS` in `stacks/owner/web/src/lib/consents.ts`, which is the list
/// of labels actually in circulation across the portals; there is no shared source across the
/// language boundary, so the two move together or not at all. The label is keccak-reduced by
/// `purpose_key` into the word the contract stores under `verificationKey(purpose)`, so a label that
/// exists nowhere else grants under a key `canVerify` is never asked about - a transaction that
/// succeeds and grants nothing, reached through the label axis rather than the derived-key one.
pub const KNOWN_VERIFY_PURPOSES: &[&str] = &[
    "boarding_intake",
    "travel_check",
    "grooming_intake",
    "daycare_access",
    "service_animal",
];

/// `providerId` is `bytes20` - 40 hex characters. Zero is the one value the contract refuses
/// (`ZeroProviderId()`, `ProviderRegistry.sol:288`); there is deliberately no format, checksum or
/// derivation rule, because the id is an opaque KYC registrar assignment.
pub fn is_valid_provider_id(s: &str) -> bool {
    let t = s.trim();
    let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) else {
        return false;
    };
    hex.len() == 40 && hex.chars().all(|c| c.is_ascii_hexdigit()) && hex.chars().any(|c| c != '0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_labels(_: &str) -> Option<String> {
        None
    }

    const RT_A: &str = "0x00000000000000000000000000000000000000000000000000000000000000aa";
    const RT_B: &str = "0x00000000000000000000000000000000000000000000000000000000000000bb";

    #[test]
    fn the_last_write_for_a_record_type_wins() {
        let events = vec![
            (RT_A.to_string(), true),
            (RT_B.to_string(), true),
            (RT_A.to_string(), false),
        ];
        let folded = fold_approvals(&events, &no_labels);
        assert_eq!(folded.len(), 2);
        let a = folded.iter().find(|e| e.record_type_key == RT_A).unwrap();
        assert!(!a.allowed, "the later withdrawal must win over the earlier grant");
        let b = folded.iter().find(|e| e.record_type_key == RT_B).unwrap();
        assert!(b.allowed);
    }

    /// A withdrawn approval is KEPT as an explicit `false` rather than dropped. Dropping it would
    /// render "approved then withdrawn" identically to "never approved", and those are different
    /// facts about what the registrar did.
    #[test]
    fn a_withdrawn_approval_is_reported_rather_than_dropped() {
        let folded = fold_approvals(&[(RT_A.to_string(), true), (RT_A.to_string(), false)], &no_labels);
        assert_eq!(folded.len(), 1);
        assert!(!folded[0].allowed);
    }

    /// The whole point of the type: an unreadable log may not answer the approval question at all.
    #[test]
    fn an_unavailable_read_is_not_an_empty_approval_set() {
        let unavailable = ApprovalsRead::Unavailable {
            reason: "eth_getLogs failed".into(),
        };
        assert!(unavailable.entries().is_none());
        assert_eq!(unavailable.approved(RT_A), None);

        let empty = ApprovalsRead::Resolved { entries: vec![] };
        assert_eq!(empty.entries().map(<[ApprovalEntry]>::len), Some(0));
        // A log that resolved and mentions nothing IS evidence: nothing was ever approved.
        assert_eq!(empty.approved(RT_A), Some(false));
    }

    #[test]
    fn a_record_type_the_log_never_mentions_is_a_definite_no() {
        let read = ApprovalsRead::Resolved {
            entries: fold_approvals(&[(RT_A.to_string(), true)], &no_labels),
        };
        assert_eq!(read.approved(RT_A), Some(true));
        assert_eq!(read.approved(RT_B), Some(false));
    }

    /// Only ACTIVE/SUSPENDED/RETIRED are settable; the contract reverts `InvalidStanding()` on the
    /// other two, so a screen that offered them would produce a revert the admin cannot act on.
    #[test]
    fn only_the_three_settable_standings_parse() {
        assert_eq!(Standing::parse("active"), Some(Standing::Active));
        assert_eq!(Standing::parse("SUSPENDED"), Some(Standing::Suspended));
        assert_eq!(Standing::parse(" retired "), Some(Standing::Retired));
        assert_eq!(Standing::parse("pending"), None);
        assert_eq!(Standing::parse("none"), None);
        assert!(Standing::Active.is_settable());
        assert!(!Standing::Pending.is_settable());
        assert!(!Standing::None.is_settable());
    }

    #[test]
    fn standing_round_trips_through_its_wire_byte() {
        for s in [
            Standing::None,
            Standing::Pending,
            Standing::Active,
            Standing::Suspended,
            Standing::Retired,
        ] {
            assert_eq!(Standing::from_u8(s.as_u8()), s);
        }
    }

    /// Zero is the ONE providerId the contract refuses. Everything else well-formed is legal,
    /// because the id is opaque by design.
    #[test]
    fn provider_id_validation_refuses_only_malformed_and_zero() {
        assert!(is_valid_provider_id("0x00000000000000000000000000000000000000a1"));
        assert!(is_valid_provider_id("0xABCDEF0123456789abcdef0123456789ABCDEF01"));
        assert!(!is_valid_provider_id(
            "0x0000000000000000000000000000000000000000"
        ));
        assert!(!is_valid_provider_id("0x00a1"));
        assert!(!is_valid_provider_id("00000000000000000000000000000000000000a1"));
        assert!(!is_valid_provider_id(
            "0x00000000000000000000000000000000000000g1"
        ));
    }

    /// The identity schema must not silently be S-17's provider-profile schema - that anchor is
    /// provider-written content and this one is a registrar assertion.
    #[test]
    fn the_identity_schema_is_its_own_namespace_and_is_non_zero() {
        assert_eq!(PROVIDER_IDENTITY_SCHEMA_ID, "dogtag/provider-identity/1");
        // `BadIdentityAnchor()` fires on a zero schema or hash algorithm, so both must be non-zero
        // or every registration reverts.
        assert_ne!(PROVIDER_IDENTITY_SCHEMA, 0);
        assert_ne!(HASH_ALGORITHM_KECCAK256, 0);
        assert_eq!(HASH_ALGORITHM_KECCAK256, 0x1b);
    }

    #[test]
    fn known_record_type_labels_reverse_and_unknown_keys_stay_unlabelled() {
        let key = crate::chain::record_type_key("VACCINATION");
        assert_eq!(record_type_label(&key).as_deref(), Some("VACCINATION"));
        assert_eq!(record_type_label(RT_A), None);
    }
}
