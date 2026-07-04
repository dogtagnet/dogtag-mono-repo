//! Signer → business-name directory (the oversight "naming" join, arch §4.3).
//!
//! The indexed events carry signer/clone *addresses*. To render "The Seaport Paw" instead of
//! `0xAB…12`, the feed joins each address against the **admin business directory** — the one stack
//! that owns business identity. This reads only business identity (names + the signer addresses on
//! issuer applications); it NEVER touches any role's PII Mongo, preserving the doctrine that the
//! indexer reads the chain + the admin directory and nothing else.
//!
//! Two layers, best-effort ("name where possible"):
//!   1. **Static seeds** (`INDEXER_DIRECTORY` JSON: `{ "0xabc…": "DogTag Government Authority" }`) —
//!      operator-authoritative; always applied; works offline / in demo.
//!   2. **Admin-API enrichment** (optional, `ADMIN_API_BASE`): periodically GETs the admin
//!      `/v1/businesses` (public) + `/v1/issuer-applications` (admin-token) and joins each
//!      application signer address → the business name (matched on the shared `domain`, else the
//!      application domain). Any failure degrades to static-only; the feed never hard-fails on it.

use std::collections::HashMap;
use std::sync::RwLock;

use serde::Deserialize;

pub struct Directory {
    /// Operator-provided authoritative names (lowercased address -> name).
    static_map: HashMap<String, String>,
    /// Admin-API-derived names (lowercased address -> name), refreshed in the background.
    dynamic: RwLock<HashMap<String, String>>,
    admin_base: Option<String>,
    admin_token: Option<String>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct BusinessesResp {
    #[serde(default)]
    businesses: Vec<BusinessRow>,
}
#[derive(Deserialize)]
struct BusinessRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
    #[serde(rename = "businessId", default)]
    business_id: String,
}

#[derive(Deserialize)]
struct ApplicationsResp {
    #[serde(default)]
    applications: Vec<ApplicationRow>,
}
#[derive(Deserialize)]
struct ApplicationRow {
    #[serde(default)]
    addresses: Vec<String>,
    #[serde(default)]
    domain: String,
    #[serde(rename = "issuerEntityId", default)]
    issuer_entity_id: String,
}

impl Directory {
    pub fn new(
        static_map: HashMap<String, String>,
        admin_base: Option<String>,
        admin_token: Option<String>,
    ) -> Self {
        let static_map = static_map
            .into_iter()
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v))
            .collect();
        Directory {
            static_map,
            dynamic: RwLock::new(HashMap::new()),
            admin_base: admin_base.filter(|s| !s.trim().is_empty()),
            admin_token: admin_token.filter(|s| !s.trim().is_empty()),
            http: reqwest::Client::new(),
        }
    }

    /// Resolve an address to a business name. Static seeds win over admin-API-derived names.
    pub fn resolve(&self, addr: &str) -> Option<String> {
        let a = addr.to_ascii_lowercase();
        if let Some(n) = self.static_map.get(&a) {
            return Some(n.clone());
        }
        self.dynamic.read().unwrap().get(&a).cloned()
    }

    /// `true` when admin-API enrichment is configured (so `main` should spawn the refresh loop).
    pub fn has_admin(&self) -> bool {
        self.admin_base.is_some()
    }

    /// Rebuild the dynamic map from the admin API. Fully best-effort: any HTTP/parse error leaves the
    /// previous map intact and logs a warning.
    pub async fn refresh(&self) {
        let Some(base) = &self.admin_base else {
            return;
        };
        let base = base.trim_end_matches('/');

        // Businesses (public): domain -> name, businessId -> name.
        let mut by_domain: HashMap<String, String> = HashMap::new();
        let mut by_entity: HashMap<String, String> = HashMap::new();
        match self
            .http
            .get(format!("{base}/v1/businesses"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => match resp.json::<BusinessesResp>().await {
                Ok(b) => {
                    for row in b.businesses {
                        if !row.name.trim().is_empty() {
                            if !row.domain.trim().is_empty() {
                                by_domain.insert(row.domain.to_ascii_lowercase(), row.name.clone());
                            }
                            if !row.business_id.trim().is_empty() {
                                by_entity.insert(row.business_id.clone(), row.name.clone());
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("directory: businesses parse failed: {e}"),
            },
            Err(e) => tracing::warn!("directory: businesses fetch failed: {e}"),
        }

        // Applications (admin-token): addresses[] -> name. This is the only admin surface that ties a
        // signer address to a business today (adminportal §3.5). Without a token it 401s; we skip.
        let mut next: HashMap<String, String> = HashMap::new();
        if let Some(token) = &self.admin_token {
            match self
                .http
                .get(format!("{base}/v1/issuer-applications"))
                .bearer_auth(token)
                .send()
                .await
                .and_then(|r| r.error_for_status())
            {
                Ok(resp) => match resp.json::<ApplicationsResp>().await {
                    Ok(a) => {
                        for app in a.applications {
                            let name = by_entity
                                .get(&app.issuer_entity_id)
                                .cloned()
                                .or_else(|| by_domain.get(&app.domain.to_ascii_lowercase()).cloned())
                                .unwrap_or_else(|| {
                                    if app.domain.trim().is_empty() {
                                        "unknown issuer".to_string()
                                    } else {
                                        app.domain.clone()
                                    }
                                });
                            for addr in app.addresses {
                                next.insert(addr.to_ascii_lowercase(), name.clone());
                            }
                        }
                    }
                    Err(e) => tracing::warn!("directory: applications parse failed: {e}"),
                },
                Err(e) => tracing::warn!("directory: applications fetch failed: {e}"),
            }
        }

        if !next.is_empty() {
            *self.dynamic.write().unwrap() = next;
            tracing::info!(
                "directory refreshed: {} signer→business mappings",
                self.dynamic.read().unwrap().len()
            );
        }
    }
}
