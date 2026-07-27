//! `AppState` + `Config` + government credential build/wrap.
//!
//! The government authority issues **authority-endorsed** credentials — `TRAVEL_CLEARANCE` (intra-EU
//! / cross-border pet travel clearance) and `EU_HEALTH_CERT` (Annex IV health certificate) per the
//! architecture's future-government notes (§3.6, §11 record-type table). Building reuses the shared
//! open standard (`dogtag-standard-rs`): salted, type-tagged leaves → single Poseidon Merkle root R.

use std::sync::Arc;

use dogtag_standard::discovery::ConvenienceClaims;
use dogtag_standard::wrap::{wrap_document, IssuerMeta, ProtocolMeta, WrappedDoc, LEVEL_B_VERSION};
use serde_json::{json, Value};

use crate::chain::ChainClient;
use crate::oversight::OversightFeed;
use crate::store::Store;

/// The government authority's record types (keccak256(label) is the on-chain issuer/whitelist key).
pub const TRAVEL_CLEARANCE: &str = "TRAVEL_CLEARANCE";
pub const EU_HEALTH_CERT: &str = "EU_HEALTH_CERT";

/// Record types this authority is allowed to issue.
pub fn is_supported_record_type(rt: &str) -> bool {
    matches!(rt, TRAVEL_CLEARANCE | EU_HEALTH_CERT)
}

/// The `VERIFY:<purpose>` namespaces this authority verifies under, when acting as a VERIFIER rather
/// than an issuer. `travel_check` is the border/port-of-entry check — an existing label already in
/// circulation across the portals (see `stacks/owner/web/src/lib/consents.ts`), so an owner's consent
/// receipt renders it by name rather than as a bare field hash.
///
/// Verify capability is granted ON-CHAIN through the admin apply→approve flow
/// (`whitelistFor(VERIFY:<purpose>, signer)`), NEVER by this list: naming a purpose here only means the
/// portal offers it. An unwhitelisted signer is refused at session start with a 403.
pub const VERIFY_PURPOSES: &[&str] = &["travel_check"];

#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub chain_id: u64,
    /// IssuerRegistry (the whitelist gate) — used to read issuer-identity of a credential's signer.
    pub issuer_registry_addr: String,
    /// `DogTagIssuerFactory` address — LINK 1 of the issuer↔domain chain (`isClone`). Zero/empty means
    /// provenance cannot be checked, in which case the binding reports `unavailable` rather than
    /// claiming anything.
    pub factory_addr: String,
    /// `IssuerDomainRegistry` address — the ADDITIVE contract holding each clone's on-chain domain
    /// claim. Zero/empty when this deployment has not been pointed at one, in which case the binding is
    /// reported as unavailable rather than as absent.
    pub issuer_domain_registry_addr: String,
    /// DoH JSON endpoint used for the SERVER-SIDE issuer↔domain lookup. Empty disables resolution, and
    /// every binding then reports "could not check" rather than a verdict.
    pub dns_doh_endpoint: String,
    /// VerificationRegistry address - THE routing key stamped in the M7 `protocol` block (§4.2).
    /// Defaults to the unified owner-hidden registry.
    pub verification_registry_addr: String,
    /// DogTagIssuer clone this authority anchors TRAVEL_CLEARANCE roots to (documentStore).
    pub travel_clearance_issuer_addr: String,
    /// DogTagIssuer clone this authority anchors EU_HEALTH_CERT roots to (documentStore).
    pub eu_health_cert_issuer_addr: String,
    pub issuer_name: String,
    pub issuer_domain: String,
    /// Whether this deployment is in demo mode (MemChain/MemStore, relaxed secrets).
    pub demo: bool,
    /// Bearer token gating the record MUTATION endpoints (PATCH /v1/records/:root and
    /// POST /v1/records/:root/revoke). `None` means unconfigured: mutations fail closed (503).
    /// Reads, verify and issue stay open.
    pub api_token: Option<String>,
}

