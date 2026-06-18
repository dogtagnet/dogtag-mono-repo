//! Central DB collections (impl §9.1) behind a `Store` trait. `MemStore` (Arc<RwLock>, used by tests)
//! and an optional `MongoStore` (behind the `mongo` feature) implement the same trait.
//!
//! Collections: owners, sessions, pets, credentials, share_refs, jti (one-time), businesses,
//! issuer_applications, appointments, consents, consent_receipts, verification_records, deletions.
//!
//! PII fields (owner profile, credential data, verification consent copies, receipts) are stored as
//! crypto-shred `Sealed` blobs (crypto.rs): erasure destroys the DEK + deletes the row.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::crypto::Sealed;

// ---- owners / auth ----

/// A mobile user. `password_hash` is the salted-hash store; `profile_pii` is the crypto-shred blob.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Owner {
    pub owner_id: String,
    pub email: String,
    pub password_hash: String,
    /// self-custodial / embedded-MPC wallet address the SBT is minted to (§4.1).
    pub wallet_address: String,
    pub push_token: Option<String>,
    /// encrypted owner PII (name etc.) under a per-record DEK — erasure shreds this.
    pub profile_pii: Option<Sealed>,
}

// ---- pets ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Microchip {
    pub code: String,
    pub standard: String,
    #[serde(rename = "implantDate")]
    pub implant_date: String,
    #[serde(rename = "bodyLocation")]
    pub body_location: String,
}

/// One dated, unit-bearing weight measurement (DOG_PROFILE `weightHistory[i]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeightEntry {
    /// "kg" | "lb"
    pub unit: String,
    /// decimal string (e.g. "22.7") — NEVER a float (precision/leading-zero loss).
    pub value: String,
    #[serde(rename = "measuredOn")]
    pub measured_on: String,
}

/// Optional DOG_PROFILE identity fields supplied at `POST /v1/pets` (or defaulted at mint).
/// All optional on input; `build_profile_vc` fills sensible defaults so the wrapped VC always
/// passes `validate_schema` (impl §1.6 / CHANGESPEC §0/§1.8).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PetProfile {
    /// taxonomic species, defaults to "Canis lupus familiaris".
    #[serde(default)]
    pub species: Option<String>,
    /// VBO breed id, e.g. "VBO:0200798".
    #[serde(rename = "breedVbo", default)]
    pub breed_vbo: Option<String>,
    /// human breed label.
    #[serde(rename = "breedLabel", default)]
    pub breed_label: Option<String>,
    /// "male" | "female".
    #[serde(default)]
    pub sex: Option<String>,
    /// "intact" | "neutered" | "spayed".
    #[serde(rename = "neuterStatus", default)]
    pub neuter_status: Option<String>,
    /// ISO date "YYYY-MM-DD".
    #[serde(rename = "dateOfBirth", default)]
    pub date_of_birth: Option<String>,
    /// unit-bearing, dated weight history.
    #[serde(rename = "weightHistory", default)]
    pub weight_history: Vec<WeightEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pet {
    pub pet_id: String,
    pub owner_id: String,
    pub name: String,
    pub microchip: Microchip,
    /// optional DOG_PROFILE identity fields (species/breed/sex/neuterStatus/dateOfBirth/weightHistory).
    #[serde(default)]
    pub profile: PetProfile,
    /// assigned at mint (non-personal random/sequential id — NEVER a hash of the microchip).
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: Option<String>,
    pub root: Option<String>,
    pub mint_tx: Option<String>,
    /// encrypted credential salts/data blob (the wrapped profile VC) — erasure shreds it.
    pub sealed_doc: Option<Sealed>,
}

// ---- credentials ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credential {
    pub credential_id: String,
    pub owner_id: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    pub root: String,
    /// encrypted wrapped-doc reference (salts/data) — erasure shreds it.
    pub sealed_doc: Sealed,
}

// ---- share refs (one-time JWT bookkeeping) ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShareRef {
    pub ref_id: String,
    pub credential_id: String,
    pub owner_id: String,
}

