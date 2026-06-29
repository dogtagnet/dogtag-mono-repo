//! Application state + config + the server-side VC build (impl §3.3/§11.6: build is ALWAYS server-side).

use std::sync::Arc;

use dogtag_standard::schema::{validate_schema, DOGTAG_CONTEXT_URI};
use dogtag_standard::wrap::{wrap_document, IssuerMeta, WrappedDoc};
use serde_json::{json, Value};

use crate::auth::JwtKeys;
use crate::calendar::{CalendarProvider, CentralClient};
use crate::chain::{record_type_key, ChainClient};
use crate::custody::Custody;
use crate::prover::ProverClient;
use crate::store::Store;

/// Resolved issuer/contract addresses + deployment config.
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    pub verification_registry_addr: String,
    /// ConsentKeyRegistry address (env `CONSENT_KEY_REGISTRY_ADDR`) — the relayer-sponsored
    /// `bindConsentKeyFor` target and the `keyOf`/`bindNonce` read surface for the ZK consent path.
    pub consent_key_registry_addr: String,
    /// recordType (string) -> issuer clone address (documentStore).
    pub issuer_addrs: std::collections::HashMap<String, String>,
    pub issuer_name: String,
    pub issuer_domain: String,
    /// DogTagSBT contract address — the mint target for the DOG_PROFILE SBT (env `SBT_ADDR`). The vet
    /// signer must hold ISSUER_ROLE on this contract.
    pub sbt_addr: String,
    /// the documentStore the DOG_PROFILE VC anchors to (the DogTagSBT contract acts as the profile
    /// store; env `PROFILE_DOCUMENT_STORE`, conventionally == `sbt_addr`).
    pub profile_document_store: String,
    /// account index of the vet signer that mints the DOG_PROFILE SBT (holds ISSUER_ROLE). Index 0 is
    /// the unlocked custody signer used everywhere else in the vet stack.
    pub vet_signer_index: u32,
    /// operator portal password (in prod: hashed/secret-managed).
    pub operator_password: String,
    /// admin-session password for /admin/* custody routes.
    pub admin_password: String,
    /// confirmations to wait at confirm time (low for tests).
    pub confirmations: u64,
    /// this business's id (assigned by central registration) — used in the appointment contract.
    pub business_id: String,
    /// shared HMAC secret with central (verifies inbound PUTs; signs outbound appointment-events).
    pub central_hmac_secret: String,
    /// OPTIONAL on-disk path where the SEALED custody (age-encrypted seed ciphertext + non-secret
    /// keystore meta — NEVER the plaintext mnemonic, NEVER the passphrase) is persisted so the signer
    /// survives a backend restart. `None` -> in-memory only (no file; demo/tests unchanged). Env
    /// `CUSTODY_SEAL_PATH`.
    pub custody_seal_path: Option<String>,
}

impl Config {
    pub fn issuer_addr_for(&self, record_type: &str) -> Option<String> {
        self.issuer_addrs.get(record_type).cloned()
    }
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub prover: Arc<dyn ProverClient>,
    /// Google Calendar provider (real `GoogleCalendar` in prod, `MockCalendar` in tests).
    pub calendar: Arc<dyn CalendarProvider>,
    /// the appointment-events callback to central (real `ReqwestCentralClient` / mock in tests).
    pub central: Arc<dyn CentralClient>,
    pub custody: Custody,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
    /// in-memory login/unlock rate limiter (lenient; demo-safe).
    pub ratelimit: Arc<crate::auth::RateLimiter>,
}

/// Build the issuer metadata for a record type (documentStore = the issuer clone address).
pub fn issuer_meta(cfg: &Config, record_type: &str, issuer_addr: &str) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: issuer_addr.to_string(),
        record_type: record_type.to_string(),
    }
}

/// Build a typed credential (the typed-scalar `data` the SDK wraps) from operator `fields`.
///
/// The operator supplies `fields` already in the SDK's typed-scalar input shape
/// (`{tag:<u8>, value:"..."}` leaves, nested under `credentialSubject`/top-level). We inject the
/// mandatory, non-obfuscatable `credentialSubject.dogTagId` (tag 3 = INTEGER, or tag 2 if non-numeric).
pub fn build_vc(record_type: &str, fields: &Value, dog_tag_id: &str) -> Value {
    let mut cred = fields.clone();
    if !cred.is_object() {
        cred = serde_json::json!({});
    }
    let obj = cred.as_object_mut().unwrap();
    let subject = obj
        .entry("credentialSubject")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(s) = subject.as_object_mut() {
        // dogTagId is INTEGER if it parses as a decimal integer, else STRING.
        let is_int = dog_tag_id.bytes().all(|b| b.is_ascii_digit()) && !dog_tag_id.is_empty();
        let tag = if is_int { 3 } else { 2 };
        s.insert(
            "dogTagId".to_string(),
            serde_json::json!({ "tag": tag, "value": dog_tag_id }),
        );
    }
    // attach recordType passthrough for downstream context (not wrapped into leaves unless present).
    let _ = record_type;
    cred
}

