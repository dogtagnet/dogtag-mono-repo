//! Credential integrity checks and owner-hidden consent-proof submission.

use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use dogtag_standard::verify::{
    verify as sdk_verify, AdapterError, DnsAdapter, FragmentState, RegistryAdapter, RpcAdapter,
    Verdict, VerifyMode, VerifyOpts,
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

/// Reduce a purpose label into the BN254 field representation carried by the consent proof.
pub fn purpose_key(label: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    let r = U256::from_str_radix(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
        10,
    )
    .unwrap();
    let full = U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0);
    format!("0x{}", hex::encode((full % r).to_be_bytes::<32>()))
}

/// IssuerRegistry key for a relayer authorized to verify a given purpose label.
pub fn verify_key(label: &str) -> String {
    verify_key_from_purpose_word(&purpose_key(label))
}

/// Extract the cleartext dogTagId value from a wrapped document.
pub fn dog_tag_id_of(doc: &WrappedDoc) -> Option<String> {
    use dogtag_standard::wrap::flatten_data;
    let entry = flatten_data(&doc.data)
        .into_iter()
        .find(|(kp, _)| kp == "credentialSubject.dogTagId")?;
    let parts: Vec<&str> = entry.1.splitn(3, ':').collect();
    parts.get(2).map(|s| s.to_string())
}

pub fn verdict_json(v: &Verdict) -> Value {
    let fragment = |s: FragmentState| match s {
        FragmentState::Valid => "VALID",
        FragmentState::Invalid => "INVALID",
        FragmentState::Error => "ERROR",
        FragmentState::NotApplicable => "NOT_APPLICABLE",
    };
    json!({
        "valid": v.valid,
        "integrity": fragment(v.integrity),
        "issuance": fragment(v.issuance),
        "identity": fragment(v.identity),
        "ownership": fragment(v.ownership),
    })
}

struct ChainRpcAdapter<'a> {
    st: &'a AppState,
    rt: tokio::runtime::Handle,
}

impl RpcAdapter for ChainRpcAdapter<'_> {
    fn is_valid(
        &self,
        document_store: &str,
        merkle_root: &str,
        _conf: u32,
    ) -> Result<bool, AdapterError> {
        let st = self.st.clone();
        let document_store = document_store.to_string();
        let merkle_root = merkle_root.to_string();
        tokio::task::block_in_place(|| {
            self.rt
                .block_on(async move { st.chain.is_valid(&document_store, &merkle_root).await })
                .map_err(|e| AdapterError(e.to_string()))
        })
    }

    fn owner_of(&self, _dog_tag_id: &str) -> Result<String, AdapterError> {
        Err(AdapterError("ownerOf not used for third-party verification".to_string()))
    }
}

struct ConfigDnsAdapter<'a> {
    st: &'a AppState,
}

impl DnsAdapter for ConfigDnsAdapter<'_> {
    fn txt_matches(
        &self,
        domain: &str,
        document_store: &str,
        _chain_id: u64,
    ) -> Result<bool, AdapterError> {
        Ok(self.st.cfg.issuer_domain.eq_ignore_ascii_case(domain)
            && self
                .st
                .cfg
                .issuer_addrs
                .values()
                .any(|a| a.eq_ignore_ascii_case(document_store)))
    }
}

struct ConfigRegistryAdapter<'a> {
    st: &'a AppState,
}

impl RegistryAdapter for ConfigRegistryAdapter<'_> {
    fn knows(&self, domain: &str, document_store: &str) -> Result<bool, AdapterError> {
        Ok(self.st.cfg.issuer_domain.eq_ignore_ascii_case(domain)
            && self
                .st
                .cfg
                .issuer_addrs
                .values()
                .any(|a| a.eq_ignore_ascii_case(document_store)))
    }
}

