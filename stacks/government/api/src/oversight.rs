//! Oversight-indexer client — the government authority's **UNSCOPED** consumer of the PR-4 indexer
//! (`stacks/indexer/api`, port 46001). The government is the cross-issuer oversight principal: the
//! bearer it presents resolves **server-side** to `Scope::Unscoped`, so it sees EVERY issuer's events
//! (across all vets/groomers/authorities), not just its own. `crate::trace` then joins the feed to the
//! government's OWN issued credentials so the authority can see which on-chain activity is its own.
//!
//! Structurally identical to the admin stack's `OversightFeed` (`stacks/admin/api/src/indexer.rs`) and
//! the vet stack's scoped `crate::oversight`: a trait (testable handlers), a real `reqwest` impl
//! (`HttpOversightFeed`), a `DisabledFeed` (unconfigured indexer → `NotConfigured` → 503), and a
//! `MemFeed` (hermetic tests). The feed is non-PII by doctrine.

use async_trait::async_trait;
use serde_json::Value;

/// The oversight feed filters passed through to the indexer's `/v1/events` query params. Every field
/// only ever NARROWS the (already unscoped) result set; none can widen the token's ceiling.
#[derive(Clone, Debug, Default)]
pub struct FeedQuery {
    /// Event kind: `issuerCreated|rootRegistered|whitelisted|delisted|rootIssued|rootRevoked|verified`.
    pub event_type: Option<String>,
    /// Acting signer address (`by`/`signer`/`relayer`).
    pub signer: Option<String>,
    /// `DogTagIssuer` clone address.
    pub issuer: Option<String>,
    /// recordType — a human label (`TRAVEL_CLEARANCE`) or an already-hashed `0x…` key; hashed server-side.
    pub record_type: Option<String>,
    pub root: Option<String>,
    pub dog_tag_id: Option<String>,
    /// `finalized` | `pending`.
    pub finality: Option<String>,
    /// Inclusive lower/upper bounds on `blockTimestamp` (unix seconds).
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl FeedQuery {
    /// The `(key, value)` query-string pairs for the indexer `/v1/events` request (only set fields).
    pub fn to_pairs(&self) -> Vec<(&'static str, String)> {
        let mut p: Vec<(&'static str, String)> = Vec::new();
        let mut push_str = |k: &'static str, v: &Option<String>| {
            if let Some(s) = v {
                let t = s.trim();
                if !t.is_empty() {
                    p.push((k, t.to_string()));
                }
            }
        };
        push_str("type", &self.event_type);
        push_str("signer", &self.signer);
        push_str("issuer", &self.issuer);
        push_str("recordType", &self.record_type);
        push_str("root", &self.root);
        push_str("dogTagId", &self.dog_tag_id);
        push_str("finality", &self.finality);
        if let Some(v) = self.since {
            p.push(("since", v.to_string()));
        }
        if let Some(v) = self.until {
            p.push(("until", v.to_string()));
        }
        if let Some(v) = self.limit {
            p.push(("limit", v.to_string()));
        }
        if let Some(v) = self.offset {
            p.push(("offset", v.to_string()));
        }
        p
    }
}

/// Failure modes of an oversight-feed call.
#[derive(Debug)]
pub enum FeedError {
    /// No indexer configured (`INDEXER_API_BASE` unset) — handlers map this to 503.
    NotConfigured,
    /// Transport error (DNS, connect, timeout, body read).
    Transport(String),
    /// The indexer answered with a non-2xx status.
    Status(u16, String),
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedError::NotConfigured => {
                write!(f, "oversight indexer not configured (set INDEXER_API_BASE)")
            }
            FeedError::Transport(e) => write!(f, "indexer transport error: {e}"),
            FeedError::Status(code, body) => write!(f, "indexer returned {code}: {body}"),
        }
    }
}

impl std::error::Error for FeedError {}

/// The oversight-indexer read surface the government oversight portal consumes (unscoped).
#[async_trait]
pub trait OversightFeed: Send + Sync {
    /// `GET /v1/events` — the cross-issuer activity feed (paginated, newest-first), narrowed by `q`.
    async fn events(&self, q: &FeedQuery) -> Result<Value, FeedError>;
    /// `GET /v1/stats` — cross-issuer aggregate counters (active/revoked/verifications/…).
    async fn stats(&self) -> Result<Value, FeedError>;
    /// `GET /v1/issuers` — per-clone issued/revoked/active counts across ALL issuers.
    async fn issuers(&self) -> Result<Value, FeedError>;
    /// `GET /v1/status` — indexer progress + finality watermark.
    async fn status(&self) -> Result<Value, FeedError>;
    /// Whether a real indexer is wired (false ⇒ every call yields `NotConfigured`).
    fn configured(&self) -> bool {
        true
    }
}