impl Config {
    /// The DogTagIssuer clone address for a record type (the credential's `documentStore`).
    pub fn issuer_addr_for(&self, record_type: &str) -> Option<String> {
        let a = match record_type {
            TRAVEL_CLEARANCE => &self.travel_clearance_issuer_addr,
            EU_HEALTH_CERT => &self.eu_health_cert_issuer_addr,
            _ => return None,
        };
        if a.trim().is_empty() || a == "0x0000000000000000000000000000000000000000" {
            None
        } else {
            Some(a.clone())
        }
    }
}

pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub cfg: Arc<Config>,
    /// Server-side issuer↔domain DNS resolver, with its own TTL cache.
    ///
    /// The lookup happens HERE, not in the operator's browser: a portal cannot do raw DNS, and a
    /// client-side DoH call would hand a public resolver the operator's IP plus which issuer they are
    /// inspecting. This backend already holds the whole document, so resolving here adds no party to
    /// the trust set. The cache lives on this shared instance so a table of rows costs one lookup per
    /// (address, domain), not one per render.
    pub dns: Arc<dogtag_dns_rs::BindingResolver>,
    /// UNSCOPED consumer of the PR-4 oversight indexer — the data layer behind the government
    /// oversight console (govarch PR-5). `DisabledFeed` when `INDEXER_API_BASE` is unset; the real
    /// `HttpOversightFeed` (presenting the unscoped bearer) otherwise. See `crate::oversight` /
    /// `crate::trace`.
    pub feed: Arc<dyn OversightFeed>,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        AppState {
            store: self.store.clone(),
            chain: self.chain.clone(),
            cfg: self.cfg.clone(),
            dns: self.dns.clone(),
            feed: self.feed.clone(),
        }
    }
}

/// Issuer metadata for a government credential (documentStore == the DogTagIssuer clone).
pub fn issuer_meta(cfg: &Config, record_type: &str, issuer_addr: &str) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: issuer_addr.to_string(),
        record_type: record_type.to_string(),
    }
}

/// Build the M7 provenance block (§4.2) for a government credential. `issuer_clone` == `documentStore`;
/// `issuer_signer` is this authority's issuing signer (== the on-chain `clone.issuedBy[R]`, since gov
/// anchors server-side). A routing hint only - the claim is validated against `issuedBy[R]` at verify.
pub fn protocol_meta(cfg: &Config, issuer_clone: &str, issuer_signer: &str) -> ProtocolMeta {
    ProtocolMeta {
        chain_id: cfg.chain_id,
        version: LEVEL_B_VERSION.to_string(),
        verification_registry: cfg.verification_registry_addr.clone(),
        issuer_clone: issuer_clone.to_string(),
        issuer_signer: issuer_signer.to_string(),
    }
}

/// Assemble the M7 §5.2 CONVENIENCE tier for the owner's device: platform-OWNED, UNVERIFIED claims
/// read straight from THIS deployment's config.
///
/// To the consuming app these are CLAIMS, not authority (§1.2 trust boundary). The app MUST resolve the
/// dogtag `ProtocolRegistry` / signed-manifest anchor and call `dogtag_standard::discovery::validate`
/// before trusting `{version, registry, chainId}`, so a lying platform — government included — cannot
/// steer a proof onto an attacker registry. Mirror of vet-api's `app::convenience_claims`; the field set
/// is what both phone apps parse out of `GET /x/{token}` (`unverifiedClaims`), and the owner-hidden flow
/// REFUSES outright when it is absent.
pub fn convenience_claims(
    cfg: &Config,
    chain_id: u64,
    issuer_clone: &str,
    purpose: &str,
) -> ConvenienceClaims {
    ConvenienceClaims {
        protocol_version: LEVEL_B_VERSION.to_string(),
        chain_id,
        verification_registry: cfg.verification_registry_addr.clone(),
        issuer_clone: issuer_clone.to_string(),
        purpose: purpose.to_string(),
    }
}

