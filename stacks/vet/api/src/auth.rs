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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
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
    /// Generate an EPHEMERAL keypair. Per-process: tokens it signs do NOT survive a restart and are
    /// not accepted by a sibling instance. Use ONLY for demo/local; production loads a shared key via
    /// [`JwtKeys::from_seed_hex`] (audit L4).
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
        let signing = SigningKey::from_bytes(&bytes);
        let verifying = signing.verifying_key();
        JwtKeys { signing, verifying }
    }

    /// Load a FIXED Ed25519 signing key from a 32-byte hex seed (`0x`-prefix optional). A deployment
    /// sets the same seed across restarts and across horizontally-scaled instances so share/record
    /// tokens remain verifiable everywhere (audit L4). Errors on non-hex / wrong-length input so the
    /// caller can fail closed.
    pub fn from_seed_hex(seed_hex: &str) -> Result<Self, String> {
        let s = seed_hex.trim();
        let raw = hex::decode(s.strip_prefix("0x").unwrap_or(s)).map_err(|e| format!("not hex: {e}"))?;
        let bytes: [u8; 32] =
            raw.as_slice().try_into().map_err(|_| format!("expected 32 bytes, got {}", raw.len()))?;
        let signing = SigningKey::from_bytes(&bytes);
        let verifying = signing.verifying_key();
        Ok(JwtKeys { signing, verifying })
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

// NOTE: the EXPORT (formerly "verify") session no longer mints an EdDSA `VerifyClaims` JWT for its
// QR. It now uses a one-time export TOKEN (16 random bytes hex, 180s, consumed on submit) — symmetric
// with the import `/r/<token>` flow. See `routes.rs::export_session_start` / `GET /x/{token}`. The
// `ShareClaims` record-share JWT and the generic `sign_jwt`/`verify_jwt` below remain in use by
// `GET /records/{id}`.

/// The canonical message the device signs to prove wallet ownership at `POST /profiles/issue/bind`.
/// The walletAddress is lowercased so the message is deterministic regardless of input checksum
/// casing. MUST byte-match the admin stack's `register_message` (the mobile signs the same string).
pub fn register_message(wallet_address: &str) -> String {
    format!(
        "DogTag wallet registration: {}",
        wallet_address.to_lowercase()
    )
}

/// Recover the EIP-191 (`personal_sign`) signer of `message` from a 65-byte `0x..` signature,
/// returning the recovered address as a lowercase `0x..` hex string. The digest is
/// `keccak256("\x19Ethereum Signed Message:\n" + len(message) + message)` — alloy's
/// `recover_address_from_msg` applies the prefix internally. Returns None on a malformed signature.
/// Ported from the admin stack (`stacks/admin/api/src/auth.rs`).
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
    // Validate exp/nbf generically by re-parsing as a map.
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

/// Mint a fresh operator session token (opaque random bearer).
pub fn new_op_token() -> String {
    let mut b = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut b);
    format!("op_{}", hex::encode(b))
}

// --------------------------------------------------------------------------------------------
// HMAC — cross-backend appointment sync (impl §3.7 / §11.4 C-2). The signature binds
// METHOD\nPATH\nBODY to the per-business shared secret. This MIRRORS the central backend's
// `hmac_sign`/`hmac_verify` (stacks/admin/api/src/auth.rs) byte-for-byte so the contract holds.
// --------------------------------------------------------------------------------------------

