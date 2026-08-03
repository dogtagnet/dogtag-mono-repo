//! # The issuer↔domain DNS TXT binding
//!
//! The on-chain [`IssuerDomainRegistry`] holds one half of a bidirectional binding: an issuer clone
//! asserts "my domain is D". This crate resolves the OTHER half — whether D's DNS zone asserts "clone C
//! is mine" — and reports it as one of exactly three states that must never be collapsed.
//!
//! ## The normative record convention
//!
//! ```text
//! <clone-address-lowercase>._dogtag.<domain>.   IN TXT   "<free-form description>"
//! ```
//!
//! Concretely, for clone `0x6F8a0B1c2D3e4F5A6B7c8D9E0F1A2b3C4d5E6f70` claiming `moh.gov.sg`:
//!
//! ```text
//! 0x6f8a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f70._dogtag.moh.gov.sg. IN TXT "Travel clearance issuance"
//! ```
//!
//! **The address is in the NAME, and the VALUE is free-form.** That split is the whole design: the
//! domain owner chooses what the record says about the address, so the value cannot also carry the
//! address. Presence of the record at that exact name IS the assertion — this crate therefore never
//! pattern-matches the value, it only reports it.
//!
//! ### Why a `_dogtag` label
//!
//! An underscore-prefixed label cannot collide with a real hostname (underscores are not legal in host
//! names), so the binding can never shadow something the domain owner actually serves. Putting the
//! selector-like address label FIRST and the underscore label second follows DKIM's
//! `<selector>._domainkey.<domain>`, which is the closest established precedent for "one machine-read
//! TXT record per key". A `0x`-prefixed 20-byte address is 42 characters, comfortably inside the
//! 63-octet DNS label limit.
//!
//! ### Casing
//!
//! Addresses are frequently written EIP-55 checksummed (mixed case), so the query name is always
//! lowercased by [`txt_name`]. DNS is case-insensitive, and resolvers may additionally randomise the
//! case of the name they echo back (DNS 0x20), so every name comparison here is case-insensitive too.
//!
//! ## The three states, and why they stay distinct
//!
//! [`BindingStatus`] is deliberately not a boolean:
//!
//! * [`BindingStatus::Verified`] — the records were actually fetched and the record actually exists.
//! * [`BindingStatus::NotListed`] — the records were actually fetched and it is actually absent. A
//!   NORMAL state: most issuers publish nothing on day one.
//! * [`BindingStatus::CouldNotCheck`] — a real attempt actually failed. This says NOTHING about the
//!   issuer and must never be rendered as either of the other two.
//!
//! Collapsing the third into either of the others is the fail-open bug this repo has had elsewhere
//! (`.unknown -> VALID` in the mobile importer). The DoH `Status` field is what keeps them apart:
//! `NOERROR` with no matching TXT and `NXDOMAIN` are definitive absences; `SERVFAIL` / `REFUSED` /
//! transport errors / timeouts are non-answers.
//!
//! ## No synthesized results, ever
//!
//! There is no stub, fixture or demo shortcut in this crate's shipped path. Every [`BindingStatus`] a
//! caller receives is the outcome of a real resolution — or of a real resolution that really failed.
//! Test doubles exist only behind the [`DohTransport`] seam and only in tests.
//!
//! Cached results are real prior observations, not synthesized ones: every [`BindingCheck`] carries the
//! [`BindingCheck::checked_at`] it was actually observed at, so a surface can show its own freshness
//! rather than implying it just looked.
//!
//! [`IssuerDomainRegistry`]: https://github.com/dogtagnet/dogtag-mono-repo/blob/main/contracts/src/IssuerDomainRegistry.sol

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The underscore label that namespaces every DogTag binding record.
pub const DOGTAG_LABEL: &str = "_dogtag";

/// DNS record type code for TXT (RFC 1035 §3.2.2) — used to filter the DoH `Answer` array, which may
/// also carry CNAME records when the queried name is aliased.
const RR_TYPE_TXT: u64 = 16;

// -------------------------------------------------------------------------------------------------
// The convention
// -------------------------------------------------------------------------------------------------

