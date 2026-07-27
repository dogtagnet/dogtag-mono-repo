//! Contextual verification (impl §11.3 — supersedes §1.7).
//!
//! Validity = integrity AND issuance AND identity AND the factory-anchored issuer-whitelist pillar.
//! `ownership` is a CONTEXTUAL 5th fragment: gates only the owner's self-import; NOT_APPLICABLE for
//! third parties.
//!
//! **This is NO LONGER a mirror of `packages/dogtag-standard-ts/src/verify.ts`.** That file still
//! carries the pre-#97 shape (every read against `doc.issuer.documentStore`, `issuedBy` compared to
//! `doc.protocol.issuerSigner`, `catch -> VALID`). It has no production consumer — only its own unit
//! test — so it was recorded rather than re-implemented here; see AGENTS.md. Do not read either file
//! as evidence about the other until they are reconciled.
//!
//! The TS network adapters are async; in Rust they are modeled as synchronous TRAITS whose
//! methods return `Result<_, AdapterError>` (an `Err` signals a transient ERROR, matching the
//! TS `try { ... } catch { state = "ERROR" }` shape). Only the pure `check_integrity` pillar and
//! the contextual `verify` orchestration shape are implemented here.
use ark_bn254::Fr;
use sha3::{Digest, Keccak256};

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

/// What this verifier's own factory answered for a root — the adapter-facing anchor result.
///
/// `doc.issuer.documentStore` sits OUTSIDE the Merkle root, so it is chosen by whoever built the
/// document. Reading `isValid`/`issuedBy`/`recordType` against it is asking the suspect for its own
/// references. The verifier's OWN `DogTagIssuerFactory.rootIssuer[R]` is write-once and writable only
/// from inside an `isClone` contract's `issue()`, so it names the clone that issued THIS root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuerAnchor {
    /// The factory names this clone as the issuer of the root.
    Resolved(String),
    /// We ASKED and the factory has no record of this root. Evidence ABOUT the credential.
    NoRecord,
    /// This verifier has NO factory configured, so we never asked. OUR gap, not evidence.
    NoFactoryConfigured,
}

/// How the issuing clone was arrived at, as REPORTED on the verdict. Mirrors [`IssuerAnchor`] plus
/// the case the adapter signals with `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuerResolution {
    /// This verifier's factory names a clone for this root. Every read below is made against it.
    Resolved,
    /// We ASKED and the factory has no record of this root. That is evidence ABOUT the credential.
    NoRecord,
    /// This verifier has NO factory configured, so we never asked. That is OUR gap, not evidence
    /// about the credential, and it must not condemn it.
    NoFactoryConfigured,
    /// The question could not be put to the chain at all — an unreachable node, or a deployment
    /// whose configured factory address is malformed. Neither a pass nor evidence: it gates.
    ReadFailed,
}

/// The issuer-whitelist pillar's outcome. Deliberately NOT a [`FragmentState`]: `NotApplicable`
/// already means "does not gate" for `ownership`, and folding "we never asked" into it would make
/// our own misconfiguration indistinguishable in kind from a deliberate skip.
///
/// The vocabulary is the one vet-api's `POST /verify/credential` already puts on the wire as
/// `issuerWhitelistState` (PR #96), so the two surfaces read identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssuerWhitelistState {
    /// Resolved, and the on-chain originator is whitelisted for the clone's own record type.
    Passed,
    /// Resolved, and NOT authorised (or the envelope relabelled the record type): a real failure.
    Failed,
    /// Indeterminate — the anchor, the originator or the whitelist read could not be established.
    /// Never a pass. This is where `NoRecord` and `ReadFailed` land: both gate.
    Unresolved,
    /// We never asked, because this verifier has no factory configured. The ONLY non-`Passed` state
    /// that does not gate the verdict.
    UnavailableNoFactoryConfigured,
}

impl IssuerWhitelistState {
    /// May this pillar contribute to a pass?
    ///
    /// Only a DEFINITE `Passed` does — plus the one case that is OUR gap rather than evidence about
    /// the credential. Everything else is indeterminate, and an indeterminate pillar is a failure to
    /// establish a claim, never a neutral skipped step.
    pub fn permits_pass(self) -> bool {
        matches!(
            self,
            IssuerWhitelistState::Passed | IssuerWhitelistState::UnavailableNoFactoryConfigured
        )
    }
}