/// Project a plain VC into the typed-scalar `{tag,value}` form the flatten/Merkle pipeline requires,
/// PRESERVING any leaf that `build_vc` already typed (e.g. dogTagId). Mirrors the central `to_typed_vc`.
fn to_typed(v: &Value) -> Value {
    match v {
        Value::Object(m) => {
            // already a typed scalar -> keep as-is
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
        Value::Null => serde_json::json!({ "tag": 0u8, "value": Value::Null }),
        Value::Bool(b) => serde_json::json!({ "tag": 1u8, "value": b }),
        Value::String(s) => serde_json::json!({ "tag": 2u8, "value": s }),
        Value::Number(n) => {
            let tag = if n.is_i64() || n.is_u64() { 3u8 } else { 4u8 };
            serde_json::json!({ "tag": tag, "value": n.to_string() })
        }
    }
}

/// Wrap a VC into a `WrappedDoc` using a cryptographically-random salt provider.
pub fn wrap(record_type: &str, issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    let _ = record_type;
    let typed = to_typed(vc); // type every scalar leaf (fixes "non-typed leaf at ..." on form fields)
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    wrap_document(&typed, issuer_meta, &mut salt).map_err(|e| format!("wrap: {e}"))
}

/// Convenience: the bytes32 whitelist/issuer key for a record type.
pub fn rt_key(record_type: &str) -> String {
    record_type_key(record_type)
}

// --------------------------------------------------------------------------------------------
// DOG_PROFILE (SBT) build/wrap — ported from the admin stack (stacks/admin/api/src/app.rs). The vet
// now ISSUES dog tags: it builds the DOG_PROFILE VC with the owner-identity baked into the merkle
// leaves, computes the root, and mints the SBT to the device wallet.
// --------------------------------------------------------------------------------------------

/// The DOG_PROFILE record type the vet mints SBT profiles under.
pub const DOG_PROFILE: &str = "DOG_PROFILE";

/// Build the issuer metadata for the vet's DOG_PROFILE issuer (documentStore == the DogTagSBT
/// contract, which acts as the profile store).
pub fn profile_issuer_meta(cfg: &Config) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: cfg.profile_document_store.clone(),
        record_type: DOG_PROFILE.to_string(),
    }
}

/// Build a COMPLETE, valid DOG_PROFILE VC (plain JSON — the shape `validate_schema` operates on) from
/// the session's pet record + owner identity (ported from admin `build_profile_vc`). Missing optional
/// DOG_PROFILE subject fields fall back to sensible defaults. The returned VC passes `validate_schema`
/// for recordType DOG_PROFILE; `wrap_vc` then converts it to typed-scalar leaves + the Merkle root.
pub fn build_profile_vc(
    cfg: &Config,
    name: &str,
    microchip: &crate::store::Microchip,
    profile: &crate::store::PetProfile,
    owner_identity: &crate::store::OwnerIdentity,
    owner_address: &str,
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
    // projection wraps it as an INTEGER (tag 3), matching `build_vc`.
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
        // legal/trust meta (owner-asserted profile minted by the vet's issuer signer).
        "attestationType": "identity",
        "signatureTrustTier": "self_attested",
        "legalEffect": "evidentiary",
        "legalBasisVersion": "DOGTAG-PROFILE-v1",
        "jurisdiction": "GLOBAL",
        "credentialSubject": {
            // subject/holder ownership is established on-chain by the SBT mint (ownerOf[dogTagId] ==
            // the device wallet that scanned the vet's QR). The VC ALSO bakes the device wallet as the
            // `ownerAddress` leaf + the 3 owner-identity leaves — committed (salted) record attestations
            // in the DOG_PROFILE merkle root (the on-chain `ownerOf`/consent-key remain the ZK binding;
            // these leaves do not feed the export circuit — verified safe).
            "dogTagId": dog_tag_id_val,
            // the device's wallet address (the on-chain owner), committed as a record-anchored leaf.
            "ownerAddress": owner_address.to_lowercase(),
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
            // owner's official identity, entered by the vet operator at session-start (wrap_vc turns
            // the 3 scalars into typed Merkle leaves automatically).
            "ownerIdentity": {
                "countryOfIdentification": owner_identity.country_of_identification,
                "identification": owner_identity.identification,
                "name": owner_identity.name,
            },
        },
    })
}