// ---- businesses / discovery ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Business {
    pub business_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub lat: f64,
    pub lng: f64,
    pub services: Vec<String>,
    #[serde(rename = "apiBaseUrl")]
    pub api_base_url: String,
    pub domain: String,
    #[serde(rename = "documentStores")]
    pub document_stores: Vec<String>,
    #[serde(rename = "hmacKeyId")]
    pub hmac_key_id: String,
    /// HMAC shared secret (server-side only; never returned in discovery).
    #[serde(rename = "hmacSecret")]
    pub hmac_secret: String,
}

// ---- issuer applications (whitelisting) ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct License {
    pub number: String,
    pub jurisdiction: String,
    pub expiry: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerApplication {
    pub application_id: String,
    #[serde(rename = "issuerEntityId")]
    pub issuer_entity_id: String,
    /// MULTIPLE signer addresses per issuer entity (one-to-many).
    pub addresses: Vec<String>,
    /// recordType human labels (keccak256'd on-chain).
    #[serde(rename = "recordTypes")]
    pub record_types: Vec<String>,
    pub domain: String,
    #[serde(rename = "usdaNan")]
    pub usda_nan: Option<String>,
    pub license: Option<License>,
    /// the documentStore used as the DNS TXT challenge subject.
    #[serde(rename = "documentStore")]
    pub document_store: String,
    pub status: String, // "pending" | "approved" | "rejected"
    /// per (address,recordType) tx hashes recorded on approval.
    pub whitelist_txs: Vec<String>,
}

// ---- appointments ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Appointment {
    pub appointment_id: String,
    #[serde(rename = "businessId")]
    pub business_id: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    pub owner_id: String,
    pub slot: String,
    pub rev: u64,
    pub state: String, // REQUESTED | CONFIRMED | ... terminal
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
}

// ---- consent / retention / erasure ----

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Consent {
    pub consent_id: String,
    pub owner_id: String,
    pub purpose: String,
    #[serde(rename = "lawfulBasis")]
    pub lawful_basis: String,
    #[serde(rename = "grantedAt")]
    pub granted_at: u64,
    pub withdrawn: bool,
}

/// Tamper-evident consent receipt (off-chain, deletable — erasure scope). PII-bearing fields sealed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsentReceipt {
    pub receipt_id: String,
    pub owner_id: String,
    pub hash: String,
    #[serde(rename = "issuedAt")]
    pub issued_at: u64,
    /// encrypted receipt body — erasure shreds it.
    pub sealed: Sealed,
}

/// A relayed verification record (impl §4.1) — the off-chain copy of a VerificationConsent + receipt.
/// Deletable under erasure (the on-chain Verified tuple persists but is unlinkable).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub record_id: String,
    pub owner_id: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    pub purpose: String,
    pub relayer: String,
    pub mode: String,
    pub status: String,
    /// encrypted consent + receipt body — erasure shreds it.
    pub sealed: Sealed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Deletion {
    pub request_id: String,
    pub owner_id: String,
    pub scope: String, // "all" | "credentials" | "verifications" | ...
    #[serde(rename = "dueBy")]
    pub due_by: u64,
    pub status: String, // "pending" | "completed"
}

// --------------------------------------------------------------------------------------------
// Store trait
// --------------------------------------------------------------------------------------------

#[async_trait]
pub trait Store: Send + Sync {
    // owners + sessions
    async fn put_owner(&self, o: Owner);
    async fn get_owner(&self, id: &str) -> Option<Owner>;
    async fn get_owner_by_email(&self, email: &str) -> Option<Owner>;
    async fn put_session(&self, token: String, owner_id: String);
    async fn session_owner(&self, token: &str) -> Option<String>;
    async fn put_admin_session(&self, token: String);
    async fn has_admin_session(&self, token: &str) -> bool;

    // pets
    async fn put_pet(&self, p: Pet);
    async fn get_pet(&self, id: &str) -> Option<Pet>;
    async fn pets_of_owner(&self, owner_id: &str) -> Vec<Pet>;
    /// true if any pet already uses this microchip code (uniqueness enforcement).
    async fn microchip_exists(&self, code: &str) -> bool;
    /// Atomic: reserve a microchip code; true if newly reserved (unique), false if taken.
    async fn reserve_microchip(&self, code: &str) -> bool;
    /// Allocate the next non-personal sequential dogTagId.
    async fn next_dog_tag_id(&self) -> u64;

