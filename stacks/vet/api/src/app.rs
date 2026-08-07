//! Application state + config + the server-side VC build (impl §3.3/§11.6: build is ALWAYS server-side).

use std::sync::Arc;

use dogtag_standard::discovery::ConvenienceClaims;
use dogtag_standard::wrap::{
    wrap_document, IssuerMeta, ProtocolMeta, WrappedDoc, LEVEL_B_VERSION,
};
use serde_json::Value;

use crate::auth::JwtKeys;
use crate::calendar::{CalendarProvider, CentralClient};
use crate::chain::{record_type_key, ChainClient};
use crate::custody::Custody;
use crate::oversight::OversightFeed;
use crate::store::Store;

/// Resolved issuer/contract addresses + deployment config.
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    /// The `DogTagIssuerFactory` (env `FACTORY_ADDR`) whose write-once `rootIssuer[R]` index resolves
    /// which clone issued a given root.
    ///
    /// This is what makes `POST /verify/credential` answerable at all: the document's own
    /// `issuer.documentStore` sits OUTSIDE the Merkle root, so reading `isValid` against it lets a
    /// forged document nominate the contract that vouches for it. Unset (or zero) means this
    /// deployment cannot evaluate the factory-anchored issuer pillar, which is reported as
    /// `unavailableNoFactoryConfigured` rather than silently passing.
    pub factory_addr: String,
    /// Owner-hidden `VerificationRegistryConsent` address (env
    /// `VERIFICATION_REGISTRY_CONSENT_ADDR`) — the sole verification submit target.
    pub verification_registry_consent_addr: String,
    /// recordType (string) -> issuer clone address (documentStore).
    pub issuer_addrs: std::collections::HashMap<String, String>,
    pub issuer_name: String,
    pub issuer_domain: String,
    /// `DogTagSBTConsent` address (env `SBT_CONSENT_ADDR`) — the owner-blind mint target for
    /// `mintCustodial(id, R)`. The vet signer must hold ISSUER_ROLE here.
    pub sbt_consent_addr: String,
    /// The `DogTagIssuer` clone that anchors profile roots (env `PROFILE_ISSUER_ADDR`) — the
    /// `issue(R)` target that makes `rootIssuer[R]` resolve.
    pub profile_issuer_addr: String,
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
    /// The DEPLOYMENT ROLE this instance runs as (env `BUSINESS_TYPE`). The groomer stack runs the
    /// SAME `vet-api` binary as the vet, so the role — not the binary — decides which surfaces exist:
    /// a `groomer` VERIFIES and does not ISSUE, so `public_router` does not mount the issuance routes
    /// for it (see [`Config::issuance_enabled`]). Unset/anything else -> `vet` (issuance ON), so every
    /// existing vet deployment and every test is byte-for-byte unchanged.
    pub business_type: String,
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

    /// True iff this deployment runs as a GROOMER (env `BUSINESS_TYPE=groomer`, case-insensitive).
    pub fn is_groomer(&self) -> bool {
        self.business_type.eq_ignore_ascii_case("groomer")
    }

    /// Whether the ISSUANCE surfaces (`/credentials/*`, `/records/*`, `/r/{token}`,
    /// `/profiles/issue/*`, `/p/{token}`) are mounted on this instance.
    ///
    /// A groomer is a VERIFIER: it records proofs-of-verification against the `VERIFY:<purpose>`
    /// whitelist namespace and never mints credentials, so those routes are not mounted at all (the
    /// same "the route does not exist" posture the `prover` feature takes for `/prove-verification`).
    /// Every other role — notably the vet — keeps them.
    pub fn issuance_enabled(&self) -> bool {
        !self.is_groomer()
    }

    /// The operator-facing refusal for a backend that cannot ANCHOR a dog tag, or `None` when both
    /// owner-hidden anchor contracts are configured: the `DogTagIssuer` clone `issue(R)` anchors
    /// into (`PROFILE_ISSUER_ADDR`) and the `DogTagSBTConsent` the tag is minted on
    /// (`SBT_CONSENT_ADDR`). Shape-checked via `verify::valid_contract_addr`, the same predicate the
    /// custodial-bind route fails closed on — so the two surfaces cannot disagree about what
    /// "configured" means.
    ///
    /// This is a CONFIG fact, never a chain read: `None` claims the addresses are set and
    /// well-formed, not that either contract works. It exists because the bind-time 503 is correct
    /// but INVISIBLE where it is actionable — it surfaces on the owner's phone, after the operator
    /// has already allocated a tag and had a second human scan a QR. Every surface that would start
    /// that journey (the session-start route, the portal's Register pet screen via `/health`) asks
    /// this ONE function instead of re-deriving the rule.
    ///
    /// The message names the missing thing in the operator's vocabulary and points at the fixing
    /// step; the env variable rides in parentheses as the detail, never as the whole instruction.
    pub fn dog_tag_anchor_refusal(&self) -> Option<String> {
        let profile_ok = crate::verify::valid_contract_addr(&self.profile_issuer_addr);
        let sbt_ok = crate::verify::valid_contract_addr(&self.sbt_consent_addr);
        if profile_ok && sbt_ok {
            return None;
        }
        let mut parts = Vec::new();
        if !profile_ok {
            parts.push(
                "the clinic's DOG_PROFILE issuer contract is not set — deploy or select it on the \
                 Provider page, configure this backend with its address (PROFILE_ISSUER_ADDR), and \
                 restart the backend",
            );
        }
        if !sbt_ok {
            parts.push(
                "the DogTag SBT contract is not set — configure this backend with the \
                 DogTagSBTConsent address from the deployment ledger, contracts/deployments/\
                 roax.json (SBT_CONSENT_ADDR), and restart the backend",
            );
        }
        Some(format!(
            "this backend cannot anchor a dog tag on-chain: {}",
            parts.join("; and ")
        ))
    }
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    /// The consent prover, lazily loaded per request (fail-closed per request, not at
    /// boot — see [`crate::prover::ConsentProver`]). `disabled()` on instances that do not serve
    /// `/prove-consent`; `from_env()` (reads `CIRCUITS_BUILD_DIR`) on the prover-service.
    pub consent_prover: Arc<crate::prover::ConsentProver>,
    /// Google Calendar provider (real `GoogleCalendar` in prod, `MockCalendar` in tests).
    pub calendar: Arc<dyn CalendarProvider>,
    /// the appointment-events callback to central (real `ReqwestCentralClient` / mock in tests).
    pub central: Arc<dyn CentralClient>,
    pub custody: Custody,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
    /// in-memory login/unlock rate limiter (lenient; demo-safe).
    pub ratelimit: Arc<crate::auth::RateLimiter>,
    /// SCOPED consumer of the PR-4 oversight indexer — the data layer behind the traceability portal
    /// (govarch PR-5). `DisabledFeed` when `INDEXER_API_BASE` is unset; the real `HttpOversightFeed`
    /// (presenting this business's scoped bearer) otherwise. See `crate::oversight` / `crate::trace`.
    pub feed: Arc<dyn OversightFeed>,
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