/// The fully-qualified TXT name that binds `clone_addr` to `domain`, per the convention above.
///
/// Both inputs are lowercased and trimmed; `domain` may be given with a trailing dot or without.
/// Returns `None` when either input is empty or obviously not a name/address, so a caller can report
/// [`BindingStatus::CouldNotCheck`] rather than firing a nonsense query.
pub fn txt_name(clone_addr: &str, domain: &str) -> Option<String> {
    let addr = clone_addr.trim().to_ascii_lowercase();
    let dom = domain.trim().trim_end_matches('.').to_ascii_lowercase();

    if addr.is_empty() || dom.is_empty() {
        return None;
    }
    // The address label must be a 0x-prefixed 20-byte hex string: it is the record's identity, so a
    // malformed one would query a name no domain owner could ever have published.
    if !(addr.len() == 42
        && addr.starts_with("0x")
        && addr[2..].bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    // A bare single label is never a resolvable issuer domain, and anything with a scheme, path, port
    // or whitespace is not a name at all.
    if !dom.contains('.')
        || dom.contains('/')
        || dom.contains(':')
        || dom.contains(char::is_whitespace)
    {
        return None;
    }

    Some(format!("{addr}.{DOGTAG_LABEL}.{dom}"))
}

// -------------------------------------------------------------------------------------------------
// The three states
// -------------------------------------------------------------------------------------------------

/// Why a lookup could not produce an answer. Carried so a surface can show the operator something
/// actionable without ever implying it learned anything about the issuer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CouldNotCheckReason {
    /// The claimed domain or the address was unusable, so no query was possible.
    Unresolvable,
    /// The resolver answered, but with a server-side failure code (SERVFAIL / REFUSED / …).
    ResolverError {
        #[serde(rename = "dnsStatus")]
        dns_status: u64,
    },
    /// The DoH request itself failed: transport error, timeout, non-2xx, or an unparseable body.
    Transport { detail: String },
    /// No resolver is configured for this deployment.
    NoResolver,
}

/// The outcome of a binding lookup. Three variants, never collapsible to a boolean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum BindingStatus {
    /// The domain publishes a binding record for this address. `description` is exactly what the zone
    /// says — free-form text chosen by the domain owner, never interpreted here.
    Verified { description: String },
    /// The records were fetched and this address is not among them. A normal, unremarkable state.
    NotListed,
    /// A real attempt failed. Evidence for nothing.
    CouldNotCheck { reason: CouldNotCheckReason },
}

impl BindingStatus {
    /// True only for [`BindingStatus::Verified`]. Provided so callers never hand-roll a match that
    /// accidentally lets `CouldNotCheck` through as a pass.
    pub fn is_verified(&self) -> bool {
        matches!(self, BindingStatus::Verified { .. })
    }
}

/// A binding status plus the provenance a surface needs to be honest about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingCheck {
    /// The on-chain-claimed domain that was actually queried.
    pub domain: String,
    /// The clone address whose binding was sought (lowercased).
    pub clone_address: String,
    /// The fully-qualified TXT name queried, so an operator can reproduce the lookup by hand.
    pub txt_name: Option<String>,
    pub status: BindingStatus,
    /// Unix seconds at which this observation was actually made. A cached hit keeps its ORIGINAL
    /// timestamp — the surface shows when DNS was really consulted, not when it rendered.
    pub checked_at: u64,
    /// The smallest TTL the zone advertised for this answer, when it gave one. Informational on the
    /// wire; the resolver uses it (clamped) to bound its own cache entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_ttl: Option<u64>,
}

// -------------------------------------------------------------------------------------------------
// The transport seam
// -------------------------------------------------------------------------------------------------

/// A DoH JSON fetch. This is the ONLY seam a test may substitute; the shipped path always has a real
/// HTTP client behind it.
#[async_trait]
pub trait DohTransport: Send + Sync {
    /// GET `url` with `accept: application/dns-json`. `Ok` carries (HTTP status, body); `Err` carries a
    /// transport-level description (connect failure, timeout, TLS error).
    async fn get(&self, url: &str) -> Result<(u16, String), String>;
}

/// The production transport: reqwest over rustls, with a hard timeout.
pub struct ReqwestDoh {
    client: reqwest::Client,
}

impl ReqwestDoh {
    pub fn new(timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        ReqwestDoh { client }
    }
}

impl Default for ReqwestDoh {
    fn default() -> Self {
        Self::new(Duration::from_secs(4))
    }
}

