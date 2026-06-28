//! Three-pillar contextual verification (impl §11.3 — supersedes §1.7) —
//! mirror of packages/dogtag-standard-ts/src/verify.ts.
//!
//! Validity = integrity AND issuance AND identity (the 3 authenticity pillars). `ownership` is a
//! CONTEXTUAL 4th fragment: gates only the owner's self-import; NOT_APPLICABLE for third parties.
//!
//! The TS network adapters are async; in Rust they are modeled as synchronous TRAITS whose
//! methods return `Result<_, AdapterError>` (an `Err` signals a transient ERROR, matching the
//! TS `try { ... } catch { state = "ERROR" }` shape). Only the pure `check_integrity` pillar and
//! the contextual `verify` orchestration shape are implemented here.
use ark_bn254::Fr;

use crate::merkle::{build_merkle, process_proof};
use crate::wrap::{flatten_data, from_hex32, leaf_from_packed, WrappedDoc};

/// 4-state fragment result (impl §11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentState {
    Valid,
    Invalid,
    Error,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub valid: bool,
    pub integrity: FragmentState,
    pub issuance: FragmentState,
    pub identity: FragmentState,
    pub ownership: FragmentState,
}

/// A transient adapter failure -> the corresponding fragment becomes ERROR.
#[derive(Debug)]
pub struct AdapterError(pub String);

/// Network adapters are injected so the core SDK stays pure/offline (mobile + server share it).
pub trait RpcAdapter {
    /// DogTagIssuer.isValid(root) at >= `confirmations` blocks. Err -> transient ERROR.
    fn is_valid(
        &self,
        document_store: &str,
        merkle_root: &str,
        confirmations: u32,
    ) -> Result<bool, AdapterError>;
    /// DogTagSBT.ownerOf(dogTagId). Err -> transient ERROR.
    fn owner_of(&self, dog_tag_id: &str) -> Result<String, AdapterError>;
}

pub trait DnsAdapter {
    /// True iff a TXT record of `domain` binds `documentStore` on `chainId`. Err -> ERROR.
    fn txt_matches(
        &self,
        domain: &str,
        document_store: &str,
        chain_id: u64,
    ) -> Result<bool, AdapterError>;
}

pub trait RegistryAdapter {
    /// The admin-written central registry knows this (domain, documentStore) pair.
    fn knows(&self, domain: &str, document_store: &str) -> Result<bool, AdapterError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    SelfImport,
    ThirdParty,
}

pub struct VerifyOpts<'a> {
    pub rpc: &'a dyn RpcAdapter,
    pub dns: &'a dyn DnsAdapter,
    pub registry: &'a dyn RegistryAdapter,
    pub mode: VerifyMode,
    pub user_wallet_address: Option<String>,
    pub confirmations: Option<u32>,
}

/// Paths that must be present and are NON-obfuscatable (audit-05 V3/V6).
const NON_OBFUSCATABLE: &[&str] = &["credentialSubject.dogTagId"];

/// `^0x[0-9a-fA-F]{64}$`
fn is_hex32(h: &str) -> bool {
    match h.strip_prefix("0x") {
        Some(rest) => rest.len() == 64 && rest.bytes().all(|c| c.is_ascii_hexdigit()),
        None => false,
    }
}

