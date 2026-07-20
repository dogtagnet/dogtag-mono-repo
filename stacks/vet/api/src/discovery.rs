//! Server-side trust-tier assembly (M7 §5.2 / §5.3, brick P4).
//!
//! Maps the P3 discovery-anchor types (`dogtag_prover::manifest`) into the pure, prover-free
//! [`dogtag_standard::discovery::TrustedAnchor`] the shared validator consumes. This is the seam that
//! keeps the standard crate — which the mobile app links — free of any dependency on the ark-heavy
//! prover crate: the field-copy mapping lives HERE, in vet-api, which already links both.
//!
//! Why the manifest is the source of the READABLE trust fields: the on-chain
//! `ProtocolRegistry.getContractSet`
//! returns `circuitId` as a keccak `bytes32` and carries no version STRING and no chain id, whereas the
//! signed manifest mirrors the same version with the human-readable `version`/`circuit_id`/`chain_id`
//! the validator needs (§3.2). The on-chain record is still the ROOT OF TRUTH — its precedence is
//! enforced by P3 [`dogtag_prover::manifest::reconcile`] BEFORE a manifest is allowed to feed the
//! validator, via [`anchor_from_reconciliation`].

use dogtag_prover::manifest::{FieldConflict, Manifest, Reconciliation};
use dogtag_standard::discovery::TrustedAnchor;

/// Assemble a [`TrustedAnchor`] from a signed-manifest CONTENT (§5.2 TRUST tier, the 1B fallback).
///
/// The two lifecycle bits are supplied separately because the manifest carries NEITHER — they are
/// on-chain members, one per axis (R-5). This is the OFFLINE (manifest-only) path, whose callers pass
/// `true` for both: a served manifest is presumed active, and deprecation is authoritative only on-chain
/// (`minAppVersion` remains enforced offline). The ONLINE path must go through
/// [`anchor_from_reconciliation`], which sources both bits from the chain itself.
pub fn anchor_from_manifest(m: &Manifest, contract_set_active: bool, artifact_set_active: bool) -> TrustedAnchor {
    TrustedAnchor {
        version: m.version.clone(),
        version_id: m.version_id.clone(),
        // The ARTIFACT axis (R-5). The manifest carries both halves, so the offline path can name the
        // artifact set it describes without a second resolution.
        artifact_set: m.artifact_set.clone(),
        artifact_set_id: m.artifact_set_id.clone(),
        chain_id: m.chain_id,
        verification_registry: m.verification_registry.clone(),
        circuit_id: m.circuit_id.clone(),
        min_app_version: m.min_app_version.clone(),
        contract_set_active,
        artifact_set_active,
    }
}

/// Assemble a [`TrustedAnchor`] from a manifest ALREADY reconciled against the on-chain record (P3
/// [`dogtag_prover::manifest::reconcile`]), enforcing on-chain precedence.
///
/// Returns `Err(conflicts)` if the signed manifest disagrees with the chain on ANY field — so a
/// stale/compromised manifest can never feed the validator (on-chain wins, exactly as P3 designed). On
/// agreement the readable fields come from the (verified, agreeing) manifest.
///
/// Neither lifecycle bit is a parameter: both are sourced from the reconciled ON-CHAIN records, because a
/// caller-supplied lifecycle bit would leave the anti-downgrade check (§8.4) attested by nobody. This is
/// what wires `deprecateContractSet`/`deprecateArtifactSet` end-to-end: a set deprecated on-chain yields
/// a false bit here, and [`dogtag_standard::discovery::validate`] then fails closed with
/// `DeprecatedVersion`, naming the axis.
///
/// Since R-5 there are TWO independent `active` bits — one per axis — and they are carried through
/// SEPARATELY rather than pre-combined here. `validate` requires both, so deprecating EITHER the
/// on-chain contract set OR the bound proving-artifact set still refuses the version; keeping them
/// distinct is what lets the abort say WHICH lever fired, and mirrors the shape a native app must
/// build from `getContractSet` + `getActiveArtifactSet`.
pub fn anchor_from_reconciliation(r: &Reconciliation) -> Result<TrustedAnchor, Vec<FieldConflict>> {
    if !r.manifest_agrees() {
        return Err(r.conflicts.clone());
    }
    Ok(anchor_from_manifest(&r.manifest, r.contract_set.active, r.artifact_set.active))
}