/// Run the SDK's three-pillar credential check in third-party mode.
pub async fn third_party_verify(st: &AppState, doc: &WrappedDoc) -> Verdict {
    let handle = tokio::runtime::Handle::current();
    let rpc = ChainRpcAdapter {
        st,
        rt: handle.clone(),
    };
    let dns = ConfigDnsAdapter { st };
    let registry = ConfigRegistryAdapter { st };
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

type ClientProof = ([String; 2], [[String; 2]; 2], [String; 2], [String; 7]);

/// Parse `{a:[2], b:[2][2], c:[2], pubSignals:[7]}` into the chain encoder's fixed arrays.
fn parse_client_proof(v: &Value) -> Result<ClientProof, String> {
    let one = |x: &Value, what: &str| -> Result<String, String> {
        match x {
            Value::String(s) => Ok(s.trim().to_string()),
            Value::Number(n) => Ok(n.to_string()),
            _ => Err(format!("{what}: not a string/number")),
        }
    };
    let arr2 = |key: &str| -> Result<[String; 2], String> {
        let a = v
            .get(key)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{key}: missing/!array"))?;
        if a.len() != 2 {
            return Err(format!("{key}: expected len 2"));
        }
        Ok([one(&a[0], key)?, one(&a[1], key)?])
    };
    let a = arr2("a")?;
    let c = arr2("c")?;
    let bv = v
        .get("b")
        .and_then(Value::as_array)
        .ok_or_else(|| "b: missing/!array".to_string())?;
    if bv.len() != 2 {
        return Err("b: expected len 2".to_string());
    }
    let row = |i: usize| -> Result<[String; 2], String> {
        let r = bv[i]
            .as_array()
            .ok_or_else(|| format!("b[{i}]: !array"))?;
        if r.len() != 2 {
            return Err(format!("b[{i}]: expected len 2"));
        }
        Ok([one(&r[0], "b")?, one(&r[1], "b")?])
    };
    let b = [row(0)?, row(1)?];
    let pv = v
        .get("pubSignals")
        .and_then(Value::as_array)
        .ok_or_else(|| "pubSignals: missing/!array".to_string())?;
    if pv.len() != dogtag_standard::public_signals::NUM_PUBLIC {
        return Err(format!(
            "pubSignals: expected len {}, got {}",
            dogtag_standard::public_signals::NUM_PUBLIC,
            pv.len()
        ));
    }
    let mut pub_signals: [String; 7] = Default::default();
    for (i, x) in pv.iter().enumerate() {
        pub_signals[i] = one(x, "pubSignals")?;
    }
    Ok((a, b, c, pub_signals))
}

fn pub_signal_u256(s: &str) -> Option<alloy::primitives::U256> {
    use alloy::primitives::U256;
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x") {
        U256::from_str_radix(hex, 16).ok()
    } else {
        U256::from_str_radix(s, 10).ok()
    }
}

fn pub_signal_to_address(s: &str) -> Option<String> {
    let bytes = pub_signal_u256(s)?.to_be_bytes::<32>();
    Some(format!("0x{}", hex::encode(&bytes[12..])))
}

fn pub_signal_eq(a: &str, b: &str) -> bool {
    matches!((pub_signal_u256(a), pub_signal_u256(b)), (Some(x), Some(y)) if x == y)
}

const BN254_R_DEC: &str =
    "21888242871839275222246405745257275088548364400416034343698204186575808495617";
const MIN_DEADLINE_MARGIN_SECS: u64 = 120;

