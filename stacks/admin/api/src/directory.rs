//! Signer → business directory (the oversight "naming" join, plan PR-B §3.5).
//!
//! The indexed on-chain events (consumed via the oversight indexer, `crate::indexer`) carry signer /
//! clone *addresses*. To render "The Seaport Paw" instead of `0xAB…12` the admin portal joins each
//! address against the **admin business registry** - the one stack that owns business identity. This
//! is the AUTHORITATIVE source of that mapping: the indexer's own directory (`stacks/indexer` →
//! `directory.rs`) is a best-effort *copy* it pulls from this same admin API; the admin backend holds
//! the source of truth and re-enriches every feed it serves with it.
//!
//! Doctrine (plan Part 4): this reads ONLY business identity - the business `name`/`domain` and the
//! signer `addresses[]` an issuer application declared. It NEVER touches any role's PII Mongo (owner /
//! pet / consent records). The directory carries zero client PII, only business + signer identity.
//!
//! The join key is `IssuerApplication.addresses[]` (`store.rs`), the one-to-many set of on-chain
//! signer addresses an issuer entity declared. Each application is resolved to a business `name` by
//! matching `issuerEntityId → Business.business_id`, else `domain → Business.domain` (mirroring the
//! indexer's own join), falling back to the raw application `domain` when no business row exists yet.
//!
//! The directory is a proper indexed `HashMap<signer, entry>` built once from the store and keyed on
//! the actual signer addresses (plan §3.5).

use std::collections::HashMap;

use serde::Serialize;

use crate::store::{Business, IssuerApplication, Store};

/// One resolved signer → business directory entry. Non-PII: business + signer identity only.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DirectoryEntry {
    /// The on-chain signer address (lowercased `0x…`).
    pub signer: String,
    /// The business display name, if the application resolved to a known business (or the application
    /// `domain` as a fallback label). `None` only when neither a business nor a domain is known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business: Option<String>,
    /// The resolved `Business.business_id`, when the join landed on a registered business row.
    #[serde(rename = "businessId", skip_serializing_if = "Option::is_none")]
    pub business_id: Option<String>,
    /// The issuer entity this signer belongs to (`IssuerApplication.issuerEntityId`).
    pub entity: String,
    /// The recordType human labels the issuer application declared for this signer.
    #[serde(rename = "recordTypes")]
    pub record_types: Vec<String>,
    /// The `VERIFY:<purpose>` labels this signer may relay verifications for.
    #[serde(rename = "verifyPurposes")]
    pub verify_purposes: Vec<String>,
    /// The issuer application domain.
    pub domain: String,
    /// The originating issuer application id.
    #[serde(rename = "applicationId")]
    pub application_id: String,
    /// The application status (`pending` | `approved` | `rejected`) - so a consumer can distinguish a
    /// live signer from one whose whitelisting was never approved.
    pub status: String,
}

/// The signer → business directory: a pre-built index over the admin business registry + issuer
/// applications. Built once per lookup from the store (the registry is small; this matches the
/// existing O(n) `all_businesses()` scan pattern while giving O(1) resolution).
#[derive(Clone, Debug, Default)]
pub struct SignerDirectory {
    by_signer: HashMap<String, DirectoryEntry>,
}

