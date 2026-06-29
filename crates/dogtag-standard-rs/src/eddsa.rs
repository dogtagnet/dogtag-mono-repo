//! EdDSA-BabyJubjub (Poseidon) consent SIGNING — circomlibjs-compatible (impl §1.10 / §11.9(d)).
//!
//! This is the crypto that was previously deferred: deterministic key derivation + signature over
//! the §1.10 consent message `M = Poseidon6(dogTagId, purpose, relayer, subject, R, nonce)`. It is a
//! byte-for-byte reimplementation of circomlibjs `buildEddsa().prv2pub` / `.signPoseidon`, asserted
//! against a fixed circomlibjs vector (see tests) so the on-chain ZK circuit's `EdDSAPoseidonVerifier`
//! and the registry's `keyHash = Poseidon(Ax,Ay)` accept what mobile produces.
//!
//! ADDITIVE — built on the existing trusted `poseidon` (light-poseidon, circom-compatible) and
//! `ark-bn254::Fr`; does NOT modify poseidon/field/leaf/merkle/encode/consent algorithm code.
//!
//! BabyJubjub is the twisted Edwards curve `a*x^2 + y^2 = 1 + d*x^2*y^2` with a=168700, d=168696
//! over the BN254 scalar field. circomlibjs signs with `Base8` (the order-8-cofactor generator) and
//! the sub-group order `subOrder = l = order >> 3`.

use ark_bn254::Fr;
use ark_ff::{BigInteger, Field, One, PrimeField, Zero};
use num_bigint::BigUint;

use crate::blake512::blake512;
use crate::poseidon::poseidon;

/// Domain separation tag for the BabyJubjub consent-key seed derivation. Distinct from the
/// secp256k1 wallet path (§6) so the two keys are independent even from the same root seed.
const CONSENT_KEY_DOMAIN: &[u8] = b"DogTag/consent-key/babyjubjub/v1";

/// BabyJubjub curve constant `a` (168700).
fn coeff_a() -> Fr {
    Fr::from(168700u64)
}

/// BabyJubjub curve constant `d` (168696).
fn coeff_d() -> Fr {
    Fr::from(168696u64)
}

/// `Base8` generator (cofactor-cleared base point), decimal coordinates from circomlibjs.
fn base8() -> Point {
    Point {
        x: fr_from_dec("5299619240641551281634865583518297030282874472190772894086521144482721001553"),
        y: fr_from_dec("16950150798460657717958625567821834550301663161624707787222815936182638968203"),
    }
}

/// The BabyJubjub sub-group order `l` (== order >> 3) as a BigUint.
fn sub_order() -> BigUint {
    BigUint::parse_bytes(b"2736030358979909402780800718157159386076813972158567259200215660948447373041", 10).unwrap()
}

fn fr_from_dec(s: &str) -> Fr {
    Fr::from_str_radix(s, 10).expect("valid decimal field element")
}

// ark-ff 0.5 dropped `Fr::from_str_radix`; provide a tiny BigUint-based helper.
trait FrFromStrRadix: Sized {
    fn from_str_radix(s: &str, radix: u32) -> Option<Self>;
}
impl FrFromStrRadix for Fr {
    fn from_str_radix(s: &str, radix: u32) -> Option<Self> {
        let b = BigUint::parse_bytes(s.as_bytes(), radix)?;
        Some(biguint_to_fr(&b))
    }
}

/// Reduce a BigUint into the BN254 scalar field.
fn biguint_to_fr(b: &BigUint) -> Fr {
    let be = b.to_bytes_be();
    Fr::from_be_bytes_mod_order(&be)
}

/// Fr -> canonical BigUint in [0, p).
fn fr_to_biguint(f: &Fr) -> BigUint {
    BigUint::from_bytes_be(&f.into_bigint().to_bytes_be())
}

/// Error returned when a caller-supplied point fails validation. Surfaced (instead of a panic) so
/// untrusted signature bytes from mobile cannot DoS the verifier with an off-curve / zero-denominator
/// input (audit L3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EddsaError {
    /// The point does not satisfy the BabyJubjub curve equation.
    NotOnCurve,
    /// The point is on-curve but outside the prime-order subgroup (small-subgroup / cofactor input).
    NotInSubgroup,
}

