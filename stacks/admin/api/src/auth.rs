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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
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
    let sig =
        ed25519_dalek::Signature::from_slice(&sig_bytes).map_err(|_| AuthError::BadSignature)?;
    keys.verifying
        .verify(signing_input.as_bytes(), &sig)
        .map_err(|_| AuthError::BadSignature)?;
    let payload = b64d(parts[1])?;
    let map: serde_json::Value =
        serde_json::from_slice(&payload).map_err(|_| AuthError::BadToken)?;
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

/// The canonical message a device signs to prove wallet ownership. The walletAddress is lowercased
/// so the message is deterministic regardless of input checksum casing.
pub fn register_message(wallet_address: &str) -> String {
    format!(
        "DogTag wallet registration: {}",
        wallet_address.to_lowercase()
    )
}

/// Recover the EIP-191 (`personal_sign`) signer of `message` from a 65-byte `0x..` signature, returning
/// the recovered address as a lowercase `0x..` hex string. The digest is
/// `keccak256("\x19Ethereum Signed Message:\n" + len(message) + message)` — alloy's
/// `recover_address_from_msg` applies the prefix internally. Returns None on a malformed signature.
pub fn recover_personal_sign(message: &str, signature_hex: &str) -> Option<String> {
    use alloy::primitives::PrimitiveSignature;
    let raw = hex::decode(
        signature_hex
            .trim()
            .strip_prefix("0x")
            .unwrap_or(signature_hex.trim()),
    )
    .ok()?;
    let sig = PrimitiveSignature::from_raw(&raw).ok()?;
    let addr = sig.recover_address_from_msg(message.as_bytes()).ok()?;
    Some(format!("{addr:#x}"))
}

// --------------------------------------------------------------------------------------------
// In-memory rate limiter for the password endpoints (/v1/admin/login, etc.).
//
// Thresholds are intentionally LENIENT so the demo + e2e-smoke never trip: a client IP is locked
// out for `lockout_secs` only after `per_ip_max` failed attempts inside `window_secs`, and a global
// cap guards against a distributed flood. Successful auth clears the IP's failure record. Demo
// behavior is unaffected — the limiter only ever rejects *repeated bad* passwords.
// --------------------------------------------------------------------------------------------

use std::sync::Mutex;

#[derive(Default)]
struct IpState {
    /// failure timestamps (unix secs) inside the current window.
    failures: Vec<u64>,
    /// if Some, locked out until this unix-secs instant.
    locked_until: Option<u64>,
}

pub struct RateLimiter {
    inner: Mutex<std::collections::HashMap<String, IpState>>,
    /// rolling global failure timestamps (DoS guard).
    global: Mutex<Vec<u64>>,
    window_secs: u64,
    per_ip_max: usize,
    global_max: usize,
    lockout_secs: u64,
}

