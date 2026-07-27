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

use crate::merkle::build_merkle;
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

    /// DogTagIssuer.issuedBy(root) - the authoritative signer that issued `R` (== `clone.issuedBy[R]`),
    /// used to validate the envelope's `protocol.issuerSigner` *claim* (§4.3). Default: unwired => `Err`,
    /// which makes the additive issuer-signer check skip (base validity still governs). Live per-backend
    /// eth-calls are the later verify-path hardening brick (M7 P5); the SDK enforces it whenever wired.
    fn issued_by(&self, _document_store: &str, _merkle_root: &str) -> Result<String, AdapterError> {
        Err(AdapterError("issued_by not wired".to_string()))
    }
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
    // Single-document credentials only: `signature.proof` MUST be empty, so `targetHash` IS the
    // anchored root `R`. Doc→batch-root inclusion (a non-empty `proof`) never shipped, and the C1
    // invariant forbids trusting a permissive commutative fold in the trust path — so a non-empty
    // proof is rejected outright rather than folded (see merkle::process_proof, DSDP plan §2.3).
    let ok = doc.signature.proof.is_empty() && merkle_root == target_hash;
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

    // M7 provenance (§4.2/§4.3): a stamped `protocol.issuerSigner` is only the envelope's CLAIM of
    // who issued; validate it against the authoritative on-chain `clone.issuedBy[R]`. A wrong/forged
    // claim fails closed (issuance -> INVALID). This can only make verification STRICTER - the on-chain
    // validity re-derivation above still targets `doc.issuer.document_store`, NEVER the untrusted
    // block, so a forged `protocol` block can neither reroute validation nor make an invalid record
    // verify. Skipped when the block is absent (pre-M7) or the adapter is unwired (`Err`) - the base
    // validity governs. Wiring live per-backend `issuedBy` reads is the later hardening brick (M7 P5).
    let issuance = match (&doc.protocol, issuance) {
        (Some(p), FragmentState::Valid) => {
            match opts
                .rpc
                .issued_by(&doc.issuer.document_store, &doc.signature.merkle_root)
            {
                Ok(onchain) if onchain.eq_ignore_ascii_case(&p.issuer_signer) => FragmentState::Valid,
                Ok(_) => FragmentState::Invalid,
                Err(_) => FragmentState::Valid,
            }
        }
        (_, other) => other,
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

    /// Stamping `protocol.statusBaseUrl` into an ALREADY-ISSUED document must not disturb its anchored
    /// root. This is the premise the receipt-QR fix rests on: `check_integrity` folds only `data` plus
    /// `privacy.obfuscated`, so the whole `protocol` block is outside `R` and issuers can start
    /// stamping a reachable status host without invalidating a single credential already in the wild.
    #[test]
    fn stamping_a_status_base_url_does_not_move_the_merkle_root() {
        let doc = good_doc();
        let (before_state, before_root) = check_integrity(&doc);

        let mut stamped = doc.clone();
        stamped.protocol = Some(ProtocolMeta {
            status_base_url: Some("https://receipts.gov.example.org".to_string()),
            ..protocol_block("0x00000000000000000000000000000000000000a1")
        });
        let (after_state, after_root) = check_integrity(&stamped);

        assert_eq!(before_state, FragmentState::Valid);
        assert_eq!(after_state, FragmentState::Valid);
        assert_eq!(before_root, after_root, "the protocol block is outside R");
        assert_eq!(stamped.signature.merkle_root, doc.signature.merkle_root);
    }

    /// The field is optional and serializes as ABSENT, not `null`, so a renderer's "is there a status
    /// page?" test stays a plain presence check and pre-stamping documents round-trip byte-identically.
    #[test]
    fn an_unstamped_status_base_url_is_omitted_from_the_json_entirely() {
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block("0x00000000000000000000000000000000000000a1"));
        let json = serde_json::to_string(&doc).unwrap();
        assert!(!json.contains("statusBaseUrl"), "{json}");

        let back: WrappedDoc = serde_json::from_str(&json).unwrap();
        assert_eq!(back.protocol.unwrap().status_base_url, None);
    }

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

    #[test]
    fn nonempty_signature_proof_is_invalid() {
        // Batching (doc→batch-root inclusion via `signature.proof`) never shipped, and C1 forbids
        // trusting the permissive commutative fold; a non-empty proof is now rejected outright even
        // when it names the (single-doc) root itself. Documents the intentional tightening.
        let mut sp = fixed_salts();
        let mut doc = wrap_document(&sample_credential(), issuer(), &mut sp).unwrap();
        doc.signature.proof.push(doc.signature.target_hash.clone());
        assert_eq!(check_integrity(&doc).0, FragmentState::Invalid);
    }

    // --- M7 provenance: the `protocol` block is a routing hint, NEVER authority (§4.2/§4.3) ----
    //
    // A stamped `protocol.issuerSigner` is only the envelope's CLAIM of who issued; verify()
    // validates it against the authoritative on-chain `clone.issuedBy[R]` (here the injected
    // `issued_by`). A wrong/forged claim must NOT make a record verify.

    use crate::wrap::{ProtocolMeta, LEVEL_B_VERSION};

    /// The signer that actually issued R on-chain (== `clone.issuedBy[R]`).
    const SIGNER: &str = "0x00000000000000000000000000000000000515e6";

    struct MockRpcSigner {
        is_valid: bool,
        issued_by: String,
    }
    impl RpcAdapter for MockRpcSigner {
        fn is_valid(&self, _ds: &str, _root: &str, _c: u32) -> Result<bool, AdapterError> {
            Ok(self.is_valid)
        }
        fn owner_of(&self, _id: &str) -> Result<String, AdapterError> {
            Err(AdapterError("ownerOf unused (third-party)".into()))
        }
        fn issued_by(&self, _ds: &str, _root: &str) -> Result<String, AdapterError> {
            Ok(self.issued_by.clone())
        }
    }

    fn protocol_block(issuer_signer: &str) -> ProtocolMeta {
        ProtocolMeta {
            chain_id: 135,
            version: LEVEL_B_VERSION.to_string(),
            verification_registry: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".to_string(),
            issuer_clone: issuer().document_store,
            issuer_signer: issuer_signer.to_string(),
            status_base_url: None,
        }
    }

    fn provenance_opts<'a>(rpc: &'a MockRpcSigner) -> VerifyOpts<'a> {
        VerifyOpts {
            rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        }
    }

    #[test]
    fn provenance_matching_issuer_signer_verifies() {
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block(SIGNER)); // claim == on-chain issuedBy[R]
        let rpc = MockRpcSigner { is_valid: true, issued_by: SIGNER.to_string() };
        let v = verify(&doc, &provenance_opts(&rpc));
        assert_eq!(v.issuance, FragmentState::Valid);
        assert!(v.valid);
    }

    #[test]
    fn provenance_wrong_issuer_signer_does_not_verify() {
        // The load-bearing property: a forged `issuerSigner` claim must NOT make the record verify,
        // even though the record is otherwise on-chain-valid (is_valid == true). Provenance is a
        // routing hint, never authority.
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block("0x00000000000000000000000000000000deadbeef"));
        let rpc = MockRpcSigner { is_valid: true, issued_by: SIGNER.to_string() };
        let v = verify(&doc, &provenance_opts(&rpc));
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn provenance_absent_block_verifies_unchanged() {
        // §4.4 back-compat: a pre-M7 record (no `protocol` block) verifies exactly as before -
        // the additive issuer-signer check is skipped entirely.
        let doc = good_doc(); // protocol == None
        assert!(doc.protocol.is_none());
        let rpc = MockRpcSigner { is_valid: true, issued_by: SIGNER.to_string() };
        let v = verify(&doc, &provenance_opts(&rpc));
        assert_eq!(v.issuance, FragmentState::Valid);
        assert!(v.valid);
    }

    #[test]
    fn provenance_unwired_adapter_skips_check_not_fails() {
        // If the on-chain read is unwired (the default `issued_by` -> Err), the additive check is
        // skipped and base validity governs - a stamped block never regresses an unwired backend.
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block("0xanything"));
        let rpc = MockRpc { is_valid_res: Ok(true), owner_res: Ok(OWNER.to_string()) }; // no issued_by
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &MockDns(Ok(true)),
            registry: &MockRegistry(Ok(true)),
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: None,
        };
        let v = verify(&doc, &opts);
        assert_eq!(v.issuance, FragmentState::Valid);
        assert!(v.valid);
    }

    #[test]
    fn provenance_matching_block_cannot_rescue_onchain_invalid_record() {
        // The literal acceptance property: a forged/present block must NOT make an INVALID record
        // verify. Even a block whose issuerSigner MATCHES on-chain cannot rescue a record the chain
        // reports invalid (is_valid == false) - the additive check only ever tightens.
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block(SIGNER)); // claim == on-chain, yet base validity is false
        let rpc = MockRpcSigner { is_valid: false, issued_by: SIGNER.to_string() };
        let v = verify(&doc, &provenance_opts(&rpc));
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn provenance_block_does_not_weaken_integrity() {
        // A stamped block cannot make an integrity-tampered doc verify.
        let mut doc = good_doc();
        let subj = doc.data["credentialSubject"].as_object_mut().unwrap();
        let packed = subj["name"].as_str().unwrap();
        let parts: Vec<&str> = packed.splitn(3, ':').collect();
        subj.insert("name".to_string(), Value::String(format!("{}:{}:Max", parts[0], parts[1])));
        doc.protocol = Some(protocol_block(SIGNER));
        let rpc = MockRpcSigner { is_valid: true, issued_by: SIGNER.to_string() };
        let v = verify(&doc, &provenance_opts(&rpc));
        assert_eq!(v.integrity, FragmentState::Invalid);
        assert!(!v.valid);
    }

}