    // credentials
    async fn put_credential(&self, c: Credential);
    async fn get_credential(&self, id: &str) -> Option<Credential>;
    async fn credentials_of_owner(&self, owner_id: &str) -> Vec<Credential>;

    // share refs + jti
    async fn put_share_ref(&self, s: ShareRef);
    async fn get_share_ref(&self, ref_id: &str) -> Option<ShareRef>;
    /// Atomic one-time consume: true if the jti was unused (now consumed), false if already used.
    async fn consume_jti(&self, jti: &str) -> bool;

    // businesses
    async fn put_business(&self, b: Business);
    async fn get_business(&self, id: &str) -> Option<Business>;
    async fn all_businesses(&self) -> Vec<Business>;

    // issuer applications
    async fn put_application(&self, a: IssuerApplication);
    async fn get_application(&self, id: &str) -> Option<IssuerApplication>;
    async fn all_applications(&self) -> Vec<IssuerApplication>;

    // appointments — central is the SOLE rev allocator
    async fn put_appointment(&self, a: Appointment);
    async fn get_appointment(&self, id: &str) -> Option<Appointment>;
    async fn appointments_updated_since(&self, owner_id: &str, since: u64) -> Vec<Appointment>;
    /// Atomically allocate the next monotonic rev for `appointment_id` and apply `update` to the
    /// appointment under the same lock (or create it). Returns the resulting appointment. The closure
    /// is given (current_appt, allocated_rev). Returns None if the appointment is absent and `create`
    /// is None.
    async fn alloc_rev_and_apply(
        &self,
        appointment_id: &str,
        f: RevApply,
    ) -> Option<Appointment>;

    // consent / receipts / verification records / deletions
    async fn put_consent(&self, c: Consent);
    async fn get_consent(&self, id: &str) -> Option<Consent>;
    async fn consents_of_owner(&self, owner_id: &str) -> Vec<Consent>;
    async fn put_consent_receipt(&self, r: ConsentReceipt);
    async fn receipts_of_owner(&self, owner_id: &str) -> Vec<ConsentReceipt>;
    async fn put_verification_record(&self, v: VerificationRecord);
    async fn verification_records_of_owner(&self, owner_id: &str) -> Vec<VerificationRecord>;
    async fn put_deletion(&self, d: Deletion);
    async fn due_deletions(&self, now: u64) -> Vec<Deletion>;
    async fn update_deletion(&self, d: Deletion);

    // erasure deletes (rows; DEK shredding is done by the caller via the vault)
    async fn delete_credential(&self, id: &str);
    async fn delete_verification_record(&self, id: &str);
    async fn delete_consent_receipt(&self, id: &str);
    async fn clear_owner_pii(&self, owner_id: &str);
    async fn clear_pet_doc(&self, pet_id: &str);
}

/// A closure that, given the current appointment (if any) and the freshly-allocated rev, returns the
/// appointment to persist (or None to abort). Boxed `'static` so the trait stays object-safe and the
/// async_trait future owns it.
pub type RevApply = Box<dyn FnOnce(Option<Appointment>, u64) -> Option<Appointment> + Send + 'static>;

// --------------------------------------------------------------------------------------------
// MemStore
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    owners: HashMap<String, Owner>,
    email_index: HashMap<String, String>,
    sessions: HashMap<String, String>,
    admin_sessions: std::collections::HashSet<String>,
    pets: HashMap<String, Pet>,
    microchips: std::collections::HashSet<String>,
    dog_tag_seq: u64,
    credentials: HashMap<String, Credential>,
    share_refs: HashMap<String, ShareRef>,
    jtis: std::collections::HashSet<String>,
    businesses: HashMap<String, Business>,
    applications: HashMap<String, IssuerApplication>,
    appointments: HashMap<String, Appointment>,
    consents: HashMap<String, Consent>,
    receipts: HashMap<String, ConsentReceipt>,
    verification_records: HashMap<String, VerificationRecord>,
    deletions: HashMap<String, Deletion>,
}