#[async_trait]
impl DohTransport for ReqwestDoh {
    async fn get(&self, url: &str) -> Result<(u16, String), String> {
        let resp = self
            .client
            .get(url)
            .header("accept", "application/dns-json")
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    "timeout".to_string()
                } else {
                    e.to_string()
                }
            })?;
        let code = resp.status().as_u16();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        Ok((code, body))
    }
}

// -------------------------------------------------------------------------------------------------
// The resolver
// -------------------------------------------------------------------------------------------------

/// How long a given outcome may be reused from cache. Deliberately asymmetric: a transient failure
/// must recover quickly, while a stable answer should not be re-queried per render.
#[derive(Debug, Clone, Copy)]
pub struct CacheTtl {
    /// Cap on a `Verified`/`NotListed` entry's lifetime. The DNS-advertised TTL is honoured when it is
    /// SHORTER than this, so a domain owner who publishes a 60s TTL gets 60s.
    pub answer_max: Duration,
    /// Floor under the DNS-advertised TTL, so a 1-second TTL cannot turn into a lookup per render.
    pub answer_min: Duration,
    /// Lifetime of a `CouldNotCheck` entry — short, so a resolver blip clears on its own.
    pub failure: Duration,
}

impl Default for CacheTtl {
    fn default() -> Self {
        CacheTtl {
            answer_max: Duration::from_secs(900), // 15 min
            answer_min: Duration::from_secs(60),
            failure: Duration::from_secs(30),
        }
    }
}

struct CacheEntry {
    check: BindingCheck,
    expires_at: SystemTime,
}

/// Resolves issuer↔domain bindings over DNS-over-HTTPS, with a TTL cache.
///
/// The endpoint is configurable because it is a real third party in the trust picture. Callers should
/// document which one they point at and what it learns (see the crate-level privacy note in the PR).
pub struct BindingResolver {
    transport: Box<dyn DohTransport>,
    endpoint: String,
    ttl: CacheTtl,
    cache: RwLock<HashMap<String, CacheEntry>>,
}