impl SignerDirectory {
    /// Build the index by joining every issuer application's `addresses[]` to a business name.
    ///
    /// Resolution order for a business name (matching the indexer's directory join):
    ///   1. `issuerEntityId` == `Business.business_id`,
    ///   2. else application `domain` == `Business.domain`,
    ///   3. else the raw application `domain` as a bare label (or `None` if empty).
    ///
    /// When one signer address appears on multiple applications an **approved** application wins over a
    /// non-approved one (a live whitelisting is more authoritative than a stale/pending draft); among
    /// same-status collisions the last one seen wins (deterministic given a stable store order).
    pub fn build(businesses: &[Business], applications: &[IssuerApplication]) -> Self {
        let mut by_business_id: HashMap<&str, &str> = HashMap::new();
        let mut by_domain: HashMap<String, &str> = HashMap::new();
        for b in businesses {
            if b.name.trim().is_empty() {
                continue;
            }
            if !b.business_id.trim().is_empty() {
                by_business_id.insert(b.business_id.as_str(), b.name.as_str());
            }
            if !b.domain.trim().is_empty() {
                by_domain
                    .entry(b.domain.to_ascii_lowercase())
                    .or_insert(b.name.as_str());
            }
        }

        let mut by_signer: HashMap<String, DirectoryEntry> = HashMap::new();
        for app in applications {
            let resolved_id = by_business_id.get(app.issuer_entity_id.as_str());
            let resolved_name: Option<String> = resolved_id
                .map(|n| n.to_string())
                .or_else(|| {
                    by_domain
                        .get(&app.domain.to_ascii_lowercase())
                        .map(|n| n.to_string())
                })
                .or_else(|| {
                    if app.domain.trim().is_empty() {
                        None
                    } else {
                        Some(app.domain.clone())
                    }
                });
            // Only carry a concrete businessId when the entity actually matched a registered row.
            let business_id = if resolved_id.is_some() {
                Some(app.issuer_entity_id.clone())
            } else {
                None
            };

            for addr in &app.addresses {
                let key = addr.trim().to_ascii_lowercase();
                if key.is_empty() {
                    continue;
                }
                let candidate = DirectoryEntry {
                    signer: key.clone(),
                    business: resolved_name.clone(),
                    business_id: business_id.clone(),
                    entity: app.issuer_entity_id.clone(),
                    record_types: app.record_types.clone(),
                    verify_purposes: app.verify_purposes.clone(),
                    domain: app.domain.clone(),
                    application_id: app.application_id.clone(),
                    status: app.status.clone(),
                };
                match by_signer.get(&key) {
                    // An existing approved mapping is not overwritten by a non-approved one.
                    Some(existing)
                        if existing.status == "approved" && app.status != "approved" => {}
                    _ => {
                        by_signer.insert(key, candidate);
                    }
                }
            }
        }
        SignerDirectory { by_signer }
    }

    /// Build the directory from the live store (businesses + issuer applications).
    pub async fn from_store(store: &dyn Store) -> Self {
        let businesses = store.all_businesses().await;
        let applications = store.all_applications().await;
        Self::build(&businesses, &applications)
    }

    /// Resolve a signer/clone address to its full directory entry (case-insensitive).
    pub fn resolve(&self, addr: &str) -> Option<&DirectoryEntry> {
        self.by_signer.get(&addr.trim().to_ascii_lowercase())
    }

    /// The display name for an address, if resolvable - used to enrich the activity feed
    /// (`actorName`/`cloneName`).
    pub fn name(&self, addr: &str) -> Option<String> {
        self.resolve(addr).and_then(|e| e.business.clone())
    }

    /// Every directory entry, sorted by signer address for a stable listing.
    pub fn entries(&self) -> Vec<DirectoryEntry> {
        let mut out: Vec<DirectoryEntry> = self.by_signer.values().cloned().collect();
        out.sort_by(|a, b| a.signer.cmp(&b.signer));
        out
    }