/// Format a unix-seconds timestamp as ISO `YYYY-MM-DD` (UTC). Howard Hinnant's civil algorithm so we
/// don't pull a date crate just for `validFrom` (ported from admin).
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

/// Convert a plain JSON VC into the SDK's typed-scalar wrap input (`{tag, value}` leaves). Mirrors the
/// admin `to_typed_vc`. Tag mapping: 0=null, 1=bool, 2=string, 3=integer, 4=decimal.
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
            let s = n.to_string();
            let tag = if n.is_i64() || n.is_u64() { 3u8 } else { 4u8 };
            json!({ "tag": tag, "value": s })
        }
    }
}

/// Validate a plain DOG_PROFILE VC against the SDK schema, then wrap it into a `WrappedDoc` (ported
/// from admin `wrap_vc`). The validator runs on the plain VC; we wrap the typed-scalar projection so
/// the on-chain root covers every field.
pub fn wrap_vc(issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    validate_schema(vc).map_err(|violations| format!("schema: {}", violations.join("; ")))?;
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

    // ---- build_vc: dogTagId injection + INTEGER/STRING tag selection ----

    #[test]
    fn build_vc_injects_numeric_dog_tag_id_as_integer() {
        let fields = json!({ "credentialSubject": { "name": { "tag": 2u8, "value": "Rex" } } });
        let vc = build_vc("DOG_PROFILE", &fields, "42");
        let id = &vc["credentialSubject"]["dogTagId"];
        assert_eq!(id["tag"], 3, "all-digit id -> INTEGER (tag 3)");
        assert_eq!(id["value"], "42");
        // pre-existing sibling leaf is preserved untouched
        assert_eq!(vc["credentialSubject"]["name"]["value"], "Rex");
    }

    #[test]
    fn build_vc_injects_nonnumeric_dog_tag_id_as_string() {
        let vc = build_vc("DOG_PROFILE", &json!({}), "abc-1");
        assert_eq!(
            vc["credentialSubject"]["dogTagId"]["tag"], 2,
            "non-digit -> STRING (tag 2)"
        );
        assert_eq!(vc["credentialSubject"]["dogTagId"]["value"], "abc-1");
    }

    #[test]
    fn build_vc_treats_empty_id_as_string_not_integer() {
        // is_int requires non-empty AND all-ascii-digit; "" must fall to STRING.
        let vc = build_vc("DOG_PROFILE", &json!({}), "");
        assert_eq!(vc["credentialSubject"]["dogTagId"]["tag"], 2);
        assert_eq!(vc["credentialSubject"]["dogTagId"]["value"], "");
    }

    #[test]
    fn build_vc_creates_credential_subject_when_missing() {
        let vc = build_vc("DOG_PROFILE", &json!({ "other": 1 }), "7");
        assert!(vc["credentialSubject"].is_object());
        assert_eq!(vc["credentialSubject"]["dogTagId"]["value"], "7");
    }

    #[test]
    fn build_vc_replaces_non_object_fields_with_empty_object() {
        // A non-object `fields` is discarded; we still get a well-formed subject.
        let vc = build_vc("DOG_PROFILE", &json!("not-an-object"), "9");
        assert!(vc.is_object());
        assert_eq!(vc["credentialSubject"]["dogTagId"]["value"], "9");
    }

    // ---- to_typed: PRESERVES already-typed {tag,value} leaves ----

    #[test]
    fn to_typed_preserves_pretyped_scalar_and_types_plain_leaves() {
        let input = json!({
            "pre": { "tag": 3u8, "value": "5" },
            "name": "Rex",
            "alive": true,
            "weight": 4.5,
            "count": 3,
            "missing": Value::Null,
        });
        let out = to_typed(&input);
        // a 2-key {tag,value} object is kept verbatim, not re-wrapped
        assert_eq!(out["pre"], json!({ "tag": 3u8, "value": "5" }));
        assert_eq!(out["name"], json!({ "tag": 2u8, "value": "Rex" }));
        assert_eq!(out["alive"], json!({ "tag": 1u8, "value": true }));
        assert_eq!(out["weight"], json!({ "tag": 4u8, "value": "4.5" }));
        assert_eq!(out["count"], json!({ "tag": 3u8, "value": "3" }));
        assert_eq!(out["missing"], json!({ "tag": 0u8, "value": Value::Null }));
    }

    #[test]
    fn to_typed_recurses_into_arrays() {
        let out = to_typed(&json!(["a", 2]));
        assert_eq!(
            out,
            json!([{ "tag": 2u8, "value": "a" }, { "tag": 3u8, "value": "2" }])
        );
    }

    // ---- to_typed_vc: does NOT preserve pre-typed leaves (re-wraps every scalar) ----

    #[test]
    fn to_typed_vc_maps_every_scalar_kind() {
        let input = json!({ "s": "x", "b": false, "i": 10, "d": 1.5, "n": Value::Null });
        let out = to_typed_vc(&input);
        assert_eq!(out["s"], json!({ "tag": 2u8, "value": "x" }));
        assert_eq!(out["b"], json!({ "tag": 1u8, "value": false }));
        assert_eq!(out["i"], json!({ "tag": 3u8, "value": "10" }));
        assert_eq!(out["d"], json!({ "tag": 4u8, "value": "1.5" }));
        assert_eq!(out["n"], json!({ "tag": 0u8, "value": Value::Null }));
    }

    #[test]
    fn to_typed_vc_double_wraps_a_pretyped_leaf_unlike_to_typed() {
        // Documented divergence: to_typed_vc has no is-typed-scalar short-circuit, so a
        // {tag,value} object is walked as a plain object and each member re-typed. build_profile_vc
        // never feeds it pre-typed leaves, so this divergence is safe in practice.
        let pretyped = json!({ "tag": 3u8, "value": "5" });
        let out = to_typed_vc(&pretyped);
        assert_eq!(out["tag"], json!({ "tag": 3u8, "value": "3" }));
        assert_eq!(out["value"], json!({ "tag": 2u8, "value": "5" }));
        // to_typed, by contrast, keeps it verbatim
        assert_eq!(to_typed(&pretyped), pretyped);
    }

    // ---- iso_date_utc: Howard Hinnant civil-date algorithm ----

    #[test]
    fn iso_date_utc_anchors() {
        assert_eq!(iso_date_utc(0), "1970-01-01");
        assert_eq!(iso_date_utc(1_700_000_000), "2023-11-14");
        // leap day: 2020-02-29T00:00:00Z
        assert_eq!(iso_date_utc(1_582_934_400), "2020-02-29");
        // intra-day seconds are truncated to the day
        assert_eq!(iso_date_utc(86_400 - 1), "1970-01-01");
        assert_eq!(iso_date_utc(86_400), "1970-01-02");
    }

    // ---- issuer metadata helpers ----

    #[test]
    fn issuer_meta_copies_identity_and_record_type() {
        let cfg = test_cfg();
        let m = issuer_meta(&cfg, "BOARDING", "0xstore");
        assert_eq!(m.name, "Test Vet");
        assert_eq!(m.domain, "vet.example");
        assert_eq!(m.document_store, "0xstore");
        assert_eq!(m.record_type, "BOARDING");
    }

    #[test]
    fn profile_issuer_meta_uses_profile_document_store_and_dog_profile() {
        let cfg = test_cfg();
        let m = profile_issuer_meta(&cfg);
        assert_eq!(m.document_store, "0xprofilestore");
        assert_eq!(m.record_type, DOG_PROFILE);
        assert_eq!(m.record_type, "DOG_PROFILE");
    }

    #[test]
    fn issuer_addr_for_returns_mapped_or_none() {
        let mut cfg = test_cfg();
        cfg.issuer_addrs
            .insert("BOARDING".to_string(), "0xboarding".to_string());
        assert_eq!(
            cfg.issuer_addr_for("BOARDING").as_deref(),
            Some("0xboarding")
        );
        assert_eq!(cfg.issuer_addr_for("UNKNOWN"), None);
    }

    #[test]
    fn rt_key_matches_record_type_key() {
        assert_eq!(rt_key("BOARDING"), record_type_key("BOARDING"));
    }

    fn test_cfg() -> Config {
        Config {
            deployment_url: String::new(),
            rpc_url: String::new(),
            issuer_registry_addr: String::new(),
            verification_registry_addr: String::new(),
            consent_key_registry_addr: String::new(),
            issuer_addrs: std::collections::HashMap::new(),
            issuer_name: "Test Vet".to_string(),
            issuer_domain: "vet.example".to_string(),
            sbt_addr: String::new(),
            profile_document_store: "0xprofilestore".to_string(),
            vet_signer_index: 0,
            operator_password: String::new(),
            admin_password: String::new(),
            confirmations: 0,
            business_id: String::new(),
            central_hmac_secret: String::new(),
            custody_seal_path: None,
        }
    }
}
