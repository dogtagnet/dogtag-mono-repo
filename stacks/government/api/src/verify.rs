//! Owner-hidden consent-proof submission — the government acting as a VERIFIER.
//!
//! The authority verifies a credential through the SAME zero-knowledge consent path as any other
//! verifier (architecture §4.3/§4.7): the owner approves an owner-hidden Groth16 proof on their own
//! device and the authority merely relays it to the shared `VerificationRegistryConsent`. There is
//! deliberately no authority shortcut — being a government does not buy a weaker privacy model, and
//! nothing on this path learns the owner's identity or any credential leaf.
//!
//! This is a MIRROR of `stacks/vet/api/src/verify.rs::consent_submit_levelb` — the same preflight, in
//! the same order, with the same error strings, so the owner's phone gets identical responses from
//! either verifier and needs no government-specific branch. The one structural difference is the
//! relayer: the vet resolves it from its custody vault (`custody.active_address()`), the government
//! from its single configured signer (`GOV_SIGNER_KEY`, `chain.signer_address()`).
//!
//! The preflight MIRRORS the registry rather than replacing it: every check here is also enforced
//! on-chain by `VerificationRegistryConsent.recordVerificationZK`. Its purpose is to refuse before
//! spending gas — and, at session start, before the owner spends tens of seconds proving.

use axum::{http::StatusCode, Json};
use serde_json::{json, Value};

use crate::app::AppState;
use crate::store::VerifySession;

type Resp = (StatusCode, Json<Value>);

fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}

fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

const BN254_R_DEC: &str =
    "21888242871839275222246405745257275088548364400416034343698204186575808495617";

/// The registry rejects a deadline that has passed by the time the tx mines. Broadcast is detached and
/// retried, so refuse anything within this margin up front rather than burning gas on a proof that
/// cannot land.
const MIN_DEADLINE_MARGIN_SECS: u64 = 120;

/// Reduce a purpose/recordType label into the BN254 field element the consent proof carries
/// (`keccak256(label) mod r`). Mirror of vet-api's `verify::purpose_key` — the field boundary
/// reduction is what makes the registry store/nullify the same value the circuit proved.
pub fn purpose_key(label: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    let r = U256::from_str_radix(BN254_R_DEC, 10).expect("BN254 r parses");
    let full = U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0);
    format!("0x{}", hex::encode((full % r).to_be_bytes::<32>()))
}

/// `IssuerRegistry` key for a relayer authorized to VERIFY a given purpose label. The `VERIFY:`
/// namespace is deliberately distinct from the issuer record-type keys: this authority's power to
/// ISSUE `TRAVEL_CLEARANCE` grants it no power to verify anything.
pub fn verify_key(label: &str) -> String {
    verify_key_from_purpose_word(&purpose_key(label))
}

