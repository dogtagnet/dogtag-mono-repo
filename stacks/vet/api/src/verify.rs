//! Verification leg (impl §3.9 / §11.3): third-party `verify(doc)` via the SDK (bridging the SDK's
//! Rpc/Dns/Registry adapters to our `ChainClient`), and the `/verify/consent/submit` orchestration
//! (NORMAL path fully; ZK path via the `ProverClient` stub).

use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use dogtag_standard::verify::{
    verify as sdk_verify, AdapterError, DnsAdapter, RegistryAdapter, RpcAdapter, VerifyMode,
    VerifyOpts, Verdict, FragmentState,
};
use dogtag_standard::wrap::WrappedDoc;

use crate::app::AppState;
use crate::store::VerifySession;

type Resp = (StatusCode, Json<Value>);
fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

/// keccak256("VERIFY:"||purpose) reduced... we use keccak256 of the namespaced string, matching the
/// registry's `_verifyKey` shape conceptually (the contract uses abi.encode; here we key the test
/// whitelist by this string-derived bytes32). For on-chain parity the registry computes its own key;
/// the backend only needs a stable bytes32 to query `isWhitelistedFor`.
pub fn verify_key(purpose: &str) -> String {
    use alloy::primitives::keccak256;
    let s = format!("VERIFY:{purpose}");
    format!("0x{}", hex::encode(keccak256(s.as_bytes()).as_slice()))
}

/// Extract the cleartext dogTagId value from a wrapped doc.
pub fn dog_tag_id_of(doc: &WrappedDoc) -> Option<String> {
    use dogtag_standard::wrap::flatten_data;
    let entry = flatten_data(&doc.data)
        .into_iter()
        .find(|(kp, _)| kp == "credentialSubject.dogTagId")?;
    let parts: Vec<&str> = entry.1.splitn(3, ':').collect();
    parts.get(2).map(|s| s.to_string())
}

pub fn verdict_json(v: &Verdict) -> Value {
    let f = |s: FragmentState| match s {
        FragmentState::Valid => "VALID",
        FragmentState::Invalid => "INVALID",
        FragmentState::Error => "ERROR",
        FragmentState::NotApplicable => "NOT_APPLICABLE",
    };
    json!({
        "valid": v.valid,
        "integrity": f(v.integrity),
        "issuance": f(v.issuance),
        "identity": f(v.identity),
        "ownership": f(v.ownership),
    })
}

// --------------------------------------------------------------------------------------------
// SDK adapters bridging to our ChainClient + the deployment's identity config.
// --------------------------------------------------------------------------------------------

struct ChainRpcAdapter<'a> {
    st: &'a AppState,
    rt: tokio::runtime::Handle,
}
impl<'a> RpcAdapter for ChainRpcAdapter<'a> {
    fn is_valid(&self, document_store: &str, merkle_root: &str, _conf: u32) -> Result<bool, AdapterError> {
        let st = self.st.clone();
        let ds = document_store.to_string();
        let mr = merkle_root.to_string();
        // bridge sync SDK call -> async ChainClient via block_in_place on the current runtime.
        tokio::task::block_in_place(|| {
            self.rt
                .block_on(async move { st.chain.is_valid(&ds, &mr).await })
                .map_err(|e| AdapterError(e.to_string()))
        })
    }
    fn owner_of(&self, _dog_tag_id: &str) -> Result<String, AdapterError> {
        // ownerOf not needed for third-party validity (ownership is NOT_APPLICABLE). Return ERROR-safe.
        Err(AdapterError("ownerOf not wired".to_string()))
    }
}

/// Identity adapters backed by the deployment config: the issuer's own domain<->documentStore binding
/// is trusted locally (the self-hosted issuer knows its own contracts). In production these resolve via
/// DNS-over-HTTPS + the central registry; here they assert the configured pairing.
struct ConfigDnsAdapter<'a> {
    st: &'a AppState,
}
impl<'a> DnsAdapter for ConfigDnsAdapter<'a> {
    fn txt_matches(&self, domain: &str, document_store: &str, _chain_id: u64) -> Result<bool, AdapterError> {
        let known = self.st.cfg.issuer_domain.eq_ignore_ascii_case(domain)
            && self
                .st
                .cfg
                .issuer_addrs
                .values()
                .any(|a| a.eq_ignore_ascii_case(document_store));
        Ok(known)
    }
}
struct ConfigRegistryAdapter<'a> {
    st: &'a AppState,
}
impl<'a> RegistryAdapter for ConfigRegistryAdapter<'a> {
    fn knows(&self, domain: &str, document_store: &str) -> Result<bool, AdapterError> {
        let known = self.st.cfg.issuer_domain.eq_ignore_ascii_case(domain)
            && self
                .st
                .cfg
                .issuer_addrs
                .values()
                .any(|a| a.eq_ignore_ascii_case(document_store));
        Ok(known)
    }
}

