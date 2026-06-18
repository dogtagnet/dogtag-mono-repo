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
    dog_tag_id: &str,
) -> Value {
    let species = profile.species.clone().unwrap_or_else(|| "Canis lupus familiaris".to_string());
    let breed_vbo = profile.breed_vbo.clone().unwrap_or_else(|| "VBO:0200000".to_string());
    let breed_label = profile.breed_label.clone().unwrap_or_else(|| "Mixed Breed".to_string());
    let sex = profile.sex.clone().unwrap_or_else(|| "male".to_string());
    let neuter_status = profile.neuter_status.clone().unwrap_or_else(|| "intact".to_string());
    let date_of_birth = profile.date_of_birth.clone().unwrap_or_else(|| "2020-01-01".to_string());

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
