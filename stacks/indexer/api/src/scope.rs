//! Server-side scoping — the doctrine that "government sees ALL, each vet/groomer sees only its own
//! signer/clone" (arch §4.3/§4.4) is enforced **here**, not by trusting a client filter.
//!
//! A caller authenticates with a bearer token. The token resolves to a [`Scope`] — either
//! [`Scope::Unscoped`] (government oversight: every event) or [`Scope::Signers`] (a fixed set of
//! signer + clone addresses: that business's own on-chain activity). Client query params (`type`,
//! `recordType`, time, and an in-scope `signer`/`issuer`) only ever **narrow** within the token's
//! ceiling; they can never widen it. This is the load-bearing property: a scoped vet token can never
//! see another issuer's events, no matter what it puts in the query string.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::events::IndexedEvent;

/// The access ceiling a token grants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Government oversight: no filtering — every indexed event across all issuers.
    Unscoped,
    /// A single business: only events whose `actor` ∈ `signers` OR whose `clone` ∈ `clones`.
    Signers {
        signers: BTreeSet<String>,
        clones: BTreeSet<String>,
    },
}

impl Scope {
    /// Does `ev` fall within this scope? Unscoped admits everything; a signer-scope admits an event
    /// iff its acting signer is one of ours OR it pertains to one of our clones. Address comparison
    /// is lowercase-normalized (both the scope and the event store lowercase hex).
    pub fn admits(&self, ev: &IndexedEvent) -> bool {
        match self {
            Scope::Unscoped => true,
            Scope::Signers { signers, clones } => {
                if let Some(a) = &ev.actor {
                    if signers.contains(&a.to_ascii_lowercase()) {
                        return true;
                    }
                }
                if let Some(c) = &ev.clone {
                    if clones.contains(&c.to_ascii_lowercase()) {
                        return true;
                    }
                }
                false
            }
        }
    }

    pub fn is_unscoped(&self) -> bool {
        matches!(self, Scope::Unscoped)
    }
}

/// A named principal resolved from a bearer token: a human label (for logs/telemetry) + its scope.
#[derive(Clone, Debug)]
pub struct Principal {
    pub label: String,
    pub scope: Scope,
}

/// One configured token→scope entry (parsed from `INDEXER_SCOPES` JSON at boot).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScopeConfig {
    /// The bearer token the caller presents.
    pub token: String,
    /// Human label for this principal (e.g. "government-oversight", "The Seaport Paw").
    #[serde(default)]
    pub label: String,
    /// `true` for the unscoped government/oversight principal. When set, `signers`/`clones` are ignored.
    #[serde(default)]
    pub unscoped: bool,
    /// The signer addresses this principal may see (lowercased on load).
    #[serde(default)]
    pub signers: Vec<String>,
    /// The `DogTagIssuer` clone addresses this principal may see (lowercased on load).
    #[serde(default)]
    pub clones: Vec<String>,
}

/// The boot-time registry mapping bearer tokens to principals. Empty ⇒ no authenticated access
/// (every request 401s) — fail-closed, matching the government stack's "no token ⇒ refuse" posture.
#[derive(Clone, Debug, Default)]
pub struct ScopeRegistry {
    entries: Vec<(String, Principal)>,
}

impl ScopeRegistry {
    /// Build from parsed configs. Addresses are lowercase-normalized so scope checks are canonical.
    pub fn from_configs(cfgs: Vec<ScopeConfig>) -> Self {
        let entries = cfgs
            .into_iter()
            .filter(|c| !c.token.trim().is_empty())
            .map(|c| {
                let scope = if c.unscoped {
                    Scope::Unscoped
                } else {
                    Scope::Signers {
                        signers: c
                            .signers
                            .iter()
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        clones: c
                            .clones
                            .iter()
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty())
                            .collect(),
                    }
                };
                let label = if c.label.trim().is_empty() {
                    if c.unscoped {
                        "unscoped".to_string()
                    } else {
                        "scoped".to_string()
                    }
                } else {
                    c.label.trim().to_string()
                };
                (c.token.trim().to_string(), Principal { label, scope })
            })
            .collect();
        ScopeRegistry { entries }
    }

    /// Resolve a presented bearer token to its principal, if any. A constant-ish linear scan is fine:
    /// the registry holds a handful of operator tokens, not a user table.
    pub fn resolve(&self, token: &str) -> Option<&Principal> {
        self.entries
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, p)| p)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventType, IndexedEvent};

    fn ev(actor: Option<&str>, clone: Option<&str>) -> IndexedEvent {
        IndexedEvent {
            id: "0xtx:0".into(),
            event_type: EventType::RootIssued,
            contract: clone.unwrap_or("0xreg").into(),
            generation: "0xfactory".into(),
            block_number: 1,
            block_hash: "0xbh".into(),
            tx_hash: "0xtx".into(),
            log_index: 0,
            block_timestamp: 100,
            finality: crate::events::Finality::Finalized,
            actor: actor.map(|s| s.to_string()),
            clone: clone.map(|s| s.to_string()),
            record_type: None,
            name: None,
            root: None,
            dog_tag_id: None,
            purpose: None,
            nullifier: None,
            deadline: None,
            onchain_ts: None,
        }
    }

    #[test]
    fn unscoped_admits_everything() {
        assert!(Scope::Unscoped.admits(&ev(Some("0xaa"), None)));
        assert!(Scope::Unscoped.admits(&ev(None, None)));
    }

    #[test]
    fn signer_scope_admits_own_signer_and_clone_only() {
        let reg = ScopeRegistry::from_configs(vec![ScopeConfig {
            token: "vet-tok".into(),
            label: "Seaport Paw".into(),
            unscoped: false,
            signers: vec!["0xAA".into()],
            clones: vec!["0xCC".into()],
        }]);
        let scope = &reg.resolve("vet-tok").unwrap().scope;
        // own signer (case-insensitive)
        assert!(scope.admits(&ev(Some("0xaa"), None)));
        // own clone
        assert!(scope.admits(&ev(Some("0xother"), Some("0xcc"))));
        // someone else's signer + clone
        assert!(!scope.admits(&ev(Some("0xbb"), Some("0xdd"))));
        // factory event with no actor/clone in scope
        assert!(!scope.admits(&ev(None, None)));
    }

    #[test]
    fn empty_registry_resolves_nothing() {
        let reg = ScopeRegistry::default();
        assert!(reg.is_empty());
        assert!(reg.resolve("anything").is_none());
    }
}