impl core::fmt::Display for EddsaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EddsaError::NotOnCurve => write!(f, "point not on BabyJubjub curve"),
            EddsaError::NotInSubgroup => write!(f, "point not in BabyJubjub prime-order subgroup"),
        }
    }
}

impl std::error::Error for EddsaError {}

/// A point on BabyJubjub in affine coordinates (x, y) over BN254 Fr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: Fr,
    pub y: Fr,
}

impl Point {
    /// The twisted-Edwards identity (0, 1).
    fn identity() -> Point {
        Point { x: Fr::zero(), y: Fr::one() }
    }

    /// Complete twisted-Edwards addition (a=168700, d=168696). Valid for all inputs (no special
    /// cases), matching circomlibjs `addPoint`. NOTE: the `.expect("nonzero denom")` below is
    /// unreachable for ON-CURVE inputs — BabyJubjub is a *complete* twisted Edwards curve (`a` a
    /// square, `d` a non-square in Fr), so `1 ± d·x1·x2·y1·y2 ≠ 0` for all on-curve points. Callers
    /// MUST validate untrusted points with `is_on_curve` before doing arithmetic; the trusted signing
    /// path only ever feeds Base8-derived points.
    fn add(&self, q: &Point) -> Point {
        let a = coeff_a();
        let d = coeff_d();
        let x1y2 = self.x * q.y;
        let y1x2 = self.y * q.x;
        let y1y2 = self.y * q.y;
        let x1x2 = self.x * q.x;
        let dxxyy = d * x1x2 * y1y2;
        let x3 = (x1y2 + y1x2) * (Fr::one() + dxxyy).inverse().expect("nonzero denom");
        let y3 = (y1y2 - a * x1x2) * (Fr::one() - dxxyy).inverse().expect("nonzero denom");
        Point { x: x3, y: y3 }
    }

    /// Variable-time scalar multiplication `n * self` via double-and-add. The branch on each bit makes
    /// this a timing/power side-channel on `n` — use ONLY for PUBLIC scalars (e.g. the verifier path).
    /// For secret scalars use [`Point::mul_scalar_ct`].
    fn mul_scalar(&self, n: &BigUint) -> Point {
        let mut result = Point::identity();
        let mut addend = *self;
        // little-endian bit iteration
        let bytes = n.to_bytes_le();
        for byte in bytes {
            let mut b = byte;
            for _ in 0..8 {
                if b & 1 == 1 {
                    result = result.add(&addend);
                }
                addend = addend.add(&addend);
                b >>= 1;
            }
        }
        result
    }

    /// Constant-time scalar multiplication `n * self` for SECRET scalars (the on-device consent key
    /// and per-signature nonce). Unlike [`Point::mul_scalar`], the control flow is independent of the
    /// bits of `n`: every step performs the same doubling and a branchless masked select, and the loop
    /// runs a fixed 256 iterations (our secret scalars are `< 2^253`). The mathematical result is
    /// bit-identical to `mul_scalar`, so circomlibjs / in-circuit `EdDSAPoseidonVerifier` parity is
    /// preserved (asserted by the fixed-vector tests). Audit L3.
    fn mul_scalar_ct(&self, n: &BigUint) -> Point {
        let mut result = Point::identity();
        let mut addend = *self;
        let mut bytes = n.to_bytes_le();
        // Fixed width: pad to >= 32 bytes so the iteration count does not depend on n's bit length.
        // (Never truncates: callers only pass scalars < 2^253, i.e. <= 32 bytes.)
        if bytes.len() < 32 {
            bytes.resize(32, 0);
        }
        for byte in bytes {
            for i in 0..8u32 {
                let bit = (byte >> i) & 1;
                let summed = result.add(&addend);
                // Branchless select: result = bit ? summed : result. Multiplication by the {0,1} mask
                // uses the same (Montgomery) field path regardless of the bit value -> no data branch.
                let m = Fr::from(bit as u64);
                let nm = Fr::one() - m;
                result = Point { x: summed.x * m + result.x * nm, y: summed.y * m + result.y * nm };
                addend = addend.add(&addend);
            }
        }
        result
    }

