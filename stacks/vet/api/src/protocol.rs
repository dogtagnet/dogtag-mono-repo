//! Signed-manifest SERVING path (M7 §5.1 1B).
//!
//! dogtag serves the SAME version content the on-chain `ProtocolRegistry` holds as a
//! **dogtag-key-signed HTTPS JSON** an app can verify OFFLINE (no RPC, no server liveness). It is a
//! CACHE/FALLBACK, never a second authority: on any conflict the on-chain record wins, enforced by
//! [`dogtag_prover::manifest::reconcile`] on the consuming side.
//!
//! The signing key is the dogtag manifest key; the content + signature + offline-verify helper all live
//! in [`dogtag_prover::manifest`]. This module is only the HTTP surface + key loading.

use axum::{extract::Query, http::StatusCode, response::IntoResponse, Json};
use dogtag_prover::manifest::{self, SignedManifest};
use ed25519_dalek::SigningKey;
use serde::Deserialize;
use serde_json::json;

/// Env var holding the dogtag manifest signing key: a 32-byte ed25519 seed as 64 lowercase hex chars.
/// UNSET ⇒ the serving path is DISABLED (503) — no key, no manifest (fail-closed). The corresponding
/// PUBLIC key is what an app pins to verify offline (`dogtag_prover::manifest::DOGTAG_MANIFEST_PUBKEY`).
pub const SIGNING_KEY_ENV: &str = "DOGTAG_MANIFEST_SIGNING_KEY";

/// Why the manifest signing key could not be loaded — kept distinct so the serving path can tell an
/// UNSET key (feature simply off) apart from a SET-but-broken one (operator misconfiguration worth
/// surfacing), instead of collapsing both into a silent "not configured" 503.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigningKeyError {
    /// `DOGTAG_MANIFEST_SIGNING_KEY` is unset — the serving path is intentionally disabled.
    NotConfigured,
    /// The env var IS set but is not a 32-byte ed25519 seed as 64 hex chars — a misconfiguration.
    Malformed,
}

/// Parse a raw env value into the signing key. Pure (no env access) so the unset/malformed distinction
/// is unit-testable without mutating process-global env vars. `None` ⇒ unset.
fn parse_signing_key(raw: Option<&str>) -> Result<SigningKey, SigningKeyError> {
    let hex_seed = raw.ok_or(SigningKeyError::NotConfigured)?;
    let bytes: [u8; 32] = hex::decode(hex_seed.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(SigningKeyError::Malformed)?;
    Ok(SigningKey::from_bytes(&bytes))
}

/// Load the dogtag manifest signing key from env.
///
/// Fail-closed and case-distinguished: [`SigningKeyError::NotConfigured`] when the env var is unset,
/// [`SigningKeyError::Malformed`] when it is set but not a valid 32-byte hex seed (including a
/// non-Unicode value). Either way no key is returned, so no manifest is ever served with a bad key.
pub fn signing_key() -> Result<SigningKey, SigningKeyError> {
    match std::env::var(SIGNING_KEY_ENV) {
        Ok(v) => parse_signing_key(Some(&v)),
        Err(std::env::VarError::NotPresent) => Err(SigningKeyError::NotConfigured),
        // Set, but not valid UTF-8 — it is present-but-broken, not unset.
        Err(std::env::VarError::NotUnicode(_)) => Err(SigningKeyError::Malformed),
    }
}

/// The env vars carrying this deployment's on-chain contract set - the same names
/// `scripts/gen-deployment-env.sh vet` emits from the deploy ledger, so the deploy WRITES this
/// configuration rather than an operator transcribing it.
pub const DEPLOYMENT_ENV: [&str; 5] = [
    "CHAIN_ID",
    "FACTORY_ADDR",
    "VERIFICATION_REGISTRY_CONSENT_ADDR",
    "SBT_CONSENT_ADDR",
    "GROTH16_VERIFIER_CONSENT_ADDR",
];

/// The manifest's on-chain half could not be assembled, naming the variable at fault.
///
/// A manifest MIRRORS chain state, so this is configuration - `dogtag_prover` deliberately ships no
/// deployment constant. It used to, and by the time that was looked at every one of its four
/// addresses had been superseded: this stack would have served a manifest disagreeing with the chain
/// on every member. `reconcile` treats the chain as authoritative, so the damage was bounded to a
/// pile of phantom conflicts rather than a wrong answer - but a fallback that can only ever conflict
/// is not a fallback, and nothing said so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentConfigError {
    /// The env var that is unset or unusable. Named so the remedy is the message.
    pub var: &'static str,
}