#[derive(Clone, Default)]
pub struct MemStore {
    inner: Arc<RwLock<MemInner>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Store for MemStore {
    async fn put_owner(&self, o: Owner) {
        let mut g = self.inner.write().unwrap();
        g.email_index.insert(o.email.to_lowercase(), o.owner_id.clone());
        g.owners.insert(o.owner_id.clone(), o);
    }
    async fn get_owner(&self, id: &str) -> Option<Owner> {
        self.inner.read().unwrap().owners.get(id).cloned()
    }
    async fn get_owner_by_email(&self, email: &str) -> Option<Owner> {
        let g = self.inner.read().unwrap();
        let id = g.email_index.get(&email.to_lowercase())?;
        g.owners.get(id).cloned()
    }
    async fn put_session(&self, token: String, owner_id: String) {
        self.inner.write().unwrap().sessions.insert(token, owner_id);
    }
    async fn session_owner(&self, token: &str) -> Option<String> {
        self.inner.read().unwrap().sessions.get(token).cloned()
    }
    async fn put_admin_session(&self, token: String) {
        self.inner.write().unwrap().admin_sessions.insert(token);
    }
    async fn has_admin_session(&self, token: &str) -> bool {
        self.inner.read().unwrap().admin_sessions.contains(token)
    }

    async fn put_pet(&self, p: Pet) {
        self.inner.write().unwrap().pets.insert(p.pet_id.clone(), p);
    }
    async fn get_pet(&self, id: &str) -> Option<Pet> {
        self.inner.read().unwrap().pets.get(id).cloned()
    }
    async fn pets_of_owner(&self, owner_id: &str) -> Vec<Pet> {
        self.inner
            .read()
            .unwrap()
            .pets
            .values()
            .filter(|p| p.owner_id == owner_id)
            .cloned()
            .collect()
    }
    async fn microchip_exists(&self, code: &str) -> bool {
        self.inner.read().unwrap().microchips.contains(code)
    }
    async fn reserve_microchip(&self, code: &str) -> bool {
        self.inner.write().unwrap().microchips.insert(code.to_string())
    }
    async fn next_dog_tag_id(&self) -> u64 {
        let mut g = self.inner.write().unwrap();
        g.dog_tag_seq += 1;
        g.dog_tag_seq
    }

    async fn put_credential(&self, c: Credential) {
        self.inner.write().unwrap().credentials.insert(c.credential_id.clone(), c);
    }
    async fn get_credential(&self, id: &str) -> Option<Credential> {
        self.inner.read().unwrap().credentials.get(id).cloned()
    }
    async fn credentials_of_owner(&self, owner_id: &str) -> Vec<Credential> {
        self.inner
            .read()
            .unwrap()
            .credentials
            .values()
            .filter(|c| c.owner_id == owner_id)
            .cloned()
            .collect()
    }

    async fn put_share_ref(&self, s: ShareRef) {
        self.inner.write().unwrap().share_refs.insert(s.ref_id.clone(), s);
    }
    async fn get_share_ref(&self, ref_id: &str) -> Option<ShareRef> {
        self.inner.read().unwrap().share_refs.get(ref_id).cloned()
    }
    async fn consume_jti(&self, jti: &str) -> bool {
        self.inner.write().unwrap().jtis.insert(jti.to_string())
    }

    async fn put_business(&self, b: Business) {
        self.inner.write().unwrap().businesses.insert(b.business_id.clone(), b);
    }
    async fn get_business(&self, id: &str) -> Option<Business> {
        self.inner.read().unwrap().businesses.get(id).cloned()
    }
    async fn all_businesses(&self) -> Vec<Business> {
        self.inner.read().unwrap().businesses.values().cloned().collect()
    }

    async fn put_application(&self, a: IssuerApplication) {
        self.inner.write().unwrap().applications.insert(a.application_id.clone(), a);
    }
    async fn get_application(&self, id: &str) -> Option<IssuerApplication> {
        self.inner.read().unwrap().applications.get(id).cloned()
    }
    async fn all_applications(&self) -> Vec<IssuerApplication> {
        self.inner.read().unwrap().applications.values().cloned().collect()
    }

    async fn put_appointment(&self, a: Appointment) {
        self.inner.write().unwrap().appointments.insert(a.appointment_id.clone(), a);
    }
    async fn get_appointment(&self, id: &str) -> Option<Appointment> {
        self.inner.read().unwrap().appointments.get(id).cloned()
    }
    async fn appointments_updated_since(&self, owner_id: &str, since: u64) -> Vec<Appointment> {
        self.inner
            .read()
            .unwrap()
            .appointments
            .values()
            .filter(|a| a.owner_id == owner_id && a.updated_at >= since)
            .cloned()
            .collect()
    }
    async fn alloc_rev_and_apply(&self, appointment_id: &str, f: RevApply) -> Option<Appointment> {
        // hold the write lock across read-current + allocate-rev + apply -> rev never collides even
        // under concurrent creates/events (central is the sole monotonic rev allocator).
        let mut g = self.inner.write().unwrap();
        let current = g.appointments.get(appointment_id).cloned();
        let next_rev = current.as_ref().map(|a| a.rev + 1).unwrap_or(1);
        let result = f(current, next_rev)?;
        g.appointments.insert(result.appointment_id.clone(), result.clone());
        Some(result)
    }

    async fn put_consent(&self, c: Consent) {
        self.inner.write().unwrap().consents.insert(c.consent_id.clone(), c);
    }
    async fn get_consent(&self, id: &str) -> Option<Consent> {
        self.inner.read().unwrap().consents.get(id).cloned()
    }
    async fn consents_of_owner(&self, owner_id: &str) -> Vec<Consent> {
        self.inner
            .read()
            .unwrap()
            .consents
            .values()
            .filter(|c| c.owner_id == owner_id)
            .cloned()
            .collect()
    }
    async fn put_consent_receipt(&self, r: ConsentReceipt) {
        self.inner.write().unwrap().receipts.insert(r.receipt_id.clone(), r);
    }
    async fn receipts_of_owner(&self, owner_id: &str) -> Vec<ConsentReceipt> {
        self.inner
            .read()
            .unwrap()
            .receipts
            .values()
            .filter(|r| r.owner_id == owner_id)
            .cloned()
            .collect()
    }
    async fn put_verification_record(&self, v: VerificationRecord) {
        self.inner.write().unwrap().verification_records.insert(v.record_id.clone(), v);
    }
    async fn verification_records_of_owner(&self, owner_id: &str) -> Vec<VerificationRecord> {
        self.inner
            .read()
            .unwrap()
            .verification_records
            .values()
            .filter(|v| v.owner_id == owner_id)
            .cloned()
            .collect()
    }
    async fn put_deletion(&self, d: Deletion) {
        self.inner.write().unwrap().deletions.insert(d.request_id.clone(), d);
    }
    async fn due_deletions(&self, now: u64) -> Vec<Deletion> {
        self.inner
            .read()
            .unwrap()
            .deletions
            .values()
            .filter(|d| d.status == "pending" && d.due_by <= now)
            .cloned()
            .collect()
    }
    async fn update_deletion(&self, d: Deletion) {
        self.inner.write().unwrap().deletions.insert(d.request_id.clone(), d);
    }

    async fn delete_credential(&self, id: &str) {
        self.inner.write().unwrap().credentials.remove(id);
    }
    async fn delete_verification_record(&self, id: &str) {
        self.inner.write().unwrap().verification_records.remove(id);
    }
    async fn delete_consent_receipt(&self, id: &str) {
        self.inner.write().unwrap().receipts.remove(id);
    }
    async fn clear_owner_pii(&self, owner_id: &str) {
        if let Some(o) = self.inner.write().unwrap().owners.get_mut(owner_id) {
            o.profile_pii = None;
        }
    }
    async fn clear_pet_doc(&self, pet_id: &str) {
        if let Some(p) = self.inner.write().unwrap().pets.get_mut(pet_id) {
            p.sealed_doc = None;
        }
    }
}