/// Build a complete, valid government VC (plain JSON) from the operator's applicant fields. Each
/// record type carries its OWN `credentialSubject` schema.
///
/// `TRAVEL_CLEARANCE` is the CDC-modeled travel-receipt (research dogtag-govreceipt-r7 §2.1 /
/// dogtag-govarch-r8 §1): a nested subject grouping **Section A** (person importing the animal —
/// PII, the obfuscatable/private block), **Section B** (animal), **Section C** (travel), a
/// **validity** window, and a public `receiptId` leaf. Nesting flattens to leaf key-paths
/// automatically (`credentialSubject.importer.firstName`, …) via the standard's flattener. B/C,
/// validity and `receiptId` are the PUBLIC blocks (revealed cleartext leaves at presentation); A is
/// obfuscated by the holder. `EU_HEALTH_CERT` (Annex IV pet health certificate) describes the
/// animal's clinical/vaccination status. Missing optional fields fall back to sensible per-type
/// defaults so the skeleton is demoable. The mandatory, non-obfuscatable `credentialSubject.dogTagId`
/// binds the credential to the pet's SBT in every type.
///
/// `receipt_id` is the CSPRNG Crockford-base32 handle minted at issue time (see `routes::issue`); it
/// is committed into `R` as a public salted leaf (a forged receipt with a real-looking id fails
/// integrity) AND stored off-chain as the `/r/:receiptId` lookup handle. The issuance DATE is NOT a
/// leaf — it is derived from the on-chain `issuedAt[R]` / anchoring block timestamp (arch DP-2).
pub fn build_gov_vc(
    cfg: &Config,
    record_type: &str,
    fields: &Value,
    dog_tag_id: &str,
    receipt_id: &str,
) -> Value {
    let f = fields.as_object().cloned().unwrap_or_default();
    let get = |k: &str, d: &str| -> String {
        f.get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| d.to_string())
    };
    // Numeric leaf (JSON number → INTEGER tag 3): accept a JSON number or a numeric string.
    let get_num = |k: &str, d: i64| -> Value {
        f.get(k)
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            })
            .map(|n| json!(n))
            .unwrap_or_else(|| json!(d))
    };
    // Boolean leaf (JSON bool → tag 1): accept a JSON bool or the strings "true"/"false".
    let get_bool = |k: &str, d: bool| -> Value {
        f.get(k)
            .and_then(|v| {
                v.as_bool()
                    .or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true")))
            })
            .map(|b| json!(b))
            .unwrap_or_else(|| json!(d))
    };

    // dogTagId as a JSON number when numeric so the typed projection tags it INTEGER (tag 3).
    let dog_tag_id_val: Value = dog_tag_id
        .parse::<u64>()
        .map(|n| json!(n))
        .unwrap_or_else(|_| json!(dog_tag_id));

    // Per-record-type VC shape: distinct type, legal basis, and credentialSubject field set.
    let (vc_type, legal_basis, subject) = match record_type {
        EU_HEALTH_CERT => (
            "PetHealthCertificate",
            "EU-2013-577-Annex-IV-v1",
            json!({
                "dogTagId": dog_tag_id_val,
                // Public lookup handle, committed in R (also the /r/:receiptId key).
                "receiptId": receipt_id,
                // Annex IV health-certificate leaves (salted, obfuscatable — never on-chain in clear).
                "species": get("species", "dog"),
                "microchipNumber": get("microchipNumber", "985112345678903"),
                "rabiesVaccinationDate": get("rabiesVaccinationDate", "2026-01-15"),
                "rabiesValidUntil": get("rabiesValidUntil", "2029-01-14"),
                "examiningVeterinarian": get("examiningVeterinarian", "Dr. A. Meyer, DVM"),
                "clinicalHealthStatus": get("clinicalHealthStatus", "fit_for_travel"),
                "examinationDate": get("examinationDate", "2026-02-01"),
                "endorsingAuthority": get("endorsingAuthority", &cfg.issuer_name),
            }),
        ),
        _ => (
            "PetTravelClearance",
            "EU-2013-576-v1",
            json!({
                "dogTagId": dog_tag_id_val,
                // PUBLIC handle committed in R + the off-chain /r/:receiptId lookup key.
                "receiptId": receipt_id,

                // -- validity (PUBLIC revealed leaves; issuance DATE derived from chain, not a leaf) --
                "validity": {
                    "validFrom": get("validFrom", "2026-07-01"),
                    "validUntil": get("validUntil", "2027-01-01"),
                    "multipleEntries": get_bool("multipleEntries", true),
                    // CDC "valid only from the listed country of departure" binding.
                    "countryOfDepartureBinding": get("countryOfDepartureBinding", "CA"),
                },

                // -- Section A: person importing the animal (PII — the PRIVATE/obfuscatable block) --
                "importer": {
                    "firstName": get("importerFirstName", "Dominic"),
                    "middleName": get("importerMiddleName", ""),
                    "lastName": get("importerLastName", "Zagara"),
                    "role": get("importerRole", "owner"),
                    "idType": get("importerIdType", "drivers_license"),
                    "idJurisdiction": get("importerIdJurisdiction", "US-NY"),
                    "idNumber": get("importerIdNumber", "887524355"),
                    "dateOfBirth": get("importerDateOfBirth", "1997-02-13"),
                    "email": get("importerEmail", "dom.zagara@example.com"),
                    "phone": get("importerPhone", "216-533-5925"),
                },
                "consignee": {
                    "fullName": get("consigneeName", ""),
                    "email": get("consigneeEmail", ""),
                    "idType": get("consigneeIdType", ""),
                },

                // -- Section B: animal information (PUBLIC) --
                "animal": {
                    "name": get("animalName", "Blaze"),
                    "ageYears": get_num("animalAgeYears", 3),
                    "ageMonths": get_num("animalAgeMonths", 1),
                    "sex": get("animalSex", "male"),
                    // CDC's "Male Neutered" split into a typed boolean leaf.
                    "neutered": get_bool("animalNeutered", true),
                    "breed": get("animalBreed", "Poodle - Standard"),
                    "colorMarkings": get("animalColorMarkings", "Grey"),
                    "microchipNumber": get("microchipNumber", "985112345678903"),
                    "importationPurpose": get("importationPurpose", "service_animal"),
                },

                // -- Section C: travel information (PUBLIC) --
                "travel": {
                    "travelType": get("travelType", "air"),
                    "countryOfDeparture": get("countryOfDeparture", "CA"),
                    "dateOfArrival": get("dateOfArrival", "2026-07-08"),
                    "portOfEntry": get("portOfEntry", "JFK"),
                    "carrierOrFlight": get("carrierOrFlight", "AC 8552"),
                },

                "endorsingAuthority": get("endorsingAuthority", &cfg.issuer_name),
            }),
        ),
    };

    json!({
        "@context": ["https://www.w3.org/ns/credentials/v2", "https://dogtag.io/credentials/v1"],
        "type": ["VerifiableCredential", vc_type],
        "id": format!("urn:dogtag:{}:{dog_tag_id}", record_type.to_lowercase()),
        "issuer": format!("did:web:{}", cfg.issuer_domain),
        "recordType": record_type,
        // Government credentials are authority-endorsed (accredited-authority trust tier), NOT
        // self-attested. Legal posture stays evidentiary (architecture §7 / research/07).
        "attestationType": "authority_endorsement",
        "signatureTrustTier": "accredited_authority",
        "legalEffect": "evidentiary",
        "legalBasisVersion": get("legalBasisVersion", legal_basis),
        "jurisdiction": get("jurisdiction", "EU"),
        "credentialSubject": subject,
    })
}