/// Assemble the on-chain half from env, or name the first variable that is missing.
///
/// Fail-closed with NO defaults: a placeholder or a zero address here would be served, signed, as
/// dogtag's own statement of what the version is.
///
/// `provider_registry`/`root_index` stay `None`. That absence is load-bearing rather than an
/// omission - `reconcile` reads a manifest-`Some` against an on-chain-`None` as a CONFLICT, so
/// claiming a member the published record does not carry would make every reconcile report a
/// disagreement that is really ours.
pub fn deployment_from_env() -> Result<manifest::VersionDeployment, DeploymentConfigError> {
    fn need(var: &'static str) -> Result<String, DeploymentConfigError> {
        match std::env::var(var) {
            Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
            _ => Err(DeploymentConfigError { var }),
        }
    }
    let chain_id = need("CHAIN_ID")?
        .parse::<u64>()
        .map_err(|_| DeploymentConfigError { var: "CHAIN_ID" })?;
    Ok(manifest::VersionDeployment {
        chain_id,
        factory: need("FACTORY_ADDR")?,
        verification_registry: need("VERIFICATION_REGISTRY_CONSENT_ADDR")?,
        sbt: need("SBT_CONSENT_ADDR")?,
        verifier: need("GROTH16_VERIFIER_CONSENT_ADDR")?,
        provider_registry: None,
        root_index: None,
    })
}

/// Build + sign the manifest for a known version, or `None` if the version is unrecognized.
/// The content is assembled DRY from the file-verified artifact descriptor (§3.2) + the caller's
/// deployment.
pub fn signed_manifest(
    version: &str,
    key: &SigningKey,
    deployment: &manifest::VersionDeployment,
) -> Option<SignedManifest> {
    if version != dogtag_standard::wrap::LEVEL_B_VERSION {
        return None;
    }
    let content = manifest::build(version, deployment)?;
    Some(manifest::sign(&content, key))
}

#[derive(Debug, Deserialize)]
pub struct ManifestQuery {
    /// The protocol version string, e.g. `dogtag-levelb/1` (the `/` is why this is a query param, not a
    /// path segment).
    pub version: String,
}

