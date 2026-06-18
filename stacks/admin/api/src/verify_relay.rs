//! Proof-of-verification consent relay (impl §4.1). The mobile owner posts the verifier's session JWT
//! (from the verifier's `/verify/session/start`, aud `dogtag-mobile`) + a signed VerificationConsent.
//! Central:
//!   - verifies the EdDSA session JWT and consumes its jti (one-time);
//!   - asserts consent.relayer==claims.relayer && consent.subject==callerWallet
//!     && consent.recordType==keccak256(claims.recordType) && consent.deadline>=now;
//!   - saves a `verification_records` row + a `ConsentReceipt` (off-chain, deletable — erasure scope);
//!   - relays to the verifier's `/verify/consent/submit` (verifierApiBase resolved from discovery).
//!
//! The session JWT is signed by the VERIFIER's key, not central's — so we accept the verifier's public
//! key carried in the claims-bound discovery record. Here (without a verifier-key registry) we accept a
//! self-asserted `relayer` and bind it to a registered business by matching a documentStore/relayer; the
//! deployment resolves `verifierApiBase` from the discovery `businesses` collection by `relayer`.

use axum::{http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::auth::keccak256_hex;
use crate::crypto;
use crate::store::{ConsentReceipt, VerificationRecord};

type Resp = (StatusCode, Json<Value>);
fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

/// The verifier session claims (impl §3.9) — mirrors the vet stack's `VerifyClaims`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifyClaims {
    pub iss: String,
    pub sub: String, // verifier sessionId
    pub aud: String, // must be "dogtag-mobile"
    pub relayer: String,
    pub purpose: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    pub challenge: String,
    pub mode: String,
    pub exp: u64,
    pub jti: String,
    /// the verifier's API base (so central can relay the submission). Carried in the session JWT.
    #[serde(rename = "verifierApiBase", default)]
    pub verifier_api_base: Option<String>,
}

/// Resolve the verifier's API base from discovery by matching `relayer` against a business's
/// documentStores. Falls back to the JWT-carried `verifierApiBase`.
async fn resolve_verifier_api_base(st: &AppState, claims: &VerifyClaims) -> Option<String> {
    for b in st.store.all_businesses().await {
        if b.document_stores.iter().any(|d| d.eq_ignore_ascii_case(&claims.relayer)) {
            return Some(b.api_base_url);
        }
    }
    claims.verifier_api_base.clone()
}

pub async fn relay(
    st: &AppState,
    owner_id: &str,
    session_jwt: String,
    consent: Value,
    sig: String,
    mode_override: Option<String>,
) -> Resp {
    // verify the verifier's session JWT (EdDSA; signed with the SAME deployment key scheme — in this
    // single-protocol stack the verifier sessions are issued under the shared JWT key).
    let claims: VerifyClaims = match crate::auth::verify_jwt(&st.jwt, &session_jwt, 30) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::UNAUTHORIZED, &format!("sessionJwt: {e}")),
    };
    if claims.aud != "dogtag-mobile" {
        return err(StatusCode::UNAUTHORIZED, "sessionJwt aud != dogtag-mobile");
    }
    // consume the jti (one-time).
    if !st.store.consume_jti(&claims.jti).await {
        return err(StatusCode::UNAUTHORIZED, "session jti already used");
    }

    let owner = match st.store.get_owner(owner_id).await {
        Some(o) => o,
        None => return err(StatusCode::NOT_FOUND, "owner not found"),
    };

    // asserts (impl §4.1).
    let consent_relayer = consent.get("relayer").and_then(|v| v.as_str()).unwrap_or("");
    if !consent_relayer.eq_ignore_ascii_case(&claims.relayer) {
        return err(StatusCode::BAD_REQUEST, "consent.relayer != claims.relayer");
    }
    let consent_subject = consent.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    if !consent_subject.eq_ignore_ascii_case(&owner.wallet_address) {
        return err(StatusCode::BAD_REQUEST, "consent.subject != caller wallet");
    }
    let expected_rt = keccak256_hex(&claims.record_type);
    let consent_rt = consent.get("recordType").and_then(|v| v.as_str()).unwrap_or("");
    if !consent_rt.eq_ignore_ascii_case(&expected_rt) {
        return err(StatusCode::BAD_REQUEST, "consent.recordType != keccak256(claims.recordType)");
    }
    let now = crate::auth::now();
    let deadline = consent.get("deadline").and_then(|v| v.as_u64()).unwrap_or(0);
    if deadline < now {
        return err(StatusCode::BAD_REQUEST, "consent deadline passed");
    }

    let mode = mode_override.unwrap_or_else(|| claims.mode.clone());
    let dog_tag_id = consent.get("dogTagId").and_then(|v| v.as_str()).unwrap_or("0").to_string();
    let nonce = consent.get("nonce").cloned().unwrap_or(json!("0"));

    // ConsentReceipt (off-chain, deletable — erasure scope).
    let receipt_hash = keccak256_hex(&format!(
        "{}|{}|{}|{}|{}",
        owner_id, dog_tag_id, claims.purpose, consent_relayer, now
    ));
    let receipt_body = json!({
        "ownerId": owner_id, "dogTagId": dog_tag_id, "purpose": claims.purpose,
        "relayer": consent_relayer, "mode": mode, "nonce": nonce, "hash": receipt_hash, "issuedAt": now,
    });
    let receipt_sealed = match crypto::seal_json(st.vault.as_ref(), &receipt_body).await {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "seal failed"),
    };
    let receipt_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_consent_receipt(ConsentReceipt {
            receipt_id: receipt_id.clone(),
            owner_id: owner_id.to_string(),
            hash: receipt_hash.clone(),
            issued_at: now,
            sealed: receipt_sealed,
        })
        .await;

    // verification_records row (the relayed consent copy) — DELETABLE under erasure.
    let vr_sealed = match crypto::seal_json(
        st.vault.as_ref(),
        &json!({ "consent": consent, "sig": sig, "receipt": receipt_body }),
    )
    .await
    {
        Ok(s) => s,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "seal failed"),
    };
    let record_id = uuid::Uuid::new_v4().to_string();
    st.store
        .put_verification_record(VerificationRecord {
            record_id: record_id.clone(),
            owner_id: owner_id.to_string(),
            dog_tag_id,
            purpose: claims.purpose.clone(),
            relayer: consent_relayer.to_string(),
            mode: mode.clone(),
            status: "relayed".to_string(),
            sealed: vr_sealed,
        })
        .await;

    // relay to the verifier's /verify/consent/submit (resolve verifierApiBase from discovery).
    let verifier_api_base = match resolve_verifier_api_base(st, &claims).await {
        Some(b) => b,
        None => return err(StatusCode::BAD_GATEWAY, "cannot resolve verifierApiBase"),
    };
    let submit_body = json!({ "sessionId": claims.sub, "consent": consent, "sig": sig, "mode": mode });
    match st.business.relay_consent(&verifier_api_base, &submit_body).await {
        Ok(_) => ok(json!({
            "relayed": true,
            "recordId": record_id,
            "receipt": { "receiptId": receipt_id, "hash": receipt_hash, "issuedAt": now },
        })),
        Err(e) => err(StatusCode::BAD_GATEWAY, &format!("relay: {e}")),
    }
}
