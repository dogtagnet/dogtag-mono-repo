//! Asserting the DISPLAYED issuer domain against the root-covered one (audit-m9 recommendation 6).
//!
//! # Where the mirrors are — there is deliberately NO TypeScript leg
//!
//! The behavioural mirrors of this module are `IssuerIdentity.assertDomain` in
//! `apps/ios/DogTag/IssuerDomainBinding.swift` and
//! `apps/android/app/src/main/java/io/liberalize/dogtag/net/IssuerDomainBinding.kt`; keep the three in
//! lockstep. `packages/dogtag-standard-ts` has no equivalent and no file was lost: the web portals do
//! not run this assertion client-side at all — their backend runs it (`government-api`'s `/v1/verify`
//! calls straight into this module) and ships the verdict as the `issuerDidAssertion` string, which
//! `packages/ui` only renders. The mobile apps need their own copies because they have no backend in
//! the loop by design (see the DNS-privacy note in `dogtag-dns-rs`).
//!
//! # The gap this closes
//!
//! `check_integrity` hashes only `data` (flattened) plus `privacy.obfuscated`. The top-level `issuer`
//! block — `name`, `domain`, and `documentStore` — is **outside the Merkle root**. So a genuine
//! credential can be relabelled: change `issuer.name` to "Ministry of Health of Singapore" and
//! `issuer.domain` to `moh.gov.sg`, leave `data` untouched, and integrity still passes, the on-chain
//! `isValid` still passes, and every surface displays the fabricated authority.
//!
//! The true identity IS carried inside the root, as the `data.issuer` leaf (a `did:web:` DID). Nothing
//! compared the two. This module does.
//!
//! # What it does and does not prove
//!
//! This is a CONSISTENCY assertion, not a proof of who the issuer is. It catches relabelling of the
//! displayed block. It does NOT establish that the DID's domain is controlled by the named
//! organisation — that is what the DNS binding (`dogtag-dns-rs`) is for, and the two are complementary:
//!
//!   * DNS binding: the domain owner vouches for the contract address. Detects a fabricated domain.
//!   * This assertion: the document has not been relabelled since issuance. Detects a fabricated NAME,
//!     which DNS cannot see.
//!
//! Both are needed; neither substitutes for the other.

use serde_json::Value;

use crate::wrap::{parse_packed, WrappedDoc};

/// The result of comparing the displayed `issuer.domain` with the root-covered `data.issuer` DID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssuerDomainAssertion {
    /// Both present and consistent.
    Match { domain: String },
    /// Both present and DIFFERENT. This is positive evidence the document's issuer block was
    /// relabelled after issuance — the displayed authority is fabricated.
    Mismatch {
        displayed: String,
        root_covered: String,
    },
    /// The document carries no root-covered issuer DID (or no displayed domain), so the two cannot be
    /// compared.
    ///
    /// This is NOT a pass. A caller must never fold it into one: the audit's finding was that skipped
    /// pillars contributing `?? true` is exactly how an unverified claim reaches a user looking
    /// verified. Surfaces should say the identity could not be asserted from this document.
    NotAssertable,
}

impl IssuerDomainAssertion {
    /// True only for [`IssuerDomainAssertion::Match`]. Never returns true for `NotAssertable`.
    pub fn is_match(&self) -> bool {
        matches!(self, IssuerDomainAssertion::Match { .. })
    }

    /// True only for a positive contradiction. This — not `!is_match()` — is the condition that should
    /// fail a verdict, because `NotAssertable` is evidence of nothing.
    pub fn is_mismatch(&self) -> bool {
        matches!(self, IssuerDomainAssertion::Mismatch { .. })
    }

    /// A stable wire tag for the three cases.
    pub fn as_str(&self) -> &'static str {
        match self {
            IssuerDomainAssertion::Match { .. } => "match",
            IssuerDomainAssertion::Mismatch { .. } => "mismatch",
            IssuerDomainAssertion::NotAssertable => "notAssertable",
        }
    }
}

