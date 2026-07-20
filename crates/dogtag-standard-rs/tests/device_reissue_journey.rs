//! The owner-facing **re-issue** recovery journey (Level-B M6, decision D3), driven through the
//! **FFI** - the exact entry point `apps/ios/DogTag/ProfileTreeStore.swift` calls
//! (`buildProfileTreeHex` / `dogTagIdFieldHex`).
//!
//! This is the OTHER branch of recovery from `device_recovery_journey.rs`. That file pins the
//! *repair* branch: seed **+** credential rebuild the SAME `R`, so the same tag keeps working. This
//! file pins the branch taken when repair is **impossible** - the owner-secret is gone for good (no
//! seed backup, or the credential's attribute leaves cannot be re-obtained):
//!
//! > Recovery is NOT an on-chain repair. Per decision D3 (`DogTagSBTConsent.sol:35`), it is a
//! > **fresh custodial issuance under a NEW `dogTagId` with a new `R`.** The old tag is simply
//! > abandoned - `profileRoot` is write-once, so there is no on-chain remedy, and a burned/abandoned
//! > id can never be re-minted.
//!
//! The load-bearing property this asserts is **mutual unlinkability**: the abandoned tag and the
//! re-issued tag share nothing an on-chain observer can correlate. The owner-secret is the only
//! owner-private input to the on-chain `nullifier`
//! (`Poseidon6(DS_NULLIFIER, ownerSecret, dogTagId, purpose, relayer, consentNonce)`), and it is
//! bound to `dogTagId` - so a fresh id yields a fresh, independent secret even for the SAME wallet.
//! A "recovery" that reused the secret, or that linked old id -> new id anywhere off the device,
//! would reintroduce exactly the linkage Level-B removes.
//!
//! Run with `--nocapture` for the readable transcript.

use dogtag_standard::ffi::{
    build_profile_tree_hex, derive_owner_secret_hex, dog_tag_id_field_hex, AttributeLeafFfi,
};
use dogtag_standard::types::TypeTag;

/// Clearly-labelled TEST MATERIAL. A real seed is the 64-byte BIP-39 seed behind the 24-word phrase.
const PHRASE_SEED: &str = "0x000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
/// A brand-new wallet the owner sets up when even the 24-word phrase is gone.
const FRESH_WALLET_SEED: &str =
    "0xa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf";
const OWNER_ADDR: &str = "0x00000000000000000000000000000000deadbeef";
/// A brand-new wallet has a brand-new address, too.
const FRESH_OWNER_ADDR: &str = "0x000000000000000000000000000000000badf00d";

/// The abandoned tag's decimal handle, and the FRESH handle the issuer allocates for the re-issue.
/// They MUST differ: a burned/abandoned `dogTagId` is retired forever (`mintCustodial` rejects any
/// id whose `profileRoot` is already set), so recovery can never reuse the old id.
const OLD_TAG_DEC: &str = "424242";
const NEW_TAG_DEC: &str = "515151";