impl BindingResolver {
    /// `endpoint` is a DoH JSON endpoint, e.g. `https://cloudflare-dns.com/dns-query`. An empty
    /// endpoint is a configuration error and makes every lookup report
    /// [`CouldNotCheckReason::NoResolver`] rather than silently passing or failing.
    pub fn new(
        transport: Box<dyn DohTransport>,
        endpoint: impl Into<String>,
        ttl: CacheTtl,
    ) -> Self {
        BindingResolver {
            transport,
            endpoint: endpoint.into(),
            ttl,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// The default production resolver: reqwest + Cloudflare DoH + default TTLs.
    pub fn production(endpoint: impl Into<String>) -> Self {
        Self::new(
            Box::new(ReqwestDoh::default()),
            endpoint,
            CacheTtl::default(),
        )
    }

    /// Resolve the binding for `clone_addr` under the on-chain-claimed `domain`.
    ///
    /// `domain` MUST be the domain read from the chain, never one read from a credential document: the
    /// document's `issuer` block is outside the Merkle root and can be relabelled at will.
    pub async fn check(&self, clone_addr: &str, domain: &str) -> BindingCheck {
        let addr_lc = clone_addr.trim().to_ascii_lowercase();
        let dom_lc = domain.trim().trim_end_matches('.').to_ascii_lowercase();

        let name = match txt_name(&addr_lc, &dom_lc) {
            Some(n) => n,
            None => {
                return BindingCheck {
                    domain: dom_lc,
                    clone_address: addr_lc,
                    txt_name: None,
                    status: BindingStatus::CouldNotCheck {
                        reason: CouldNotCheckReason::Unresolvable,
                    },
                    checked_at: now_secs(),
                    answer_ttl: None,
                }
            }
        };

        if let Some(hit) = self.cached(&name) {
            return hit;
        }

        let check = self.resolve_uncached(&name, &addr_lc, &dom_lc).await;
        self.store(&name, &check);
        check
    }

    /// Drop every cached observation. Intended for the future daily admin re-check job, which wants a
    /// genuinely fresh sweep rather than its own warm cache.
    pub fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
    }

    fn cached(&self, name: &str) -> Option<BindingCheck> {
        let guard = self.cache.read().ok()?;
        let entry = guard.get(name)?;
        if entry.expires_at > SystemTime::now() {
            Some(entry.check.clone())
        } else {
            None
        }
    }

    fn store(&self, name: &str, check: &BindingCheck) {
        let lifetime = match &check.status {
            BindingStatus::CouldNotCheck { .. } => self.ttl.failure,
            // Honour the zone's own TTL when it is shorter than our cap, but never dip below the floor
            // — a 1-second TTL must not become a DNS lookup per render.
            _ => match check.answer_ttl {
                Some(t) => Duration::from_secs(t).clamp(self.ttl.answer_min, self.ttl.answer_max),
                None => self.ttl.answer_max,
            },
        };
        if let Ok(mut c) = self.cache.write() {
            c.insert(
                name.to_string(),
                CacheEntry {
                    check: check.clone(),
                    expires_at: SystemTime::now() + lifetime,
                },
            );
        }
    }

    /// Resolve the TXT records at `name` — the low-level, UNCACHED primitive.
    ///
    /// Keeps the three-way distinction of [`classify_txt`]: `Ok(non-empty)` exists, `Ok(empty)` is a
    /// definitive absence, `Err` is a non-answer. The apex `dogtag-verify=` onboarding gate uses this
    /// too, so there is exactly one DoH client and one Status-branching rule in the codebase.
    ///
    /// Returns the parsed records plus the smallest advertised TTL, when the zone gave one.
    pub async fn lookup_txt(
        &self,
        name: &str,
    ) -> Result<(Vec<TxtRecord>, Option<u64>), CouldNotCheckReason> {
        if self.endpoint.trim().is_empty() {
            return Err(CouldNotCheckReason::NoResolver);
        }
        let sep = if self.endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        let url = format!("{}{}name={}&type=TXT", self.endpoint, sep, urlencode(name));

        let (code, body) = self
            .transport
            .get(&url)
            .await
            .map_err(|detail| CouldNotCheckReason::Transport { detail })?;
        if !(200..300).contains(&code) {
            return Err(CouldNotCheckReason::Transport {
                detail: format!("http {code}"),
            });
        }
        let records = classify_txt(&body)?;
        Ok((records, answer_ttl(&body)))
    }

    async fn resolve_uncached(&self, name: &str, addr_lc: &str, dom_lc: &str) -> BindingCheck {
        let (status, ttl) = match self.lookup_txt(name).await {
            Ok((records, ttl)) => {
                let status = match select_binding(&records, name) {
                    Some(description) => BindingStatus::Verified { description },
                    None => BindingStatus::NotListed,
                };
                (status, ttl)
            }
            Err(reason) => (BindingStatus::CouldNotCheck { reason }, None),
        };

        BindingCheck {
            domain: dom_lc.to_string(),
            clone_address: addr_lc.to_string(),
            txt_name: Some(name.to_string()),
            status,
            checked_at: now_secs(),
            answer_ttl: ttl,
        }
    }
}

/// The smallest TTL any record in a DoH answer advertises, if the body carries one.
///
/// Used to bound a cache entry's lifetime: a domain owner who publishes a 60-second TTL is telling us
/// the binding may change that fast, and honouring it is the difference between a cache and a stale
/// claim. Clamped by [`CacheTtl::answer_min`]/[`CacheTtl::answer_max`] at the call site so a
/// pathological 1-second TTL still cannot become a lookup per render.
pub fn answer_ttl(body: &str) -> Option<u64> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let answers = json.get("Answer")?.as_array()?;
    answers
        .iter()
        .filter_map(|a| a.get("TTL").and_then(|t| t.as_u64()))
        .min()
}

/// One TXT record as the zone published it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtRecord {
    /// The owner name the resolver echoed, lowercased and stripped of its trailing dot.
    pub name: String,
    /// The concatenated character-strings of the RDATA (see [`unquote_txt`]).
    pub value: String,
}

