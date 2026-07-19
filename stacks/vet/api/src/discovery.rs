//! Server-side trust-tier assembly (M7 §5.2 / §5.3, brick P4).
//!
//! Maps the P3 discovery-anchor types (`dogtag_prover::manifest`) into the pure, prover-free
//! [`dogtag_standard::discovery::TrustedAnchor`] the shared validator consumes. This is the seam that
//! keeps the standard crate — which the mobile app links — free of any dependency on the ark-heavy
//! prover crate: the field-copy mapping lives HERE, in vet-api, which already links both.
//!
//! Why the manifest is the source of the READABLE trust fields: the on-chain `ProtocolRegistry.getVersion`
//! returns `circuitId` as a keccak `bytes32` and carries no version STRING and no chain id, whereas the
//! signed manifest mirrors the same version with the human-readable `version`/`circuit_id`/`chain_id`
//! the validator needs (§3.2). The on-chain record is still the ROOT OF TRUTH — its precedence is
//! enforced by P3 [`dogtag_prover::manifest::reconcile`] BEFORE a manifest is allowed to feed the
//! validator, via [`anchor_from_reconciliation`].

use dogtag_prover::manifest::{FieldConflict, Manifest, Reconciliation};
use dogtag_standard::discovery::TrustedAnchor;

/// Assemble a [`TrustedAnchor`] from a signed-manifest CONTENT (§5.2 TRUST tier, the 1B fallback).
///
/// `active` is supplied separately because the manifest does NOT carry the on-chain lifecycle bit: an
/// ONLINE caller passes the on-chain `Version.active`; an OFFLINE (manifest-only) caller passes `true` —
/// a served manifest is presumed active, and deprecation is authoritative only on-chain (a deprecated
/// version is still refused there, and `minAppVersion` remains enforced offline).
pub fn anchor_from_manifest(m: &Manifest, active: bool) -> TrustedAnchor {
    TrustedAnchor {
        version: m.version.clone(),
        version_id: m.version_id.clone(),
        chain_id: m.chain_id,
        verification_registry: m.verification_registry.clone(),
        circuit_id: m.circuit_id.clone(),
        min_app_version: m.min_app_version.clone(),
        active,
    }
}

/// Assemble a [`TrustedAnchor`] from a manifest ALREADY reconciled against the on-chain record (P3
/// [`dogtag_prover::manifest::reconcile`]), enforcing on-chain precedence.
///
/// Returns `Err(conflicts)` if the signed manifest disagrees with the chain on ANY field — so a
/// stale/compromised manifest can never feed the validator (on-chain wins, exactly as P3 designed). On
/// agreement the readable fields come from the (verified, agreeing) manifest.
pub fn anchor_from_reconciliation(
    r: &Reconciliation,
    active: bool,
) -> Result<TrustedAnchor, Vec<FieldConflict>> {
    if !r.manifest_agrees() {
        return Err(r.conflicts.clone());
    }
    Ok(anchor_from_manifest(&r.manifest, active))
}