/// The attribute leaves the ISSUER minted into the credential. Values AND salts originate with the
/// issuer, so they are not seed-derivable. A re-issue is a FRESH credential: the issuer supplies new
/// attribute leaves (new salts), which is one more reason `R` moves.
fn old_credential_attributes() -> Vec<AttributeLeafFfi> {
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

/// The re-issued credential describes the SAME pet, but every salt is freshly drawn by the issuer.
fn reissued_credential_attributes() -> Vec<AttributeLeafFfi> {
    vec![
        AttributeLeafFfi {
            key_path: "credentialSubject.name".to_string(),
            salt_hex: "0xf1f2f3f4f5f6f7f8f9fafbfcfdfeff00".to_string(),
            tag: TypeTag::String as u8,
            value: "Rex".to_string(),
        },
        AttributeLeafFfi {
            key_path: "credentialSubject.breedLabel".to_string(),
            salt_hex: "0xe1e2e3e4e5e6e7e8e9eaebecedeeeff0".to_string(),
            tag: TypeTag::String as u8,
            value: "Shiba Inu".to_string(),
        },
    ]
}

fn build(
    seed: &str,
    tag_dec: &str,
    owner_addr: &str,
    attrs: Vec<AttributeLeafFfi>,
) -> dogtag_standard::ffi::ProfileTreeFfi {
    let tag_hex = dog_tag_id_field_hex(tag_dec.to_string()).expect("canonical dogTagId field");
    build_profile_tree_hex(seed.to_string(), tag_hex, owner_addr.to_string(), attrs)
        .expect("device-side build")
}

/// D3: repair is impossible, so recovery issues a FRESH tag. The re-issued tag must be independent of
/// the abandoned one in every value an on-chain observer can see - above all the owner-secret.
#[test]
fn a_lost_owner_secret_is_recovered_by_re_issuing_a_fresh_tag() {
    let old_tag_hex = dog_tag_id_field_hex(OLD_TAG_DEC.to_string()).unwrap();
    let new_tag_hex = dog_tag_id_field_hex(NEW_TAG_DEC.to_string()).unwrap();
    assert_ne!(old_tag_hex, new_tag_hex, "the issuer must allocate a DIFFERENT dogTagId");

    // --- Day 1: the owner's phone builds tag A locally; the issuer seals R_A as profileRoot(A) ----
    let old = build(PHRASE_SEED, OLD_TAG_DEC, OWNER_ADDR, old_credential_attributes());
    println!("\n=== DAY 1 - the owner's phone builds tag A (dogTagId {OLD_TAG_DEC}) ===");
    println!("  owner-secret A [DEVICE-ONLY]: {}", old.owner_secret_hex);
    println!("  R_A --> sealed as profileRoot(A): {}", old.root_hex);

    // --- Catastrophe: the phone is lost AND the 24-word phrase was never backed up (or the
    //     credential's attribute leaves cannot be re-obtained). The owner-secret is GONE. Tag A can
    //     never be proven again: profileRoot(A) is write-once, so there is no on-chain remedy, and
    //     `device_recovery_journey.rs`'s seed+credential repair is not available here. Per D3 the
    //     remedy is NOT a rebind - it is a fresh custodial issuance under a NEW id.

    // --- Re-issue, case 1: the seed SURVIVED (only the credential/attributes were lost). The issuer
    //     allocates a fresh id B; the same wallet builds a fresh tree for B. -------------------------
    let reissued = build(
        PHRASE_SEED,
        NEW_TAG_DEC,
        OWNER_ADDR,
        reissued_credential_attributes(),
    );
    println!("\n=== RE-ISSUE (D3) - fresh custodial issuance under NEW dogTagId {NEW_TAG_DEC} ===");
    println!("  owner-secret B [DEVICE-ONLY]: {}", reissued.owner_secret_hex);
    println!("  R_B --> sealed as profileRoot(B): {}", reissued.root_hex);

    // The re-issued tag is a genuinely fresh, independent credential.
    assert_ne!(
        reissued.root_hex, old.root_hex,
        "a re-issue must produce a new R - old proofs stay bound to the abandoned tag's immutable R"
    );
    // THE unlinkability property: the owner-secret is bound to dogTagId, so even the SAME wallet's
    // re-issued tag gets an independent secret. The abandoned tag's on-chain nullifiers therefore
    // cannot be correlated with the re-issued tag's - which is the whole point of D3 over a rebind.
    assert_ne!(
        reissued.owner_secret_hex, old.owner_secret_hex,
        "the re-issued tag must have an INDEPENDENT owner-secret (unlinkable nullifiers)"
    );
    // And the standalone KDF agrees the new secret is bound to the NEW id, not the old.
    assert_eq!(
        derive_owner_secret_hex(PHRASE_SEED.to_string(), new_tag_hex.clone()).unwrap(),
        reissued.owner_secret_hex,
        "owner-secret B must be the KDF over the NEW dogTagId"
    );
    assert_ne!(
        derive_owner_secret_hex(PHRASE_SEED.to_string(), old_tag_hex.clone()).unwrap(),
        reissued.owner_secret_hex,
        "owner-secret B must NOT equal the KDF over the OLD dogTagId"
    );

    // The consent key is ALSO bound to dogTagId, so the re-issued tag gets an independent consent
    // pubkey too. The whole owner-control core (owner-secret, reserved salts, consent key) is now
    // uniformly per-tag: there is no longer any wallet-level value shared across a wallet's tags
    // that could cross-link the abandoned tag with the re-issued one if it ever escaped the device.
    assert_ne!(
        reissued.ax_hex, old.ax_hex,
        "the re-issued tag must have an INDEPENDENT consent pubkey (bound to dogTagId)"
    );

    // --- Re-issue, case 2: even the phrase is gone, so the owner sets up a BRAND-NEW wallet. The
    //     re-issue is still just a fresh custodial issuance - now with a fresh seed and address too. -
    let reissued_fresh_wallet = build(
        FRESH_WALLET_SEED,
        NEW_TAG_DEC,
        FRESH_OWNER_ADDR,
        reissued_credential_attributes(),
    );
    println!("\n=== RE-ISSUE from a BRAND-NEW wallet (the phrase was gone too) ===");
    println!("  owner-secret : {}", reissued_fresh_wallet.owner_secret_hex);
    println!("  R            : {}", reissued_fresh_wallet.root_hex);
    assert_ne!(
        reissued_fresh_wallet.owner_secret_hex, old.owner_secret_hex,
        "a fresh wallet's re-issued tag is independent of the abandoned tag"
    );
    assert_ne!(
        reissued_fresh_wallet.root_hex, old.root_hex,
        "a fresh wallet's re-issued tag has a new R"
    );
    assert_ne!(
        reissued_fresh_wallet.ax_hex, old.ax_hex,
        "a fresh wallet has a fresh consent key too"
    );

    println!("\n  --> Tag A is abandoned (retired forever on-chain); the pet is now on a fresh,");
    println!("      mutually-unlinkable tag B. Recovery = re-issue, never a rebind (D3).\n");
}