    /// BabyJubjub curve membership: `a·x² + y² == 1 + d·x²·y²`.
    fn is_on_curve(&self) -> bool {
        let xx = self.x * self.x;
        let yy = self.y * self.y;
        coeff_a() * xx + yy == Fr::one() + coeff_d() * xx * yy
    }

    /// Prime-order subgroup membership: `subOrder · P == identity`. MUST only be called on a point that
    /// is already known to be on-curve (otherwise the scalar mul could hit a zero denominator).
    fn is_in_subgroup(&self) -> bool {
        self.mul_scalar(&sub_order()) == Point::identity()
    }

    /// Validate a caller-supplied point: on-curve AND in the prime-order subgroup. Returns a typed
    /// error (instead of a downstream panic) for untrusted input. Audit L3.
    fn validate(&self) -> Result<(), EddsaError> {
        if !self.is_on_curve() {
            return Err(EddsaError::NotOnCurve);
        }
        if !self.is_in_subgroup() {
            return Err(EddsaError::NotInSubgroup);
        }
        Ok(())
    }
}

/// A derived BabyJubjub consent key: the 32-byte private seed plus public point A = (Ax, Ay).
#[derive(Clone, Debug)]
pub struct BabyjubConsentKey {
    /// The 32-byte private key (the circomlibjs `prv` buffer).
    pub prv: [u8; 32],
    pub ax: Fr,
    pub ay: Fr,
}

/// An EdDSA-BabyJubjub Poseidon signature: R8 point + scalar S.
#[derive(Clone, Debug)]
pub struct EddsaSignature {
    pub r8x: Fr,
    pub r8y: Fr,
    pub s: BigUint,
}

/// pruneBuffer over a 64-byte blake512 digest (circomlibjs): mutate bytes [0] and [31] of the low
/// half in place; the upper half (used as the nonce key) is untouched.
fn prune(buff: &mut [u8; 64]) {
    buff[0] &= 0xF8;
    buff[31] &= 0x7F;
    buff[31] |= 0x40;
}

/// `s = LE(sBuff[0..32])` as a BigUint (the pruned scalar).
fn scalar_from_low(sbuff: &[u8; 64]) -> BigUint {
    BigUint::from_bytes_le(&sbuff[0..32])
}

/// Derive the public point A = Base8 * (s >> 3) from a 32-byte private key, circomlibjs `prv2pub`.
fn prv2pub(prv: &[u8; 32]) -> Point {
    let mut sbuff = blake512(prv);
    prune(&mut sbuff);
    let s = scalar_from_low(&sbuff);
    let s_shr3 = &s >> 3u32;
    base8().mul_scalar_ct(&s_shr3) // secret scalar -> constant-time
}

/// Derive a deterministic BabyJubjub consent key from a hex seed.
///
/// The 32-byte circomlibjs private key is `blake512(domain || seed)[0..32]` (a distinct domain from
/// the secp256k1 wallet path so the two keys never collide). Pass a 0x.. hex seed of any length.
pub fn derive_babyjub_consent_key_from_seed(seed: &[u8]) -> BabyjubConsentKey {
    let mut buf = Vec::with_capacity(CONSENT_KEY_DOMAIN.len() + seed.len());
    buf.extend_from_slice(CONSENT_KEY_DOMAIN);
    buf.extend_from_slice(seed);
    let digest = blake512(&buf);
    let mut prv = [0u8; 32];
    prv.copy_from_slice(&digest[0..32]);
    let a = prv2pub(&prv);
    BabyjubConsentKey { prv, ax: a.x, ay: a.y }
}

/// Build a consent key directly from a 32-byte circomlibjs private key (no domain wrapping) — used
/// for circomlibjs parity vectors where the raw seed *is* the private key.
pub fn consent_key_from_raw_prv(prv: &[u8; 32]) -> BabyjubConsentKey {
    let a = prv2pub(prv);
    BabyjubConsentKey { prv: *prv, ax: a.x, ay: a.y }
}

