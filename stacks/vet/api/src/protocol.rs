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

/// Load the dogtag manifest signing key from env, or `None` if unset/malformed.
pub fn signing_key() -> Option<SigningKey> {
    let hex_seed = std::env::var(SIGNING_KEY_ENV).ok()?;
    let bytes: [u8; 32] = hex::decode(hex_seed.trim()).ok()?.try_into().ok()?;
    Some(SigningKey::from_bytes(&bytes))
}

/// Build + sign the manifest for a known version, or `None` if the version is unrecognized.
/// The content is assembled DRY from the file-verified artifact descriptor (§3.2) + deployment.
pub fn signed_manifest(version: &str, key: &SigningKey) -> Option<SignedManifest> {
    let content = manifest::build(version)?;
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
    let Some(key) = signing_key() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "manifest signing key not configured" })),
        )
            .into_response();
    };
    match signed_manifest(&q.version, &key) {
        Some(sm) => (StatusCode::OK, Json(sm)).into_response(),
        None => (
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

    fn test_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// The serving path produces a manifest that OFFLINE-VERIFIES against the signer's public key —
    /// the "sign → serve → verify" round-trip through the server-side builder.
    #[test]
    fn serves_a_verifiable_manifest() {
        let key = test_key();
        let sm = signed_manifest("dogtag-levelb/1", &key).expect("known version");

        // Cross the wire as JSON and back, then verify offline.
        let wire = serde_json::to_string(&sm).unwrap();
        let received: SignedManifest = serde_json::from_str(&wire).unwrap();
        verify(&received, &key.verifying_key()).expect("served manifest verifies offline");
        assert_eq!(received.content.version, "dogtag-levelb/1");
    }

    #[test]
    fn both_current_versions_are_servable() {
        let key = test_key();
        assert!(signed_manifest("dogtag-levela/1", &key).is_some());
        assert!(signed_manifest("dogtag-levelb/1", &key).is_some());
    }

    /// An unrecognized version is not served (fail-closed → the handler 404s).
    #[test]
    fn unknown_version_is_not_served() {
        assert!(signed_manifest("dogtag-levelc/9", &test_key()).is_none());
    }
}