/// `GET /protocol/manifest?version=dogtag-levelb/1` — serve the dogtag-signed manifest for a version.
///
/// A NEW, distinct route: it does NOT touch the existing resolve GET (`/p/`, `/x/`) — extending that
/// with the convenience tier is P4. Fail-closed: 503 when no signing key is configured, 404 for an
/// unrecognized version, else 200 with the [`SignedManifest`] JSON.
pub async fn get_manifest(Query(q): Query<ManifestQuery>) -> impl IntoResponse {
    let key = match signing_key() {
        Ok(key) => key,
        Err(SigningKeyError::NotConfigured) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "manifest signing key not configured" })),
            )
                .into_response();
        }
        Err(SigningKeyError::Malformed) => {
            // Set but broken: surface it (never chase a silent unset-style 503) without logging the
            // secret value. Still fail-closed — a bad key serves no manifest.
            eprintln!(
                "protocol: {SIGNING_KEY_ENV} is set but malformed (expected a 32-byte ed25519 seed as \
                 64 hex chars); refusing to serve manifests"
            );
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": "manifest signing key misconfigured" })),
            )
                .into_response();
        }
    };
    // The on-chain half is configuration, exactly like the key above, and refused the same way: a
    // 503 naming the variable, never a manifest built on a guess. Checked BEFORE the version is
    // classified so a misconfigured stack is told what to set rather than being told its perfectly
    // good version key is unknown.
    let deployment = match deployment_from_env() {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "manifest deployment not configured",
                    "detail": format!("{} is unset or unusable", e.var),
                    "remedy": "run scripts/gen-deployment-env.sh vet and load its output",
                })),
            )
                .into_response();
        }
    };
    if let Some(sm) = signed_manifest(&q.version, &key, &deployment) {
        return (StatusCode::OK, Json(sm)).into_response();
    }
    // Still 404 in both arms — there is genuinely no manifest either way, and neither is servable.
    // What differs is the REASON, because the remedies are unrelated: a typo is fixed by the caller,
    // while a recognized key with no on-chain record is fixed by publishing that record. Reporting the
    // second as "unknown version" sends an operator hunting a misspelling that does not exist, and
    // naming a step that has already run sends them to redo it — the record itself carries the remedy.
    match manifest::deployment_status(&q.version) {
        manifest::DeploymentStatus::AwaitingDeployment(a) => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": "version not yet deployed",
                "version": q.version,
                "detail": "this build recognizes the version key but holds no on-chain record for it",
                "recordedBy": a.recorded_by,
                "outstandingSteps": a.outstanding,
            })),
        )
            .into_response(),
        _ => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "unknown version", "version": q.version })),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dogtag_prover::manifest::verify;

    /// A SYNTHETIC on-chain record. The crate ships none - a deployment mirrors chain state - and
    /// these tests are about the signing/serving path, not about which addresses it carries.
    fn test_deployment() -> manifest::VersionDeployment {
        manifest::VersionDeployment {
            chain_id: 135,
            factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607".to_string(),
            verification_registry: "0x2B4d6f8a0c1e3a5b7d9f0e2C4a6b8d0F1E3A5c70".to_string(),
            sbt: "0x3c5e7A9b0D2F4a6c8E0b1d3F5A7c9e0B2D4F6a80".to_string(),
            verifier: "0x4d6F8B0C2E4A6b8d0F2c4e6a8b0D2F4c6e8A0b90".to_string(),
            provider_registry: None,
            root_index: None,
        }
    }

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// The serving path produces a manifest that OFFLINE-VERIFIES against the signer's public key —
    /// the "sign → serve → verify" round-trip through the server-side builder.
    #[test]
    fn serves_a_verifiable_manifest() {
        let key = test_key();
        let sm = signed_manifest("dogtag-levelb/1", &key, &test_deployment()).expect("known version");

        // Cross the wire as JSON and back, then verify offline.
        let wire = serde_json::to_string(&sm).unwrap();
        let received: SignedManifest = serde_json::from_str(&wire).unwrap();
        verify(&received, &key.verifying_key()).expect("served manifest verifies offline");
        assert_eq!(received.content.version, "dogtag-levelb/1");
    }

    #[test]
    fn only_the_unified_version_is_servable() {
        let key = test_key();
        assert!(signed_manifest("dogtag-levela/1", &key, &test_deployment()).is_none());
        assert!(signed_manifest("dogtag-levelb/1", &key, &test_deployment()).is_some());
    }

    /// An unrecognized version is not served (fail-closed → the handler 404s).
    #[test]
    fn unknown_version_is_not_served() {
        assert!(signed_manifest("dogtag-levelc/9", &test_key(), &test_deployment()).is_none());
    }

    /// The signing-key loader tells UNSET apart from SET-but-malformed, so the serving path can log a
    /// misconfiguration instead of masking it as a silent unset-style 503. Pure over `parse_signing_key`
    /// so it needs no process-global env mutation.
    #[test]
    fn signing_key_distinguishes_unset_from_malformed() {
        // `.err()` collapses to Option<SigningKeyError> so this asserts without needing SigningKey: Eq.
        assert_eq!(parse_signing_key(None).err(), Some(SigningKeyError::NotConfigured));
        // non-hex
        assert_eq!(parse_signing_key(Some("not-hex-at-all")).err(), Some(SigningKeyError::Malformed));
        // valid hex but the wrong length (16 bytes, not 32)
        assert_eq!(
            parse_signing_key(Some(&hex::encode([1u8; 16]))).err(),
            Some(SigningKeyError::Malformed)
        );
        // a well-formed 32-byte seed loads (whitespace-trimmed).
        let seed = hex::encode([7u8; 32]);
        assert!(parse_signing_key(Some(&format!("  {seed}\n"))).is_ok());
    }
}
