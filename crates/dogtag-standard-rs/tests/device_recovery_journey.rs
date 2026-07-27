//! The owner-facing recovery journey, driven through the **FFI** - the exact entry point
//! `apps/ios/DogTag/ProfileTreeStore.swift` calls (`buildProfileTreeHex` / `dogTagIdFieldHex`).
//!
//! `profile_tree.rs`'s unit tests already pin determinism inside the core. This file exists for the
//! claim the core tests do NOT pin, and which `docs/MOBILE_OWNER_SECRET.md` states precisely:
//!
//! > the seed re-derives ONLY the owner-control core; attribute values+salts are NOT seed-derivable
//! > and must come back from the wrapped credential. **Seed + credential == same R; seed ALONE != same R.**
//!
//! Both halves of that sentence are asserted here, so the docs cannot quietly drift into the
//! stronger (and false) "the phrase alone restores everything".
//!
//! Run with `--nocapture` for the readable transcript.

use dogtag_standard::ffi::{
    build_profile_tree_hex, derive_owner_secret_hex, dog_tag_id_field_hex, AttributeLeafFfi,
};
use dogtag_standard::types::TypeTag;

/// Clearly-labelled TEST MATERIAL. A real seed is the 64-byte BIP-39 seed behind the 24-word phrase.
const PHRASE_SEED: &str = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
/// A DIFFERENT wallet's phrase - an attacker, or the user typing the wrong words.
const OTHER_SEED: &str = "0x1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";
const OWNER_ADDR: &str = "0x00000000000000000000000000000000deadbeef";
const TAG_DEC: &str = "424242";

/// The attribute leaves the ISSUER minted into the credential. Values AND salts originate with the
/// issuer, so they are not seed-derivable - they come back from the wrapped credential
/// (`"<saltHex>:<tag>:<value>"`), which is why recovery needs the credential too.
fn credential_attributes() -> Vec<AttributeLeafFfi> {
    vec![
        AttributeLeafFfi {
            key_path: "credentialSubject.name".to_string(),
            salt_hex: "0x0102030405060708090a0b0c0d0e0f10".to_string(),
            tag: TypeTag::String as u8,
            value: "Rex".to_string(),
        },
        AttributeLeafFfi {
            key_path: "credentialSubject.breedLabel".to_string(),
            salt_hex: "0x1112131415161718191a1b1c1d1e1f20".to_string(),
            tag: TypeTag::String as u8,
            value: "Shiba Inu".to_string(),
        },
    ]
}

fn build(seed: &str, attrs: Vec<AttributeLeafFfi>) -> dogtag_standard::ffi::ProfileTreeFfi {
    let tag_hex = dog_tag_id_field_hex(TAG_DEC.to_string()).expect("canonical dogTagId field");
    build_profile_tree_hex(seed.to_string(), tag_hex, OWNER_ADDR.to_string(), attrs)
        .expect("device-side build")
}

/// Seed + credential rebuilds the SAME `R`; a different phrase, or a lost credential, does not.
#[test]
fn a_lost_phone_is_recovered_by_the_phrase_plus_the_credential() {
    let tag_hex = dog_tag_id_field_hex(TAG_DEC.to_string()).unwrap();

    // --- Day 1: the owner's phone builds the tag's tree locally -------------------------------
    let original = build(PHRASE_SEED, credential_attributes());
    println!("\n=== DAY 1 - the owner's phone builds the tag tree (nothing leaves the device) ===");
    println!("  dogTagId (decimal handle) : {TAG_DEC}");
    println!("  dogTagId (canonical field): {tag_hex}");
    println!(
        "  owner-secret [DEVICE-ONLY]: {}",
        original.owner_secret_hex
    );
    println!("  consent key Ax [DEVICE-ONLY]: {}", original.ax_hex);
    println!(
        "  R  --> handed to the issuer, sealed as profileRoot: {}",
        original.root_hex
    );

    // --- Day 2: the phone is lost. Documents/dogtag-owner-secrets.json is backup-excluded, so
    //            NOTHING device-local survives. The owner has their 24 words + the credential.
    let recovered = build(PHRASE_SEED, credential_attributes());
    println!("\n=== DAY 2 - phone lost. New phone: 24-word phrase + the issued credential ===");
    println!(
        "  owner-secret re-derived   : {}",
        recovered.owner_secret_hex
    );
    println!("  R rebuilt                 : {}", recovered.root_hex);
    println!(
        "  --> tag recovered: {}",
        if recovered.root_hex == original.root_hex {
            "YES - R matches profileRoot"
        } else {
            "NO"
        }
    );

    assert_eq!(
        recovered.owner_secret_hex, original.owner_secret_hex,
        "the phrase must re-derive the owner-secret (the nullifier's secret leaf)"
    );
    assert_eq!(
        recovered.root_hex, original.root_hex,
        "seed + credential must rebuild the identical R - profileRoot is write-once on-chain"
    );
    // The whole owner-control core returns, not just the secret.
    assert_eq!(recovered.ax_hex, original.ax_hex);
    assert_eq!(recovered.ay_hex, original.ay_hex);
    assert_eq!(recovered.owner_salt_hex, original.owner_salt_hex);
    assert_eq!(recovered.key_salt_hex, original.key_salt_hex);
    assert_eq!(recovered.secret_salt_hex, original.secret_salt_hex);
    // The standalone KDF agrees with what the builder embedded.
    assert_eq!(
        derive_owner_secret_hex(PHRASE_SEED.to_string(), tag_hex).unwrap(),
        original.owner_secret_hex
    );

    // --- Someone else's phrase must NOT reach this tag ------------------------------------------
    let attacker = build(OTHER_SEED, credential_attributes());
    println!("\n=== A DIFFERENT phrase (wrong words / another wallet) ===");
    println!("  owner-secret : {}", attacker.owner_secret_hex);
    println!("  R            : {}", attacker.root_hex);
    println!("  --> does NOT match profileRoot: tag NOT recovered (correct)");
    assert_ne!(
        attacker.owner_secret_hex, original.owner_secret_hex,
        "a different seed must never derive this tag's owner-secret"
    );
    assert_ne!(
        attacker.root_hex, original.root_hex,
        "a different seed must not rebuild R"
    );

    // --- The precise documented limit: the phrase ALONE is not enough ---------------------------
    // The owner restores the phrase but cannot produce the credential, so they guess the attributes.
    // Attribute salts are issuer-supplied and NOT seed-derivable, so R moves.
    let mut without_credential = credential_attributes();
    without_credential[0].salt_hex = "0xffffffffffffffffffffffffffffffff".to_string();
    let guessed = build(PHRASE_SEED, without_credential);
    println!("\n=== The RIGHT phrase, but the credential is gone (attribute salts unknown) ===");
    println!(
        "  owner-secret re-derived   : {} (correct - it is seed-derived)",
        guessed.owner_secret_hex
    );
    println!("  R rebuilt                 : {}", guessed.root_hex);
    println!("  --> R does NOT match profileRoot: the phrase ALONE does not restore the tag.");
    println!("      This is the documented limit (docs/MOBILE_OWNER_SECRET.md): seed + credential == same R.\n");
    assert_eq!(
        guessed.owner_secret_hex, original.owner_secret_hex,
        "the owner-control core is seed-derived, so it still returns without the credential"
    );
    assert_ne!(
        guessed.root_hex, original.root_hex,
        "attribute salts are NOT seed-derivable - the seed alone must not be claimed to rebuild R"
    );
}