/// Preflight and submit the sole, owner-hidden consent proof shape. A session-scoped call binds the
/// proof to its purpose, relayer, and record type; a cold operator call creates a new audit row.
pub async fn consent_submit_levelb(
    st: &AppState,
    body: &Value,
    session: Option<VerifySession>,
) -> Resp {
    use alloy::primitives::U256;
    use dogtag_standard::public_signals::level_b as P;

    if let Some(s) = session.as_ref() {
        if s.status != "pending" {
            return err(StatusCode::CONFLICT, "session not pending");
        }
    }

    let registry = st.cfg.verification_registry_consent_addr.clone();
    if !valid_contract_addr(&registry) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "verification registry not configured",
        );
    }
    if !st.custody.is_unlocked() {
        return err(StatusCode::CONFLICT, "not unlocked");
    }
    let relayer = match st.custody.active_address() {
        Ok(a) => a,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };

    let proof = match body.get("proof") {
        Some(proof) => proof,
        None => return err(StatusCode::BAD_REQUEST, "proof: missing"),
    };
    let (a, b, c, pubs) = match parse_client_proof(proof) {
        Ok(proof) => proof,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("proof: {e}")),
    };

    let r = U256::from_str_radix(BN254_R_DEC, 10).expect("BN254 r parses");
    let mut pub_u = [U256::ZERO; dogtag_standard::public_signals::NUM_PUBLIC];
    for (i, signal) in pubs.iter().enumerate() {
        match pub_signal_u256(signal) {
            Some(u) => pub_u[i] = u,
            None => {
                return err(
                    StatusCode::BAD_REQUEST,
                    &format!("pubSignals[{i}]: unparseable"),
                )
            }
        }
        if pub_u[i] >= r {
            return err(StatusCode::BAD_REQUEST, &format!("pubSignals[{i}]: !field"));
        }
    }

    if pub_u[P::RELAYER] >= U256::from(1u8) << 160 {
        return err(StatusCode::BAD_REQUEST, "pubSignals[relayer]: addr range");
    }
    let pub_relayer = match pub_signal_to_address(&pubs[P::RELAYER]) {
        Some(address) => address,
        None => return err(StatusCode::BAD_REQUEST, "pubSignals[relayer]: bad relayer"),
    };
    if !pub_relayer.eq_ignore_ascii_case(&relayer) {
        return err(
            StatusCode::FORBIDDEN,
            "pubSignals[relayer] does not name this relayer",
        );
    }

    if let Some(s) = session.as_ref() {
        if !pub_signal_eq(&pubs[P::PURPOSE], &purpose_key(&s.purpose)) {
            return err(
                StatusCode::BAD_REQUEST,
                "pubSignals.purpose != purpose_key(session.purpose)",
            );
        }
        if !pub_relayer.eq_ignore_ascii_case(&s.relayer) {
            return err(
                StatusCode::BAD_REQUEST,
                "pubSignals.relayer != session relayer",
            );
        }
        if !pub_signal_eq(&pubs[P::RECORD_TYPE], &purpose_key(&s.record_type)) {
            return err(
                StatusCode::BAD_REQUEST,
                "pubSignals.recordType != purpose_key(session.recordType)",
            );
        }
    }

    let now = crate::auth::now();
    if pub_u[P::DEADLINE] <= U256::from(now.saturating_add(MIN_DEADLINE_MARGIN_SECS)) {
        return err(
            StatusCode::BAD_REQUEST,
            "pubSignals[deadline]: expired or too close to expiry — re-prove with a fresher deadline",
        );
    }

    let art9 = pub_signal_u256(&purpose_key("SERVICE_ATTESTATION"))
        .expect("SERVICE_ATTESTATION field parses");
    if pub_u[P::RECORD_TYPE] == art9 {
        return err(
            StatusCode::BAD_REQUEST,
            "pubSignals[recordType]: SERVICE_ATTESTATION is not verifiable on-chain (art9)",
        );
    }

    let purpose_word = format!("0x{}", hex::encode(pub_u[P::PURPOSE].to_be_bytes::<32>()));
    let verify_key = verify_key_from_purpose_word(&purpose_word);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &verify_key, &relayer)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return err(
                StatusCode::FORBIDDEN,
                "relayer not whitelisted for this purpose",
            )
        }
        Err(e) => return err(StatusCode::BAD_GATEWAY, &format!("verify-wl: {e}")),
    }

    let nullifier = format!(
        "0x{}",
        hex::encode(pub_u[P::NULLIFIER].to_be_bytes::<32>())
    );
    let record_type_word = format!(
        "0x{}",
        hex::encode(pub_u[P::RECORD_TYPE].to_be_bytes::<32>())
    );
    let audit = match session {
        Some(mut session) => {
            session.status = "recording".to_string();
            session.nullifier = Some(nullifier.clone());
            session.tx_hash = None;
            session.updated_at = now;
            session
        }
        None => VerifySession {
            session_id: uuid::Uuid::new_v4().to_string(),
            relayer: relayer.clone(),
            purpose: purpose_word,
            record_type: record_type_word,
            challenge: String::new(),
            status: "recording".to_string(),
            tx_hash: None,
            nullifier: Some(nullifier.clone()),
            created_at: now,
            updated_at: now,
        },
    };
    let session_id = audit.session_id.clone();
    st.store.put_session(audit.clone()).await;

    let chain = st.chain.clone();
    let store = st.store.clone();
    let mut background_session = audit;
    tokio::spawn(async move {
        match chain
            .record_verification_zk_consent(
                crate::custody::ACTIVE_SIGNER_INDEX,
                &registry,
                &a,
                &b,
                &c,
                &pubs,
            )
            .await
        {
            Ok(sent) => {
                background_session.status = "recorded".to_string();
                background_session.tx_hash = Some(sent.tx_hash);
            }
            Err(e) => {
                background_session.status = "error".to_string();
                background_session.tx_hash = Some(format!("recordVerificationZK: {e}"));
            }
        }
        background_session.updated_at = crate::auth::now();
        store.update_session(background_session).await;
    });

    ok(json!({
        "status": "recording",
        "protocolVersion": dogtag_standard::wrap::LEVEL_B_VERSION,
        "sessionId": session_id,
        "registry": st.cfg.verification_registry_consent_addr,
        "nullifier": nullifier,
    }))
}

fn verify_key_from_purpose_word(purpose_word: &str) -> String {
    use alloy::primitives::keccak256;
    let purpose = hex::decode(purpose_word.trim_start_matches("0x")).unwrap_or_default();
    let mut buf = Vec::with_capacity(160);
    let mut offset = [0u8; 32];
    offset[31] = 0x40;
    buf.extend_from_slice(&offset);
    let mut word = [0u8; 32];
    word[32 - purpose.len().min(32)..].copy_from_slice(&purpose[..purpose.len().min(32)]);
    buf.extend_from_slice(&word);
    let mut len = [0u8; 32];
    len[31] = 7;
    buf.extend_from_slice(&len);
    let mut label = [0u8; 32];
    label[..7].copy_from_slice(b"VERIFY:");
    buf.extend_from_slice(&label);
    format!("0x{}", hex::encode(keccak256(&buf).0))
}

pub(crate) fn valid_contract_addr(address: &str) -> bool {
    address.len() == 42
        && address.starts_with("0x")
        && address[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && address[2..].bytes().any(|b| b != b'0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_values_compare_across_radices() {
        assert!(pub_signal_eq("255", "0xff"));
        assert!(!pub_signal_eq("255", "256"));
        assert!(!pub_signal_eq("1", "garbage"));
    }

    #[test]
    fn contract_address_validation_rejects_zero_and_bad_shapes() {
        assert!(valid_contract_addr(
            "0x0000000000000000000000000000000000000001"
        ));
        assert!(!valid_contract_addr(
            "0x0000000000000000000000000000000000000000"
        ));
        assert!(!valid_contract_addr("not-an-address"));
    }
}
