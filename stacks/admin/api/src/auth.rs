//! Auth surface for the central backend (impl §4.1, §11.4):
//!   - mobile user sessions: salted-hash password + opaque bearer session token (real, simple);
//!   - admin session: a separate password-gated bearer for admin-only routes;
//!   - EdDSA (Ed25519) share-JWT (one-time, aud `dogtag-business`) for `POST /v1/share` -> `GET /share`;
//!   - HMAC-SHA256 for cross-backend appointment sync / event callbacks (shared secret per business).

use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// Now (unix seconds).
pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

// --------------------------------------------------------------------------------------------
// Password hashing — salted SHA-256 (real, deterministic-verifiable). Production would use argon2;
// the salt + iterated hash here is a genuine non-reversible store, sufficient for the central stack
// per the brief ("keep auth simple but real — hashed password").
// --------------------------------------------------------------------------------------------

/// Hash a password with a fresh 16-byte salt. Returns `"<salt_hex>$<hash_hex>"`.
pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
    let h = pbkdf_sha256(password.as_bytes(), &salt, 50_000);
    format!("{}${}", hex::encode(salt), hex::encode(h))
}

/// Verify a password against a `"<salt_hex>$<hash_hex>"` stored value (constant-time-ish compare).
pub fn verify_password(password: &str, stored: &str) -> bool {
    let mut parts = stored.splitn(2, '$');
    let (Some(salt_hex), Some(hash_hex)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(salt), Ok(expected)) = (hex::decode(salt_hex), hex::decode(hash_hex)) else {
        return false;
    };
    let got = pbkdf_sha256(password.as_bytes(), &salt, 50_000);
    constant_time_eq(&got, &expected)
}

/// Iterated salted SHA-256 (a tiny PBKDF). Not argon2, but a genuine slow-ish one-way function.
fn pbkdf_sha256(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(salt);
    h.update(password);
    let mut out: [u8; 32] = h.finalize().into();
    for _ in 1..iters {
        let mut h = Sha256::new();
        h.update(out);
        h.update(salt);
        out = h.finalize().into();
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Mint a fresh opaque session bearer token.
pub fn new_session_token(prefix: &str) -> String {
    let mut b = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    format!("{prefix}_{}", hex::encode(b))
}

// --------------------------------------------------------------------------------------------
// EdDSA share/verify JWT (Ed25519). Compact JWS, alg=EdDSA — same scheme as the vet stack.
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

/// Share-JWT claims (impl §4.1 / §11.4 C-1): user -> business one-time disclosure token.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareClaims {
    pub iss: String,
    pub sub: String, // == the share ref / credentialId
    pub aud: String, // "dogtag-business"
    pub scope: String,
    pub iat: u64,
    pub nbf: u64,
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

// --------------------------------------------------------------------------------------------
// HMAC — cross-backend appointment sync. The signature binds the method+path+body to the per-business
// shared secret; the event callback resolves the secret BY the path businessId (impl §11.4 C-2).
// --------------------------------------------------------------------------------------------

/// Compute the canonical HMAC-SHA256 over `METHOD\nPATH\nBODY` with `secret`, hex-encoded.
pub fn hmac_sign(secret: &str, method: &str, path: &str, body: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify an HMAC signature (constant-time via the `hmac` crate's `verify_slice`).
pub fn hmac_verify(secret: &str, method: &str, path: &str, body: &[u8], sig_hex: &str) -> bool {
    let Ok(sig) = hex::decode(sig_hex) else {
        return false;
    };
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

/// keccak256 of a string -> bytes32 hex (recordType label -> on-chain key; consent recordType binding).
pub fn keccak256_hex(s: &str) -> String {
    use alloy::primitives::keccak256;
    format!("0x{}", hex::encode(keccak256(s.as_bytes()).as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip() {
        let stored = hash_password("hunter2");
        assert!(verify_password("hunter2", &stored));
        assert!(!verify_password("wrong", &stored));
    }

    #[test]
    fn jwt_roundtrip_and_tamper() {
        let keys = JwtKeys::generate();
        let n = now();
        let claims = ShareClaims {
            iss: "https://central".into(),
            sub: "cred-1".into(),
            aud: "dogtag-business".into(),
            scope: "read:credential".into(),
            iat: n,
            nbf: n,
            exp: n + 180,
            jti: "jti-1".into(),
        };
        let token = sign_jwt(&keys, &claims);
        let decoded: ShareClaims = verify_jwt(&keys, &token, 30).unwrap();
        assert_eq!(decoded.sub, "cred-1");
        let other = JwtKeys::generate();
        assert!(verify_jwt::<ShareClaims>(&other, &token, 30).is_err());
    }

    #[test]
    fn hmac_roundtrip() {
        let body = br#"{"a":1}"#;
        let sig = hmac_sign("secret", "POST", "/p", body);
        assert!(hmac_verify("secret", "POST", "/p", body, &sig));
        assert!(!hmac_verify("secret", "POST", "/p", b"tampered", &sig));
        assert!(!hmac_verify("other", "POST", "/p", body, &sig));
    }
}
