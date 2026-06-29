//! Application state + config + the server-side DOG_PROFILE VC build/wrap (impl §4.1).

use std::sync::Arc;

use dogtag_standard::schema::{validate_schema, DOGTAG_CONTEXT_URI};
use dogtag_standard::wrap::{wrap_document, IssuerMeta, WrappedDoc};
use serde_json::{json, Value};

use crate::auth::JwtKeys;
use crate::business::BusinessClient;
use crate::chain::ChainClient;
use crate::crypto::KeyVault;
use crate::dns::DnsChecker;
use crate::store::Store;

/// The DOG_PROFILE record type the central stack mints SBT profiles under.
pub const DOG_PROFILE: &str = "DOG_PROFILE";

/// Resolved central config (contract addresses + the admin's signer roles).
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    pub sbt_addr: String,
    /// the central protocol issuer identity (for IssuerMeta on the profile VC).
    pub issuer_name: String,
    pub issuer_domain: String,
    /// the documentStore the profile VC anchors to (the SBT contract acts as the profile store).
    pub profile_document_store: String,
    /// admin-session password (admin routes: businesses register, applications, deletions).
    pub admin_password: String,
    /// account index of the admin signer (WHITELIST_ADMIN + ISSUER/PROFILE_ISSUER roles).
    pub admin_signer_index: u32,
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub dns: Arc<dyn DnsChecker>,
    pub business: Arc<dyn BusinessClient>,
    pub vault: Arc<dyn KeyVault>,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
    /// in-memory login rate limiter (lenient; demo-safe).
    pub ratelimit: Arc<crate::auth::RateLimiter>,
}

/// Build the issuer metadata for the central profile issuer.
pub fn profile_issuer_meta(cfg: &Config) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: cfg.profile_document_store.clone(),
        record_type: DOG_PROFILE.to_string(),
    }
}

/// Build a COMPLETE, valid DOG_PROFILE VC (plain JSON — string/array fields, the shape
/// `validate_schema` operates on) from a pet record (impl §1.6 / CHANGESPEC §0/§1.8).
///
/// The mint flow has no operator form, so the VC-2.0 envelope + legal/trust meta are filled from
/// the central deployment identity (a `self_attested`, `evidentiary` owner-asserted profile), and
/// any DOG_PROFILE subject fields the pet record lacks fall back to sensible defaults (species
/// "Canis lupus familiaris", etc.). The returned VC passes `validate_schema` for recordType
/// DOG_PROFILE; `wrap_vc` then converts it to typed-scalar leaves and computes the Merkle root.
pub fn build_profile_vc(
    cfg: &Config,
    name: &str,
    microchip: &crate::store::Microchip,
    profile: &crate::store::PetProfile,
    owner_identity: &crate::store::OwnerIdentity,
    dog_tag_id: &str,
) -> Value {
    let species = profile
        .species
        .clone()
        .unwrap_or_else(|| "Canis lupus familiaris".to_string());
    let breed_vbo = profile
        .breed_vbo
        .clone()
        .unwrap_or_else(|| "VBO:0200000".to_string());
    let breed_label = profile
        .breed_label
        .clone()
        .unwrap_or_else(|| "Mixed Breed".to_string());
    let sex = profile.sex.clone().unwrap_or_else(|| "male".to_string());
    let neuter_status = profile
        .neuter_status
        .clone()
        .unwrap_or_else(|| "intact".to_string());
    let date_of_birth = profile
        .date_of_birth
        .clone()
        .unwrap_or_else(|| "2020-01-01".to_string());

    let weight_history: Vec<Value> = profile
        .weight_history
        .iter()
        .map(|w| json!({ "unit": w.unit, "value": w.value, "measuredOn": w.measured_on }))
        .collect();

    let now = crate::auth::now();
    let valid_from = iso_date_utc(now);

    // dogTagId is a non-personal integer id; emit it as a JSON number when numeric so the typed
    // projection wraps it as an INTEGER (tag 3), matching the vet stack's `build_vc`. The value is
    // still present-and-non-null, satisfying the schema's `credentialSubject.dogTagId` requirement.
    let dog_tag_id_val: Value = dog_tag_id
        .parse::<u64>()
        .map(|n| json!(n))
        .unwrap_or_else(|_| json!(dog_tag_id));

    json!({
        "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
        "type": ["VerifiableCredential", "DogProfile"],
        "id": format!("urn:dogtag:profile:{dog_tag_id}"),
        "issuer": format!("did:web:{}", cfg.issuer_domain),
        "validFrom": valid_from,
        "credentialSchema": { "id": format!("https://{}/schemas/dog-profile", cfg.issuer_domain), "type": "JsonSchema" },
        "credentialStatus": { "id": format!("https://{}/status/{dog_tag_id}", cfg.issuer_domain), "type": "DogTagStatus2025" },
        "recordType": DOG_PROFILE,
        // legal/trust meta (owner-asserted profile minted by the central protocol signer).
        "attestationType": "identity",
        "signatureTrustTier": "self_attested",
        "legalEffect": "evidentiary",
        "legalBasisVersion": "DOGTAG-PROFILE-v1",
        "jurisdiction": "GLOBAL",
        "credentialSubject": {
            "dogTagId": dog_tag_id_val,
            "name": name,
            "species": species,
            "breedVbo": breed_vbo,
            "breedLabel": breed_label,
            "sex": sex,
            "neuterStatus": neuter_status,
            "dateOfBirth": date_of_birth,
            "weightHistory": weight_history,
            "microchip": {
                "code": microchip.code,
                "standard": microchip.standard,
                "implantDate": microchip.implant_date,
                "bodyLocation": microchip.body_location,
            },
            // owner's official identity, entered by the admin at mint time (wrap_vc turns the 3
            // scalars into typed Merkle leaves automatically).
            "ownerIdentity": {
                "countryOfIdentification": owner_identity.country_of_identification,
                "identification": owner_identity.identification,
                "name": owner_identity.name,
            },
        },
    })
}