/// `0x`-hex keccak256 of a record-type label — the `IssuerRegistry` whitelist key and the value a
/// clone's immutable `recordType()` returns. MUST equal the backend's `chain::record_type_key`
/// (pinned by `record_type_key_matches_the_vet_backend` in `stacks/vet/api`).
pub fn record_type_key(record_type: &str) -> String {
    format!(
        "0x{}",
        hex::encode(Keccak256::digest(record_type.as_bytes()))
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub valid: bool,
    pub integrity: FragmentState,
    pub issuance: FragmentState,
    pub identity: FragmentState,
    pub ownership: FragmentState,
    /// The factory-anchored issuer-whitelist pillar. MANDATORY: only [`IssuerWhitelistState::Passed`]
    /// (or our own [`IssuerWhitelistState::UnavailableNoFactoryConfigured`] gap) permits a pass.
    pub issuer_whitelist: IssuerWhitelistState,
    /// How [`Verdict::issuer_addr`] was arrived at. A caller MUST be able to tell "not evaluated"
    /// from "evaluated and passed", so this is reported explicitly and never as a silent absence.
    pub issuer_resolution: IssuerResolution,
    /// The clone the on-chain reads were ACTUALLY made against. With
    /// `issuer_resolution != Resolved` this is the document's own unverified claim, which is exactly
    /// why the fragments above cannot carry a verdict on their own.
    pub issuer_addr: String,
}

/// A transient adapter failure -> the corresponding fragment becomes ERROR.
#[derive(Debug)]
pub struct AdapterError(pub String);

/// Network adapters are injected so the core SDK stays pure/offline (mobile + server share it).
///
/// **No method here has a default implementation, deliberately.** `issued_by` used to default to
/// `Err(..)` = "unwired", which the orchestration then read as "skip the check" — so the one
/// implementor that never wired it silently ran with the provenance check dead and nothing said so.
/// A required method makes the compiler force every implementor to decide, and removes "unwired" as
/// a concept: a deployment with no factory says so with [`IssuerResolution::NoFactoryConfigured`].
pub trait RpcAdapter {
    /// DogTagIssuer.isValid(root) at >= `confirmations` blocks, against the RESOLVED clone.
    /// Err -> transient ERROR.
    fn is_valid(
        &self,
        issuer_addr: &str,
        merkle_root: &str,
        confirmations: u32,
    ) -> Result<bool, AdapterError>;
    /// DogTagSBT.ownerOf(dogTagId). Err -> transient ERROR.
    fn owner_of(&self, dog_tag_id: &str) -> Result<String, AdapterError>;

    /// `DogTagIssuerFactory.rootIssuer(R)` read against THIS VERIFIER'S OWN configured factory.
    ///
    /// It takes no factory address on purpose: an address the caller or the document could name is
    /// an address an attacker can name, which is the very substitution this anchor exists to close.
    /// The implementor supplies its own from configuration, or reports
    /// [`IssuerResolution::NoFactoryConfigured`].
    ///
    /// `Ok(NoRecord)` ("we asked, the chain has no record") and `Ok(NoFactoryConfigured)` ("we never
    /// asked") are distinct because only the first is evidence about the credential.
    fn root_issuer(&self, merkle_root: &str) -> Result<IssuerAnchor, AdapterError>;

    /// `DogTagIssuer.issuedBy(root)` — the originator that actually called `issue(root)` on this
    /// clone, or `Ok(None)` when the clone never issued it (the on-chain zero address).
    ///
    /// `issue()` is `onlyWhitelisted` (`DogTagIssuer.sol:40,55`), so a genuinely issued root's
    /// originator was whitelisted for its record type at issuance by construction. This is what lets
    /// the pillar resolve its own signer instead of trusting one the document names.
    fn issued_by(
        &self,
        issuer_addr: &str,
        merkle_root: &str,
    ) -> Result<Option<String>, AdapterError>;

    /// `DogTagIssuer.recordType()` — the clone's own immutable record-type key, fixed by the
    /// factory's `createIssuer` at KYC time, or `Ok(None)` for the zero word (uninitialized / not a
    /// clone). Read from the RESOLVED clone so the whitelist question is asked about the record type
    /// the CHAIN says this root belongs to, never the one the envelope claims.
    fn issuer_record_type(&self, issuer_addr: &str) -> Result<Option<String>, AdapterError>;

    /// `IssuerRegistry.isWhitelistedFor(recordTypeKey, signer)` against THIS VERIFIER'S OWN
    /// registry — same reasoning as [`RpcAdapter::root_issuer`]: the registry address is never
    /// supplied by the document or the caller.
    fn is_whitelisted_for(&self, record_type_key: &str, signer: &str)
        -> Result<bool, AdapterError>;
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
    (
        if ok {
            FragmentState::Valid
        } else {
            FragmentState::Invalid
        },
        root,
    )
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
    let root = &doc.signature.merkle_root;
    let integrity = check_integrity(doc).0;

    // ── The anchor: WHICH contract answers for this credential ────────────────────────────────
    //
    // `issuer.documentStore` is only the document's CLAIM. The `issuer` block sits OUTSIDE the
    // Merkle root, so an attacker can point it at a contract they deployed and it will answer
    // `isValid`/`issuedBy`/`recordType` however they like. This asks the verifier's OWN factory
    // instead — `rootIssuer[R]` is write-once and only an `isClone` contract can write it.
    let (issuer_resolution, resolved_clone) = match opts.rpc.root_issuer(root) {
        Ok(IssuerAnchor::Resolved(a)) => (IssuerResolution::Resolved, Some(a)),
        Ok(IssuerAnchor::NoRecord) => (IssuerResolution::NoRecord, None),
        Ok(IssuerAnchor::NoFactoryConfigured) => (IssuerResolution::NoFactoryConfigured, None),
        Err(_) => (IssuerResolution::ReadFailed, None),
    };
    // The address every read below is made against. The factory's answer wins; the document's own
    // claim is the LAST resort, reached only when the factory named nobody — and in every one of
    // those cases except `NoFactoryConfigured` the mandatory pillar is indeterminate, so nothing
    // read from that contract can produce a pass.
    let issuer_addr = resolved_clone
        .clone()
        .unwrap_or_else(|| doc.issuer.document_store.clone());

    let issuance = match opts.rpc.is_valid(&issuer_addr, root, confirmations) {
        Ok(true) => FragmentState::Valid,
        Ok(false) => FragmentState::Invalid,
        Err(_) => FragmentState::Error,
    };

    // ── The issuer-whitelist pillar — MANDATORY, and SELF-RESOLVING ───────────────────────────
    //
    // It asks the chain WHO issued the root (`issuedBy`, set to `msg.sender` under `onlyWhitelisted`,
    // `DogTagIssuer.sol:40,55`) and checks THAT signer against THIS verifier's registry. Never an
    // address the document names, or the attacker supplies both sides of the question.
    //
    // Only a definite `Passed` may contribute to a pass; see [`IssuerWhitelistState::permits_pass`].
    let claimed_rt_key = record_type_key(&doc.issuer.record_type);
    let (issuer_whitelist, onchain_signer) = match &resolved_clone {
        None => (
            match issuer_resolution {
                // We never asked. OUR misconfiguration, not evidence about the credential.
                IssuerResolution::NoFactoryConfigured => {
                    IssuerWhitelistState::UnavailableNoFactoryConfigured
                }
                // `NoRecord` = we asked and the chain has no record of this root: that IS evidence,
                // so it gates. `ReadFailed` = we could not ask: not evidence either way, and by the
                // standing rule it is not a pass — so it gates too, just never as a `Failed`.
                _ => IssuerWhitelistState::Unresolved,
            },
            None,
        ),
        Some(clone) => match opts.rpc.issued_by(clone, root) {
            Err(_) => (IssuerWhitelistState::Unresolved, None),
            // The clone the factory named never issued this root: indeterminate, never a pass.
            Ok(None) => (IssuerWhitelistState::Unresolved, None),
            Ok(Some(signer)) => match opts.rpc.issuer_record_type(clone) {
                Err(_) => (IssuerWhitelistState::Unresolved, Some(signer)),
                // Uninitialized, or not a clone at all.
                Ok(None) => (IssuerWhitelistState::Unresolved, Some(signer)),
                // The envelope claims a record type the CHAIN does not agree this clone issues.
                // `issuer.recordType` is outside the Merkle root, so relabelling a genuine
                // credential across types is free — and it is the whole point of the label. Binding
                // the claim to the clone's own immutable `recordType()` is what refuses it.
                Ok(Some(chain_rt_key)) if !chain_rt_key.eq_ignore_ascii_case(&claimed_rt_key) => {
                    (IssuerWhitelistState::Failed, Some(signer))
                }
                // Ask about the record type the CLONE declares, not the one the envelope claims,
                // so a relabelled credential is never checked against the wrong whitelist key.
                Ok(Some(chain_rt_key)) => (
                    match opts.rpc.is_whitelisted_for(&chain_rt_key, &signer) {
                        Ok(true) => IssuerWhitelistState::Passed,
                        Ok(false) => IssuerWhitelistState::Failed,
                        Err(_) => IssuerWhitelistState::Unresolved,
                    },
                    Some(signer),
                ),
            },
        },
    };

    // ── M7 provenance (§4.2/§4.3): the `protocol` block is a routing hint, NEVER authority ─────
    //
    // A stamped `protocol.issuerSigner` is only the envelope's CLAIM of who issued. It is validated
    // against the on-chain originator — and it is evaluated on EVERY path, not only the
    // factory-resolved one. Folding it inside the resolved branch would silently drop a check that
    // DID run and could fail a verdict, on exactly the factory-less deployments where it is the last
    // check standing; that regression is the one PR #96 had to undo.
    //
    // It may only ever TIGHTEN. On an unanchored path both the contract and the expected answer come
    // from the document, so a MATCH there proves nothing and promotes nothing — but a MISMATCH is
    // still a definite failure, and refusing to draw it would discard the honest case where
    // `documentStore` is real and only the claim was forged.
    //
    // The old `Err(_) => Valid` fail-open is gone: a read that could not run is not a pass.
    let issuance = match (&doc.protocol, issuance) {
        (Some(p), FragmentState::Valid) => {
            let onchain: Result<Option<String>, AdapterError> = match &resolved_clone {
                // Already read against the anchor above; reuse it rather than ask twice.
                Some(_) => Ok(onchain_signer.clone()),
                None => opts.rpc.issued_by(&issuer_addr, root),
            };
            match onchain {
                Ok(Some(s)) if s.eq_ignore_ascii_case(&p.issuer_signer) => FragmentState::Valid,
                Ok(Some(_)) => FragmentState::Invalid,
                // The chain reports no originator for this root (or the read failed), so the claim
                // names a signer nothing corroborates. Not a pass — but not evidence of forgery
                // either, so ERROR rather than INVALID.
                Ok(None) | Err(_) => FragmentState::Error,
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
        && identity == FragmentState::Valid
        && issuer_whitelist.permits_pass();

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
        issuer_whitelist,
        issuer_resolution,
        issuer_addr,
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

    use std::collections::{HashMap, HashSet};

    /// An ADDRESS-KEYED fake chain.
    ///
    /// The mock this replaces ignored the contract address entirely (`fn is_valid(&self, _ds: &str,
    /// ..)`), so it could not represent "the hostile contract answers `true` while the clone the
    /// factory named answers `false`" — every forged-issuer test written against it would have
    /// passed for the wrong reason. That exact false negative has now bitten this repo three times
    /// (#94's `MemChain::issue` hardcoding `issued_by`, #96's need for a first-class `HostileClone`),
    /// so this fake is address-keyed from the start and every read below takes the contract it is
    /// asked of seriously.
    #[derive(Default)]
    struct MockChain {
        /// Does this VERIFIER have a factory configured at all? `false` models the deployment that
        /// deliberately has none — "we never asked", not "we asked and got nothing".
        has_factory: bool,
        /// The verifier's own factory index: root -> the clone it names. Absent while `has_factory`
        /// is set == the factory has NO RECORD of this root, which is evidence about the credential.
        root_issuers: HashMap<String, String>,
        /// (contract, root) -> `isValid`. Absent == false.
        is_valid: HashMap<(String, String), bool>,
        /// (contract, root) -> `issuedBy`. Absent == the zero address == never issued HERE.
        issued_by: HashMap<(String, String), String>,
        /// contract -> its immutable `recordType()` key.
        record_types: HashMap<String, String>,
        /// (recordTypeKey, signer) whitelisted in THIS verifier's registry.
        whitelist: HashSet<(String, String)>,
        owner: Option<String>,
        /// Reads forced to fail, modelling a transient adapter error per read kind.
        failing: HashSet<&'static str>,
    }

    impl MockChain {
        /// A chain on which `doc` is exactly what it claims to be: the verifier's factory names
        /// `CLONE` for the root, `CLONE` issued it from `SIGNER`, declares record type
        /// `VACCINATION`, and `SIGNER` is whitelisted for that type. Every test below starts here
        /// and breaks ONE thing, so a failure names the term it broke.
        fn genuine(doc: &WrappedDoc) -> Self {
            let root = doc.signature.merkle_root.to_lowercase();
            let rt = record_type_key("VACCINATION");
            let mut c = MockChain {
                has_factory: true,
                owner: Some(OWNER.to_string()),
                ..Default::default()
            };
            c.root_issuers.insert(root.clone(), CLONE.to_lowercase());
            c.is_valid
                .insert((CLONE.to_lowercase(), root.clone()), true);
            c.issued_by
                .insert((CLONE.to_lowercase(), root), SIGNER.to_lowercase());
            c.record_types.insert(CLONE.to_lowercase(), rt.clone());
            c.whitelist.insert((rt, SIGNER.to_lowercase()));
            c
        }
        /// Install a contract the factory never deployed, answering with attacker-chosen values.
        fn with_hostile(mut self, addr: &str, root: &str, is_valid: bool, issued_by: &str) -> Self {
            let k = (addr.to_lowercase(), root.to_lowercase());
            self.is_valid.insert(k.clone(), is_valid);
            self.issued_by.insert(k, issued_by.to_lowercase());
            self.record_types
                .insert(addr.to_lowercase(), record_type_key("VACCINATION"));
            self
        }
        fn no_factory(mut self) -> Self {
            self.has_factory = false;
            self.root_issuers.clear();
            self
        }
        /// The factory is configured but has NO RECORD of this root.
        fn no_record(mut self) -> Self {
            self.root_issuers.clear();
            self
        }
        fn failing(mut self, what: &'static str) -> Self {
            self.failing.insert(what);
            self
        }
        fn without_whitelist(mut self) -> Self {
            self.whitelist.clear();
            self
        }
        fn with_valid_at(mut self, addr: &str, root: &str, v: bool) -> Self {
            self.is_valid
                .insert((addr.to_lowercase(), root.to_lowercase()), v);
            self
        }
        fn owner(mut self, o: Option<&str>) -> Self {
            self.owner = o.map(str::to_string);
            self
        }
        fn check(&self, what: &'static str) -> Result<(), AdapterError> {
            if self.failing.contains(what) {
                Err(AdapterError(format!("{what} read failed")))
            } else {
                Ok(())
            }
        }
    }

    impl RpcAdapter for MockChain {
        fn is_valid(&self, issuer_addr: &str, root: &str, _c: u32) -> Result<bool, AdapterError> {
            self.check("is_valid")?;
            Ok(*self
                .is_valid
                .get(&(issuer_addr.to_lowercase(), root.to_lowercase()))
                .unwrap_or(&false))
        }
        fn owner_of(&self, _id: &str) -> Result<String, AdapterError> {
            self.check("owner_of")?;
            self.owner
                .clone()
                .ok_or_else(|| AdapterError("ownerOf".into()))
        }
        fn root_issuer(&self, root: &str) -> Result<IssuerAnchor, AdapterError> {
            self.check("root_issuer")?;
            if !self.has_factory {
                return Ok(IssuerAnchor::NoFactoryConfigured);
            }
            Ok(match self.root_issuers.get(&root.to_lowercase()) {
                Some(a) => IssuerAnchor::Resolved(a.clone()),
                None => IssuerAnchor::NoRecord,
            })
        }
        fn issued_by(&self, addr: &str, root: &str) -> Result<Option<String>, AdapterError> {
            self.check("issued_by")?;
            Ok(self
                .issued_by
                .get(&(addr.to_lowercase(), root.to_lowercase()))
                .cloned())
        }
        fn issuer_record_type(&self, addr: &str) -> Result<Option<String>, AdapterError> {
            self.check("issuer_record_type")?;
            Ok(self.record_types.get(&addr.to_lowercase()).cloned())
        }
        fn is_whitelisted_for(&self, rt: &str, signer: &str) -> Result<bool, AdapterError> {
            self.check("is_whitelisted_for")?;
            Ok(self
                .whitelist
                .contains(&(rt.to_lowercase(), signer.to_lowercase())))
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

    use crate::wrap::{ProtocolMeta, LEVEL_B_VERSION};

    /// The clone the FACTORY names for the root — and, in `issuer()`, also the document's own
    /// `documentStore`, so an honest document has the two agreeing.
    const CLONE: &str = "0x0000000000000000000000000000000000000001";
    /// A contract the factory never deployed, deployed by whoever built the forged document.
    const HOSTILE: &str = "0x000000000000000000000000000000000000ba0b";
    /// The signer that actually issued R on-chain (== `clone.issuedBy[R]`).
    const SIGNER: &str = "0x00000000000000000000000000000000000515e6";

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

    fn opts_for<'a>(
        chain: &'a MockChain,
        dns: &'a MockDns,
        registry: &'a MockRegistry,
        mode: VerifyMode,
        wallet: Option<String>,
    ) -> VerifyOpts<'a> {
        VerifyOpts {
            rpc: chain,
            dns,
            registry,
            mode,
            user_wallet_address: wallet,
            confirmations: None,
        }
    }

    /// Third-party verify against a chain, with both identity sources agreeing.
    fn third_party(doc: &WrappedDoc, chain: &MockChain) -> Verdict {
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(true));
        verify(
            doc,
            &opts_for(chain, &dns, &registry, VerifyMode::ThirdParty, None),
        )
    }

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

    // --- the baseline: a genuine credential still passes every pillar --------------------------

    #[test]
    fn self_import_all_pillars_valid() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc);
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(true));
        let v = verify(
            &doc,
            &opts_for(
                &chain,
                &dns,
                &registry,
                VerifyMode::SelfImport,
                Some(OWNER.to_lowercase()), // case-insensitive match
            ),
        );
        assert_eq!(v.integrity, FragmentState::Valid);
        assert_eq!(v.issuance, FragmentState::Valid);
        assert_eq!(v.identity, FragmentState::Valid);
        assert_eq!(v.ownership, FragmentState::Valid);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Passed);
        assert_eq!(v.issuer_resolution, IssuerResolution::Resolved);
        assert!(v.valid);
    }

    #[test]
    fn self_import_owner_mismatch_gates_validity() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc);
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(true));
        let v = verify(
            &doc,
            &opts_for(
                &chain,
                &dns,
                &registry,
                VerifyMode::SelfImport,
                Some("0xdead".to_string()),
            ),
        );
        assert_eq!(v.ownership, FragmentState::Invalid);
        assert!(!v.valid); // credential pillars valid, but ownership gates self-import
    }

    #[test]
    fn self_import_owner_lookup_error_is_error_not_invalid() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc).owner(None);
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(true));
        let v = verify(
            &doc,
            &opts_for(
                &chain,
                &dns,
                &registry,
                VerifyMode::SelfImport,
                Some(OWNER.to_string()),
            ),
        );
        assert_eq!(v.ownership, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn third_party_without_wallet_is_not_applicable_and_does_not_gate() {
        let doc = good_doc();
        let v = third_party(&doc, &MockChain::genuine(&doc));
        assert_eq!(v.ownership, FragmentState::NotApplicable);
        assert!(v.valid); // third-party validity = credential pillars only
    }

    #[test]
    fn third_party_owner_mismatch_does_not_gate_validity() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc);
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(true));
        let v = verify(
            &doc,
            &opts_for(
                &chain,
                &dns,
                &registry,
                VerifyMode::ThirdParty,
                Some("0xother".to_string()),
            ),
        );
        assert_eq!(v.ownership, FragmentState::Invalid);
        assert!(v.valid); // ownership Invalid but still valid for third parties
    }

    #[test]
    fn issuance_false_makes_invalid() {
        let doc = good_doc();
        let root = doc.signature.merkle_root.clone();
        let chain = MockChain::genuine(&doc).with_valid_at(CLONE, &root, false);
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn issuance_adapter_error_is_error_state() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc).failing("is_valid");
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuance, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn identity_requires_both_txt_and_registry() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc);
        // TXT matches but registry does not know -> Invalid (not Error)
        let dns = MockDns(Ok(true));
        let registry = MockRegistry(Ok(false));
        let v = verify(
            &doc,
            &opts_for(&chain, &dns, &registry, VerifyMode::ThirdParty, None),
        );
        assert_eq!(v.identity, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn identity_adapter_error_is_error_state() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc);
        let dns = MockDns(Err(())); // transient DNS failure
        let registry = MockRegistry(Ok(true));
        let v = verify(
            &doc,
            &opts_for(&chain, &dns, &registry, VerifyMode::ThirdParty, None),
        );
        assert_eq!(v.identity, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn tampered_doc_invalid_integrity_gates_all_modes() {
        let mut doc = good_doc();
        // tamper a value so integrity recomputation fails
        let subj = doc.data["credentialSubject"].as_object_mut().unwrap();
        let packed = subj["name"].as_str().unwrap();
        let parts: Vec<&str> = packed.splitn(3, ':').collect();
        subj.insert(
            "name".to_string(),
            Value::String(format!("{}:{}:Max", parts[0], parts[1])),
        );
        let chain = MockChain::genuine(&doc);
        let v = third_party(&doc, &chain);
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

    // === the forged-issuer sweep, SDK surface ==================================================
    //
    // `issuer.documentStore` sits OUTSIDE the Merkle root, so it is chosen by whoever built the
    // document. Every read that decides a verdict is now made against the clone THIS VERIFIER'S OWN
    // factory names for the root, and the issuer-whitelist pillar is mandatory.
    //
    // Each test below breaks exactly ONE term of that, so a red test names the term it pins. The
    // mutation table in the PR body is generated from these.

    /// TERM 1 — `is_valid` is read against the FACTORY-RESOLVED clone, never `issuer.documentStore`.
    ///
    /// The forgery: deploy a contract that answers `isValid = true` for an arbitrary root and point
    /// `issuer.documentStore` at it. The factory has a record of this root and names a DIFFERENT
    /// clone, which answers `false`. Asking the document which contract to believe is asking the
    /// suspect for its own references.
    ///
    /// Mutation: read `is_valid` against `doc.issuer.document_store` again -> this test goes red.
    #[test]
    fn a_forged_document_store_cannot_verify() {
        let mut doc = good_doc();
        let root = doc.signature.merkle_root.clone();
        doc.issuer.document_store = HOSTILE.to_string();
        // The hostile contract says everything the attacker wants; the real clone says the root was
        // never validly issued.
        let chain = MockChain::genuine(&doc)
            .with_hostile(HOSTILE, &root, true, SIGNER)
            .with_valid_at(CLONE, &root, false);

        // The hostile contract genuinely would have answered `true` — this is the attack, not an
        // unrelated failure.
        assert!(
            chain.is_valid(HOSTILE, &root, 1).unwrap(),
            "the fake must be able to model a lying contract, or this test proves nothing"
        );

        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_addr.to_lowercase(), CLONE.to_lowercase());
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    /// TERM 2 — the issuer-whitelist pillar is MANDATORY: only a definite `Passed` may contribute.
    ///
    /// Everything else about this credential is genuine and on-chain-valid; the only defect is that
    /// the address that issued it is not authorised for the record type. Before this change the SDK
    /// had no such pillar at all, so this verified.
    ///
    /// Mutation: make `permits_pass` accept `Failed` (or drop the `permits_pass()` term from
    /// `credential_valid`) -> this test goes red.
    #[test]
    fn an_unwhitelisted_issuing_signer_refuses_an_otherwise_valid_credential() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc).without_whitelist();
        let v = third_party(&doc, &chain);
        assert_eq!(v.integrity, FragmentState::Valid);
        assert_eq!(
            v.issuance,
            FragmentState::Valid,
            "the chain says it is valid"
        );
        assert_eq!(v.identity, FragmentState::Valid);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Failed);
        assert!(!v.valid, "the pillar is mandatory, so this cannot pass");
    }

    /// TERM 3 — `noFactoryConfigured` (we never asked) and `noRecord` (we asked, the chain has no
    /// record) are DIFFERENT, and only the second is evidence about the credential.
    ///
    /// This is the distinction #94 ruled on and #96 carried. Our own misconfiguration must not
    /// condemn a credential; a chain that has never heard of the root must.
    ///
    /// Mutation: fold `NoRecord` into the non-gating branch (return
    /// `UnavailableNoFactoryConfigured` for it) -> the `no_record` half goes red.
    #[test]
    fn no_record_is_evidence_but_no_factory_configured_is_only_our_own_gap() {
        let doc = good_doc();

        // We asked; the chain has no record of this root.
        let asked = third_party(&doc, &MockChain::genuine(&doc).no_record());
        assert_eq!(asked.issuer_resolution, IssuerResolution::NoRecord);
        assert_eq!(asked.issuer_whitelist, IssuerWhitelistState::Unresolved);
        assert!(
            !asked.valid,
            "the chain has no record of this root: evidence"
        );

        // We never asked, because this verifier has no factory configured.
        let never = third_party(&doc, &MockChain::genuine(&doc).no_factory());
        assert_eq!(
            never.issuer_resolution,
            IssuerResolution::NoFactoryConfigured
        );
        assert_eq!(
            never.issuer_whitelist,
            IssuerWhitelistState::UnavailableNoFactoryConfigured
        );
        assert!(
            never.valid,
            "OUR misconfiguration is not evidence about the credential"
        );
    }

    /// TERM 4 — the whitelist question is keyed on the record type the CLONE declares, and an
    /// envelope that claims a different one is a definite failure.
    ///
    /// `issuer.recordType` is outside the Merkle root, so relabelling a genuine credential across
    /// types costs nothing — and the label is the entire point of the credential. Before this change
    /// `verify()` never read `recordType` at all, so a rabies certificate could be presented as a
    /// travel clearance and every pillar still passed.
    ///
    /// Mutation: key the whitelist read on the ENVELOPE's `claimed_rt_key` and drop the
    /// clone-vs-envelope comparison -> this test goes red.
    #[test]
    fn a_record_type_relabel_is_refused() {
        let mut doc = good_doc();
        assert_eq!(doc.issuer.record_type, "VACCINATION");
        let chain = MockChain::genuine(&doc);
        // The genuine credential passes.
        assert!(third_party(&doc, &chain).valid);

        // Relabel it. Nothing inside `R` changes, so integrity still reconciles.
        doc.issuer.record_type = "TRAVEL_CLEARANCE".to_string();
        assert_eq!(
            check_integrity(&doc).0,
            FragmentState::Valid,
            "the relabel is free: recordType is outside R"
        );

        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Failed);
        assert!(!v.valid);
    }

    /// TERM 5 — the `protocol.issuerSigner` claim is checked on EVERY path, not only the
    /// factory-resolved one, and it may only TIGHTEN.
    ///
    /// This is the regression PR #96 had to undo on its own branch: folding an operator/envelope
    /// assertion inside the factory-resolved branch silently discards a check that DID run and COULD
    /// fail a verdict, on exactly the factory-less deployments where it is the last check standing.
    ///
    /// Mutation: evaluate the provenance comparison only when `resolved_clone.is_some()` -> this
    /// test goes red while `a_forged_document_store_cannot_verify` stays green.
    #[test]
    fn a_factoryless_deployment_still_refuses_a_forged_issuer_signer_claim() {
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block("0x00000000000000000000000000000000deadbeef"));
        // No factory: the pillar reports itself unavailable and cannot condemn anything...
        let chain = MockChain::genuine(&doc).no_factory();
        let v = third_party(&doc, &chain);
        assert_eq!(
            v.issuer_whitelist,
            IssuerWhitelistState::UnavailableNoFactoryConfigured
        );
        // ...but the envelope's own claim about who issued is still checked against the chain, and
        // it does not match. A capability that worked before must not be lost to an unavailable one.
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    /// The other half of TERM 5: tighten-ONLY. On an unanchored path both the contract and the
    /// expected answer come from the document, so a MATCH proves nothing and must promote nothing.
    #[test]
    fn a_matching_issuer_signer_claim_cannot_promote_an_unanchored_credential() {
        let mut doc = good_doc();
        let root = doc.signature.merkle_root.clone();
        doc.issuer.document_store = HOSTILE.to_string();
        doc.protocol = Some(protocol_block(SIGNER));
        // The attacker supplies BOTH sides: their contract reports `issuedBy == SIGNER`, and their
        // envelope claims `SIGNER`. The two agree, and it buys nothing, because the factory HAS a
        // record of this root and it names someone else.
        let chain = MockChain::genuine(&doc).with_hostile(HOSTILE, &root, true, SIGNER);
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_addr.to_lowercase(), CLONE.to_lowercase());
        assert!(
            v.valid,
            "the real clone corroborates it, so this one is genuine"
        );

        // Now the same agreement on a chain whose factory has NO record: still not a pass.
        let unanchored = third_party(&doc, &chain.no_record());
        assert_eq!(unanchored.issuer_resolution, IssuerResolution::NoRecord);
        assert_eq!(
            unanchored.issuer_whitelist,
            IssuerWhitelistState::Unresolved
        );
        assert!(
            !unanchored.valid,
            "an agreeable claim never manufactures a pass"
        );
    }

    /// A read that could not run is never a pass — and never a `Failed` either. It is its own state.
    #[test]
    fn an_anchor_read_failure_is_indeterminate_not_a_pass_and_not_a_failure() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc).failing("root_issuer");
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_resolution, IssuerResolution::ReadFailed);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Unresolved);
        assert_ne!(
            v.issuer_whitelist,
            IssuerWhitelistState::Failed,
            "we could not ask; that is not evidence of forgery"
        );
        assert!(!v.valid);
    }

    /// The whitelist read itself failing is indeterminate too — not a silent pass.
    #[test]
    fn a_whitelist_read_failure_is_unresolved_not_passed() {
        let doc = good_doc();
        let chain = MockChain::genuine(&doc).failing("is_whitelisted_for");
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Unresolved);
        assert!(!v.valid);
    }

    /// The clone the factory named never issued this root — indeterminate, never a pass.
    #[test]
    fn a_clone_that_never_issued_the_root_leaves_the_pillar_unresolved() {
        let mut doc = good_doc();
        let root = doc.signature.merkle_root.clone();
        doc.issuer.document_store = CLONE.to_string();
        let mut chain = MockChain::genuine(&doc);
        chain
            .issued_by
            .remove(&(CLONE.to_lowercase(), root.to_lowercase()));
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_resolution, IssuerResolution::Resolved);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Unresolved);
        assert!(!v.valid);
    }

    /// A clone with no declared `recordType()` (uninitialized, or not a clone) cannot be asked the
    /// whitelist question at all: indeterminate, never a pass.
    #[test]
    fn a_clone_with_no_declared_record_type_leaves_the_pillar_unresolved() {
        let doc = good_doc();
        let mut chain = MockChain::genuine(&doc);
        chain.record_types.remove(&CLONE.to_lowercase());
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuer_whitelist, IssuerWhitelistState::Unresolved);
        assert!(!v.valid);
    }

    /// "Not evaluated" and "evaluated and passed" must never be the same value on the wire.
    /// This is the whole reason the pillar has an explicit state rather than an `Option<bool>`.
    #[test]
    fn not_evaluated_is_distinguishable_from_passed() {
        let doc = good_doc();
        let passed = third_party(&doc, &MockChain::genuine(&doc)).issuer_whitelist;
        let unavailable =
            third_party(&doc, &MockChain::genuine(&doc).no_factory()).issuer_whitelist;
        let unresolved = third_party(&doc, &MockChain::genuine(&doc).no_record()).issuer_whitelist;

        assert_eq!(passed, IssuerWhitelistState::Passed);
        assert_ne!(unavailable, passed);
        assert_ne!(unresolved, passed);
        assert_ne!(unavailable, unresolved);
        // ...and only the one that is OUR gap declines to gate.
        assert!(passed.permits_pass());
        assert!(unavailable.permits_pass());
        assert!(!unresolved.permits_pass());
        assert!(!IssuerWhitelistState::Failed.permits_pass());
    }

    /// `record_type_key` is the cross-boundary tie to the on-chain whitelist key. Pinned against a
    /// literal produced INDEPENDENTLY (`cast keccak VACCINATION`) so a hashing drift cannot silently
    /// make every whitelist question ask about nothing and read as an honest `Unresolved`.
    /// `record_type_key_matches_the_vet_backend` (stacks/vet/api) pins the other side of the tie.
    #[test]
    fn record_type_key_is_keccak256_of_the_label() {
        assert_eq!(
            record_type_key("VACCINATION"),
            "0x6510790a1a3e04db26bd73ea6246e7e8defb25eb4281f709e29decd6b8ca0561"
        );
    }

    // --- M7 provenance: the `protocol` block is a routing hint, NEVER authority (§4.2/§4.3) ----

    #[test]
    fn provenance_matching_issuer_signer_verifies() {
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block(SIGNER)); // claim == on-chain issuedBy[R]
        let v = third_party(&doc, &MockChain::genuine(&doc));
        assert_eq!(v.issuance, FragmentState::Valid);
        assert!(v.valid);
    }

    #[test]
    fn provenance_wrong_issuer_signer_does_not_verify() {
        // The load-bearing property: a forged `issuerSigner` claim must NOT make the record verify,
        // even though the record is otherwise on-chain-valid. Provenance is a routing hint, never
        // authority.
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block("0x00000000000000000000000000000000deadbeef"));
        let v = third_party(&doc, &MockChain::genuine(&doc));
        assert_eq!(v.issuance, FragmentState::Invalid);
        assert!(!v.valid);
    }

    #[test]
    fn provenance_absent_block_verifies_unchanged() {
        // §4.4 back-compat: a pre-M7 record (no `protocol` block) verifies exactly as before —
        // the additive issuer-signer check is skipped entirely.
        let doc = good_doc(); // protocol == None
        assert!(doc.protocol.is_none());
        let v = third_party(&doc, &MockChain::genuine(&doc));
        assert_eq!(v.issuance, FragmentState::Valid);
        assert!(v.valid);
    }

    /// REPLACES `provenance_unwired_adapter_skips_check_not_fails`, which asserted a behaviour that
    /// no longer exists.
    ///
    /// `issued_by` used to default to `Err(..)` = "unwired", and the orchestration read that as
    /// "skip the check, base validity governs" — a fail-open. It is now a REQUIRED trait method, so
    /// "unwired" is not a state an implementor can be in by accident, and a read that genuinely
    /// fails is ERROR: not a pass.
    #[test]
    fn provenance_read_failure_is_error_not_a_silent_pass() {
        let mut doc = good_doc();
        doc.protocol = Some(protocol_block(SIGNER));
        let chain = MockChain::genuine(&doc).failing("issued_by");
        let v = third_party(&doc, &chain);
        assert_eq!(v.issuance, FragmentState::Error);
        assert!(!v.valid);
    }

    #[test]
    fn provenance_matching_block_cannot_rescue_onchain_invalid_record() {
        // The literal acceptance property: a forged/present block must NOT make an INVALID record
        // verify. Even a block whose issuerSigner MATCHES on-chain cannot rescue a record the chain
        // reports invalid — the additive check only ever tightens.
        let mut doc = good_doc();
        let root = doc.signature.merkle_root.clone();
        doc.protocol = Some(protocol_block(SIGNER)); // claim == on-chain, yet base validity is false
        let chain = MockChain::genuine(&doc).with_valid_at(CLONE, &root, false);
        let v = third_party(&doc, &chain);
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
        subj.insert(
            "name".to_string(),
            Value::String(format!("{}:{}:Max", parts[0], parts[1])),
        );
        doc.protocol = Some(protocol_block(SIGNER));
        let v = third_party(&doc, &MockChain::genuine(&doc));
        assert_eq!(v.integrity, FragmentState::Invalid);
        assert!(!v.valid);
    }
}
