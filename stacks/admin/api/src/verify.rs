//! Credential-import verification helpers (impl §4.1). Full three-pillar on-chain verification lives in
//! the SDK (`dogtag_standard::verify`) and needs a live RPC; the central importer reuses the SDK's
//! canonicalization to do a STRUCTURAL integrity check — recompute the Merkle root from the doc's
//! packed `data` (salt:tag:value leaves) and require it to match the embedded `signature.merkleRoot`.
//! We never reimplement canonicalization.

use dogtag_standard::field::to_hex32;
use dogtag_standard::merkle::build_merkle;
use dogtag_standard::wrap::{flatten_data, leaf_from_packed, WrappedDoc};

/// Recompute the Merkle root over the disclosed leaves and require it == the embedded merkleRoot.
/// (Obfuscated fields would need their stored leaf hashes folded in; for a fully-disclosed import
/// this is the integrity pillar.) Returns false on any malformed leaf.
pub fn structural_valid(doc: &WrappedDoc) -> bool {
    if !doc.privacy.obfuscated.is_empty() {
        // we don't reconstruct obfuscated leaves here; treat partial docs as un-importable.
        return false;
    }
    let flat = flatten_data(&doc.data);
    if flat.is_empty() {
        return false;
    }
    let mut leaves = Vec::with_capacity(flat.len());
    for (kp, packed) in &flat {
        match leaf_from_packed(kp, packed) {
            Ok(fr) => leaves.push(fr),
            Err(_) => return false,
        }
    }
    let root = to_hex32(&build_merkle(&leaves).root);
    root.eq_ignore_ascii_case(&doc.signature.merkle_root)
}

/// Extract the cleartext dogTagId value from a wrapped doc's `data`.
pub fn dog_tag_id_of(doc: &WrappedDoc) -> Option<String> {
    let entry = flatten_data(&doc.data)
        .into_iter()
        .find(|(kp, _)| kp == "credentialSubject.dogTagId")?;
    // packed is salt:tag:value — take everything after the second colon.
    let parts: Vec<&str> = entry.1.splitn(3, ':').collect();
    parts.get(2).map(|s| s.to_string())
}