/// Sign the consent message field element `m` with the 32-byte private key, circomlibjs
/// `signPoseidon`. Returns (R8x, R8y, S).
pub fn sign_poseidon(prv: &[u8; 32], m: &Fr) -> EddsaSignature {
    let mut sbuff = blake512(prv);
    prune(&mut sbuff);
    let s = scalar_from_low(&sbuff);
    let a = base8().mul_scalar_ct(&(&s >> 3u32)); // A = Base8 * (s>>3); secret scalar -> constant-time

    // composeBuff = sBuff[32..64] || LE(m, 32 bytes); r = LE(blake512(composeBuff)) mod subOrder
    let mut compose = Vec::with_capacity(64);
    compose.extend_from_slice(&sbuff[32..64]);
    let mut m_le = fr_to_biguint(m).to_bytes_le();
    m_le.resize(32, 0); // pad to 32 bytes LE
    compose.extend_from_slice(&m_le);

    let rbuff = blake512(&compose);
    let r = BigUint::from_bytes_le(&rbuff) % sub_order();
    let r8 = base8().mul_scalar_ct(&r); // secret nonce scalar -> constant-time

    // hm = Poseidon5([R8x, R8y, Ax, Ay, m])
    let hm = poseidon(&[r8.x, r8.y, a.x, a.y, *m]);
    let hm_big = fr_to_biguint(&hm);

    // S = (r + hm * s) mod subOrder
    let s_sig = (&r + (&hm_big * &s)) % sub_order();

    EddsaSignature { r8x: r8.x, r8y: r8.y, s: s_sig }
}

/// Verify an EdDSA-BabyJubjub Poseidon signature against public key A and message m
/// (circomlibjs `verifyPoseidon`).
///
/// The public key `A` and the signature point `R8` are CALLER-SUPPLIED (untrusted bytes from mobile),
/// so they are validated as on-curve + prime-order-subgroup points BEFORE any arithmetic. An off-curve
/// / small-subgroup point returns `Err` instead of panicking on a zero denominator (audit L3, reachable
/// DoS). `Ok(false)` means well-formed inputs whose signature simply does not verify (incl. `s` out of
/// range), matching the prior reject behavior.
pub fn verify_poseidon(
    ax: &Fr,
    ay: &Fr,
    r8x: &Fr,
    r8y: &Fr,
    s: &BigUint,
    m: &Fr,
) -> Result<bool, EddsaError> {
    let a = Point { x: *ax, y: *ay };
    let r8 = Point { x: *r8x, y: *r8y };
    // Validate untrusted points first — this is also what makes the `add` denominators provably
    // nonzero (curve completeness), so no downstream panic is reachable from this path.
    a.validate()?;
    r8.validate()?;

    if s >= &sub_order() {
        return Ok(false);
    }
    let hm = poseidon(&[r8.x, r8.y, a.x, a.y, *m]);
    let hm_big = fr_to_biguint(&hm);

    // Check: Base8 * S == R8 + (8*hm)*A  (exactly circomlibjs `verifyPoseidon`). Scalars here are
    // PUBLIC (S, hm), so variable-time mul_scalar is fine.
    let lhs = base8().mul_scalar(s);
    let a_hm = a.mul_scalar(&(&hm_big * BigUint::from(8u32)));
    let rhs = r8.add(&a_hm);
    Ok(lhs == rhs)
}

