//! Evidence transcript for the rest of the provider journey - attach, service standing, and the
//! three capability/approval levers - driven over the REAL Axum admin router against `MemChain`.
//!
//! Same shape and purpose as `control_plane_evidence.rs`: it writes the actual JSON an operator
//! would see to a transcript file, so a reviewer can read the end-to-end registrar behaviour rather
//! than trust a pass/fail. Not an assertion suite - `provider_journey.rs` owns the pins - it exists
//! to produce the artifact. It asserts only the few facts a transcript would otherwise SHOW
//! silently wrong: that the journey's steps really landed, and that the verify capability carries
//! the RAW purpose word rather than the derived verification key.

mod common;

use axum::http::StatusCode;
use common::*;

use admin_api::chain::{purpose_key, record_type_key, verify_key};
use admin_api::routes::router;

const PID: &str = "0x7a1c9f3b0e5d4a28c6b1f0937de4a5b2c8017f36";
const CONTROLLER: &str = "0x0000000000000000000000000000000000c07701";
const DIGEST: &str = "0x9f2b7c1d3e4a5b6c7d8e9f0a1b2c3d4e5f60718293a4b5c6d7e8f9a0b1c2d3e4";
const SERVICE: &str = "0x00000000000000000000000000000000000005ac";
const SIGNER: &str = "0x00000000000000000000000000000000000000c3";
const RELAYER: &str = "0x00000000000000000000000000000000000000e1";
const RESOLVER: &str = "0x00000000000000000000000000000000000000d0";
const GENERATION: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const HOSTED: &str = "0x00000000000000000000000000000000000000ad";
/// A generation-1 `DogTagIssuer`: `Initializable` only, so it answers no `owner()` at all.
const GEN1_ISSUER: &str = "0x00000000000000000000000000000000000019a1";

fn out_path() -> Option<std::path::PathBuf> {
    std::env::var("EVIDENCE_DIR")
        .ok()
        .map(|d| std::path::Path::new(&d).join("provider-journey-api-transcript.txt"))
}