/// Real `reqwest` client for the oversight indexer, presenting the **unscoped** government bearer token.
pub struct HttpOversightFeed {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl HttpOversightFeed {
    /// `base` = the indexer root (e.g. `http://indexer:46001`); `token` = the unscoped oversight token
    /// (an `INDEXER_SCOPES` entry with `unscoped: true`).
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Self {
        HttpOversightFeed {
            base: base.into().trim_end_matches('/').to_string(),
            token: token.into(),
            http: reqwest::Client::new(),
        }
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, FeedError> {
        let url = format!("{}{}", self.base, path);
        let resp = self
            .http
            .get(url)
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(|e| FeedError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(FeedError::Status(status.as_u16(), body));
        }
        resp.json::<Value>()
            .await
            .map_err(|e| FeedError::Transport(e.to_string()))
    }
}

#[async_trait]
impl OversightFeed for HttpOversightFeed {
    async fn events(&self, q: &FeedQuery) -> Result<Value, FeedError> {
        self.get("/v1/events", &q.to_pairs()).await
    }
    async fn stats(&self) -> Result<Value, FeedError> {
        self.get("/v1/stats", &[]).await
    }
    async fn issuers(&self) -> Result<Value, FeedError> {
        self.get("/v1/issuers", &[]).await
    }
    async fn status(&self) -> Result<Value, FeedError> {
        self.get("/v1/status", &[]).await
    }
}

/// The unconfigured feed: no `INDEXER_API_BASE` was set, so every call fails closed with
/// `NotConfigured` (handlers translate that to 503). The rest of the government backend keeps working;
/// only the oversight surfaces are unavailable.
pub struct DisabledFeed;

#[async_trait]
impl OversightFeed for DisabledFeed {
    async fn events(&self, _q: &FeedQuery) -> Result<Value, FeedError> {
        Err(FeedError::NotConfigured)
    }
    async fn stats(&self) -> Result<Value, FeedError> {
        Err(FeedError::NotConfigured)
    }
    async fn issuers(&self) -> Result<Value, FeedError> {
        Err(FeedError::NotConfigured)
    }
    async fn status(&self) -> Result<Value, FeedError> {
        Err(FeedError::NotConfigured)
    }
    fn configured(&self) -> bool {
        false
    }
}

/// In-memory feed for hermetic tests: returns canned `/v1/events`, `/v1/stats`, `/v1/issuers`,
/// `/v1/status` payloads shaped exactly like the real indexer's responses. Mirrors `MemChain`.
#[derive(Clone, Default)]
pub struct MemFeed {
    events: Value,
    stats: Value,
    issuers: Value,
    status: Value,
}

impl MemFeed {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_events(mut self, v: Value) -> Self {
        self.events = v;
        self
    }
    pub fn with_stats(mut self, v: Value) -> Self {
        self.stats = v;
        self
    }
    pub fn with_issuers(mut self, v: Value) -> Self {
        self.issuers = v;
        self
    }
    pub fn with_status(mut self, v: Value) -> Self {
        self.status = v;
        self
    }
}

#[async_trait]
impl OversightFeed for MemFeed {
    async fn events(&self, _q: &FeedQuery) -> Result<Value, FeedError> {
        Ok(self.events.clone())
    }
    async fn stats(&self) -> Result<Value, FeedError> {
        Ok(self.stats.clone())
    }
    async fn issuers(&self) -> Result<Value, FeedError> {
        Ok(self.issuers.clone())
    }
    async fn status(&self) -> Result<Value, FeedError> {
        Ok(self.status.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_query_emits_only_set_pairs_and_skips_blanks() {
        let q = FeedQuery {
            event_type: Some("rootIssued".into()),
            signer: Some("  ".into()), // blank → skipped
            record_type: Some("TRAVEL_CLEARANCE".into()),
            limit: Some(50),
            ..Default::default()
        };
        let pairs = q.to_pairs();
        assert!(pairs.contains(&("type", "rootIssued".to_string())));
        assert!(pairs.contains(&("recordType", "TRAVEL_CLEARANCE".to_string())));
        assert!(pairs.contains(&("limit", "50".to_string())));
        assert!(
            !pairs.iter().any(|(k, _)| *k == "signer"),
            "blank signer skipped"
        );
    }

    #[tokio::test]
    async fn disabled_feed_fails_closed_not_configured() {
        let f = DisabledFeed;
        assert!(!f.configured());
        assert!(matches!(f.stats().await, Err(FeedError::NotConfigured)));
        assert!(matches!(
            f.events(&FeedQuery::default()).await,
            Err(FeedError::NotConfigured)
        ));
    }
}
