//! DNS legitimacy check (architecture §13.3 H): onboarding MUST verify a business's DNS TXT record
//! BEFORE whitelisting its issuer addresses. The expected TXT proves the operator controls the domain
//! they registered (internal consistency of domain+contract+QR is NOT legitimacy — the registry, gated
//! by this check, is the trust root for "is this a real vet").
//!
//! # The two DogTag TXT conventions, and why both exist
//!
//! * **This one — apex, value-encoded:** `<domain> IN TXT "dogtag-verify=<address>"`. Used by this
//!   onboarding gate and by the phone's pre-disclosure groomer binding. The address is in the VALUE.
//! * **The issuer↔domain binding — see `dogtag-dns-rs`:**
//!   `<address>._dogtag.<domain> IN TXT "<free-form description>"`. The address is in the NAME
//!   precisely so the VALUE can stay free-form for the domain owner's own description.
//!
//! They are deliberately separate records with different shapes. This one is load-bearing for the
//! onboarding gate and the mobile groomer check, and is NOT changed by the binding work.
//!
//! # No synthesized verdicts
//!
//! There is no stub or fixture behind this trait in the shipped path. `DohDnsChecker` is the only
//! implementation the binary constructs, and it delegates to `dogtag_dns_rs`, whose single
//! Status-branching rule keeps "the domain publishes nothing" (`Ok(false)`) distinct from "the lookup
//! did not resolve" (`Err`) — the previous inline client collapsed SERVFAIL, a timeout and a genuine
//! absence into one `Ok(false)`.
//!
//! Historically `DNS_CHECK=skip` installed a checker that returned an unconditional `Ok(true)`: a
//! FABRICATED pass on the very gate that decides whether an organisation is legitimate enough to be
//! whitelisted. That is why this module is shaped the way it is. `DNS_CHECK` is retired (accepted only
//! so `main` can warn an operator who still sets it), and there is no enforce-vs-observe switch to
//! replace it: the gate is ADVISORY for every deployment. The lookup always runs for real, a
//! non-verified observation does not block whitelisting but requires the admin's explicit
//! `proceedWithoutDns` and is recorded on the application, and this module never invents a pass. Test
//! doubles live in the test crates, not here.

use async_trait::async_trait;
use dogtag_dns_rs::{BindingResolver, CouldNotCheckReason};

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("dns lookup failed: {0}")]
    Lookup(String),
}

/// Verifies a domain publishes the expected DogTag verification TXT record.
///
/// Three outcomes, and callers MUST keep them apart: `Ok(true)` published, `Ok(false)` definitively
/// absent, `Err` the lookup did not resolve and therefore proves nothing either way.
#[async_trait]
pub trait DnsChecker: Send + Sync {
    /// `Ok(true)` iff `domain` publishes a TXT record whose value contains `expected_token`.
    /// `expected_token` is the deployment's challenge string (e.g. `dogtag-verify=<documentStore>`).
    async fn txt_contains(&self, domain: &str, expected_token: &str) -> Result<bool, DnsError>;
}

/// The canonical TXT challenge a business must publish to prove control: `dogtag-verify=<documentStore>`.
pub fn expected_txt(document_store: &str) -> String {
    format!("dogtag-verify={}", document_store.to_lowercase())
}

// --------------------------------------------------------------------------------------------
// DohDnsChecker — DNS-over-HTTPS (RFC 8484 JSON form), via the shared resolver.
// --------------------------------------------------------------------------------------------

pub struct DohDnsChecker {
    resolver: BindingResolver,
}

impl DohDnsChecker {
    /// `endpoint` is a DoH JSON endpoint. An empty one makes every lookup report a lookup FAILURE
    /// rather than a pass or a definitive absence.
    pub fn new(endpoint: impl Into<String>) -> Self {
        DohDnsChecker {
            resolver: BindingResolver::production(endpoint),
        }
    }
}

impl Default for DohDnsChecker {
    fn default() -> Self {
        Self::new("https://cloudflare-dns.com/dns-query")
    }
}

#[async_trait]
impl DnsChecker for DohDnsChecker {
    async fn txt_contains(&self, domain: &str, expected_token: &str) -> Result<bool, DnsError> {
        let name = domain.trim().trim_end_matches('.');
        if name.is_empty() {
            return Err(DnsError::Lookup("empty domain".to_string()));
        }
        match self.resolver.lookup_txt(name).await {
            // Records were really fetched, so present-or-absent is a real observation.
            Ok((records, _ttl)) => Ok(records.iter().any(|r| r.value.contains(expected_token))),
            // The lookup did not resolve. Reporting this as `Ok(false)` would be the very conflation
            // this trait's contract forbids.
            Err(reason) => Err(DnsError::Lookup(describe(&reason))),
        }
    }
}

fn describe(reason: &CouldNotCheckReason) -> String {
    match reason {
        CouldNotCheckReason::Unresolvable => "domain unusable".to_string(),
        CouldNotCheckReason::ResolverError { dns_status } => format!("resolver rcode {dns_status}"),
        CouldNotCheckReason::Transport { detail } => detail.clone(),
        CouldNotCheckReason::NoResolver => "no DoH resolver configured".to_string(),
    }
}
