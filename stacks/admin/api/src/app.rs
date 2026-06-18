//! Application state + config + the server-side DOG_PROFILE VC build/wrap (impl §4.1).

use std::sync::Arc;

use dogtag_standard::wrap::{wrap_document, IssuerMeta, WrappedDoc};
use serde_json::Value;

use crate::auth::JwtKeys;
use crate::business::BusinessClient;
use crate::chain::ChainClient;
use crate::crypto::KeyVault;
use crate::dns::DnsChecker;
use crate::store::Store;

/// The DOG_PROFILE record type the central stack mints SBT profiles under.
pub const DOG_PROFILE: &str = "DOG_PROFILE";

/// Resolved central config (contract addresses + the admin's signer roles).
#[derive(Clone)]
pub struct Config {
    pub deployment_url: String,
    pub rpc_url: String,
    pub issuer_registry_addr: String,
    pub sbt_addr: String,
    /// the central protocol issuer identity (for IssuerMeta on the profile VC).
    pub issuer_name: String,
    pub issuer_domain: String,
    /// the documentStore the profile VC anchors to (the SBT contract acts as the profile store).
    pub profile_document_store: String,
    /// admin-session password (admin routes: businesses register, applications, deletions).
    pub admin_password: String,
    /// account index of the admin signer (WHITELIST_ADMIN + ISSUER/PROFILE_ISSUER roles).
    pub admin_signer_index: u32,
}

/// The shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub chain: Arc<dyn ChainClient>,
    pub dns: Arc<dyn DnsChecker>,
    pub business: Arc<dyn BusinessClient>,
    pub vault: Arc<dyn KeyVault>,
    pub jwt: JwtKeys,
    pub cfg: Arc<Config>,
}

/// Build the issuer metadata for the central profile issuer.
pub fn profile_issuer_meta(cfg: &Config) -> IssuerMeta {
    IssuerMeta {
        name: cfg.issuer_name.clone(),
        domain: cfg.issuer_domain.clone(),
        document_store: cfg.profile_document_store.clone(),
        record_type: DOG_PROFILE.to_string(),
    }
}

/// Build the DOG_PROFILE VC (typed-scalar `data`) from a pet, injecting the non-obfuscatable
/// `credentialSubject.dogTagId`. Mirrors the vet stack's `build_vc` shape.
pub fn build_profile_vc(name: &str, microchip: &crate::store::Microchip, dog_tag_id: &str) -> Value {
    let is_int = dog_tag_id.bytes().all(|b| b.is_ascii_digit()) && !dog_tag_id.is_empty();
    let tag = if is_int { 3 } else { 2 };
    serde_json::json!({
        "type": ["VerifiableCredential", "DogProfile"],
        "credentialSubject": {
            "dogTagId": { "tag": tag, "value": dog_tag_id },
            "name": { "tag": 2, "value": name },
            "microchip": {
                "code": { "tag": 2, "value": microchip.code },
                "standard": { "tag": 2, "value": microchip.standard },
                "implantDate": { "tag": 2, "value": microchip.implant_date },
                "bodyLocation": { "tag": 2, "value": microchip.body_location }
            }
        }
    })
}

/// Wrap a VC into a `WrappedDoc` using a cryptographically-random salt provider.
pub fn wrap_vc(issuer_meta: IssuerMeta, vc: &Value) -> Result<WrappedDoc, String> {
    let mut salt = || {
        let mut s = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut s);
        s
    };
    wrap_document(vc, issuer_meta, &mut salt).map_err(|e| format!("wrap: {e}"))
}