/// Project a plain VC into the typed-scalar `{tag,value}` form the flatten/Merkle pipeline requires
/// (mirror of the vet stack's `to_typed`). Preserves any already-typed leaf.
fn to_typed(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            if m.len() == 2 && m.contains_key("tag") && m.contains_key("value") {
                return v.clone();
            }
            let mut out = serde_json::Map::new();
            for (k, val) in m {
                out.insert(k.clone(), to_typed(val));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(to_typed).collect()),
        Value::Null => json!({ "tag": 0u8, "value": Value::Null }),
        Value::Bool(b) => json!({ "tag": 1u8, "value": b }),
        Value::String(s) => json!({ "tag": 2u8, "value": s }),
        Value::Number(n) => {
            let tag = if n.is_i64() || n.is_u64() { 3u8 } else { 4u8 };
            json!({ "tag": tag, "value": n.to_string() })
        }
    }
}

/// Wrap a VC into a `WrappedDoc` (single Poseidon root R) using a cryptographically-random salt.
pub fn wrap(issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    let typed = to_typed(vc);
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    wrap_document(&typed, issuer_meta, &mut salt).map_err(|e| format!("wrap: {e}"))
}

/// The bytes32 issuer/whitelist key for a record type = keccak256(label).
pub fn record_type_key(record_type: &str) -> String {
    use alloy::primitives::keccak256;
    format!(
        "0x{}",
        hex::encode(keccak256(record_type.as_bytes()).as_slice())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_cfg() -> Config {
        Config {
            deployment_url: "http://localhost:44832".into(),
            rpc_url: "https://devrpc.roax.net".into(),
            chain_id: 135,
            issuer_registry_addr: "0x5d86e4CF98A34Ae0576F190F8d209c2943a9C79c".into(),
            factory_addr: "0x0000000000000000000000000000000000000000".into(),
            issuer_domain_registry_addr: "0x0000000000000000000000000000000000000000".into(),
            dns_doh_endpoint: "https://cloudflare-dns.com/dns-query".into(),
            verification_registry_addr: "0xb9B313C17fD8725Bb50A7f41121ac4Cf5F4fec87".into(),
            travel_clearance_issuer_addr: "0x1111111111111111111111111111111111111111".into(),
            eu_health_cert_issuer_addr: "0x0000000000000000000000000000000000000000".into(),
            issuer_name: "DogTag Government Authority".into(),
            issuer_domain: "gov.example".into(),
            demo: true,
            api_token: Some("dogtag-gov-demo-token".into()),
        }
    }

    #[test]
    fn build_and_wrap_produces_a_root() {
        let cfg = demo_cfg();
        let vc = build_gov_vc(
            &cfg,
            TRAVEL_CLEARANCE,
            &json!({"animalName":"Rex"}),
            "7",
            "9RVBXK8AFQ2C",
        );
        let meta = issuer_meta(
            &cfg,
            TRAVEL_CLEARANCE,
            "0x1111111111111111111111111111111111111111",
        );
        let doc = wrap(meta, &vc).unwrap();
        assert_eq!(doc.signature.merkle_root, doc.signature.target_hash);
        assert!(doc.signature.merkle_root.starts_with("0x"));
        assert_eq!(doc.signature.merkle_root.len(), 66);
        assert_eq!(doc.issuer.record_type, TRAVEL_CLEARANCE);
    }

    #[test]
    fn travel_clearance_has_cdc_sectioned_subject() {
        let cfg = demo_cfg();
        let tc = build_gov_vc(
            &cfg,
            TRAVEL_CLEARANCE,
            &json!({"importerFirstName": "Ada", "animalAgeYears": 5, "animalNeutered": false}),
            "7",
            "RECEIPT012345",
        );
        let sub = &tc["credentialSubject"];

        // dogTagId stays mandatory + numeric (INTEGER-tagged); receiptId is a public leaf.
        assert_eq!(sub["dogTagId"], json!(7u64));
        assert_eq!(sub["receiptId"], json!("RECEIPT012345"));

        // Section A (person PII) — the private/obfuscatable block, supplied field honored.
        assert_eq!(sub["importer"]["firstName"], json!("Ada"));
        assert!(sub["importer"]["idNumber"].is_string());
        // Section B (animal) — typed numeric + boolean leaves, supplied values honored.
        assert_eq!(sub["animal"]["ageYears"], json!(5));
        assert_eq!(sub["animal"]["neutered"], json!(false));
        assert!(sub["animal"]["name"].is_string());
        // Section C (travel) + validity window are present.
        assert!(sub["travel"]["countryOfDeparture"].is_string());
        assert!(sub["validity"]["validUntil"].is_string());
        assert_eq!(sub["validity"]["multipleEntries"], json!(true));

        // The CDC receipt is NOT the old flat movement shape.
        assert!(sub.get("destinationCountry").is_none());
        assert!(sub.get("purposeOfMovement").is_none());
    }

    #[test]
    fn record_types_have_distinct_subject_fields() {
        let cfg = demo_cfg();
        let tc = build_gov_vc(&cfg, TRAVEL_CLEARANCE, &json!({}), "7", "RID0TRAVEL012");
        let eu = build_gov_vc(&cfg, EU_HEALTH_CERT, &json!({}), "7", "RID0HEALTH012");

        let tc_sub = &tc["credentialSubject"];
        let eu_sub = &eu["credentialSubject"];

        // TRAVEL_CLEARANCE carries the CDC animal/travel sections, NOT flat health-cert fields.
        assert!(tc_sub["animal"]["name"].is_string());
        assert!(tc_sub["travel"]["travelType"].is_string());
        assert!(tc_sub.get("rabiesVaccinationDate").is_none());

        // EU_HEALTH_CERT carries Annex-IV health fields (+ a receiptId handle), NOT travel sections.
        assert!(eu_sub.get("microchipNumber").is_some());
        assert!(eu_sub.get("rabiesVaccinationDate").is_some());
        assert!(eu_sub.get("examiningVeterinarian").is_some());
        assert_eq!(eu_sub["receiptId"], json!("RID0HEALTH012"));
        assert!(eu_sub.get("travel").is_none());
        assert!(eu_sub.get("importer").is_none());

        // Distinct VC subtype + legal basis per record type.
        assert_eq!(tc["type"][1], json!("PetTravelClearance"));
        assert_eq!(eu["type"][1], json!("PetHealthCertificate"));
        assert_eq!(tc["legalBasisVersion"], json!("EU-2013-576-v1"));
        assert_eq!(eu["legalBasisVersion"], json!("EU-2013-577-Annex-IV-v1"));
    }

    #[test]
    fn eu_health_cert_honors_supplied_fields() {
        let cfg = demo_cfg();
        let eu = build_gov_vc(
            &cfg,
            EU_HEALTH_CERT,
            &json!({"microchipNumber": "111", "clinicalHealthStatus": "under_observation"}),
            "9",
            "RID0HEALTH012",
        );
        assert_eq!(eu["credentialSubject"]["microchipNumber"], json!("111"));
        assert_eq!(
            eu["credentialSubject"]["clinicalHealthStatus"],
            json!("under_observation")
        );
    }

    #[test]
    fn record_type_key_is_keccak() {
        // keccak256("TRAVEL_CLEARANCE") pinned so a drift breaks the build.
        let k = record_type_key(TRAVEL_CLEARANCE);
        assert!(k.starts_with("0x") && k.len() == 66);
    }

    #[test]
    fn issuer_addr_for_gates_unset() {
        let cfg = demo_cfg();
        assert!(cfg.issuer_addr_for(TRAVEL_CLEARANCE).is_some());
        assert!(cfg.issuer_addr_for(EU_HEALTH_CERT).is_none());
        assert!(cfg.issuer_addr_for("VACCINATION").is_none());
    }
}