/// Pure integrity pillar: rebuild the WHOLE tree (never trust processProof alone — C1) and
/// compare to targetHash, then resolve the proof to merkleRoot. Returns the recomputed root + state.
pub fn check_integrity(doc: &WrappedDoc) -> (FragmentState, Fr) {
    let zero = Fr::from(0u64);
    for h in &doc.privacy.obfuscated {
        if !is_hex32(h) {
            return (FragmentState::Invalid, zero);
        }
    }
    let data_flat = flatten_data(&doc.data);
    for req in NON_OBFUSCATABLE {
        if !data_flat.iter().any(|(kp, _)| kp == req) {
            return (FragmentState::Invalid, zero); // required + non-obfuscatable
        }
    }
    // Recompute live leaves; any malformed packed entry -> INVALID.
    let mut live_leaves: Vec<Fr> = Vec::with_capacity(data_flat.len());
    for (kp, packed) in &data_flat {
        match leaf_from_packed(kp, packed) {
            Ok(f) => live_leaves.push(f),
            Err(_) => return (FragmentState::Invalid, zero),
        }
    }
    // Parse obfuscated hashes (already hex32-validated above).
    let mut obf: Vec<Fr> = Vec::with_capacity(doc.privacy.obfuscated.len());
    for h in &doc.privacy.obfuscated {
        match from_hex32(h) {
            Ok(f) => obf.push(f),
            Err(_) => return (FragmentState::Invalid, zero),
        }
    }
    // obfuscated entries must not overlap live-leaf hashes (D1)
    for o in &obf {
        if live_leaves.iter().any(|l| l == o) {
            return (FragmentState::Invalid, zero);
        }
    }
    let mut all = live_leaves.clone();
    all.extend(obf.iter().copied());
    if all.is_empty() {
        return (FragmentState::Invalid, zero);
    }
    let root = build_merkle(&all).root;
    let target_hash = match from_hex32(&doc.signature.target_hash) {
        Ok(t) => t,
        Err(_) => return (FragmentState::Invalid, root),
    };
    if root != target_hash {
        return (FragmentState::Invalid, root);
    }
    let merkle_root = match from_hex32(&doc.signature.merkle_root) {
        Ok(m) => m,
        Err(_) => return (FragmentState::Invalid, root),
    };
    let ok = if doc.signature.proof.is_empty() {
        merkle_root == target_hash
    } else {
        let proof: Result<Vec<Fr>, _> = doc.signature.proof.iter().map(|h| from_hex32(h)).collect();
        match proof {
            Ok(p) => process_proof(&p, target_hash) == merkle_root,
            Err(_) => false,
        }
    };
    (if ok { FragmentState::Valid } else { FragmentState::Invalid }, root)
}

/// Extract the cleartext dogTagId value (the packed value after `salt:tag:`).
fn dog_tag_id_of(doc: &WrappedDoc) -> Option<String> {
    let entry = flatten_data(&doc.data)
        .into_iter()
        .find(|(kp, _)| kp == "credentialSubject.dogTagId")?;
    // packed: salt:tag:value — value may contain ':', so re-join the tail.
    let parts: Vec<&str> = entry.1.splitn(3, ':').collect();
    parts.get(2).map(|s| s.to_string())
}