/// Parse a DoH JSON body into "the TXT records that exist at the queried name", keeping the
/// definitive-absence / non-answer distinction in the `Result`:
///
/// * `Ok(non-empty)` — records were fetched and exist.
/// * `Ok(empty)` — records were fetched and there are none (NOERROR/NODATA, or NXDOMAIN). A DEFINITIVE
///   absence.
/// * `Err(reason)` — no answer was obtained (SERVFAIL/REFUSED, unparseable body, not a DoH answer).
///
/// This is the single Status-branching rule in the codebase. Both the issuer↔domain binding
/// ([`classify`]) and the apex `dogtag-verify=` onboarding gate go through it, so neither can drift
/// back into treating a resolver failure as an absence.
pub fn classify_txt(body: &str) -> Result<Vec<TxtRecord>, CouldNotCheckReason> {
    let json: serde_json::Value =
        serde_json::from_str(body).map_err(|e| CouldNotCheckReason::Transport {
            detail: format!("unparseable dns-json: {e}"),
        })?;

    // `Status` is an RCODE. An ABSENT Status is not "no error" — it means this is not a DoH JSON answer,
    // and reading it as NOERROR would turn a wrong-endpoint misconfiguration into a confident "absent".
    let status = json.get("Status").and_then(|s| s.as_u64()).ok_or_else(|| {
        CouldNotCheckReason::Transport {
            detail: "dns-json has no Status field".to_string(),
        }
    })?;

    match status {
        0 => {}                     // NOERROR — inspect the answers below.
        3 => return Ok(Vec::new()), // NXDOMAIN — a DEFINITIVE "this name does not exist".
        // Everything else (SERVFAIL=2, REFUSED=5, FORMERR=1, …) is a non-answer.
        other => return Err(CouldNotCheckReason::ResolverError { dns_status: other }),
    }

    // NOERROR with no Answer section (NODATA): the name exists but publishes no TXT.
    let Some(answers) = json.get("Answer").and_then(|a| a.as_array()) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for ans in answers {
        if ans.get("type").and_then(|t| t.as_u64()) != Some(RR_TYPE_TXT) {
            continue; // e.g. a CNAME in the chain
        }
        let Some(raw) = ans.get("data").and_then(|d| d.as_str()) else {
            continue;
        };
        // DNS 0x20 means the echoed name's case is not guaranteed, so normalise it here once.
        let name = ans
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .trim_end_matches('.')
            .to_ascii_lowercase();
        out.push(TxtRecord {
            name,
            value: unquote_txt(raw),
        });
    }
    Ok(out)
}

/// Turn a DoH JSON body into one of the three binding states.
///
/// Broken out and public so the classification rules are unit-testable without any transport, and so
/// the TS/Swift/Kotlin ports have one normative reference to mirror.
pub fn classify(body: &str, queried_name: &str) -> BindingStatus {
    let records = match classify_txt(body) {
        Ok(r) => r,
        Err(reason) => return BindingStatus::CouldNotCheck { reason },
    };
    match select_binding(&records, queried_name) {
        Some(description) => BindingStatus::Verified { description },
        None => BindingStatus::NotListed,
    }
}

/// The TXT value published AT `queried_name`, if any.
///
/// Matches case-insensitively (DNS 0x20). A single answer whose echoed name differs is still accepted,
/// because that is what a CNAME chain looks like — the resolver answered the question it was asked. Two
/// or more answers with mismatched names are not: there the echoed name is the only way to tell which
/// record belongs to our query.
fn select_binding(records: &[TxtRecord], queried_name: &str) -> Option<String> {
    let want = queried_name.trim_end_matches('.').to_ascii_lowercase();
    if records.len() == 1 {
        return Some(records[0].value.clone());
    }
    records
        .iter()
        .find(|r| r.name == want)
        .map(|r| r.value.clone())
}

/// Join a DoH TXT `data` value into the string the zone actually published.
///
/// A TXT RDATA is a sequence of character-strings, each at most 255 octets, and DoH renders them as
/// space-separated quoted chunks. RFC 1035 says a consumer concatenates them, so `"abc" "def"` is
/// `abcdef` — NOT `abc def`. Getting this wrong would corrupt any description over 255 bytes.
fn unquote_txt(raw: &str) -> String {
    let s = raw.trim();
    if !s.contains('"') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in s.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            c if in_quotes => out.push(c),
            _ => {} // whitespace between chunks — dropped, per concatenation semantics
        }
    }
    out
}