/// Extract the host from a `did:web:` DID.
///
/// `did:web:example.com` -> `example.com`; `did:web:example.com:dept:vet` -> `example.com` (path
/// segments after the host are not part of the domain); `did:web:example.com%3A8443` -> `example.com`
/// (a `did:web` encodes a port as `%3A`, and a port is not part of the DNS name we compare).
/// Returns `None` for anything that is not a `did:web`.
pub fn did_web_host(did: &str) -> Option<String> {
    let rest = did.trim().strip_prefix("did:web:")?;
    let host = rest.split(':').next()?; // drop did:web path segments
    let host = host.split("%3A").next()?; // drop a percent-encoded port
    let host = host.split("%3a").next()?; // ...in either case
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || !host.contains('.') {
        return None;
    }
    Some(host)
}

/// The root-covered issuer DID, read from the `data.issuer` leaf.
///
/// The leaf is a packed typed scalar (`<salt>:<tag>:<value>`), so the DID is its value component.
/// Returns `None` when the document has no such leaf.
pub fn root_covered_issuer_did(doc: &WrappedDoc) -> Option<String> {
    let raw = match doc.data.get("issuer") {
        Some(Value::String(s)) => s.clone(),
        // Some producers nest the identity under credentialSubject; accept either shape rather than
        // silently reporting NotAssertable for a document that does carry the leaf.
        _ => match doc
            .data
            .get("credentialSubject")
            .and_then(|c| c.get("issuer"))
        {
            Some(Value::String(s)) => s.clone(),
            _ => return None,
        },
    };
    // A packed leaf yields its value; an unpacked plain string is used as-is.
    match parse_packed(&raw) {
        Ok((_salt, _tag, value)) => Some(value),
        Err(_) => Some(raw),
    }
}

