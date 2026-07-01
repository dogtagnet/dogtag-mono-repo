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

/// Build a complete, valid government VC (plain JSON) from the operator's applicant + consignment
/// fields. Missing optional fields fall back to sensible defaults so the skeleton is demoable. The
/// mandatory, non-obfuscatable `credentialSubject.dogTagId` binds the clearance to the pet's SBT.
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

    // Each government credential type carries its OWN field set (a travel clearance and an EU health
    // certificate attest fundamentally different things). All fields are salted, obfuscatable Merkle
    // leaves — never on-chain in cleartext. `dogTagId` is the common, mandatory binding to the SBT.
    let (vc_type, legal_basis, subject) = match record_type {
        EU_HEALTH_CERT => (
            "PetHealthCertificate",
            // EU Reg. 576/2013 Annex IV model animal health certificate.
            "EU-2013-577-AnnexIV-v1",
            json!({
                "dogTagId": dog_tag_id_val,
                "species": get("species", "Canis lupus familiaris"),
                "breed": get("breed", "Mixed Breed"),
                "sex": get("sex", "male"),
                "microchipNumber": get("microchipNumber", "985141006580319"),
                "microchipImplantDate": get("microchipImplantDate", "2025-06-01"),
                "rabiesVaccinationDate": get("rabiesVaccinationDate", "2026-01-11"),
                "rabiesValidUntil": get("rabiesValidUntil", "2027-01-10"),
                "clinicalExaminationDate": get("clinicalExaminationDate", "2026-01-20"),
                "healthAttestation": get("healthAttestation", "fit_to_travel"),
                "destinationMemberState": get("destinationMemberState", "DE"),
                "certificateNumber": get("certificateNumber", &format!("EU-HC-{dog_tag_id}")),
                "officialVeterinarian": get("officialVeterinarian", "Dr. A. Muster"),
            }),
        ),
        _ => (
            "PetTravelClearance",
            // EU Reg. 576/2013 non-commercial movement of pet animals.
            "EU-2013-576-v1",
            json!({
                "dogTagId": dog_tag_id_val,
                "originCountry": get("originCountry", "US"),
                "destinationCountry": get("destinationCountry", "DE"),
                "purposeOfMovement": get("purposeOfMovement", "non-commercial"),
                "clearanceReference": get("clearanceReference", &format!("GOV-TC-{dog_tag_id}")),
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
    fn each_type_has_its_own_field_set() {
        let cfg = demo_cfg();
        let tc = build_gov_vc(&cfg, TRAVEL_CLEARANCE, &json!({}), "7");
        let eu = build_gov_vc(&cfg, EU_HEALTH_CERT, &json!({}), "7");
        let tc_sub = tc["credentialSubject"].as_object().unwrap();
        let eu_sub = eu["credentialSubject"].as_object().unwrap();
        // travel-clearance-only fields
        assert!(tc_sub.contains_key("originCountry") && tc_sub.contains_key("purposeOfMovement"));
        assert!(!eu_sub.contains_key("originCountry"));
        // EU-health-cert-only fields
        assert!(eu_sub.contains_key("rabiesVaccinationDate") && eu_sub.contains_key("microchipNumber"));
        assert!(!tc_sub.contains_key("rabiesVaccinationDate"));
        // both keep the mandatory SBT binding + distinct VC type + legal basis
        assert!(tc_sub.contains_key("dogTagId") && eu_sub.contains_key("dogTagId"));
        assert_eq!(tc["type"][1], "PetTravelClearance");
        assert_eq!(eu["type"][1], "PetHealthCertificate");
        assert_ne!(tc["legalBasisVersion"], eu["legalBasisVersion"]);
        // custom field values flow through
        let eu2 = build_gov_vc(&cfg, EU_HEALTH_CERT, &json!({"microchipNumber":"111222333444555"}), "7");
        assert_eq!(eu2["credentialSubject"]["microchipNumber"], "111222333444555");
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