/// Build the M7 provenance block (§4.2) for a record anchored on this deployment. `issuer_clone` is
/// the clone the root anchors to (== `documentStore`); `issuer_signer` is the *claim* of who issued
/// (empty until the authoritative on-chain `issuedBy[R]` is learned at confirm). A routing hint only -
/// authority stays the on-chain re-derivation; the claim is validated against `issuedBy[R]` at verify.
pub fn protocol_meta(
    cfg: &Config,
    chain_id: u64,
    issuer_clone: &str,
    issuer_signer: &str,
) -> ProtocolMeta {
    ProtocolMeta {
        chain_id,
        version: LEVEL_B_VERSION.to_string(),
        verification_registry: cfg.verification_registry_consent_addr.clone(),
        issuer_clone: issuer_clone.to_string(),
        issuer_signer: issuer_signer.to_string(),
        // No public receipt-status page here: this stack's `/r/<token>` is a one-time record-share
        // handoff, not a durable PII-free status page. Stamping `deployment_url` would point a
        // receipt QR at a route that 404s once the token is consumed.
        status_base_url: None,
    }
}

/// Build the CONVENIENCE-tier claims (§5.2, M7 P4) a resolve GET returns: the platform-OWNED,
/// UNVERIFIED `{protocolVersion, chainId, verificationRegistry, issuerClone, purpose}`. Every value is
/// read straight from THIS deployment's config — but to a consuming app they are CLAIMS, not authority
/// (§1.2 trust boundary). The app MUST resolve the dogtag ProtocolRegistry / signed-manifest anchor and
/// call `dogtag_standard::discovery::validate` before trusting the `{version, registry, chainId}` here,
/// so a lying platform cannot steer a proof onto an attacker registry. `issuer_clone`/`purpose` vary by
/// flow (verify vs issuance); the rest are deployment-wide.
pub fn convenience_claims(
    cfg: &Config,
    chain_id: u64,
    issuer_clone: &str,
    purpose: &str,
) -> ConvenienceClaims {
    ConvenienceClaims {
        protocol_version: LEVEL_B_VERSION.to_string(),
        chain_id,
        verification_registry: cfg.verification_registry_consent_addr.clone(),
        issuer_clone: issuer_clone.to_string(),
        purpose: purpose.to_string(),
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

/// The DOG_PROFILE record type used by owner-hidden profile issuance.
pub const DOG_PROFILE: &str = "DOG_PROFILE";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
            factory_addr: String::new(),
            verification_registry_consent_addr: String::new(),
            issuer_addrs: std::collections::HashMap::new(),
            issuer_name: "Test Vet".to_string(),
            issuer_domain: "vet.example".to_string(),
            sbt_consent_addr: "0xsbtconsent".to_string(),
            profile_issuer_addr: "0xprofileissuer".to_string(),
            vet_signer_index: 0,
            operator_password: String::new(),
            admin_password: String::new(),
            confirmations: 0,
            business_id: String::new(),
            business_type: "vet".to_string(),
            central_hmac_secret: String::new(),
            custody_seal_path: None,
        }
    }
}
