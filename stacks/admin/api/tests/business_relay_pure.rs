//! Behavior-preserving unit coverage for the previously-untested
//! `MockBusinessClient` call-recording contract (the hermetic test double the
//! central tests depend on) and the `VerifyClaims` JSON shape consumed by the
//! verification-consent relay (impl §4.1). No source logic is exercised that
//! shipping code does not already rely on; these pin the public contracts so a
//! silent drift in the mock or the renamed serde fields breaks the build.

use admin_api::business::{BusinessClient, BusinessError, MockBusinessClient};
use admin_api::verify_relay::VerifyClaims;
use serde_json::json;

// --------------------------------------------------------------------------------------------
// MockBusinessClient — the call recorder used by central.rs / whitelist.rs hermetic tests.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn mock_ok_put_appointment_succeeds_and_records_call() {
    let mock = MockBusinessClient::new(true);
    let body = json!({ "start": "2026-01-01T10:00:00Z" });
    let res = mock
        .put_appointment("https://biz.example", "secret", "appt-1", "idem-1", &body)
        .await;
    assert!(res.is_ok());

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "PUT");
    // The mock does NOT trim a trailing slash off api_base_url (the real client does);
    // pin the mock's verbatim formatting so its URL stays a faithful observability anchor.
    assert_eq!(calls[0].url, "https://biz.example/v1/appointments/appt-1");
    assert_eq!(calls[0].body, body);
}

#[tokio::test]
async fn mock_ok_relay_consent_returns_recorded_true_and_records_post() {
    let mock = MockBusinessClient::new(true);
    let body = json!({ "sessionId": "s1", "consent": { "relayer": "0xabc" } });
    let res = mock
        .relay_consent("https://verifier.example", &body)
        .await
        .expect("ok=true must succeed");
    assert_eq!(res, json!({ "recorded": true }));

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "POST");
    assert_eq!(
        calls[0].url,
        "https://verifier.example/verify/consent/submit"
    );
    assert_eq!(calls[0].body, body);
}

#[tokio::test]
async fn mock_not_ok_errors_with_status_502_but_still_records() {
    let mock = MockBusinessClient::new(false);
    let body = json!({});

    let put = mock
        .put_appointment("https://biz.example", "secret", "appt-2", "idem-2", &body)
        .await;
    assert!(matches!(put, Err(BusinessError::Status(502))));

    let relay = mock.relay_consent("https://verifier.example", &body).await;
    assert!(matches!(relay, Err(BusinessError::Status(502))));

    // Both calls are recorded even on the failure path (in order), so observability
    // sees the attempt regardless of the programmed verdict.
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].method, "PUT");
    assert_eq!(calls[1].method, "POST");
}

#[tokio::test]
async fn mock_accumulates_calls_in_order_across_invocations() {
    let mock = MockBusinessClient::new(true);
    let b = json!({ "n": 1 });
    mock.put_appointment("https://a", "s", "x", "k", &b)
        .await
        .unwrap();
    mock.relay_consent("https://b", &b).await.unwrap();
    mock.put_appointment("https://c", "s", "y", "k", &b)
        .await
        .unwrap();

    let calls = mock.calls();
    let methods: Vec<&str> = calls.iter().map(|c| c.method.as_str()).collect();
    assert_eq!(methods, vec!["PUT", "POST", "PUT"]);
}

// --------------------------------------------------------------------------------------------
// VerifyClaims — the verifier session-JWT claim shape (renamed serde fields, optional default).
// --------------------------------------------------------------------------------------------

#[test]
fn verify_claims_deserializes_renamed_fields() {
    let v = json!({
        "iss": "vet-1",
        "sub": "session-9",
        "aud": "dogtag-mobile",
        "relayer": "0xRELAYER",
        "purpose": "boarding_intake",
        "recordType": "VACCINATION",
        "challenge": "ch",
        "mode": "zk",
        "exp": 1_700_000_000u64,
        "jti": "j1",
        "verifierApiBase": "https://verifier.example"
    });
    let claims: VerifyClaims = serde_json::from_value(v).expect("valid claims");
    // recordType / verifierApiBase rename mappings.
    assert_eq!(claims.record_type, "VACCINATION");
    assert_eq!(
        claims.verifier_api_base.as_deref(),
        Some("https://verifier.example")
    );
    assert_eq!(claims.aud, "dogtag-mobile");
    assert_eq!(claims.exp, 1_700_000_000);
}

#[test]
fn verify_claims_verifier_api_base_defaults_to_none_when_absent() {
    let v = json!({
        "iss": "vet-1",
        "sub": "session-9",
        "aud": "dogtag-mobile",
        "relayer": "0xRELAYER",
        "purpose": "boarding_intake",
        "recordType": "VACCINATION",
        "challenge": "ch",
        "mode": "self",
        "exp": 1u64,
        "jti": "j2"
    });
    let claims: VerifyClaims = serde_json::from_value(v).expect("valid claims");
    assert!(claims.verifier_api_base.is_none());
}

#[test]
fn verify_claims_round_trips_through_json() {
    let claims = VerifyClaims {
        iss: "vet-1".into(),
        sub: "s".into(),
        aud: "dogtag-mobile".into(),
        relayer: "0xR".into(),
        purpose: "p".into(),
        record_type: "VACCINATION".into(),
        challenge: "c".into(),
        mode: "zk".into(),
        exp: 42,
        jti: "j".into(),
        verifier_api_base: Some("https://v".into()),
    };
    let s = serde_json::to_string(&claims).unwrap();
    // serialization uses the renamed JSON keys.
    assert!(s.contains("\"recordType\""));
    assert!(s.contains("\"verifierApiBase\""));
    let back: VerifyClaims = serde_json::from_str(&s).unwrap();
    assert_eq!(back.record_type, "VACCINATION");
    assert_eq!(back.verifier_api_base.as_deref(), Some("https://v"));
}