impl Default for RateLimiter {
    fn default() -> Self {
        // ~10 failures / 60s per IP -> 60s lockout; 200 failures / 60s globally.
        RateLimiter {
            inner: Mutex::new(std::collections::HashMap::new()),
            global: Mutex::new(Vec::new()),
            window_secs: 60,
            per_ip_max: 10,
            global_max: 200,
            lockout_secs: 60,
        }
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if `ip` is currently locked out (call BEFORE checking the password).
    pub fn is_locked(&self, ip: &str) -> bool {
        let now = now();
        let mut map = self.inner.lock().unwrap();
        if let Some(st) = map.get_mut(ip) {
            if let Some(until) = st.locked_until {
                if now < until {
                    return true;
                }
                st.locked_until = None;
                st.failures.clear();
            }
        }
        // global flood guard
        let mut g = self.global.lock().unwrap();
        g.retain(|t| now.saturating_sub(*t) < self.window_secs);
        g.len() >= self.global_max
    }

    /// Record a failed attempt for `ip`; locks the IP out if it crosses the per-IP threshold.
    pub fn record_failure(&self, ip: &str) {
        let now = now();
        {
            let mut g = self.global.lock().unwrap();
            g.retain(|t| now.saturating_sub(*t) < self.window_secs);
            g.push(now);
        }
        let mut map = self.inner.lock().unwrap();
        let st = map.entry(ip.to_string()).or_default();
        st.failures
            .retain(|t| now.saturating_sub(*t) < self.window_secs);
        st.failures.push(now);
        if st.failures.len() >= self.per_ip_max {
            st.locked_until = Some(now + self.lockout_secs);
        }
    }

    /// Clear an IP's failure record on a successful auth.
    pub fn record_success(&self, ip: &str) {
        let mut map = self.inner.lock().unwrap();
        map.remove(ip);
    }
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

    #[test]
    fn hmac_verify_rejects_non_hex_signature() {
        // A signature that is not valid hex fails at decode (the `is_ok` guard) rather than panicking.
        assert!(!hmac_verify("secret", "POST", "/p", b"{}", "not-hex"));
    }

    #[test]
    fn verify_password_rejects_malformed_stored() {
        // No `$` separator -> the destructuring `let-else` returns false.
        assert!(!verify_password("hunter2", "no-separator"));
        // Separator present but non-hex halves -> hex::decode fails -> false.
        assert!(!verify_password("hunter2", "zz$zz"));
        // Well-formed shape but wrong hash bytes -> constant_time_eq mismatch -> false.
        assert!(!verify_password("hunter2", "00$00"));
    }

    #[test]
    fn keccak256_hex_anchors_empty_and_shape() {
        // keccak256("") is a fixed, well-known constant; pin it so any digest swap is caught.
        assert_eq!(
            keccak256_hex(""),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        let h = keccak256_hex("boarding_intake");
        assert!(h.starts_with("0x"));
        assert_eq!(h.len(), 66); // "0x" + 32 bytes * 2 hex chars
                                 // Distinct labels hash to distinct keys.
        assert_ne!(keccak256_hex("a"), keccak256_hex("b"));
    }

    #[test]
    fn register_message_lowercases_wallet() {
        // The signed message is checksum-casing-independent (the address is lowercased).
        let mixed = register_message("0xAbCdEf0123456789");
        let lower = register_message("0xabcdef0123456789");
        assert_eq!(mixed, lower);
        assert_eq!(mixed, "DogTag wallet registration: 0xabcdef0123456789");
    }

    #[test]
    fn new_session_token_shape_and_uniqueness() {
        let a = new_session_token("sess");
        let b = new_session_token("sess");
        assert!(a.starts_with("sess_"));
        // "sess_" + 32 random bytes hex-encoded.
        assert_eq!(a.len(), "sess_".len() + 64);
        assert!(a["sess_".len()..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b); // fresh randomness each call
    }

    #[test]
    fn recover_personal_sign_rejects_malformed_signature() {
        // Non-hex and wrong-length signatures both yield None rather than panicking.
        assert!(recover_personal_sign("msg", "not-hex").is_none());
        assert!(recover_personal_sign("msg", "0x1234").is_none());
    }

    #[test]
    fn verify_jwt_rejects_expired_and_malformed() {
        let keys = JwtKeys::generate();
        let n = now();
        let expired = ShareClaims {
            iss: "https://central".into(),
            sub: "cred-1".into(),
            aud: "dogtag-business".into(),
            scope: "read:credential".into(),
            iat: n - 600,
            nbf: n - 600,
            exp: n - 300, // already past, beyond any small leeway
            jti: "jti-x".into(),
        };
        let token = sign_jwt(&keys, &expired);
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, &token, 30),
            Err(AuthError::Expired)
        ));
        // A token without three dot-separated parts is a BadToken.
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, "only.two", 30),
            Err(AuthError::BadToken)
        ));
    }

    #[test]
    fn rate_limiter_locks_after_threshold_and_clears_on_success() {
        let rl = RateLimiter::new();
        let ip = "203.0.113.7";
        assert!(!rl.is_locked(ip)); // fresh IP is not locked
                                    // The default per-IP threshold is 10 failures inside the window.
        for _ in 0..9 {
            rl.record_failure(ip);
        }
        assert!(!rl.is_locked(ip)); // 9 < 10, still allowed
        rl.record_failure(ip); // 10th failure crosses the threshold
        assert!(rl.is_locked(ip));
        // A successful auth clears the IP's failure record, unlocking it.
        rl.record_success(ip);
        assert!(!rl.is_locked(ip));
        // An unrelated IP is unaffected throughout.
        assert!(!rl.is_locked("198.51.100.2"));
    }
}
