//! Credential integrity checks and owner-hidden consent-proof submission.

use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use dogtag_standard::verify::{
    verify as sdk_verify, AdapterError, DnsAdapter, FragmentState, IssuerAnchor, IssuerResolution,
    IssuerStoreAgreement, IssuerWhitelistState, RegistryAdapter, RpcAdapter, Verdict, VerifyMode,
    VerifyOpts,
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
        // The factory-anchored issuer-whitelist pillar, reported EXPLICITLY. A caller must be able
        // to tell "not evaluated" from "evaluated and passed" — a pillar that never ran must never
        // read as one that succeeded, and a silent absence would be exactly that. Same vocabulary as
        // `POST /verify/credential` (PR #96) so the two surfaces read identically.
        //   "passed" | "failed" | "unresolved" | "unavailableNoFactoryConfigured"
        "issuerWhitelistState": whitelist_state_str(v.issuer_whitelist),
        // Whether the envelope's own `documentStore` names the clone the factory resolved. Reported
        // BESIDE the pillar, never folded into it: "the document named a different contract than the
        // chain did" and "the signer is not authorised" are different accusations with different
        // remedies, exactly as `POST /verify/credential` keeps `issuer_mismatch` apart from
        // `issuer_not_whitelisted`.
        //   "matched" | "differs" | "notEvaluated"
        "issuerStoreAgreement": store_agreement_str(v.issuer_store),
        //   "resolved" | "noRecord" | "noFactoryConfigured" | "readFailed"
        "issuerResolution": resolution_str(&v.issuer_resolution),
        // The clone the on-chain reads were ACTUALLY made against. With `issuerResolution` other
        // than "resolved" this is the document's own unverified claim.
        "issuerAddr": v.issuer_addr,
    })
}

fn whitelist_state_str(s: IssuerWhitelistState) -> &'static str {
    match s {
        IssuerWhitelistState::Passed => "passed",
        IssuerWhitelistState::Failed => "failed",
        IssuerWhitelistState::Unresolved => "unresolved",
        IssuerWhitelistState::UnavailableNoFactoryConfigured => "unavailableNoFactoryConfigured",
    }
}

fn store_agreement_str(s: IssuerStoreAgreement) -> &'static str {
    match s {
        IssuerStoreAgreement::Matched => "matched",
        IssuerStoreAgreement::Differs => "differs",
        IssuerStoreAgreement::NotEvaluated => "notEvaluated",
    }
}

fn resolution_str(r: &IssuerResolution) -> &'static str {
    match r {
        IssuerResolution::Resolved => "resolved",
        IssuerResolution::NoRecord => "noRecord",
        IssuerResolution::NoFactoryConfigured => "noFactoryConfigured",
        IssuerResolution::ReadFailed => "readFailed",
    }
}

/// How this deployment's `FACTORY_ADDR` classifies — the SINGLE source both factory-anchored
/// surfaces read, so `POST /import/pull` (via the SDK adapter below) and `POST /verify/credential`
/// cannot drift on what counts as "no factory" versus "a broken one".
///
/// They deliberately REACT differently — the handler answers HTTP 500, the SDK adapter returns an
/// `Err` that surfaces as an indeterminate pillar — because the SDK returns a `Verdict` and has no
/// 500/502 channel. What must never differ is the CLASSIFICATION, which is what lives here.
pub(crate) enum FactoryConfig {
    /// Unset, or the zero address: a deployment that deliberately has no factory. The pillar reports
    /// itself unavailable and does NOT condemn the credential.
    Absent,
    /// Set, but not a well-formed contract address. Different in kind from absent: this deployment
    /// INTENDED to check, and one fat-fingered character silently disabling a security pillar is
    /// misconfigure-to-bypass reached by accident. It fails loudly on both surfaces.
    Malformed,
    /// A usable factory address.
    Addr(String),
}

/// The one wording both surfaces report a [`FactoryConfig::Malformed`] with.
pub(crate) const FACTORY_ADDR_MALFORMED: &str =
    "FACTORY_ADDR is malformed: the issuer pillar cannot be evaluated, and this deployment is \
     configured to evaluate it. Fix the address or unset it deliberately.";

pub(crate) fn factory_config(factory_addr: &str) -> FactoryConfig {
    let factory = factory_addr.trim();
    let absent = factory.is_empty()
        || (factory.len() == 42
            && factory.starts_with("0x")
            && factory[2..].bytes().all(|b| b == b'0'));
    if absent {
        FactoryConfig::Absent
    } else if !valid_contract_addr(factory) {
        FactoryConfig::Malformed
    } else {
        FactoryConfig::Addr(factory.to_string())
    }
}

