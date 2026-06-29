//! DNS legitimacy check (architecture §13.3 H): onboarding MUST verify a business's DNS TXT record
//! BEFORE whitelisting its issuer addresses. The expected TXT proves the operator controls the domain
//! they registered (internal consistency of domain+contract+QR is NOT legitimacy — the registry, gated
//! by this check, is the trust root for "is this a real vet").
//!
//! `DnsChecker` is abstract: `DohDnsChecker` resolves TXT via DNS-over-HTTPS (Cloudflare/Google) for
//! production; `MockDnsChecker` returns a programmed verdict for hermetic tests.

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("dns lookup failed: {0}")]
    Lookup(String),
}

/// Verifies a domain publishes the expected DogTag verification TXT record.
#[async_trait]
pub trait DnsChecker: Send + Sync {
    /// True iff `domain` publishes a TXT record whose value contains `expected_token`.
    /// `expected_token` is the deployment's challenge string (e.g. `dogtag-verify=<documentStore>`).
    async fn txt_contains(&self, domain: &str, expected_token: &str) -> Result<bool, DnsError>;
}

/// The canonical TXT challenge a business must publish to prove control: `dogtag-verify=<documentStore>`.
pub fn expected_txt(document_store: &str) -> String {
    format!("dogtag-verify={}", document_store.to_lowercase())
}

// --------------------------------------------------------------------------------------------
// DohDnsChecker — DNS-over-HTTPS (RFC 8484 JSON form via Cloudflare).
// --------------------------------------------------------------------------------------------

pub struct DohDnsChecker {
    /// DoH JSON endpoint base (default Cloudflare `https://cloudflare-dns.com/dns-query`).
    pub endpoint: String,
}

impl Default for DohDnsChecker {
    fn default() -> Self {
        DohDnsChecker {
            endpoint: "https://cloudflare-dns.com/dns-query".to_string(),
        }
    }
}

#[async_trait]
impl DnsChecker for DohDnsChecker {
    async fn txt_contains(&self, domain: &str, expected_token: &str) -> Result<bool, DnsError> {
        let client = reqwest::Client::new();
        let resp = client
            .get(&self.endpoint)
            .query(&[("name", domain), ("type", "TXT")])
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| DnsError::Lookup(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(DnsError::Lookup(format!("status {}", resp.status())));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DnsError::Lookup(e.to_string()))?;
        let answers = body.get("Answer").and_then(|a| a.as_array());
        let found = answers
            .map(|arr| {
                arr.iter().any(|ans| {
                    ans.get("data")
                        .and_then(|d| d.as_str())
                        .map(|s| s.trim_matches('"').contains(expected_token))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        Ok(found)
    }
}

// --------------------------------------------------------------------------------------------
// MockDnsChecker — programmable verdict for hermetic tests.
// --------------------------------------------------------------------------------------------

/// A mock that returns a fixed verdict (optionally only when the token matches a programmed value).
pub struct MockDnsChecker {
    pub ok: bool,
}

impl MockDnsChecker {
    pub fn ok() -> Self {
        MockDnsChecker { ok: true }
    }
    pub fn fail() -> Self {
        MockDnsChecker { ok: false }
    }
}

#[async_trait]
impl DnsChecker for MockDnsChecker {
    async fn txt_contains(&self, _domain: &str, _expected_token: &str) -> Result<bool, DnsError> {
        Ok(self.ok)
    }
}