/// Compute the canonical HMAC-SHA256 over `METHOD\nPATH\nBODY` with `secret`, hex-encoded.
pub fn hmac_sign(secret: &str, method: &str, path: &str, body: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify an HMAC signature (constant-time via the `hmac` crate's `verify_slice`).
pub fn hmac_verify(secret: &str, method: &str, path: &str, body: &[u8], sig_hex: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let Ok(sig) = hex::decode(sig_hex) else {
        return false;
    };
    let mut mac = <Hmac<Sha256>>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(body);
    mac.verify_slice(&sig).is_ok()
}

// --------------------------------------------------------------------------------------------
// In-memory rate limiter for the password endpoints (/login, /admin/login, /admin/unlock).
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
    fn hmac_roundtrip_and_tamper() {
        let sig = hmac_sign("secret", "PUT", "/v1/appointments/a1", b"{\"rev\":1}");
        assert!(hmac_verify(
            "secret",
            "PUT",
            "/v1/appointments/a1",
            b"{\"rev\":1}",
            &sig
        ));
        // tampered body / path / method / key all fail.
        assert!(!hmac_verify(
            "secret",
            "PUT",
            "/v1/appointments/a1",
            b"{\"rev\":2}",
            &sig
        ));
        assert!(!hmac_verify(
            "secret",
            "PUT",
            "/v1/appointments/a2",
            b"{\"rev\":1}",
            &sig
        ));
        assert!(!hmac_verify(
            "secret",
            "POST",
            "/v1/appointments/a1",
            b"{\"rev\":1}",
            &sig
        ));
        assert!(!hmac_verify(
            "other",
            "PUT",
            "/v1/appointments/a1",
            b"{\"rev\":1}",
            &sig
        ));
    }

    #[test]
    fn register_message_lowercases_wallet() {
        // Mixed-case and already-lowercase wallets must yield the identical message so the
        // device signature is deterministic regardless of input checksum casing. MUST byte-match
        // the admin stack's `register_message`.
        let mixed = register_message("0xAbCdEf0123456789");
        let lower = register_message("0xabcdef0123456789");
        assert_eq!(mixed, lower);
        assert_eq!(mixed, "DogTag wallet registration: 0xabcdef0123456789");
    }

    #[test]
    fn new_op_token_shape_and_uniqueness() {
        let a = new_op_token();
        let b = new_op_token();
        assert!(a.starts_with("op_"));
        // "op_" + 32 random bytes hex-encoded = 64 hex chars.
        assert_eq!(a.len(), "op_".len() + 64);
        assert!(a["op_".len()..].chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b); // fresh randomness each call
    }

    #[test]
    fn recover_personal_sign_rejects_malformed_signature() {
        // Non-hex and wrong-length signatures must return None, never panic.
        assert!(recover_personal_sign("msg", "not-hex").is_none());
        assert!(recover_personal_sign("msg", "0x1234").is_none());
    }

    #[test]
    fn verify_jwt_rejects_malformed_and_future_nbf() {
        let keys = JwtKeys::generate();
        // Wrong number of compact-JWS segments -> BadToken before any crypto.
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, "only.two", 30),
            Err(AuthError::BadToken)
        ));
        // A not-yet-valid token (nbf far in the future, beyond leeway) -> BadToken.
        let claims = ShareClaims {
            iss: "i".into(),
            sub: "s".into(),
            aud: "dogtag-mobile".into(),
            scope: "read:record".into(),
            iat: now(),
            nbf: now() + 10_000,
            exp: now() + 20_000,
            jti: "j".into(),
        };
        let token = sign_jwt(&keys, &claims);
        assert!(matches!(
            verify_jwt::<ShareClaims>(&keys, &token, 30),
            Err(AuthError::BadToken)
        ));
    }

    #[test]
    fn rate_limiter_locks_after_threshold_and_clears_on_success() {
        let rl = RateLimiter::new();
        let ip = "203.0.113.7";
        assert!(!rl.is_locked(ip)); // fresh IP is not locked
                                    // default per_ip_max is 10: the first 9 failures stay under the threshold.
        for _ in 0..9 {
            rl.record_failure(ip);
        }
        assert!(!rl.is_locked(ip)); // 9 < 10, still allowed
        rl.record_failure(ip); // 10th failure crosses the threshold
        assert!(rl.is_locked(ip));
        // a successful auth clears the IP's failure record and lockout.
        rl.record_success(ip);
        assert!(!rl.is_locked(ip));
        // lockout is per-IP: a different IP is unaffected by the first IP's failures.
        assert!(!rl.is_locked("198.51.100.9"));
    }

    #[test]
    fn jwt_from_seed_is_stable_across_instances() {
        // audit L4: two independently-loaded instances with the SAME seed produce the SAME key, so a
        // token signed by one verifies on the other (survives restart / horizontal scaling).
        let seed = "0x".to_string() + &"ab".repeat(32);
        let a = JwtKeys::from_seed_hex(&seed).unwrap();
        let b = JwtKeys::from_seed_hex(&seed).unwrap();
        let claims = ShareClaims {
            iss: "i".into(), sub: "s".into(), aud: "dogtag-mobile".into(), scope: "read:record".into(),
            iat: now(), nbf: now(), exp: now() + 180, jti: "j".into(),
        };
        let token = sign_jwt(&a, &claims);
        let decoded: ShareClaims = verify_jwt(&b, &token, 30).expect("cross-instance verify");
        assert_eq!(decoded.sub, "s");
        // a DIFFERENT seed must not verify it.
        let other = JwtKeys::from_seed_hex(&("cd".repeat(32))).unwrap();
        assert!(verify_jwt::<ShareClaims>(&other, &token, 30).is_err());
        // malformed seeds fail closed.
        assert!(JwtKeys::from_seed_hex("zz").is_err());
        assert!(JwtKeys::from_seed_hex(&"ab".repeat(16)).is_err(), "wrong length");
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