struct ChainRpcAdapter<'a> {
    st: &'a AppState,
    rt: tokio::runtime::Handle,
}

impl ChainRpcAdapter<'_> {
    /// Run one async chain read from inside the SDK's synchronous adapter contract.
    fn blocking<T, F>(&self, f: F) -> Result<T, AdapterError>
    where
        F: std::future::Future<Output = Result<T, crate::chain::ChainError>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::task::block_in_place(|| self.rt.block_on(f).map_err(|e| AdapterError(e.to_string())))
    }
}

impl RpcAdapter for ChainRpcAdapter<'_> {
    fn is_valid(
        &self,
        issuer_addr: &str,
        merkle_root: &str,
        _conf: u32,
    ) -> Result<bool, AdapterError> {
        let st = self.st.clone();
        let (issuer_addr, merkle_root) = (issuer_addr.to_string(), merkle_root.to_string());
        self.blocking(async move { st.chain.is_valid(&issuer_addr, &merkle_root).await })
    }

    fn owner_of(&self, _dog_tag_id: &str) -> Result<String, AdapterError> {
        Err(AdapterError(
            "ownerOf not used for third-party verification".to_string(),
        ))
    }

    /// The factory address comes from THIS DEPLOYMENT'S config and from nowhere else — never from
    /// the document, never from the caller. That is the whole point: an address an attacker can name
    /// is an address that cannot anchor anything.
    ///
    /// Absent (unset, or the zero address) is a deployment that deliberately has no factory: the
    /// pillar reports itself unavailable and does not condemn the credential. MALFORMED is different
    /// in kind — that deployment INTENDED to check, and one fat-fingered character silently
    /// disabling a security pillar is misconfigure-to-bypass reached by accident. It is therefore an
    /// `Err`, which the SDK reports as `ReadFailed` and which GATES.
    ///
    /// **Divergence from `POST /verify/credential` (PR #96), stated rather than implied.** That
    /// handler answers a malformed `FACTORY_ADDR` with HTTP 500 and no verdict. The SDK has no 502/500
    /// channel — it returns a `Verdict` — so the same posture arrives as an indeterminate pillar that
    /// refuses the credential, and `POST /import/pull` turns that into its 422. Neither degrades into
    /// the fail-open unavailable state, which is the property that matters.
    fn root_issuer(&self, merkle_root: &str) -> Result<IssuerAnchor, AdapterError> {
        let factory = match factory_config(&self.st.cfg.factory_addr) {
            FactoryConfig::Absent => return Ok(IssuerAnchor::NoFactoryConfigured),
            FactoryConfig::Malformed => {
                return Err(AdapterError(FACTORY_ADDR_MALFORMED.to_string()))
            }
            FactoryConfig::Addr(a) => a,
        };
        let st = self.st.clone();
        let merkle_root = merkle_root.to_string();
        let resolved =
            self.blocking(async move { st.chain.root_issuer(&factory, &merkle_root).await })?;
        Ok(match resolved {
            Some(clone) => IssuerAnchor::Resolved(clone),
            // We ASKED and the chain has no record of this root. Evidence about the credential.
            None => IssuerAnchor::NoRecord,
        })
    }

    fn issued_by(
        &self,
        issuer_addr: &str,
        merkle_root: &str,
    ) -> Result<Option<String>, AdapterError> {
        let st = self.st.clone();
        let (issuer_addr, merkle_root) = (issuer_addr.to_string(), merkle_root.to_string());
        self.blocking(async move { st.chain.issued_by(&issuer_addr, &merkle_root).await })
    }

    fn issuer_record_type(&self, issuer_addr: &str) -> Result<Option<String>, AdapterError> {
        let st = self.st.clone();
        let issuer_addr = issuer_addr.to_string();
        self.blocking(async move { st.chain.issuer_record_type(&issuer_addr).await })
    }

    /// The registry address is this deployment's own, for the same reason the factory's is.
    fn is_whitelisted_for(
        &self,
        record_type_key: &str,
        signer: &str,
    ) -> Result<bool, AdapterError> {
        let st = self.st.clone();
        let registry = st.cfg.issuer_registry_addr.clone();
        let (record_type_key, signer) = (record_type_key.to_string(), signer.to_string());
        self.blocking(async move {
            st.chain
                .is_whitelisted_for(&registry, &record_type_key, &signer)
                .await
        })
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
        let r = bv[i].as_array().ok_or_else(|| format!("b[{i}]: !array"))?;
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

    // D1: an OPTIONAL, holder-initiated `ProfileDisclosure` rides alongside the consent proof.
    // The consent proof stays frozen and leaf-blind; the disclosure is a cryptographically
    // independent Merkle opening of owner-picked `owner.identity.*` leaves against the SAME `R`.
    // Binding the envelope to THIS proof's `R` and `dogTagId` is the anti-replay context (q4
    // §1.3.4): a bare envelope is a replayable bearer credential, but bound here it inherits the
    // proof's relayer/deadline/nullifier scope. All checks run BEFORE any gas is spent; a bad
    // disclosure rejects the whole submission. Nothing disclosed ever reaches the chain.
    let disclosed_key_paths =
        match verify_profile_disclosure_submission(st, body, &pubs, &pub_u).await {
            Ok(kps) => kps,
            Err(e) => return e,
        };

    let nullifier = format!("0x{}", hex::encode(pub_u[P::NULLIFIER].to_be_bytes::<32>()));
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
            session.disclosed_key_paths = disclosed_key_paths.clone();
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
            disclosed_key_paths: disclosed_key_paths.clone(),
            // A cold submit carries no operator-started session, so there is no appointment to
            // link it to; it lands in the history as an ad-hoc verification.
            appointment_id: None,
            client_id: None,
            pet_id: None,
        },
    };
    let session_id = audit.session_id.clone();
    st.store.put_session(audit.clone()).await;
    // Business-history evidence, recorded against the shop's own row: the OPAQUE on-chain dogTagId
    // this consent is bound to (`pub[0]`, already a public verification fact) plus the keyPaths the
    // owner CHOSE to reveal. The owner's `subject` wallet is deliberately never stored, and the
    // disclosed list is EMPTY on an ordinary owner-hidden verification — that emptiness IS the
    // privacy guarantee, never something to backfill from another source.
    let evidence_dog_tag_id = format!(
        "0x{}",
        hex::encode(pub_u[P::DOG_TAG_ID].to_be_bytes::<32>())
    );
    crate::crm::attach_evidence(
        &st.store,
        &audit,
        Some(evidence_dog_tag_id),
        disclosed_key_paths.clone(),
    )
    .await;

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
        store.update_session(background_session.clone()).await;
        // Settle the shop's history row at the SAME terminal point the session settles — both arms,
        // so the two can never disagree. A failed write here can never fail the verification.
        let status = background_session.status.clone();
        let tx_hash = background_session.tx_hash.clone();
        let nullifier = background_session.nullifier.clone();
        crate::crm::finish_log(&store, &background_session, &status, tx_hash, nullifier).await;
    });

    ok(json!({
        "status": "recording",
        "protocolVersion": dogtag_standard::wrap::LEVEL_B_VERSION,
        "sessionId": session_id,
        "registry": st.cfg.verification_registry_consent_addr,
        "nullifier": nullifier,
        "disclosedKeyPaths": disclosed_key_paths,
    }))
}