/// Full contextual verify (impl §11.3).
pub fn verify(doc: &WrappedDoc, opts: &VerifyOpts) -> Verdict {
    let confirmations = opts.confirmations.unwrap_or(5);
    let integrity = check_integrity(doc).0;

    let issuance = match opts.rpc.is_valid(
        &doc.issuer.document_store,
        &doc.signature.merkle_root,
        confirmations,
    ) {
        Ok(true) => FragmentState::Valid,
        Ok(false) => FragmentState::Invalid,
        Err(_) => FragmentState::Error,
    };

    let identity = match (
        opts.dns
            .txt_matches(&doc.issuer.domain, &doc.issuer.document_store, 135),
        opts.registry
            .knows(&doc.issuer.domain, &doc.issuer.document_store),
    ) {
        (Ok(txt), Ok(known)) => {
            if txt && known {
                FragmentState::Valid
            } else {
                FragmentState::Invalid
            }
        }
        _ => FragmentState::Error,
    };

    let credential_valid = integrity == FragmentState::Valid
        && issuance == FragmentState::Valid
        && identity == FragmentState::Valid;

    let ownership;
    let valid;
    match opts.mode {
        VerifyMode::SelfImport => {
            let wallet = opts
                .user_wallet_address
                .as_ref()
                .expect("self-import requires userWalletAddress");
            ownership = match dog_tag_id_of(doc).map(|id| opts.rpc.owner_of(&id)) {
                Some(Ok(owner)) => {
                    if owner.to_lowercase() == wallet.to_lowercase() {
                        FragmentState::Valid
                    } else {
                        FragmentState::Invalid
                    }
                }
                _ => FragmentState::Error,
            };
            valid = credential_valid && ownership == FragmentState::Valid;
        }
        VerifyMode::ThirdParty => {
            ownership = match &opts.user_wallet_address {
                Some(wallet) => match dog_tag_id_of(doc).map(|id| opts.rpc.owner_of(&id)) {
                    Some(Ok(owner)) => {
                        if owner.to_lowercase() == wallet.to_lowercase() {
                            FragmentState::Valid
                        } else {
                            FragmentState::Invalid
                        }
                    }
                    _ => FragmentState::Error,
                },
                None => FragmentState::NotApplicable,
            };
            valid = credential_valid; // ownership does NOT gate third-party validity
        }
    }

    Verdict {
        valid,
        integrity,
        issuance,
        identity,
        ownership,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::to_hex32;
    use crate::wrap::{obfuscate, wrap_document, IssuerMeta};
    use serde_json::{json, Value};

    fn fixed_salts() -> impl FnMut() -> [u8; 16] {
        let mut n: u8 = 1;
        move || {
            let s = [n; 16];
            n = n.wrapping_add(1);
            s
        }
    }

    fn sample_credential() -> Value {
        json!({
            "credentialSubject": {
                "dogTagId": {"tag": 3, "value": "42"},
                "name": {"tag": 2, "value": "Rex"},
                "microchip": {"code": {"tag": 2, "value": "985141006580311"}},
                "weightHistory": [{"value": {"tag": 4, "value": "22.7"}}]
            }
        })
    }

    fn issuer() -> IssuerMeta {
        IssuerMeta {
            name: "Acme Vet".to_string(),
            domain: "acme.example".to_string(),
            document_store: "0x0000000000000000000000000000000000000001".to_string(),
            record_type: "VACCINATION".to_string(),
        }
    }

    #[test]
    fn integrity_valid_and_root_matches_target() {
        let mut sp = fixed_salts();
        let doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        let (state, root) = check_integrity(&doc);
        assert_eq!(state, FragmentState::Valid);
        assert_eq!(to_hex32(&root), doc.signature.target_hash);
        assert_eq!(to_hex32(&root), doc.signature.merkle_root);
    }

    #[test]
    fn obfuscate_keeps_target_hash_and_integrity() {
        let mut sp = fixed_salts();
        let doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        let target = doc.signature.target_hash.clone();
        let obf = obfuscate(&doc, &["credentialSubject.name".to_string()]).unwrap();
        assert_eq!(obf.signature.target_hash, target);
        assert_eq!(obf.privacy.obfuscated.len(), 1);
        // cleartext "Rex" gone
        let flat = flatten_data(&obf.data);
        assert!(!flat.iter().any(|(k, _)| k == "credentialSubject.name"));
        let (state, root) = check_integrity(&obf);
        assert_eq!(state, FragmentState::Valid);
        assert_eq!(to_hex32(&root), target);
    }

    #[test]
    fn tampered_value_is_invalid() {
        let mut sp = fixed_salts();
        let mut doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        // tamper name: keep salt+tag, change value Rex -> Max
        let subj = doc.data["credentialSubject"].as_object_mut().unwrap();
        let packed = subj["name"].as_str().unwrap();
        let parts: Vec<&str> = packed.splitn(3, ':').collect();
        let tampered = format!("{}:{}:Max", parts[0], parts[1]);
        subj.insert("name".to_string(), Value::String(tampered));
        assert_eq!(check_integrity(&doc).0, FragmentState::Invalid);
    }

    #[test]
    fn missing_dog_tag_id_is_invalid() {
        let mut sp = fixed_salts();
        let mut doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        doc.data["credentialSubject"]
            .as_object_mut()
            .unwrap()
            .remove("dogTagId");
        assert_eq!(check_integrity(&doc).0, FragmentState::Invalid);
    }

    #[test]
    fn malformed_obfuscated_entry_is_invalid() {
        let mut sp = fixed_salts();
        let mut doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        doc.privacy.obfuscated.push("0xdeadbeef".to_string()); // not 32 bytes
        assert_eq!(check_integrity(&doc).0, FragmentState::Invalid);
    }

    // --- verify() orchestration via injected mock adapters ------------------
    //
    // The pure orchestration shape (mode gating, ownership semantics, ERROR
    // propagation) is exercised here with deterministic mocks. `Result<_, ()>`
    // is mapped to AdapterError so an `Err` arm models a transient adapter
    // failure exactly as the trait contract requires.

    struct MockRpc {
        is_valid_res: Result<bool, ()>,
        owner_res: Result<String, ()>,
    }
    impl RpcAdapter for MockRpc {
        fn is_valid(&self, _ds: &str, _root: &str, _conf: u32) -> Result<bool, AdapterError> {
            self.is_valid_res.map_err(|_| AdapterError("rpc".into()))
        }
        fn owner_of(&self, _id: &str) -> Result<String, AdapterError> {
            self.owner_res.clone().map_err(|_| AdapterError("rpc".into()))
        }
    }

    struct MockDns(Result<bool, ()>);
    impl DnsAdapter for MockDns {
        fn txt_matches(&self, _d: &str, _ds: &str, _c: u64) -> Result<bool, AdapterError> {
            self.0.map_err(|_| AdapterError("dns".into()))
        }
    }

    struct MockRegistry(Result<bool, ()>);
    impl RegistryAdapter for MockRegistry {
        fn knows(&self, _d: &str, _ds: &str) -> Result<bool, AdapterError> {
            self.0.map_err(|_| AdapterError("reg".into()))
        }
    }

    fn good_doc() -> WrappedDoc {
        let mut sp = fixed_salts();
        wrap_document(&sample_credential(), issuer(), &mut sp).unwrap()
    }

    // sample_credential's dogTagId value is "42"; owner_of receives that id.
    const OWNER: &str = "0xAbC0000000000000000000000000000000000001";

    #[test]
    fn self_import_all_pillars_valid() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::SelfImport,
            user_wallet_address: Some(OWNER.to_lowercase()), // case-insensitive match
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.integrity, FragmentState::Valid);
        assert_eq!(v.issuance, FragmentState::Valid);
        assert_eq!(v.identity, FragmentState::Valid);
        assert_eq!(v.ownership, FragmentState::Valid);
        assert!(v.valid);
    }

    #[test]
    fn self_import_owner_mismatch_gates_validity() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::SelfImport,
            user_wallet_address: Some("0xdead".to_string()),
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.ownership, FragmentState::Invalid);
        assert!(!v.valid); // credential pillars valid, but ownership gates self-import
    }

    #[test]
    fn self_import_owner_lookup_error_is_error_not_invalid() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Err(()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::SelfImport,
            user_wallet_address: Some(OWNER.to_string()),
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.ownership, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn third_party_without_wallet_is_not_applicable_and_does_not_gate() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.ownership, FragmentState::NotApplicable);
        assert!(v.valid); // third-party validity = credential pillars only
    }

    #[test]
    fn third_party_owner_mismatch_does_not_gate_validity() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: Some("0xother".to_string()),
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.ownership, FragmentState::Invalid);
        assert!(v.valid); // ownership Invalid but still valid for third parties
    }

    #[test]
    fn issuance_false_makes_invalid() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(false), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn issuance_adapter_error_is_error_state() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Err(()), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.issuance, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn identity_requires_both_txt_and_registry() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        // TXT matches but registry does not know -> Invalid (not Error)
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(false)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.identity, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn identity_adapter_error_is_error_state() {
        let doc = good_doc();
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Err(())), // transient DNS failure
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.identity, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn tampered_doc_invalid_integrity_gates_all_modes() {
        let mut doc = good_doc();
        // tamper dogTagId value so integrity recomputation fails
        let subj = doc.data["credentialSubject"].as_object_mut().unwrap();
        let packed = subj["name"].as_str().unwrap();
        let parts: Vec<&str> = packed.splitn(3, ':').collect();
        subj.insert("name".to_string(), Value::String(format!("{}:{}:Max", parts[0], parts[1])));
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) };
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.integrity, FragmentState::Invalid);
        assert!(!v.valid);
    }
}