#[tokio::test]
async fn provider_journey_api_transcript() {
    let mut log = String::new();
    macro_rules! w { ($($a:tt)*) => {{ log.push_str(&format!($($a)*)); log.push('\n'); }} }

    let (state, chain, _v, _b) = hermetic_state();
    chain.set_factory_owner(PROVIDER_REGISTRY, HOSTED);
    chain.set_factory_generation(PROVIDER_REGISTRY, GENERATION, FACTORY, true);
    chain.set_clone(FACTORY, SERVICE);
    chain.set_service_metadata(SERVICE, CONTROLLER, &record_type_key("VACCINATION"));
    let app = router(state);
    let tok = admin_token(&app).await;

    macro_rules! step {
        ($method:expr, $path:expr, $body:expr, $note:expr) => {{
            let body: Option<serde_json::Value> = $body;
            match &body {
                Some(b) => w!("{} {}\n  {}", $method, $path, b),
                None => w!("{} {}", $method, $path),
            }
            let (s, b) = call(&app, $method, $path, Some(&tok), body).await;
            w!(
                "HTTP {}\n{}",
                s.as_u16(),
                serde_json::to_string_pretty(&b).unwrap()
            );
            w!(">> {}\n", $note);
            (s, b)
        }};
    }

    w!("================================================================================");
    w!("  The rest of the generation-2 provider journey - live admin API transcript");
    w!("  Real admin-api Axum router over MemChain (hermetic). Every JSON body below is");
    w!("  the actual endpoint output, admin-session gated, with the hosted key holding the");
    w!("  ProviderRegistry owner so registrar writes EXECUTE rather than propose.");
    w!("================================================================================\n");

    w!("--------------------------------------------------------------------------------");
    w!("  PRECONDITION - register + activate (the surface that already existed)");
    w!("--------------------------------------------------------------------------------\n");

    step!(
        "POST",
        "/v1/admin/providers",
        Some(serde_json::json!({
            "providerId": PID, "controller": CONTROLLER, "identityDigest": DIGEST,
        })),
        "registerProvider lands the record at PENDING - registration alone lets the provider do \
         nothing. Note `standingAfterRegistration` and `nextStep` say so."
    );
    step!(
        "POST",
        &format!("/v1/admin/providers/{PID}/standing"),
        Some(serde_json::json!({ "standing": "active" })),
        "setProviderStanding(ACTIVE). Without this every downstream predicate refuses."
    );

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 1 - PREFLIGHT: what the chain says, before anything is signed");
    w!("--------------------------------------------------------------------------------\n");

    step!(
        "POST",
        &format!("/v1/admin/providers/{PID}/services/preflight"),
        Some(serde_json::json!({ "serviceAddress": GEN1_ISSUER })),
        "A generation-1 DogTagIssuer. It is Initializable only and has no owner() at all, so it can \
         NEVER be attached - the preflight names that permanent property rather than letting a raw \
         revert arrive from a send and read as a form error a different owner would fix."
    );
    step!(
        "POST",
        &format!("/v1/admin/providers/{PID}/services/preflight"),
        Some(serde_json::json!({ "serviceAddress": SERVICE })),
        "The real contract. The generation id and the owner are READ off the chain - an admin types \
         neither. `expectedOwner` is carried into the send as a transaction GUARD, never a selector."
    );

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 2 - ATTACH, and the guard that makes expectedOwner a guard");
    w!("--------------------------------------------------------------------------------\n");

    let (s, _) = step!(
        "POST",
        &format!("/v1/admin/providers/{PID}/services"),
        Some(serde_json::json!({
            "serviceAddress": SERVICE,
            "generationId": GENERATION,
            "expectedOwner": SIGNER,
        })),
        "A WRONG expected owner refuses the attach rather than redirecting it to whoever the chain \
         happens to report. This is the whole point of sending the owner as reviewed."
    );
    assert_ne!(s, StatusCode::OK, "a wrong expected owner must refuse");

    let (s, _) = step!(
        "POST",
        &format!("/v1/admin/providers/{PID}/services"),
        Some(serde_json::json!({
            "serviceAddress": SERVICE,
            "generationId": GENERATION,
            "expectedOwner": CONTROLLER,
        })),
        "attachService lands. `standingAfterAttach` is PENDING - exactly as registerProvider lands \
         a provider there. Every registrar CREATION lands its record inert."
    );
    assert_eq!(s, StatusCode::OK);

    let (_, b) = step!(
        "GET",
        &format!("/v1/admin/providers/{PID}/services"),
        None,
        "The five effectiveService terms, reported APART. `serviceStanding` is pending and \
         `hasActiveIssuer` is false - two different remedies, which one folded bool could not carry."
    );
    assert_eq!(b["services"][0]["effective"]["serviceStanding"], "pending");

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 3 - SERVICE STANDING: the step easiest to leave out");
    w!("--------------------------------------------------------------------------------\n");

    step!(
        "POST",
        &format!("/v1/admin/services/{SERVICE}/standing"),
        Some(serde_json::json!({ "standing": "active" })),
        "setServiceStanding(ACTIVE). canIssue folds the service standing, so attach + an issuance \
         grant WITHOUT this produces a service that still issues nothing, silently."
    );

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 4 - THE ISSUE RIGHT, on the ADDRESS");
    w!("--------------------------------------------------------------------------------\n");

    let (s, _) = step!(
        "POST",
        &format!("/v1/admin/rights/{SIGNER}/issue"),
        Some(serde_json::json!({ "allowed": true })),
        "setRights(account, RIGHT_ISSUE). The path carries an ADDRESS and no service, because the \
         grant does not - which is what makes an approval possible before the applicant has a \
         clone at all. It is the registrar's grant to make and nobody else's: a service delegate \
         carries content-write permissions and does not satisfy canIssue. It also reaches EVERY \
         service in effective standing, including another provider's."
    );
    assert_eq!(s, StatusCode::OK, "the grant must actually land");

    let (_, b) = step!(
        "GET",
        &format!("/v1/admin/providers/{PID}/services"),
        None,
        "Four terms now hold and `hasActiveIssuer` is STILL false - the chain re-folds the \
         provider's CURRENT POINTER into that term, and only the provider's own repointService can \
         set it. This is exactly what the live ROAX walk recorded, and why the term carries a \
         remedy the registrar cannot perform."
    );
    let view = &b["services"][0];
    assert_eq!(view["effective"]["serviceStanding"], "active");
    // The grant DID land, so the narration above is provably about the pointer term rather than
    // about an issuance capability that was never written.
    assert_eq!(view["issuance"]["entries"][0]["holder"], SIGNER);
    assert_eq!(view["issuance"]["entries"][0]["allowed"], true);
    assert_eq!(view["currentPointer"]["isCurrent"], false);
    assert_eq!(view["effective"]["hasActiveIssuer"], false);

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 5 - VERIFY CAPABILITY: the ORTHOGONAL axis, keyed by purpose");
    w!("--------------------------------------------------------------------------------\n");

    let (_, b) = step!(
        "POST",
        "/v1/admin/verifier-capabilities",
        Some(serde_json::json!({
            "purpose": "travel_check", "relayer": RELAYER, "allowed": true,
        })),
        "Keyed by PURPOSE and taking NO service - an issuer is not implicitly a verifier."
    );
    let raw = purpose_key("travel_check");
    let derived = verify_key("travel_check");
    w!("  purposeKey sent   = {}", b["purposeKey"]);
    w!("  purpose_key(RAW)  = {raw}");
    w!("  verify_key(DERIV) = {derived}");
    w!(
        ">> The RAW purpose word is what goes on the wire. The contract derives \
         verificationKey(purpose) ITSELF, so sending the derived key would derive TWICE and write \
         the capability under a key canVerify never reads - a transaction that succeeds, costs gas \
         and grants nothing, with no error anywhere.\n"
    );
    assert_eq!(b["purposeKey"], raw, "must carry the RAW purpose");
    assert_ne!(b["purposeKey"], derived, "must NOT carry the derived key");

    w!("--------------------------------------------------------------------------------");
    w!("  STEP 6 - TYPED RESOLVER APPROVAL: reported as HALF a step");
    w!("--------------------------------------------------------------------------------\n");

    // `setResolverApproved` refuses a non-contract; the fake's clone set stands in for "has code".
    chain.set_clone(FACTORY, RESOLVER);
    let (_, b) = step!(
        "POST",
        "/v1/admin/resolvers",
        Some(serde_json::json!({
            "kind": "directory", "resolver": RESOLVER, "approved": true,
        })),
        "setResolverApproved is the registrar's whole part. The response says so in `nextStep`: a \
         typed resolver answers nothing until the provider ALSO selects it, and the core never \
         clears a stored selection when a resolver is deapproved - which is exactly why the two \
         halves must never be pre-ANDed into one 'working' bool."
    );
    assert!(b["nextStep"].as_str().is_some_and(|s| !s.is_empty()));

    w!("--------------------------------------------------------------------------------");
    w!("  THE DELETED WHITELIST CONSOLE - its routes are gone, not merely unlinked");
    w!("--------------------------------------------------------------------------------\n");

    for path in ["/v1/admin/whitelist/grant", "/v1/admin/whitelist/revoke"] {
        let (s, _b) = call(
            &app,
            "POST",
            path,
            Some(&tok),
            Some(serde_json::json!({ "signer": SIGNER, "recordType": "VACCINATION" })),
        )
        .await;
        w!("POST {path}  ->  HTTP {}", s.as_u16());
        assert_eq!(s, StatusCode::NOT_FOUND, "{path} must no longer exist");
    }
    w!(
        "\n>> It called isWhitelistedFor, which the single authority answers off the orthogonal \
         VERIFY axis for a caller that is not itself an attached service - a confident false for \
         every genuine issuer signer - and granted through whitelistFor, which that contract does \
         not implement at all. There was nothing to fix. Its replacements are steps 4 and 5 above.\n"
    );

    w!("================================================================================");
    w!("  END TRANSCRIPT");
    w!("================================================================================");

    println!("{log}");
    if let Some(p) = out_path() {
        std::fs::write(&p, &log).expect("write evidence transcript");
        println!("\n[evidence written to {}]", p.display());
    }
}