    /// Number of indexed signer addresses.
    pub fn len(&self) -> usize {
        self.by_signer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_signer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn business(id: &str, name: &str, domain: &str) -> Business {
        Business {
            business_id: id.to_string(),
            kind: "vet".to_string(),
            name: name.to_string(),
            lat: 0.0,
            lng: 0.0,
            services: vec![],
            api_base_url: "http://x".to_string(),
            domain: domain.to_string(),
            document_stores: vec![],
            hmac_key_id: "k".to_string(),
            hmac_secret: "s".to_string(),
        }
    }

    fn application(
        app_id: &str,
        entity: &str,
        domain: &str,
        addresses: &[&str],
        record_types: &[&str],
        status: &str,
    ) -> IssuerApplication {
        IssuerApplication {
            application_id: app_id.to_string(),
            issuer_entity_id: entity.to_string(),
            addresses: addresses.iter().map(|s| s.to_string()).collect(),
            record_types: record_types.iter().map(|s| s.to_string()).collect(),
            verify_purposes: vec!["boarding_intake".to_string()],
            domain: domain.to_string(),
            usda_nan: None,
            license: None,
            document_store: "0xstore".to_string(),
            status: status.to_string(),
            whitelist_txs: vec![],
            dns_state: String::new(),
            dns_checked_at: 0,
            dns_state_at_approval: String::new(),
            dns_proceeded_unverified: false,
        }
    }

    #[test]
    fn joins_signer_to_business_name_by_entity_id() {
        let biz = vec![business("biz-1", "The Seaport Paw", "seaportpaw.example")];
        let apps = vec![application(
            "app-1",
            "biz-1",
            "seaportpaw.example",
            &["0xAAbb"],
            &["DOG_PROFILE"],
            "approved",
        )];
        let dir = SignerDirectory::build(&biz, &apps);
        let e = dir.resolve("0xaabb").expect("resolvable");
        assert_eq!(e.business.as_deref(), Some("The Seaport Paw"));
        assert_eq!(e.business_id.as_deref(), Some("biz-1"));
        assert_eq!(e.entity, "biz-1");
        assert_eq!(e.record_types, vec!["DOG_PROFILE"]);
        assert_eq!(e.status, "approved");
        // case-insensitive resolution + name() helper.
        assert_eq!(dir.name("0xAABB").as_deref(), Some("The Seaport Paw"));
    }

    #[test]
    fn falls_back_to_domain_match_then_bare_domain() {
        // no business_id match, but the domain matches a business row.
        let biz = vec![business("biz-9", "Domain Match Vet", "vet.example")];
        let apps = vec![
            application("app-d", "unknown-entity", "vet.example", &["0x01"], &["X"], "pending"),
            // neither entity nor domain matches any business row → bare domain label.
            application("app-b", "no-biz", "orphan.example", &["0x02"], &["Y"], "pending"),
        ];
        let dir = SignerDirectory::build(&biz, &apps);
        assert_eq!(dir.name("0x01").as_deref(), Some("Domain Match Vet"));
        assert_eq!(dir.resolve("0x01").unwrap().business_id, None);
        assert_eq!(dir.name("0x02").as_deref(), Some("orphan.example"));
    }

    #[test]
    fn approved_application_wins_over_pending_for_same_signer() {
        let biz = vec![
            business("biz-a", "Approved Co", "a.example"),
            business("biz-p", "Pending Co", "p.example"),
        ];
        let apps = vec![
            application("app-p", "biz-p", "p.example", &["0xdead"], &["P"], "pending"),
            application("app-a", "biz-a", "a.example", &["0xDEAD"], &["A"], "approved"),
        ];
        let dir = SignerDirectory::build(&biz, &apps);
        let e = dir.resolve("0xdead").unwrap();
        assert_eq!(e.business.as_deref(), Some("Approved Co"));
        assert_eq!(e.status, "approved");
    }

    #[test]
    fn unknown_address_resolves_to_none() {
        let dir = SignerDirectory::build(&[], &[]);
        assert!(dir.resolve("0xffff").is_none());
        assert!(dir.name("0xffff").is_none());
        assert!(dir.is_empty());
    }

    #[test]
    fn entries_are_sorted_and_blank_addresses_skipped() {
        let apps = vec![application(
            "app-1",
            "e",
            "d.example",
            &["0xbb", "", "0xaa"],
            &["R"],
            "approved",
        )];
        let dir = SignerDirectory::build(&[], &apps);
        let entries = dir.entries();
        assert_eq!(entries.len(), 2, "blank address skipped");
        assert_eq!(entries[0].signer, "0xaa");
        assert_eq!(entries[1].signer, "0xbb");
    }
}