/// Format a unix-seconds timestamp as an ISO `YYYY-MM-DD` (UTC). Uses Howard Hinnant's civil
/// algorithm so we don't pull a date crate just for `validFrom`.
fn iso_date_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64; // days since 1970-01-01
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Convert a plain JSON VC (string/number/array/object leaves) into the SDK's typed-scalar
/// wrap input (`{tag, value}` leaves). This is the shape `wrap_document`'s flatten expects:
/// `validate_schema` runs on the plain form, the wrap pipeline on the typed form.
///
/// Tag mapping (impl §11.2): 0=null, 1=bool, 2=string, 3=integer, 4=decimal. Numbers serialize
/// as integer/decimal strings; booleans/strings/null map directly. Arrays/objects recurse.
fn to_typed_vc(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = serde_json::Map::new();
            for (k, val) in m {
                out.insert(k.clone(), to_typed_vc(val));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(to_typed_vc).collect()),
        Value::Null => json!({ "tag": 0u8, "value": Value::Null }),
        Value::Bool(b) => json!({ "tag": 1u8, "value": b }),
        Value::String(s) => json!({ "tag": 2u8, "value": s }),
        Value::Number(n) => {
            // integers (no fractional/exponent part) -> tag 3, else decimal -> tag 4.
            let s = n.to_string();
            let tag = if n.is_i64() || n.is_u64() { 3u8 } else { 4u8 };
            json!({ "tag": tag, "value": s })
        }
    }
}