/// Compare the DISPLAYED `issuer.domain` against the root-covered `data.issuer` DID's host.
///
/// Call this before displaying either value. Comparison is case-insensitive and ignores a trailing
/// dot, since neither is significant in a DNS name.
pub fn assert_issuer_domain(doc: &WrappedDoc) -> IssuerDomainAssertion {
    let displayed = doc
        .issuer
        .domain
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();

    let did_host = root_covered_issuer_did(doc)
        .as_deref()
        .and_then(did_web_host);

    match (displayed.is_empty(), did_host) {
        (false, Some(root)) if root == displayed => IssuerDomainAssertion::Match { domain: root },
        (false, Some(root)) => IssuerDomainAssertion::Mismatch {
            displayed,
            root_covered: root,
        },
        // Either side missing: nothing to compare. Deliberately NOT a pass.
        _ => IssuerDomainAssertion::NotAssertable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrap::{IssuerMeta, Signature, WrappedDoc};
    use serde_json::json;

    fn doc_with(displayed_domain: &str, data_issuer: Option<&str>) -> WrappedDoc {
        let mut data = json!({ "credentialSubject": { "name": "s:0:Max" } });
        if let Some(d) = data_issuer {
            data["issuer"] = Value::String(d.to_string());
        }
        WrappedDoc {
            version: "1".into(),
            data,
            signature: Signature {
                type_: "t".into(),
                target_hash: "0x00".into(),
                proof: vec![],
                merkle_root: "0x00".into(),
            },
            privacy: crate::wrap::Privacy { obfuscated: vec![] },
            issuer: IssuerMeta {
                name: "DogTag Government Authority".into(),
                domain: displayed_domain.into(),
                document_store: "0x6F8a0B1c2D3e4F5A6B7c8D9E0F1A2b3C4d5E6f70".into(),
                record_type: "TRAVEL_CLEARANCE".into(),
            },
            protocol: None,
        }
    }

    #[test]
    fn matching_domain_and_did_asserts() {
        let d = doc_with("gov.example", Some("abcd:2:did:web:gov.example"));
        assert_eq!(
            assert_issuer_domain(&d),
            IssuerDomainAssertion::Match {
                domain: "gov.example".into()
            }
        );
        assert!(assert_issuer_domain(&d).is_match());
        assert!(!assert_issuer_domain(&d).is_mismatch());
    }

    /// The exact attack the audit demonstrated live: relabel `issuer` to a real authority, leave `data`
    /// untouched. Integrity still passes; THIS is what catches it.
    #[test]
    fn the_audit_relabelling_attack_is_detected() {
        let d = doc_with("moh.gov.sg", Some("abcd:2:did:web:gov.example"));
        match assert_issuer_domain(&d) {
            IssuerDomainAssertion::Mismatch {
                displayed,
                root_covered,
            } => {
                assert_eq!(displayed, "moh.gov.sg");
                assert_eq!(root_covered, "gov.example");
            }
            other => panic!("expected Mismatch, got {other:?}"),
        }
        assert!(assert_issuer_domain(&d).is_mismatch());
        assert!(!assert_issuer_domain(&d).is_match());
    }

    #[test]
    fn comparison_ignores_case_and_a_trailing_dot() {
        let d = doc_with("GOV.Example.", Some("abcd:2:did:web:gov.example"));
        assert!(assert_issuer_domain(&d).is_match());
    }

    #[test]
    fn a_document_without_the_root_covered_leaf_is_not_assertable_not_a_pass() {
        let d = doc_with("gov.example", None);
        assert_eq!(
            assert_issuer_domain(&d),
            IssuerDomainAssertion::NotAssertable
        );
        assert!(
            !assert_issuer_domain(&d).is_match(),
            "NotAssertable must never read as a pass"
        );
        assert!(
            !assert_issuer_domain(&d).is_mismatch(),
            "NotAssertable is not evidence of forgery either"
        );
    }

    #[test]
    fn an_empty_displayed_domain_is_not_assertable() {
        let d = doc_with("", Some("abcd:2:did:web:gov.example"));
        assert_eq!(
            assert_issuer_domain(&d),
            IssuerDomainAssertion::NotAssertable
        );
    }

    #[test]
    fn a_non_did_web_identity_is_not_assertable() {
        let d = doc_with("gov.example", Some("abcd:2:did:key:z6Mk"));
        assert_eq!(
            assert_issuer_domain(&d),
            IssuerDomainAssertion::NotAssertable
        );
    }

    #[test]
    fn the_leaf_is_read_from_credential_subject_too() {
        let mut d = doc_with("gov.example", None);
        d.data["credentialSubject"]["issuer"] = Value::String("abcd:2:did:web:gov.example".into());
        assert!(assert_issuer_domain(&d).is_match());
    }

    #[test]
    fn did_web_host_drops_path_segments_and_ports() {
        assert_eq!(
            did_web_host("did:web:example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            did_web_host("did:web:example.com:dept:vet").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            did_web_host("did:web:example.com%3A8443").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            did_web_host("did:web:EXAMPLE.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(did_web_host("did:key:z6Mk"), None);
        assert_eq!(
            did_web_host("did:web:localhost"),
            None,
            "a single label is not a domain"
        );
        assert_eq!(did_web_host("not a did"), None);
    }

    #[test]
    fn an_unpacked_plain_did_string_is_still_read() {
        // Not every producer packs the leaf; a bare DID must not be reported as absent.
        let d = doc_with("gov.example", Some("did:web:gov.example"));
        assert!(assert_issuer_domain(&d).is_match());
    }

    #[test]
    fn wire_tags_are_stable_and_distinct() {
        assert_eq!(
            IssuerDomainAssertion::Match {
                domain: "x.io".into()
            }
            .as_str(),
            "match"
        );
        assert_eq!(
            IssuerDomainAssertion::Mismatch {
                displayed: "a.io".into(),
                root_covered: "b.io".into()
            }
            .as_str(),
            "mismatch"
        );
        assert_eq!(
            IssuerDomainAssertion::NotAssertable.as_str(),
            "notAssertable"
        );
    }
}