/// D1: validate the optional `profileDisclosure` block of an owner-hidden verify submission and
/// return the revealed keyPaths (empty when absent).
///
/// Four checks, all preflight (nothing here reaches the chain as a write):
///  1. PURE: every entry's leaf, recomputed from `(keyPath, salt, tag, value)` under `DS_LEAF`,
///     folds through its proof to the envelope's `R` (`disclosure::verify_profile_disclosure`).
///  2. BINDING: the envelope's `R` and `dogTagId` equal THIS consent proof's `pub[R]`/`pub[dogTagId]`
///     — the disclosure inherits the proof's relayer/deadline/nullifier anti-replay context.
///  3. ANCHOR: `R == profileRoot(dogTagId)` on the owner-hidden SBT.
///  4. VALIDITY: `isValid(R)` on the DOG_PROFILE issuer clone (anchored + not revoked).
///
/// Checks 3-4 are defense-in-depth preflight: the registry re-runs both on-chain when the consent
/// proof records (`VerificationRegistryConsent.sol:163,188-192`), and the binding in (2) is what
/// extends that enforcement to the disclosure. They are therefore gated on the addresses being
/// configured — a verify-only deployment without the issuance addresses still refuses any
/// disclosure whose (1)/(2) fail, and the chain remains the authority.
async fn verify_profile_disclosure_submission(
    st: &AppState,
    body: &Value,
    pubs: &[String; 7],
    pub_u: &[alloy::primitives::U256; dogtag_standard::public_signals::NUM_PUBLIC],
) -> Result<Vec<String>, Resp> {
    use dogtag_standard::public_signals::level_b as P;

    let raw = match body.get("profileDisclosure") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(raw) => raw,
    };
    let disclosure: dogtag_standard::disclosure::ProfileDisclosure =
        match serde_json::from_value(raw.clone()) {
            Ok(d) => d,
            Err(e) => {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    &format!("profileDisclosure: {e}"),
                ))
            }
        };

    // (1) every disclosed leaf folds to the envelope's R.
    match dogtag_standard::disclosure::verify_profile_disclosure(&disclosure) {
        Ok(true) => {}
        Ok(false) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                "profileDisclosure: a disclosed leaf does not fold to R",
            ))
        }
        Err(e) => {
            return Err(err(
                StatusCode::BAD_REQUEST,
                &format!("profileDisclosure: {e}"),
            ))
        }
    }

    // (2) the envelope is bound to THIS consent proof: same R, same dogTagId.
    if !pub_signal_eq(&disclosure.root, &pubs[P::ROOT]) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "profileDisclosure.R != the consent proof's R",
        ));
    }
    if !pub_signal_eq(&disclosure.dog_tag_id, &pubs[P::DOG_TAG_ID]) {
        return Err(err(
            StatusCode::BAD_REQUEST,
            "profileDisclosure.dogTagId != the consent proof's dogTagId",
        ));
    }

    // (3) on-chain anchor: R == profileRoot(dogTagId) on the owner-hidden SBT.
    let root_hex = format!("0x{}", hex::encode(pub_u[P::ROOT].to_be_bytes::<32>()));
    let onchain_id = format!(
        "0x{}",
        hex::encode(pub_u[P::DOG_TAG_ID].to_be_bytes::<32>())
    );
    if valid_contract_addr(&st.cfg.sbt_consent_addr) {
        match st
            .chain
            .profile_root_of(&st.cfg.sbt_consent_addr, &onchain_id)
            .await
        {
            Ok(sealed) if pub_signal_eq(&sealed, &root_hex) => {}
            Ok(_) | Err(crate::chain::ChainError::NotFound) => {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "profileDisclosure: R != profileRoot(dogTagId) on-chain",
                ))
            }
            // Fail CLOSED on an inconclusive read: a disclosure must never be recorded as revealed
            // on the strength of an anchor that could not be confirmed.
            Err(e) => {
                return Err(err(
                    StatusCode::BAD_GATEWAY,
                    &format!("profileDisclosure: profileRoot read failed: {e}"),
                ))
            }
        }
    }

    // (4) issuer validity: anchored and not revoked on the DOG_PROFILE clone.
    if valid_contract_addr(&st.cfg.profile_issuer_addr) {
        match st
            .chain
            .is_valid(&st.cfg.profile_issuer_addr, &root_hex)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                return Err(err(
                    StatusCode::BAD_REQUEST,
                    "profileDisclosure: R is not valid on the DOG_PROFILE issuer (unanchored or revoked)",
                ))
            }
            Err(e) => {
                return Err(err(
                    StatusCode::BAD_GATEWAY,
                    &format!("profileDisclosure: isValid read failed: {e}"),
                ))
            }
        }
    }

    Ok(disclosure
        .disclosures
        .iter()
        .map(|entry| entry.key_path.clone())
        .collect())
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

/// True for a well-formed, non-zero contract address (`0x` + exactly 40 hex digits, not all zero).
///
/// The zero check alone is NOT enough, and the gap is expensive. `chain::parse_addr` coerces an
/// unparseable address to `Address::ZERO` (`h.parse::<Address>().unwrap_or(Address::ZERO)`), so a
/// typo'd `SBT_CONSENT_ADDR` / `PROFILE_ISSUER_ADDR` would pass a non-zero-string test, consume the
/// one-time bind token, and dispatch both `issue(R)` and `mintCustodial` at the zero address — txs
/// that SUCCEED against a codeless address, so the failure only surfaces at the read-back, after gas
/// is spent and the operator's QR is burned. Shape-check at the edge.
///
/// This is the ONE definition; `routes.rs` used to carry a byte-identical copy, which is how two
/// surfaces required to hold the same posture become free to drift.
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