/// Validate a plain DOG_PROFILE VC against the SDK schema, then wrap it into a `WrappedDoc`.
///
/// The validator (`dogtag_standard::schema::validate_schema`) operates on the plain VC; we wrap the
/// typed-scalar projection of the SAME credential so the on-chain commitment covers every field.
pub fn wrap_vc(issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    // (1) schema-validate the PLAIN VC (string/array fields) — surfaces ALL violations.
    validate_schema(vc).map_err(|violations| format!("schema: {}", violations.join("; ")))?;
    // (2) wrap the typed-scalar projection (the shape the flatten/Merkle pipeline expects).
    let typed = to_typed_vc(vc);
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    wrap_document(&typed, issuer_meta, &mut salt).map_err(|e| format!("wrap: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Microchip, OwnerIdentity, PetProfile, WeightEntry};

    fn test_cfg() -> Config {
        Config {
            deployment_url: "http://localhost:8080".to_string(),
            rpc_url: "http://localhost:8545".to_string(),
            issuer_registry_addr: "0x1111".to_string(),
            sbt_addr: "0x2222".to_string(),
            issuer_name: "DogTag Central".to_string(),
            issuer_domain: "central.dogtag.test".to_string(),
            profile_document_store: "0x3333".to_string(),
            admin_password: "pw".to_string(),
            admin_signer_index: 0,
        }
    }

    fn test_microchip() -> Microchip {
        Microchip {
            code: "900012345678901".to_string(),
            standard: "ISO_11784_11785".to_string(),
            implant_date: "2021-05-01".to_string(),
            body_location: "left neck".to_string(),
        }
    }

    fn test_owner() -> OwnerIdentity {
        OwnerIdentity {
            country_of_identification: "GB".to_string(),
            identification: "P12345".to_string(),
            name: "Jane Doe".to_string(),
        }
    }

    // ---- profile_issuer_meta ----

    #[test]
    fn profile_issuer_meta_copies_config_and_pins_record_type() {
        let cfg = test_cfg();
        let m = profile_issuer_meta(&cfg);
        assert_eq!(m.name, "DogTag Central");
        assert_eq!(m.domain, "central.dogtag.test");
        assert_eq!(m.document_store, "0x3333");
        // record_type is always the DOG_PROFILE constant, never config-derived.
        assert_eq!(m.record_type, DOG_PROFILE);
        assert_eq!(m.record_type, "DOG_PROFILE");
    }

    // ---- iso_date_utc (Howard Hinnant civil algorithm) ----

    #[test]
    fn iso_date_utc_anchors() {
        assert_eq!(iso_date_utc(0), "1970-01-01");
        assert_eq!(iso_date_utc(1_700_000_000), "2023-11-14");
        // 2020 is a leap year: 1_582_934_400 is exactly 2020-02-29 00:00:00 UTC.
        assert_eq!(iso_date_utc(1_582_934_400), "2020-02-29");
    }

    #[test]
    fn iso_date_utc_truncates_intra_day_seconds() {
        // 1_582_934_400 is exactly 2020-02-29 00:00:00 UTC; any second within that day maps to
        // the same date, and the first second of the next day rolls over.
        assert_eq!(iso_date_utc(1_582_934_400 + 86_399), "2020-02-29");
        assert_eq!(iso_date_utc(1_582_934_400 + 86_400), "2020-03-01");
    }

    // ---- to_typed_vc (plain JSON -> {tag,value} typed leaves) ----

    #[test]
    fn to_typed_vc_maps_each_scalar_to_its_tag() {
        assert_eq!(
            to_typed_vc(&Value::Null),
            json!({ "tag": 0u8, "value": Value::Null })
        );
        assert_eq!(
            to_typed_vc(&json!(true)),
            json!({ "tag": 1u8, "value": true })
        );
        assert_eq!(
            to_typed_vc(&json!("hi")),
            json!({ "tag": 2u8, "value": "hi" })
        );
        // integer numbers -> tag 3 with stringified value.
        assert_eq!(
            to_typed_vc(&json!(42)),
            json!({ "tag": 3u8, "value": "42" })
        );
        // fractional numbers -> tag 4 (decimal).
        assert_eq!(
            to_typed_vc(&json!(1.5)),
            json!({ "tag": 4u8, "value": "1.5" })
        );
    }

    #[test]
    fn to_typed_vc_recurses_into_objects_and_arrays() {
        let v = json!({ "a": "x", "b": [1, "y"] });
        let typed = to_typed_vc(&v);
        assert_eq!(typed["a"], json!({ "tag": 2u8, "value": "x" }));
        assert_eq!(typed["b"][0], json!({ "tag": 3u8, "value": "1" }));
        assert_eq!(typed["b"][1], json!({ "tag": 2u8, "value": "y" }));
    }

    // ---- build_profile_vc ----

    #[test]
    fn build_profile_vc_numeric_id_emits_json_number_and_string_id_stays_string() {
        let cfg = test_cfg();
        let mc = test_microchip();
        let owner = test_owner();
        let profile = PetProfile::default();

        let vc = build_profile_vc(&cfg, "Rex", &mc, &profile, &owner, "12345");
        // numeric dogTagId is emitted as a JSON number so to_typed_vc wraps it as INTEGER (tag 3).
        assert_eq!(vc["credentialSubject"]["dogTagId"], json!(12345u64));
        assert!(vc["credentialSubject"]["dogTagId"].is_number());

        let vc2 = build_profile_vc(&cfg, "Rex", &mc, &profile, &owner, "tag-abc");
        assert_eq!(vc2["credentialSubject"]["dogTagId"], json!("tag-abc"));
        assert!(vc2["credentialSubject"]["dogTagId"].is_string());
    }

    #[test]
    fn build_profile_vc_fills_defaults_for_empty_profile() {
        let cfg = test_cfg();
        let vc = build_profile_vc(
            &cfg,
            "Rex",
            &test_microchip(),
            &PetProfile::default(),
            &test_owner(),
            "1",
        );
        let cs = &vc["credentialSubject"];
        assert_eq!(cs["species"], "Canis lupus familiaris");
        assert_eq!(cs["breedVbo"], "VBO:0200000");
        assert_eq!(cs["breedLabel"], "Mixed Breed");
        assert_eq!(cs["sex"], "male");
        assert_eq!(cs["neuterStatus"], "intact");
        assert_eq!(cs["dateOfBirth"], "2020-01-01");
        assert_eq!(cs["weightHistory"], json!([]));
        // envelope/identity fields pull from config + the DOG_PROFILE constant.
        assert_eq!(vc["recordType"], "DOG_PROFILE");
        assert_eq!(vc["issuer"], "did:web:central.dogtag.test");
        assert_eq!(vc["id"], "urn:dogtag:profile:1");
        assert_eq!(vc["signatureTrustTier"], "self_attested");
        assert_eq!(vc["legalEffect"], "evidentiary");
    }

    #[test]
    fn build_profile_vc_maps_weight_history_and_owner_identity() {
        let cfg = test_cfg();
        let profile = PetProfile {
            species: Some("Felis catus".to_string()),
            weight_history: vec![WeightEntry {
                unit: "kg".to_string(),
                value: "22.7".to_string(),
                measured_on: "2024-01-02".to_string(),
            }],
            ..PetProfile::default()
        };
        let vc = build_profile_vc(&cfg, "Rex", &test_microchip(), &profile, &test_owner(), "7");
        let cs = &vc["credentialSubject"];
        // supplied profile fields override defaults.
        assert_eq!(cs["species"], "Felis catus");
        assert_eq!(cs["weightHistory"][0]["unit"], "kg");
        assert_eq!(cs["weightHistory"][0]["value"], "22.7");
        assert_eq!(cs["weightHistory"][0]["measuredOn"], "2024-01-02");
        // owner identity is copied verbatim (wrap_vc later turns these into typed leaves).
        assert_eq!(cs["ownerIdentity"]["countryOfIdentification"], "GB");
        assert_eq!(cs["ownerIdentity"]["identification"], "P12345");
        assert_eq!(cs["ownerIdentity"]["name"], "Jane Doe");
        assert_eq!(cs["microchip"]["code"], "900012345678901");
    }

    // ---- wrap_vc (validate_schema then wrap_document) ----

    #[test]
    fn wrap_vc_validates_and_wraps_a_built_profile() {
        let cfg = test_cfg();
        let vc = build_profile_vc(
            &cfg,
            "Rex",
            &test_microchip(),
            &PetProfile::default(),
            &test_owner(),
            "12345",
        );
        let meta = profile_issuer_meta(&cfg);
        let wrapped = wrap_vc(meta, &vc).expect("a complete DOG_PROFILE VC must wrap cleanly");
        assert!(wrapped.signature.merkle_root.starts_with("0x"));
        assert_eq!(wrapped.signature.type_, "DogTagMerkleProof");
        assert_eq!(wrapped.issuer.record_type, "DOG_PROFILE");
        // the wrapped data carries typed-scalar leaves, not the plain VC strings.
        assert!(wrapped.data.is_object());
    }

    #[test]
    fn wrap_vc_surfaces_schema_violations_with_a_schema_prefix() {
        let cfg = test_cfg();
        // a VC missing every required field fails validate_schema before any wrapping happens.
        let bad = json!({ "recordType": DOG_PROFILE });
        let err = wrap_vc(profile_issuer_meta(&cfg), &bad).unwrap_err();
        assert!(err.starts_with("schema:"), "got: {err}");
    }
}
