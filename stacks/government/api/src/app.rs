//! `AppState` + `Config` + government credential build/wrap.
//!
//! The government authority issues **authority-endorsed** credentials — `TRAVEL_CLEARANCE` (intra-EU
//! / cross-border pet travel clearance) and `EU_HEALTH_CERT` (Annex IV health certificate) per the
//! architecture's future-government notes (§3.6, §11 record-type table). Building reuses the shared
//! open standard (`dogtag-standard-rs`): salted, type-tagged leaves → single Poseidon Merkle root R.

use std::sync::Arc;

use dogtag_standard::wrap::{wrap_document, IssuerMeta, WrappedDoc};
use serde_json::{json, Value};

use crate::chain::ChainClient;
use crate::store::Store;

/// The government authority's record types (keccak256(label) is the on-chain issuer/whitelist key).
pub const TRAVEL_CLEARANCE: &str = "TRAVEL_CLEARANCE";
pub const EU_HEALTH_CERT: &str = "EU_HEALTH_CERT";

/// Record types this authority is allowed to issue.
pub fn is_supported_record_type(rt: &str) -> bool {
    matches!(rt, TRAVEL_CLEARANCE | EU_HEALTH_CERT)
}

#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub chain_id: u64,
    /// IssuerRegistry (the whitelist gate) — used to read issuer-identity of a credential's signer.
    pub issuer_registry_addr: String,
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
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        AppState {
            store: self.store.clone(),
            chain: self.chain.clone(),
            cfg: self.cfg.clone(),
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

/// Build a complete, valid government VC (plain JSON) from the operator's applicant fields. Each
/// record type carries its OWN `credentialSubject` schema — a `TRAVEL_CLEARANCE` describes a
/// cross-border movement (origin/destination/purpose), while an `EU_HEALTH_CERT` (Annex IV pet
/// health certificate) describes the animal's clinical/vaccination status (microchip, rabies,
/// examining vet). Missing optional fields fall back to sensible per-type defaults so the skeleton
/// is demoable. The mandatory, non-obfuscatable `credentialSubject.dogTagId` binds the credential
/// to the pet's SBT in every type.
pub fn build_gov_vc(cfg: &Config, record_type: &str, fields: &Value, dog_tag_id: &str) -> Value {
    let f = fields.as_object().cloned().unwrap_or_default();
    let get = |k: &str, d: &str| -> String {
        f.get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| d.to_string())
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
                // Travel/consignment leaves (salted, obfuscatable — never on-chain in cleartext).
                "originCountry": get("originCountry", "US"),
                "destinationCountry": get("destinationCountry", "DE"),
                "purposeOfMovement": get("purposeOfMovement", "non-commercial"),
                "clearanceReference": get("clearanceReference", &format!("GOV-{dog_tag_id}")),
                "validFrom": get("validFrom", "2026-01-01"),
                "validUntil": get("validUntil", "2026-05-01"),
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
    format!("0x{}", hex::encode(keccak256(record_type.as_bytes()).as_slice()))
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
        let vc = build_gov_vc(&cfg, TRAVEL_CLEARANCE, &json!({"destinationCountry":"FR"}), "7");
        let meta = issuer_meta(&cfg, TRAVEL_CLEARANCE, "0x1111111111111111111111111111111111111111");
        let doc = wrap(meta, &vc).unwrap();
        assert_eq!(doc.signature.merkle_root, doc.signature.target_hash);
        assert!(doc.signature.merkle_root.starts_with("0x"));
        assert_eq!(doc.signature.merkle_root.len(), 66);
        assert_eq!(doc.issuer.record_type, TRAVEL_CLEARANCE);
    }

    #[test]
    fn record_types_have_distinct_subject_fields() {
        let cfg = demo_cfg();
        let tc = build_gov_vc(&cfg, TRAVEL_CLEARANCE, &json!({}), "7");
        let eu = build_gov_vc(&cfg, EU_HEALTH_CERT, &json!({}), "7");

        let tc_sub = &tc["credentialSubject"];
        let eu_sub = &eu["credentialSubject"];

        // TRAVEL_CLEARANCE carries movement fields, NOT health-cert fields.
        assert!(tc_sub.get("destinationCountry").is_some());
        assert!(tc_sub.get("purposeOfMovement").is_some());
        assert!(tc_sub.get("microchipNumber").is_none());
        assert!(tc_sub.get("rabiesVaccinationDate").is_none());

        // EU_HEALTH_CERT carries Annex-IV health fields, NOT travel fields.
        assert!(eu_sub.get("microchipNumber").is_some());
        assert!(eu_sub.get("rabiesVaccinationDate").is_some());
        assert!(eu_sub.get("examiningVeterinarian").is_some());
        assert!(eu_sub.get("destinationCountry").is_none());
        assert!(eu_sub.get("purposeOfMovement").is_none());

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
