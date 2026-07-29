//! Admin business directory snapshot + signer → business-name join.
//!
//! The indexed events carry signer/clone *addresses*. To render "The Seaport Paw" instead of
//! `0xAB…12`, the feed joins each address against the **admin business directory** — the one stack
//! that owns business identity. This reads only business identity (names + the signer addresses on
//! issuer applications); it NEVER touches any role's PII Mongo, preserving the doctrine that the
//! indexer reads the chain + the admin directory and nothing else.
//!
//! The same bare admin `/v1/businesses` read is also retained as a complete provider snapshot for the
//! indexer's public discovery route. Static signer-name seeds and issuer applications are deliberately
//! NOT provider rows: neither source carries the provider kind, contacts, services, or optional
//! location, and inventing those fields would turn a naming hint into a directory entry.
//!
//! Two layers, best-effort ("name where possible"):
//!   1. **Static seeds** (`INDEXER_DIRECTORY` JSON: `{ "0xabc…": "DogTag Government Authority" }`) —
//!      operator-authoritative; always applied; works offline / in demo.
//!   2. **Admin-API enrichment** (optional, `ADMIN_API_BASE`): periodically GETs the admin
//!      `/v1/businesses` (public) + `/v1/issuer-applications` (admin-token) and joins each
//!      application signer address → the business name (matched on the shared `domain`, else the
//!      application domain). Failed refreshes retain the last successful snapshots; the feed never
//!      hard-fails on enrichment.

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const ADMIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Directory {
    /// Operator-provided authoritative names (lowercased address -> name).
    static_map: HashMap<String, String>,
    /// Admin-API-derived names (lowercased address -> name), refreshed in the background.
    dynamic: RwLock<HashMap<String, String>>,
    /// Last successfully decoded full provider set.
    ///
    /// `None` means no successful admin read has happened and must surface as unavailable. `Some([])`
    /// means the source was read successfully and authoritatively contained no providers.
    businesses: RwLock<Option<Vec<BusinessRow>>>,
    admin_base: Option<String>,
    admin_token: Option<String>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct BusinessesResp {
    businesses: Vec<BusinessRow>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BusinessRow {
    pub(crate) business_id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) name: String,
    /// `None` is an ordinary contact-only provider, serialized as explicit JSON `null`.
    #[serde(default)]
    pub(crate) geo: Option<BusinessGeo>,
    #[serde(default)]
    pub(crate) contact: BusinessContact,
    pub(crate) services: Vec<String>,
    pub(crate) api_base_url: String,
    pub(crate) domain: String,
    pub(crate) document_stores: Vec<String>,
    pub(crate) hmac_key_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub(crate) struct BusinessGeo {
    pub(crate) lat: f64,
    pub(crate) lng: f64,
}

impl BusinessGeo {
    pub(crate) fn is_valid(self) -> bool {
        self.lat.is_finite()
            && self.lng.is_finite()
            && (-90.0..=90.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lng)
    }
}

/// The five public BUSINESS contact channels from the admin directory. These are the provider's
/// published contact details, never an owner's or operator's personal contact data.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct BusinessContact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) phone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) whatsapp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) telegram: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) website: Option<String>,
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
            businesses: RwLock::new(None),
            admin_base: admin_base.filter(|s| !s.trim().is_empty()),
            admin_token: admin_token.filter(|s| !s.trim().is_empty()),
            // The one directory refresh task must not be held forever by an accepting-but-silent
            // admin server. This bounds connect, response, and body reads for each source request.
            http: reqwest::Client::builder()
                .timeout(ADMIN_REQUEST_TIMEOUT)
                .build()
                .expect("build admin directory HTTP client"),
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

    /// Clone the last complete provider snapshot.
    ///
    /// The clone makes the request independent of the refresh lock: a slow client can never block the
    /// background source refresh. At today's directory size this is tiny; the route comment records
    /// the scale at which a different transport strategy becomes a live question.
    pub(crate) fn businesses(&self) -> Option<Vec<BusinessRow>> {
        self.businesses.read().unwrap().clone()
    }

    /// Refresh the provider and signer-name snapshots from the admin API.
    ///
    /// A failed/invalid business read leaves both previous snapshots intact. Once a complete business
    /// read succeeds, its provider rows may advance independently; a later application failure still
    /// preserves the previous signer-name map.
    pub async fn refresh(&self) {
        let Some(base) = &self.admin_base else {
            return;
        };
        let base = base.trim_end_matches('/');

        // Businesses (public): domain -> name, businessId -> name.
        let business_names = match self
            .http
            .get(format!("{base}/v1/businesses"))
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => match resp.json::<BusinessesResp>().await {
                Ok(b) => {
                    if b.businesses
                        .iter()
                        .filter_map(|row| row.geo)
                        .any(|geo| !geo.is_valid())
                    {
                        tracing::warn!(
                            "directory: businesses response contains an invalid coordinate; \
                             preserving the previous complete provider snapshot"
                        );
                        None
                    } else {
                        let mut by_domain: HashMap<String, String> = HashMap::new();
                        let mut by_entity: HashMap<String, String> = HashMap::new();
                        for row in &b.businesses {
                            if !row.name.trim().is_empty() {
                                if !row.domain.trim().is_empty() {
                                    by_domain
                                        .insert(row.domain.to_ascii_lowercase(), row.name.clone());
                                }
                                if !row.business_id.trim().is_empty() {
                                    by_entity.insert(row.business_id.clone(), row.name.clone());
                                }
                            }
                        }
                        // Swap only after the WHOLE response decoded and validated. Empty is a real,
                        // successful snapshot and must replace a previously non-empty one.
                        *self.businesses.write().unwrap() = Some(b.businesses);
                        Some((by_domain, by_entity))
                    }
                }
                Err(e) => {
                    tracing::warn!("directory: businesses parse failed: {e}");
                    None
                }
            },
            Err(e) => {
                tracing::warn!("directory: businesses fetch failed: {e}");
                None
            }
        };
        let Some((by_domain, by_entity)) = business_names else {
            return;
        };

        // Applications (admin-token): addresses[] -> name. This is the only admin surface that ties a
        // signer address to a business today (adminportal §3.5). Without a token it 401s; we skip.
        let Some(token) = &self.admin_token else {
            return;
        };
        let applications = match self
            .http
            .get(format!("{base}/v1/issuer-applications"))
            .bearer_auth(token)
            .send()
            .await
            .and_then(|r| r.error_for_status())
        {
            Ok(resp) => match resp.json::<ApplicationsResp>().await {
                Ok(a) => a.applications,
                Err(e) => {
                    tracing::warn!("directory: applications parse failed: {e}");
                    return;
                }
            },
            Err(e) => {
                tracing::warn!("directory: applications fetch failed: {e}");
                return;
            }
        };

        let mut next: HashMap<String, String> = HashMap::new();
        for app in applications {
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
        let count = next.len();
        *self.dynamic.write().unwrap() = next;
        tracing::info!("directory refreshed: {count} signer→business mappings");
    }
}
