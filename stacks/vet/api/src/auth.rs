//! Operator session auth (impl §11.7(e)) + admin-session gate for /admin/* custody (impl §11.4) +
//! the EdDSA (Ed25519) record/verify share-JWT (impl §3.4 / §3.9, architecture §7).
//!
//! A bearer session token (issued by `/admin/login` against the configured operator password) guards
//! ALL issuance/settings/signer/import/verify routes. Only `GET /records/{id}` (record-JWT) and HMAC
//! cross-backend routes are unauthenticated.

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

/// Now (unix seconds).
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// --------------------------------------------------------------------------------------------
// EdDSA record/verify share-JWT (Ed25519). Compact JWS, alg=EdDSA. We implement the encode/decode
// directly (base64url over the per-deployment keypair) to avoid an extra JWT crate dependency.
// --------------------------------------------------------------------------------------------

/// The per-deployment Ed25519 JWT signing keypair.
#[derive(Clone)]
pub struct JwtKeys {
    signing: SigningKey,
    verifying: VerifyingKey,
}

impl JwtKeys {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
        let signing = SigningKey::from_bytes(&bytes);
        let verifying = signing.verifying_key();
        JwtKeys { signing, verifying }
    }
}

/// Record-share claims (impl §3.4).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareClaims {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub scope: String,
    pub iat: u64,
    pub nbf: u64,
    pub exp: u64,
    pub jti: String,
}

/// Verify-session claims (impl §3.9). Extra fields beyond the standard set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyClaims {
    pub iss: String,
    pub sub: String, // sessionId
    pub aud: String,
    pub relayer: String,
    pub purpose: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    pub challenge: String,
    pub mode: String,
    pub exp: u64,
    pub jti: String,
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
fn b64d(s: &str) -> Result<Vec<u8>, AuthError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| AuthError::BadToken)
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("bad token")]
    BadToken,
    #[error("expired")]
    Expired,
    #[error("bad signature")]
    BadSignature,
}

/// Encode a compact EdDSA JWT over `claims`.
pub fn sign_jwt<T: Serialize>(keys: &JwtKeys, claims: &T) -> String {
    let header = serde_json::json!({"alg": "EdDSA", "typ": "JWT"});
    let h = b64(serde_json::to_vec(&header).unwrap().as_slice());
    let c = b64(serde_json::to_vec(claims).unwrap().as_slice());
    let signing_input = format!("{h}.{c}");
    let sig = keys.signing.sign(signing_input.as_bytes());
    let s = b64(&sig.to_bytes());
    format!("{signing_input}.{s}")
}

/// Verify + decode an EdDSA JWT (with a `leeway` seconds clock-skew allowance on exp/nbf).
pub fn verify_jwt<T: for<'de> Deserialize<'de>>(
    keys: &JwtKeys,
    token: &str,
    leeway: u64,
) -> Result<T, AuthError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::BadToken);
    }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig_bytes = b64d(parts[2])?;
    let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).map_err(|_| AuthError::BadSignature)?;
    keys.verifying
        .verify(signing_input.as_bytes(), &sig)
        .map_err(|_| AuthError::BadSignature)?;
    let payload = b64d(parts[1])?;
    // Validate exp/nbf generically by re-parsing as a map.
    let map: serde_json::Value = serde_json::from_slice(&payload).map_err(|_| AuthError::BadToken)?;
    let n = now();
    if let Some(exp) = map.get("exp").and_then(|v| v.as_u64()) {
        if n > exp + leeway {
            return Err(AuthError::Expired);
        }
    }
    if let Some(nbf) = map.get("nbf").and_then(|v| v.as_u64()) {
        if n + leeway < nbf {
            return Err(AuthError::BadToken);
        }
    }
    serde_json::from_slice(&payload).map_err(|_| AuthError::BadToken)
}

/// Mint a fresh operator session token (opaque random bearer).
pub fn new_op_token() -> String {
    let mut b = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    format!("op_{}", hex::encode(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwt_roundtrip_and_tamper() {
        let keys = JwtKeys::generate();
        let claims = ShareClaims {
            iss: "https://vet.example".into(),
            sub: "rec-1".into(),
            aud: "dogtag-mobile".into(),
            scope: "read:record".into(),
            iat: now(),
            nbf: now(),
            exp: now() + 180,
            jti: "jti-1".into(),
        };
        let token = sign_jwt(&keys, &claims);
        let decoded: ShareClaims = verify_jwt(&keys, &token, 30).unwrap();
        assert_eq!(decoded.sub, "rec-1");
        assert_eq!(decoded.scope, "read:record");

        // tamper the payload -> bad signature.
        let mut parts: Vec<&str> = token.split('.').collect();
        let bad_payload = b64(b"{\"sub\":\"evil\"}");
        parts[1] = &bad_payload;
        let tampered = parts.join(".");
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, &tampered, 30),
            Err(AuthError::BadSignature)
        ));

        // a different deployment's key cannot verify.
        let other = JwtKeys::generate();
        assert!(verify_jwt::<ShareClaims>(&other, &token, 30).is_err());
    }

    #[test]
    fn jwt_expiry_enforced() {
        let keys = JwtKeys::generate();
        let claims = ShareClaims {
            iss: "i".into(),
            sub: "s".into(),
            aud: "dogtag-mobile".into(),
            scope: "read:record".into(),
            iat: now() - 1000,
            nbf: now() - 1000,
            exp: now() - 500,
            jti: "j".into(),
        };
        let token = sign_jwt(&keys, &claims);
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, &token, 30),
            Err(AuthError::Expired)
        ));
    }
}
