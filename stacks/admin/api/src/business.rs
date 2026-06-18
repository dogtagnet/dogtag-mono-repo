//! `BusinessClient` — outbound cross-backend HTTP the central stack makes:
//!   - PUT an appointment to a business `apiBaseUrl` (Idempotency-Key + HMAC), §4.4/architecture §8.3;
//!   - relay a verification consent to a verifier's `/verify/consent/submit`, §4.1.
//!
//! `ReqwestBusinessClient` is the real impl; `MockBusinessClient` records calls for hermetic tests.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use crate::auth::hmac_sign;

#[derive(Debug, thiserror::Error)]
pub enum BusinessError {
    #[error("http: {0}")]
    Http(String),
    #[error("status {0}")]
    Status(u16),
}

/// A recorded outbound call (for the mock + observability).
#[derive(Clone, Debug)]
pub struct CallRecord {
    pub method: String,
    pub url: String,
    pub body: Value,
}

#[async_trait]
pub trait BusinessClient: Send + Sync {
    /// PUT /v1/appointments/{id} to the business backend with an Idempotency-Key + HMAC signature.
    async fn put_appointment(
        &self,
        api_base_url: &str,
        hmac_secret: &str,
        appointment_id: &str,
        idempotency_key: &str,
        body: &Value,
    ) -> Result<(), BusinessError>;

    /// POST {verifierApiBase}/verify/consent/submit to relay a consent for on-chain submission.
    async fn relay_consent(
        &self,
        verifier_api_base: &str,
        body: &Value,
    ) -> Result<Value, BusinessError>;
}

// --------------------------------------------------------------------------------------------
// ReqwestBusinessClient — real HTTP.
// --------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct ReqwestBusinessClient {
    client: reqwest::Client,
}

impl ReqwestBusinessClient {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl BusinessClient for ReqwestBusinessClient {
    async fn put_appointment(
        &self,
        api_base_url: &str,
        hmac_secret: &str,
        appointment_id: &str,
        idempotency_key: &str,
        body: &Value,
    ) -> Result<(), BusinessError> {
        let path = format!("/v1/appointments/{appointment_id}");
        let url = format!("{}{}", api_base_url.trim_end_matches('/'), path);
        let body_bytes = serde_json::to_vec(body).unwrap_or_default();
        let sig = hmac_sign(hmac_secret, "PUT", &path, &body_bytes);
        let resp = self
            .client
            .put(&url)
            .header("Idempotency-Key", idempotency_key)
            .header("X-DogTag-HMAC", sig)
            .header("content-type", "application/json")
            .body(body_bytes)
            .send()
            .await
            .map_err(|e| BusinessError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(BusinessError::Status(resp.status().as_u16()));
        }
        Ok(())
    }

    async fn relay_consent(
        &self,
        verifier_api_base: &str,
        body: &Value,
    ) -> Result<Value, BusinessError> {
        let url = format!("{}/verify/consent/submit", verifier_api_base.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .map_err(|e| BusinessError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(BusinessError::Status(resp.status().as_u16()));
        }
        resp.json().await.map_err(|e| BusinessError::Http(e.to_string()))
    }
}

// --------------------------------------------------------------------------------------------
// MockBusinessClient — records calls; returns a programmed verdict.
// --------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct MockBusinessClient {
    pub calls: Arc<Mutex<Vec<CallRecord>>>,
    /// if true, relay/put succeed; else they error.
    pub ok: bool,
}

impl MockBusinessClient {
    pub fn new(ok: bool) -> Self {
        MockBusinessClient { calls: Arc::new(Mutex::new(Vec::new())), ok }
    }
    pub fn calls(&self) -> Vec<CallRecord> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl BusinessClient for MockBusinessClient {
    async fn put_appointment(
        &self,
        api_base_url: &str,
        _hmac_secret: &str,
        appointment_id: &str,
        _idempotency_key: &str,
        body: &Value,
    ) -> Result<(), BusinessError> {
        self.calls.lock().unwrap().push(CallRecord {
            method: "PUT".into(),
            url: format!("{api_base_url}/v1/appointments/{appointment_id}"),
            body: body.clone(),
        });
        if self.ok {
            Ok(())
        } else {
            Err(BusinessError::Status(502))
        }
    }

    async fn relay_consent(
        &self,
        verifier_api_base: &str,
        body: &Value,
    ) -> Result<Value, BusinessError> {
        self.calls.lock().unwrap().push(CallRecord {
            method: "POST".into(),
            url: format!("{verifier_api_base}/verify/consent/submit"),
            body: body.clone(),
        });
        if self.ok {
            Ok(serde_json::json!({ "recorded": true }))
        } else {
            Err(BusinessError::Status(502))
        }
    }
}