/// `keccak256(abi.encode("VERIFY:", purposeWord))` — the ABI encoding is hand-rolled to match the
/// registry byte-for-byte (offset word, the bytes32 purpose, then the 7-byte `"VERIFY:"` string with
/// its length prefix). Mirror of vet-api's `verify_key_from_purpose_word`.
pub fn verify_key_from_purpose_word(purpose_word: &str) -> String {
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

/// A configured, non-zero contract address. The zero address is the "unset" sentinel every stack's
/// config defaults to, so it must never be treated as a live registry.
pub fn valid_contract_addr(address: &str) -> bool {
    address.len() == 42
        && address.starts_with("0x")
        && address[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && address[2..].bytes().any(|b| b != b'0')
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

/// Compare two field elements by VALUE, so a decimal `pubSignals` entry and a `0x` config word agree.
fn pub_signal_eq(a: &str, b: &str) -> bool {
    matches!((pub_signal_u256(a), pub_signal_u256(b)), (Some(x), Some(y)) if x == y)
}

/// The owner-hidden verification registry this authority submits to. Government config carries ONE
/// registry address (`VERIFICATION_REGISTRY_ADDR`, aliased `VERIFICATION_REGISTRY_CONSENT_ADDR`) which
/// is the same unified consent registry the vet/groomer stacks name — it is both the routing key
/// stamped in the M7 `protocol` block and the verification submit target.
pub fn consent_registry(st: &AppState) -> String {
    st.cfg.verification_registry_addr.clone()
}

/// Preflight and submit the sole, owner-hidden consent proof shape.
///
/// A session-scoped call binds the proof to its purpose, relayer and record type; a cold operator call
/// (no session) mints its own audit row. Nothing in the proof, and nothing recorded on-chain, names the
/// owner: the government learns only that SOME owner consented to this purpose within the window.
pub async fn consent_submit_levelb(
    st: &AppState,
    body: &Value,
    session: Option<VerifySession>,
) -> Resp {
    use alloy::primitives::U256;
    use dogtag_standard::public_signals::level_b as P;

    // Replay guard for the session path: a recorded/errored session can never accept another proof.
    if let Some(s) = session.as_ref() {
        if s.status != "pending" {
            return err(StatusCode::CONFLICT, "session not pending");
        }
    }

    let registry = consent_registry(st);
    if !valid_contract_addr(&registry) {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "verification registry not configured",
        );
    }
    let relayer = match st.chain.signer_address() {
        Some(a) if !a.is_empty() => a,
        _ => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                "no government signer configured (set GOV_SIGNER_KEY) to record a verification",
            )
        }
    };

    let proof = match body.get("proof") {
        Some(proof) => proof,
        None => return err(StatusCode::BAD_REQUEST, "proof: missing"),
    };
    let (a, b, c, pubs) = match parse_client_proof(proof) {
        Ok(proof) => proof,
        Err(e) => return err(StatusCode::BAD_REQUEST, &format!("proof: {e}")),
    };

    // Every public signal must be a canonical field element (snarkjs #358 — an out-of-range signal is
    // a soundness hazard, not a formatting nit).
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

    // The proof must name THIS authority as the relayer — the registry additionally requires
    // `relayer == msg.sender`, so a proof bound to anyone else would revert.
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

    // Session binding: the proof must be for the purpose/relayer/recordType the operator started.
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

    let now = crate::routes::now();
    if pub_u[P::DEADLINE] <= U256::from(now.saturating_add(MIN_DEADLINE_MARGIN_SECS)) {
        return err(
            StatusCode::BAD_REQUEST,
            "pubSignals[deadline]: expired or too close to expiry — re-prove with a fresher deadline",
        );
    }

    // Art. 9 special-category data is never verifiable on-chain (architecture §7).
    let art9 = pub_signal_u256(&purpose_key("SERVICE_ATTESTATION"))
        .expect("SERVICE_ATTESTATION field parses");
    if pub_u[P::RECORD_TYPE] == art9 {
        return err(
            StatusCode::BAD_REQUEST,
            "pubSignals[recordType]: SERVICE_ATTESTATION is not verifiable on-chain (art9)",
        );
    }

    // The `VERIFY:` whitelist is checked UNCONDITIONALLY against the proof's own purpose word (not the
    // session's), so a cold submit cannot bypass it.
    let purpose_word = format!("0x{}", hex::encode(pub_u[P::PURPOSE].to_be_bytes::<32>()));
    let verify_key = verify_key_from_purpose_word(&purpose_word);
    match st
        .chain
        .is_whitelisted_for(&st.cfg.issuer_registry_addr, &verify_key, &relayer, None)
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

    // D1: an OPTIONAL, holder-initiated `ProfileDisclosure` rides alongside the consent proof. The
    // consent proof stays frozen and leaf-blind; the disclosure is a cryptographically independent
    // Merkle opening of owner-picked `owner.identity.*` leaves against the SAME `R`.
    let disclosed_key_paths = match verify_profile_disclosure_submission(body, &pubs) {
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
        },
    };
    let session_id = audit.session_id.clone();
    st.store.put_session(audit.clone()).await;

    // Broadcast is DETACHED: the phone polls the session (and the on-chain nullifier) rather than
    // holding an HTTP request open across a block time.
    let chain = st.chain.clone();
    let store = st.store.clone();
    let mut background_session = audit;
    tokio::spawn(async move {
        match chain
            .record_verification_zk_consent(&registry, &a, &b, &c, &pubs)
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
        background_session.updated_at = crate::routes::now();
        store.update_session(background_session).await;
    });

    ok(json!({
        "status": "recording",
        "protocolVersion": dogtag_standard::wrap::LEVEL_B_VERSION,
        "sessionId": session_id,
        "registry": consent_registry(st),
        "nullifier": nullifier,
        "disclosedKeyPaths": disclosed_key_paths,
    }))
}

/// D1: validate the optional `profileDisclosure` block and return the revealed keyPaths (empty when
/// absent). Two checks, both preflight and both PURE (no chain read):
///
///  1. PURE: every entry's leaf, recomputed from `(keyPath, salt, tag, value)` under `DS_LEAF`, folds
///     through its proof to the envelope's `R` (`disclosure::verify_profile_disclosure`).
///  2. BINDING: the envelope's `R` and `dogTagId` equal THIS consent proof's `pub[R]`/`pub[dogTagId]`
///     — which is what makes the disclosure inherit the proof's relayer/deadline/nullifier anti-replay
///     context. A bare envelope would otherwise be a replayable bearer credential.
///
/// vet-api additionally preflights `R == profileRoot(dogTagId)` on the owner-hidden SBT and
/// `isValid(R)` on the DOG_PROFILE clone — both gated there on those addresses being configured. This
/// authority is a VERIFY-ONLY deployment for `DOG_PROFILE` (it issues travel/health credentials, not
/// dog tags, and holds neither address), so it takes exactly the branch vet takes when they are unset.
/// Those two conditions are not lost: `VerificationRegistryConsent.recordVerificationZK` re-runs both
/// ON-CHAIN before recording, and check (2) is what extends that enforcement to the disclosure.
fn verify_profile_disclosure_submission(
    body: &Value,
    pubs: &[String; 7],
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

    Ok(disclosure
        .disclosures
        .iter()
        .map(|entry| entry.key_path.clone())
        .collect())
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

    /// The `VERIFY:` namespace keys MUST match the vet/groomer stacks bit-for-bit — they gate against
    /// the same `IssuerRegistry`. Both literals below are the values `cast` computes for the canonical
    /// definitions (`keccak256(label) mod r`, then `keccak256(abi.encode("VERIFY:", purposeWord))`), so
    /// any drift in the hand-rolled ABI encoding is caught in-crate rather than as a revert on ROAX.
    #[test]
    fn verify_key_matches_the_shared_registry_namespace() {
        assert_eq!(
            purpose_key("travel_check"),
            "0x26e532b6f5b66521fea3ea9a33d09f9b6cc6ca45a765074832784d636e02ba01"
        );
        assert_eq!(
            verify_key("travel_check"),
            "0xc466709709c12abd200c3a9467bfa0da0b3810ba705829d94926a195c0058dca"
        );
        // A different purpose yields a different key (no cross-purpose authority).
        assert_ne!(verify_key("travel_check"), verify_key("grooming_intake"));
    }
}
