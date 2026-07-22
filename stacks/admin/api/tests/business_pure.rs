//! Unit coverage for the appointment-only `MockBusinessClient` call recorder.

use admin_api::business::{BusinessClient, BusinessError, MockBusinessClient};
use serde_json::json;

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
    assert_eq!(calls[0].url, "https://biz.example/v1/appointments/appt-1");
    assert_eq!(calls[0].body, body);
}

#[tokio::test]
async fn mock_not_ok_errors_but_still_records() {
    let mock = MockBusinessClient::new(false);
    let body = json!({});

    let put = mock
        .put_appointment("https://biz.example", "secret", "appt-2", "idem-2", &body)
        .await;
    assert!(matches!(put, Err(BusinessError::Status(502))));

    let calls = mock.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "PUT");
}

#[tokio::test]
async fn mock_accumulates_calls_in_order() {
    let mock = MockBusinessClient::new(true);
    let body = json!({ "n": 1 });
    mock.put_appointment("https://a", "s", "x", "k", &body)
        .await
        .unwrap();
    mock.put_appointment("https://c", "s", "y", "k", &body)
        .await
        .unwrap();

    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].url, "https://a/v1/appointments/x");
    assert_eq!(calls[1].url, "https://c/v1/appointments/y");
}