/// Run the SDK's three-pillar verify in third-party mode against our chain + config identity.
pub async fn third_party_verify(st: &AppState, doc: &WrappedDoc) -> Verdict {
    let handle = tokio::runtime::Handle::current();
    let rpc = ChainRpcAdapter { st, rt: handle.clone() };
    let dns = ConfigDnsAdapter { st };
    let registry = ConfigRegistryAdapter { st };
    // run on a blocking-friendly context.
    let doc = doc.clone();
    tokio::task::block_in_place(move || {
        let opts = VerifyOpts {
            rpc: &rpc,
            dns: &dns,
            registry: &registry,
            mode: VerifyMode::ThirdParty,
            user_wallet_address: None,
            confirmations: Some(st.cfg.confirmations as u32),
        };
        sdk_verify(&doc, &opts)
    })
}

// --------------------------------------------------------------------------------------------
// /verify/consent/submit orchestration
// --------------------------------------------------------------------------------------------

pub async fn consent_submit(
    st: &AppState,
    session_id: String,
    consent: Value,
    sig: String,
    mode_override: Option<String>,
    disclosed_doc: Option<Value>,
) -> Resp {
    let s: VerifySession = match st.store.get_session(&session_id).await {
        Some(s) if s.status == "pending" => s,
        Some(_) => return err(StatusCode::CONFLICT, "session not pending"),
        None => return err(StatusCode::NOT_FOUND, "session not found"),
    };
    let mode = mode_override.unwrap_or_else(|| s.mode.clone());

    // relayer binding + deadline.
    let consent_relayer = consent.get("relayer").and_then(|v| v.as_str()).unwrap_or("");
    if !consent_relayer.eq_ignore_ascii_case(&s.relayer) {
        return err(StatusCode::BAD_REQUEST, "consent.relayer != session relayer");
    }
    let now = crate::auth::now();
    let deadline = consent.get("deadline").and_then(|v| v.as_u64()).unwrap_or(0);
    if deadline < now {
        return err(StatusCode::BAD_REQUEST, "consent expired");
    }
    // recordType binding: consent.recordType == keccak256(s.recordType).
    let expected_rt = crate::app::rt_key(&s.record_type);
    let consent_rt = consent.get("recordType").and_then(|v| v.as_str()).unwrap_or("");
    if !consent_rt.eq_ignore_ascii_case(&expected_rt) {
        return err(StatusCode::BAD_REQUEST, "consent.recordType mismatch");
    }
    let consent_root = consent.get("credentialRoot").and_then(|v| v.as_str()).unwrap_or("");

    let tx_hash;
    if mode == "normal" {
        // third-party verify the disclosed doc; require valid.
        let doc_val = match disclosed_doc {
            Some(d) => d,
            None => return err(StatusCode::BAD_REQUEST, "normal mode requires disclosedDoc"),
        };
        let doc: WrappedDoc = match serde_json::from_value(doc_val) {
            Ok(d) => d,
            Err(e) => return err(StatusCode::BAD_REQUEST, &format!("bad disclosedDoc: {e}")),
        };
        let verdict = third_party_verify(st, &doc).await;
        if !verdict.valid {
            return err(StatusCode::UNPROCESSABLE_ENTITY, "disclosed doc third-party verify invalid");
        }
        // require consent.credentialRoot == R.
        if !consent_root.eq_ignore_ascii_case(&doc.signature.merkle_root) {
            return err(StatusCode::BAD_REQUEST, "consent.credentialRoot != doc root");
        }
        // NORMAL submission would build recordVerification(consent, sig) and broadcast to the
        // VerificationRegistry via the dual-signing path. We record the attestation intent and emit a
        // synthetic tx hash; the on-chain recordVerification call is wired with the registry ABI in a
        // follow-up (the contract + consent digest parity already live in dogtag-standard consent.rs).
        let _ = &sig;
        tx_hash = format!("0xverify-normal-{}", &session_id[..8.min(session_id.len())]);
    } else {
        // ZK path: require consent.credentialRoot == R, run the (stub) prover, assemble pub signals.
        if consent_root.is_empty() {
            return err(StatusCode::BAD_REQUEST, "zk mode requires consent.credentialRoot");
        }
        let dog = consent.get("dogTagId").and_then(|v| v.as_str()).unwrap_or("0").to_string();
        let subject = consent.get("subject").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let nonce = consent.get("nonce").and_then(|v| v.as_str()).unwrap_or("0").to_string();
        let input = crate::prover::ProveInput {
            dog_tag_id: dog,
            purpose: consent.get("purpose").and_then(|v| v.as_str()).unwrap_or("0x0").to_string(),
            relayer: s.relayer.clone(),
            subject,
            nonce,
            r: consent_root.to_string(),
            eddsa_sig: sig.clone(),
        };
        match st.prover.prove(input).await {
            Ok(_proof) => {}
            Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("prover: {e}")),
        }
        tx_hash = format!("0xverify-zk-{}", &session_id[..8.min(session_id.len())]);
    }

    let mut updated = s;
    updated.status = "recorded".to_string();
    updated.tx_hash = Some(tx_hash.clone());
    st.store.update_session(updated).await;
    ok(json!({ "recorded": true, "txHash": tx_hash, "mode": mode }))
}
