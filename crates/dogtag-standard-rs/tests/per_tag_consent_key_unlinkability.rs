//! One wallet, two dog tags: the consent key must not cross-link them.
//!
//! This exercises the property through the SAME UniFFI entry point the holder apps call
//! (`build_profile_tree_hex` — iOS `ProfileTreeStore`, Android likewise), so it checks what a real
//! owner's device actually produces rather than an internal helper. Two pets registered under ONE
//! recovery phrase must yield independent `(Ax, Ay)`, independent `owner.consentKey` leaves and
//! therefore independent `R`.
//!
//! (The retired per-wallet `v1` consent key was by construction identical for both tags — exactly
//! the cross-linking vector the per-tag derivation removes.)
//!
//! Run with `-- --nocapture` to read the transcript.

use dogtag_standard::ffi::{
    build_profile_tree_hex, dog_tag_id_field_hex, AttributeLeafFfi, ProfileTreeFfi,
};

/// One recovery phrase's BIP-39 seed. TEST MATERIAL ONLY, never holds value.
const SEED_HEX: &str = "0x6f6e652d77616c6c65742d74776f2d746167732d746573742d736565642d6d6174";
const OWNER_ADDR_HEX: &str = "0x00000000000000000000000000000000deadbeef";

fn attrs(pet_name: &str) -> Vec<AttributeLeafFfi> {
    vec![
        AttributeLeafFfi {
            key_path: "credentialSubject.name".to_string(),
            salt_hex: "0x000102030405060708090a0b0c0d0e0f".to_string(),
            tag: 1,
            value: pet_name.to_string(),
        },
        AttributeLeafFfi {
            key_path: "credentialSubject.species".to_string(),
            salt_hex: "0x0f0e0d0c0b0a09080706050403020100".to_string(),
            tag: 1,
            value: "dog".to_string(),
        },
    ]
}

fn register(handle: &str, pet_name: &str) -> ProfileTreeFfi {
    let id_field = dog_tag_id_field_hex(handle.to_string()).expect("canonical dogTagId field");
    build_profile_tree_hex(
        SEED_HEX.to_string(),
        id_field,
        OWNER_ADDR_HEX.to_string(),
        attrs(pet_name),
    )
    .expect("device-side profile tree build")
}

fn short(hex: &str) -> String {
    format!("{}…{}", &hex[..10], &hex[hex.len() - 6..])
}

#[test]
fn one_wallet_two_tags_get_independent_consent_keys() {
    // The owner registers two pets on ONE phone, under ONE recovery phrase.
    let rex = register("424242", "Rex");
    let milo = register("424243", "Milo");

    println!("\n=== One wallet (one recovery phrase), two dog tags ===\n");
    println!("{:<24} {:<24} {:<24}", "", "tag 424242 (Rex)", "tag 424243 (Milo)");
    for (label, a, b) in [
        ("consent Ax", &rex.ax_hex, &milo.ax_hex),
        ("consent Ay", &rex.ay_hex, &milo.ay_hex),
        ("consent keyHash", &rex.key_hash_hex, &milo.key_hash_hex),
        (
            "owner.consentKey leaf",
            &rex.consent_key_leaf_hex,
            &milo.consent_key_leaf_hex,
        ),
        ("owner secret", &rex.owner_secret_hex, &milo.owner_secret_hex),
        ("profile root R", &rex.root_hex, &milo.root_hex),
    ] {
        let verdict = if a == b { "SHARED  <-- cross-linkable" } else { "independent" };
        println!("{label:<24} {:<24} {:<24} {verdict}", short(a), short(b));
    }

    println!(
        "\nR sealed on-chain as profileRoot(dogTagId):\n  Rex : {}\n  Milo: {}\n",
        rex.root_hex, milo.root_hex
    );

    // The per-tag binding, all the way down the chain the owner's privacy depends on.
    assert_ne!(rex.ax_hex, milo.ax_hex, "consent Ax must differ per tag");
    assert_ne!(rex.ay_hex, milo.ay_hex, "consent Ay must differ per tag");
    assert_ne!(
        rex.key_hash_hex, milo.key_hash_hex,
        "consent keyHash must differ per tag"
    );
    assert_ne!(
        rex.consent_key_leaf_hex, milo.consent_key_leaf_hex,
        "the committed owner.consentKey leaf must differ per tag"
    );
    assert_ne!(rex.root_hex, milo.root_hex, "R must differ per tag");

    // Determinism: re-deriving on a restored wallet reproduces the same tag, so recovery still works.
    assert_eq!(
        rex.root_hex,
        register("424242", "Rex").root_hex,
        "re-deriving the same tag from the same seed must reproduce R"
    );
}