/// Minimal percent-encoding for a DNS name in a query string. A DNS name is LDH plus dots and
/// underscores, so only a tiny set can appear; anything else is escaped rather than trusted.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const CLONE: &str = "0x6F8a0B1c2D3e4F5A6B7c8D9E0F1A2b3C4d5E6f70";
    const CLONE_LC: &str = "0x6f8a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f70";

    // ---- the convention ----

    #[test]
    fn txt_name_lowercases_a_checksummed_address() {
        let n = txt_name(CLONE, "moh.gov.sg").unwrap();
        assert_eq!(n, format!("{CLONE_LC}._dogtag.moh.gov.sg"));
    }

    #[test]
    fn txt_name_tolerates_a_trailing_dot_and_padding() {
        assert_eq!(
            txt_name(
                "  0x6F8a0B1c2D3e4F5A6B7c8D9E0F1A2b3C4d5E6f70 ",
                " MOH.GOV.SG. "
            )
            .unwrap(),
            format!("{CLONE_LC}._dogtag.moh.gov.sg")
        );
    }

    #[test]
    fn txt_name_rejects_unusable_inputs() {
        assert!(txt_name("", "moh.gov.sg").is_none());
        assert!(txt_name(CLONE, "").is_none());
        assert!(txt_name("0xnothex", "moh.gov.sg").is_none()); // not an address
        assert!(txt_name("0xb5d6", "moh.gov.sg").is_none()); // too short
        assert!(txt_name(CLONE, "localhost").is_none()); // single label
        assert!(txt_name(CLONE, "https://moh.gov.sg").is_none()); // a URL
        assert!(txt_name(CLONE, "moh.gov.sg:443").is_none()); // a port
        assert!(txt_name(CLONE, "moh gov sg").is_none()); // whitespace
    }

    #[test]
    fn the_address_label_fits_a_dns_label() {
        // 42 chars, well under the 63-octet limit — the reason the address can live in the name at all.
        assert_eq!(CLONE_LC.len(), 42);
        assert!(CLONE_LC.len() <= 63);
    }

    // ---- classification: the three states ----

    fn doh(status: u64, answers: &str) -> String {
        format!(r#"{{"Status":{status},"Answer":[{answers}]}}"#)
    }

    fn txt_answer(name: &str, data: &str) -> String {
        format!(r#"{{"name":"{name}","type":16,"TTL":300,"data":"{data}"}}"#)
    }

    #[test]
    fn verified_when_the_record_exists() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let body = doh(0, &txt_answer(&name, "\\\"Travel clearance issuance\\\""));
        match classify(&body, &name) {
            BindingStatus::Verified { description } => {
                assert_eq!(description, "Travel clearance issuance")
            }
            other => panic!("expected Verified, got {other:?}"),
        }
    }

    #[test]
    fn verified_tolerates_0x20_case_randomisation_in_the_echoed_name() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let echoed = format!("{CLONE_LC}._DogTag.MOH.gov.SG.");
        let body = doh(0, &txt_answer(&echoed, "\\\"ok\\\""));
        assert!(classify(&body, &name).is_verified());
    }

    #[test]
    fn verified_concatenates_chunked_txt_strings() {
        // A >255-byte description arrives as multiple quoted chunks and MUST be joined, not spaced.
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let body = doh(0, &txt_answer(&name, "\\\"part-one\\\" \\\"part-two\\\""));
        match classify(&body, &name) {
            BindingStatus::Verified { description } => assert_eq!(description, "part-onepart-two"),
            other => panic!("expected Verified, got {other:?}"),
        }
    }

    #[test]
    fn not_listed_on_noerror_with_no_answer_section() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        assert_eq!(classify(r#"{"Status":0}"#, &name), BindingStatus::NotListed);
    }

    #[test]
    fn not_listed_on_noerror_with_only_unrelated_records() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        // A CNAME-only chain with no TXT: the name resolves but publishes no binding.
        let body = doh(
            0,
            &format!(r#"{{"name":"{name}","type":5,"TTL":300,"data":"elsewhere.example."}}"#),
        );
        assert_eq!(classify(&body, &name), BindingStatus::NotListed);
    }

    #[test]
    fn not_listed_on_nxdomain() {
        // NXDOMAIN is a DEFINITIVE absence, not a failure — this is the distinction the whole feature
        // rests on, so it gets its own guard.
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        assert_eq!(classify(r#"{"Status":3}"#, &name), BindingStatus::NotListed);
    }

    #[test]
    fn could_not_check_on_servfail() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        assert_eq!(
            classify(r#"{"Status":2}"#, &name),
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::ResolverError { dns_status: 2 }
            }
        );
    }

    #[test]
    fn could_not_check_on_refused() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        assert_eq!(
            classify(r#"{"Status":5}"#, &name),
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::ResolverError { dns_status: 5 }
            }
        );
    }

    #[test]
    fn could_not_check_on_unparseable_or_statusless_body() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        assert!(matches!(
            classify("not json at all", &name),
            BindingStatus::CouldNotCheck { .. }
        ));
        // A 200 with a JSON body that is not a DoH answer must NOT read as NOERROR.
        assert!(matches!(
            classify(r#"{"hello":"world"}"#, &name),
            BindingStatus::CouldNotCheck { .. }
        ));
    }

    /// The regression guard for the bug this crate exists to avoid: SERVFAIL and "no record" must not
    /// produce the same value.
    #[test]
    fn servfail_and_absence_are_not_the_same_state() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let absent = classify(r#"{"Status":0}"#, &name);
        let broken = classify(r#"{"Status":2}"#, &name);
        assert_ne!(absent, broken);
        assert!(!absent.is_verified() && !broken.is_verified());
    }

    // ---- resolver-level behaviour ----

    struct StubTransport {
        body: String,
        code: u16,
        fail: Option<String>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl DohTransport for StubTransport {
        async fn get(&self, _url: &str) -> Result<(u16, String), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok((self.code, self.body.clone())),
            }
        }
    }

    fn resolver_with(
        body: &str,
        code: u16,
        fail: Option<&str>,
        calls: Arc<AtomicUsize>,
    ) -> BindingResolver {
        BindingResolver::new(
            Box::new(StubTransport {
                body: body.to_string(),
                code,
                fail: fail.map(|s| s.to_string()),
                calls,
            }),
            "https://doh.example/dns-query",
            CacheTtl::default(),
        )
    }

    #[tokio::test]
    async fn check_reports_verified_with_the_published_description() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(
            &doh(0, &txt_answer(&name, "\\\"Ministry issuance contract\\\"")),
            200,
            None,
            calls,
        );
        let c = r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(c.clone_address, CLONE_LC, "the address is lowercased");
        assert_eq!(c.txt_name.as_deref(), Some(name.as_str()));
        assert_eq!(
            c.status,
            BindingStatus::Verified {
                description: "Ministry issuance contract".to_string()
            }
        );
        assert!(c.checked_at > 0, "a real observation carries its timestamp");
    }

    #[tokio::test]
    async fn check_reports_not_listed_for_a_domain_publishing_nothing() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(r#"{"Status":3}"#, 200, None, calls);
        let c = r.check(CLONE, "unbound.example.com").await;
        assert_eq!(c.status, BindingStatus::NotListed);
    }

    #[tokio::test]
    async fn check_reports_could_not_check_on_a_transport_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with("", 0, Some("timeout"), calls);
        let c = r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(
            c.status,
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::Transport {
                    detail: "timeout".to_string()
                }
            }
        );
    }

    #[tokio::test]
    async fn check_reports_could_not_check_on_a_non_2xx() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with("", 503, None, calls);
        let c = r.check(CLONE, "moh.gov.sg").await;
        assert!(matches!(
            c.status,
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::Transport { .. }
            }
        ));
    }

    #[tokio::test]
    async fn an_unconfigured_resolver_reports_no_resolver_not_a_verdict() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = BindingResolver::new(
            Box::new(StubTransport {
                body: String::new(),
                code: 200,
                fail: None,
                calls: calls.clone(),
            }),
            "",
            CacheTtl::default(),
        );
        let c = r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(
            c.status,
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::NoResolver
            }
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no query is fired with no resolver"
        );
    }

    #[tokio::test]
    async fn an_unusable_domain_reports_could_not_check_without_querying() {
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(r#"{"Status":0}"#, 200, None, calls.clone());
        let c = r.check(CLONE, "localhost").await;
        assert_eq!(
            c.status,
            BindingStatus::CouldNotCheck {
                reason: CouldNotCheckReason::Unresolvable
            }
        );
        assert!(c.txt_name.is_none());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_second_check_is_served_from_cache() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(
            &doh(0, &txt_answer(&name, "\\\"x\\\"")),
            200,
            None,
            calls.clone(),
        );

        let first = r.check(CLONE, "moh.gov.sg").await;
        let second = r.check(CLONE, "moh.gov.sg").await;

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one lookup, not one per render"
        );
        assert_eq!(first, second);
        assert_eq!(
            first.checked_at, second.checked_at,
            "a cached hit keeps the ORIGINAL observation time, so a surface cannot imply it just looked"
        );
    }

    #[tokio::test]
    async fn casing_of_the_query_does_not_split_the_cache() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(
            &doh(0, &txt_answer(&name, "\\\"x\\\"")),
            200,
            None,
            calls.clone(),
        );

        r.check(CLONE, "moh.gov.sg").await; // EIP-55 mixed case
        r.check(CLONE_LC, "MOH.GOV.SG").await; // lowercase addr, uppercase domain

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn clear_cache_forces_a_fresh_lookup() {
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(
            &doh(0, &txt_answer(&name, "\\\"x\\\"")),
            200,
            None,
            calls.clone(),
        );

        r.check(CLONE, "moh.gov.sg").await;
        r.clear_cache();
        r.check(CLONE, "moh.gov.sg").await;

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failure_is_cached_only_briefly() {
        // A transient blip must not pin "could not check" for the answer TTL.
        let calls = Arc::new(AtomicUsize::new(0));
        let r = BindingResolver::new(
            Box::new(StubTransport {
                body: String::new(),
                code: 0,
                fail: Some("timeout".into()),
                calls: calls.clone(),
            }),
            "https://doh.example/dns-query",
            CacheTtl {
                answer_max: Duration::from_secs(900),
                answer_min: Duration::from_secs(60),
                failure: Duration::from_millis(1),
            },
        );
        r.check(CLONE, "moh.gov.sg").await;
        tokio::time::sleep(Duration::from_millis(5)).await;
        r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the failure entry expired");
    }

    #[tokio::test]
    async fn a_short_zone_ttl_is_honoured_but_floored() {
        // The zone says 1s. We must not cache for 15 minutes (that would show a stale claim), and we
        // must not re-query every render either — the floor wins.
        let name = format!("{CLONE_LC}._dogtag.moh.gov.sg");
        let body = format!(
            r#"{{"Status":0,"Answer":[{{"name":"{name}","type":16,"TTL":1,"data":"\"x\""}}]}}"#
        );
        assert_eq!(answer_ttl(&body), Some(1));

        let calls = Arc::new(AtomicUsize::new(0));
        let r = resolver_with(&body, 200, None, calls.clone());
        let c = r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(c.answer_ttl, Some(1));

        tokio::time::sleep(Duration::from_millis(20)).await;
        r.check(CLONE, "moh.gov.sg").await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "floored to answer_min, not re-queried"
        );
    }

    #[test]
    fn answer_ttl_takes_the_minimum_and_tolerates_absence() {
        let body = r#"{"Status":0,"Answer":[{"type":16,"TTL":600},{"type":16,"TTL":120}]}"#;
        assert_eq!(answer_ttl(body), Some(120));
        assert_eq!(answer_ttl(r#"{"Status":3}"#), None);
        assert_eq!(answer_ttl("nonsense"), None);
    }

    #[test]
    fn is_verified_is_false_for_both_non_verified_states() {
        assert!(!BindingStatus::NotListed.is_verified());
        assert!(!BindingStatus::CouldNotCheck {
            reason: CouldNotCheckReason::NoResolver
        }
        .is_verified());
        assert!(BindingStatus::Verified {
            description: "x".into()
        }
        .is_verified());
    }

    #[test]
    fn status_serialises_with_a_discriminated_state_tag() {
        // The wire shape the web/mobile clients parse: a tagged union, so a client cannot accidentally
        // read a missing boolean as a pass.
        let v = serde_json::to_string(&BindingStatus::Verified {
            description: "hi".into(),
        })
        .unwrap();
        assert_eq!(v, r#"{"state":"verified","description":"hi"}"#);
        assert_eq!(
            serde_json::to_string(&BindingStatus::NotListed).unwrap(),
            r#"{"state":"notListed"}"#
        );
        let c = serde_json::to_string(&BindingStatus::CouldNotCheck {
            reason: CouldNotCheckReason::ResolverError { dns_status: 2 },
        })
        .unwrap();
        assert_eq!(
            c,
            r#"{"state":"couldNotCheck","reason":{"resolverError":{"dnsStatus":2}}}"#
        );
    }
}
