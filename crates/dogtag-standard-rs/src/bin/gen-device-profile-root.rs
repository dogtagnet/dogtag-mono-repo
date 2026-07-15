//! gen-device-profile-root - emit `contracts/test/device-profile-root.json`.
//!
//! Runs the REAL device-side builder ([`dogtag_standard::profile_tree::build_profile_tree`]) over a
//! fixed demo seed and writes the resulting `R` where Foundry can read it. `CustodialIssuance.t.sol`
//! then mints that `R` through `mintCustodial` and asserts `profileRoot(dogTagId) == R` - closing
//! the loop that the root an owner's app builds locally is exactly what the M5 contract seals.
//!
//! The committed output is drift-guarded by `tests/profile_tree_parity.rs`
//! (`committed_device_profile_root_matches_a_fresh_device_build`), so a builder change that would
//! move `R` fails the Rust gate rather than silently desyncing the contract test.
//!
//! Regenerate: `cargo run -p dogtag-standard-rs --bin gen-device-profile-root`
use dogtag_standard::field::to_hex32;
use dogtag_standard::profile_tree::{build_profile_tree, device_root_fixture_witness};

fn main() {
    let w = device_root_fixture_witness();
    let tree = build_profile_tree(&w.seed, w.dog_tag_id, &w.owner_address, &w.attributes)
        .unwrap_or_else(|e| panic!("build_profile_tree: {e}"));

    let json = serde_json::json!({
        "_comment":
            "R built by the DEVICE-side builder (dogtag-standard-rs profile_tree::build_profile_tree) \
             from the fixed demo wallet seed in profile_tree::device_root_fixture_witness(). \
             CustodialIssuance.t.sol mints this R and asserts profileRoot(dogTagId) == R. \
             Regenerate: cargo run -p dogtag-standard-rs --bin gen-device-profile-root",
        "dogTagId": w.dog_tag_id_dec,
        "R": to_hex32(&tree.root),
        "_ownerAddress": format!("0x{}", hex::encode(w.owner_address)),
    });

    let out = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../contracts/test/device-profile-root.json"
    );
    std::fs::write(out, serde_json::to_string_pretty(&json).unwrap() + "\n")
        .unwrap_or_else(|e| panic!("write {out}: {e}"));
    println!("wrote {out}\n  R = {}", to_hex32(&tree.root));
}
