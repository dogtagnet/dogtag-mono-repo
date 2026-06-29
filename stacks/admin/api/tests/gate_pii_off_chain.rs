//! PHASE-8 GATE — PII-off-chain negative test (BUILD_PROMPT §8 acceptance; impl §4.5/§11.1).
//!
//! Principle 7: "nothing personal on-chain, ever." Two backend-side assertions, the off-chain
//! analogs of the contracts' `test_dogTagId_is_not_hash_of_microchip`:
//!
//!   (A) The `dogTagId` allocated at mint/issue time (via `next_dog_tag_id`, exactly what
//!       `mint_pet` calls) is a NON-PERSONAL identifier — it must NEVER be derived from the
//!       low-entropy, brute-forceable microchip number, neither as `keccak256(microchip)` nor as
//!       `Poseidon(microchip)`.
//!
//!   (B) The only personal-derived value that touches chain is the SALTED Merkle leaf/root: hashing
//!       the same microchip into a leaf WITH a random 16-byte salt does NOT equal an UNSALTED hash
//!       of the microchip — salting is the privacy mechanism (an unsalted hash of a 15-digit chip is
//!       brute-forceable). We prove a fresh salt changes the leaf, so the on-chain commitment is not
//!       a recoverable function of the raw chip.
//!
//! Hermetic: MemStore + the pinned circomlib Poseidon SDK; no chain/network.

use admin_api::store::{MemStore, Store};
use dogtag_standard::leaf::hash_leaf;
use dogtag_standard::poseidon::poseidon;
use dogtag_standard::types::TypedScalar;
use dogtag_standard::{bytes_to_field, to_hex32};

/// A real ISO-11784/11785 15-digit microchip number (low entropy: ~10^15, brute-forceable).
const MICROCHIP: &str = "985141006580319";

#[tokio::test]
async fn dog_tag_id_is_never_a_hash_of_the_microchip() {
    // Allocate a dogTagId via the SAME path mint_pet uses (next_dog_tag_id on the central store).
    let store = MemStore::new();
    let dog_tag_id = store.next_dog_tag_id().await.to_string();

    // The two forbidden derivations of the chip.
    let keccak_chip = admin_api::auth::keccak256_hex(MICROCHIP); // "0x..." 32-byte hex
    let poseidon_chip = to_hex32(&poseidon(&[bytes_to_field(MICROCHIP.as_bytes())]));

    // (A1) dogTagId is NOT keccak256(microchip).
    assert!(
        !same_numeric(&dog_tag_id, &keccak_chip),
        "dogTagId must NOT be keccak256(microchip)"
    );
    // (A2) dogTagId is NOT Poseidon(microchip).
    assert!(
        !same_numeric(&dog_tag_id, &poseidon_chip),
        "dogTagId must NOT be Poseidon(microchip)"
    );
    // (A3) dogTagId is a low, non-personal sequential id — plainly not a 32-byte hash digest.
    assert!(
        dog_tag_id.bytes().all(|c| c.is_ascii_digit()),
        "dogTagId is a plain non-personal numeric id, not a hash digest: {dog_tag_id}"
    );
    assert!(
        dog_tag_id.len() < 20,
        "dogTagId is a small sequential id, not a hash"
    );

    // (B) the microchip only ever enters the commitment as a SALTED leaf. A salted leaf of the chip
    //     does NOT equal an unsalted Poseidon/keccak of the chip, and two fresh salts give different
    //     leaves -> the on-chain commitment is not a brute-forceable function of the raw chip.
    let scalar = TypedScalar::Str(MICROCHIP.to_string());
    let salt_a: [u8; 16] = [0x11; 16];
    let salt_b: [u8; 16] = [0x22; 16];
    let leaf_a =
        to_hex32(&hash_leaf("credentialSubject.microchip.code", &salt_a, &scalar).unwrap());
    let leaf_b =
        to_hex32(&hash_leaf("credentialSubject.microchip.code", &salt_b, &scalar).unwrap());

    assert_ne!(
        leaf_a, leaf_b,
        "different salts MUST give different leaves (salting works)"
    );
    assert!(
        !same_numeric(&leaf_a, &poseidon_chip) && !same_numeric(&leaf_a, &keccak_chip),
        "the salted leaf must NOT equal an unsalted hash of the microchip"
    );
}

/// True if both hex/decimal strings denote the same integer value (defends against an accidental
/// decimal-vs-hex representation collision). dogTagId is a small int and the hashes are 32-byte, so
/// they can never collide numerically — but we check rather than assume.
fn same_numeric(a: &str, b: &str) -> bool {
    match (parse_any(a), parse_any(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Canonicalize a decimal or 0x-hex string to a lowercase no-0x hex string with no leading zeros.
fn parse_any(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(hex) = t.strip_prefix("0x") {
        let canon = hex.trim_start_matches('0').to_lowercase();
        Some(if canon.is_empty() { "0".into() } else { canon })
    } else if !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit()) {
        let n: u128 = t.parse().ok()?;
        Some(format!("{n:x}"))
    } else {
        None
    }
}