/// Convenience: Fr -> decimal string (for FFI / parity output).
pub fn fr_to_dec(f: &Fr) -> String {
    fr_to_biguint(f).to_str_radix(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    // circomlibjs reference vector for seed = 0x07 * 32 (the raw private key) and the anchor
    // consent message. Generated with `buildEddsa()` (circomlibjs 0.1.7) — see commit log.
    const SEED7: [u8; 32] = [7u8; 32];
    const AX: &str = "14422859473778768188622151430526693594403470008420308922992775064941455773685";
    const AY: &str = "7592518773672929099542717438998516546396504563265155469693554058278098107299";
    const MSG: &str = "8453154477584343887478389844545598795962583039369853412305694095390935992699";
    const R8X: &str = "902064620424496881921101910457335166452907362670474296709799481663161455483";
    const R8Y: &str = "2905613237943813585459385900172512868038628297396815629248623731388399618720";
    const S: &str = "880907594470456950988239052178620804384023364539879359665304279941876276164";

    #[test]
    fn prv2pub_matches_circomlibjs() {
        let key = consent_key_from_raw_prv(&SEED7);
        assert_eq!(fr_to_dec(&key.ax), AX, "Ax mismatch vs circomlibjs");
        assert_eq!(fr_to_dec(&key.ay), AY, "Ay mismatch vs circomlibjs");
    }

    #[test]
    fn sign_poseidon_matches_circomlibjs() {
        let m = fr_from_dec(MSG);
        let sig = sign_poseidon(&SEED7, &m);
        assert_eq!(fr_to_dec(&sig.r8x), R8X, "R8x mismatch vs circomlibjs");
        assert_eq!(fr_to_dec(&sig.r8y), R8Y, "R8y mismatch vs circomlibjs");
        assert_eq!(sig.s.to_str_radix(10), S, "S mismatch vs circomlibjs");
    }

    #[test]
    fn signature_round_trip_verifies() {
        let key = consent_key_from_raw_prv(&SEED7);
        let m = fr_from_dec(MSG);
        let sig = sign_poseidon(&SEED7, &m);
        assert_eq!(
            verify_poseidon(&key.ax, &key.ay, &sig.r8x, &sig.r8y, &sig.s, &m),
            Ok(true),
            "self-verify must succeed"
        );
        // tamper -> reject (well-formed inputs, signature mismatch)
        let bad_m = fr_from_dec("123");
        assert_eq!(
            verify_poseidon(&key.ax, &key.ay, &sig.r8x, &sig.r8y, &sig.s, &bad_m),
            Ok(false),
            "tampered message must be rejected"
        );
    }

    // ---- audit L3: constant-time secret-scalar mul is bit-identical to the variable-time path ----
    // The fixed circomlibjs vectors above already pin sign/prv2pub output; this asserts the CT and
    // VT muls agree directly so in-circuit EdDSAPoseidonVerifier parity cannot silently drift.
    #[test]
    fn mul_scalar_ct_matches_variable_time() {
        let scalars = [
            BigUint::from(0u32),
            BigUint::from(1u32),
            BigUint::from(2u32),
            BigUint::from(255u32),
            BigUint::parse_bytes(b"123456789012345678901234567890", 10).unwrap(),
            sub_order() - BigUint::from(1u32),
        ];
        for n in scalars {
            assert_eq!(base8().mul_scalar(&n), base8().mul_scalar_ct(&n), "CT vs VT mismatch for {n}");
        }
    }

    // ---- audit L3: caller-supplied off-curve / small-subgroup points return Err, never panic ----
    #[test]
    fn verify_rejects_off_curve_point_without_panic() {
        let key = consent_key_from_raw_prv(&SEED7);
        let m = fr_from_dec(MSG);
        let sig = sign_poseidon(&SEED7, &m);
        // (1, 1) is not on BabyJubjub: a·1 + 1 = 168701 != 1 + d·1 = 168697.
        let off = Fr::one();
        assert_eq!(
            verify_poseidon(&off, &off, &sig.r8x, &sig.r8y, &sig.s, &m),
            Err(EddsaError::NotOnCurve),
            "off-curve public key must error, not panic"
        );
        // off-curve R8 too
        assert_eq!(
            verify_poseidon(&key.ax, &key.ay, &off, &off, &sig.s, &m),
            Err(EddsaError::NotOnCurve),
            "off-curve R8 must error, not panic"
        );
    }

    #[test]
    fn point_validation_basics() {
        assert!(base8().is_on_curve() && base8().is_in_subgroup(), "Base8 must validate");
        assert!(Point::identity().validate().is_ok(), "identity must validate");
        assert_eq!(Point { x: Fr::one(), y: Fr::one() }.validate(), Err(EddsaError::NotOnCurve));
    }

    #[test]
    fn domain_derivation_is_deterministic_and_distinct() {
        let a = derive_babyjub_consent_key_from_seed(b"root-seed-material");
        let b = derive_babyjub_consent_key_from_seed(b"root-seed-material");
        assert_eq!(a.prv, b.prv, "derivation must be deterministic");
        // The domain-wrapped key differs from using the raw seed as the private key.
        let raw = consent_key_from_raw_prv(b"root-seed-material\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
        assert_ne!(a.ax, raw.ax, "domain separation must change the key");
    }
}
