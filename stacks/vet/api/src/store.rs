//! Persistence abstraction (impl §11.4). `MemStore` (in-memory, used by tests) and an optional
//! `MongoStore` (production, behind the `mongo` feature) implement the same `Store` trait.
//!
//! The store holds: issuance/verification records, verify sessions, one-time JWT jti set,
//! issuer settings (signing mode), and keystore metadata (addresses/labels only — NEVER the seed).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// A prepared/issued credential record. `prepared_calldata` pins the exact `issue(bytes32)` calldata
/// so confirm can bind the broadcast tx to THIS draft (impl §11.6).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub record_id: String,
    pub record_type: String,
    pub dog_tag_id: String,
    /// The wrapped document (dogtag-standard WrappedDoc), serialized.
    pub wrapped_doc: serde_json::Value,
    /// The single Poseidon root R (`0x..` hex32) == doc.signature.merkleRoot.
    pub root: String,
    /// hex calldata for issue(root), pinned at prepare time.
    pub prepared_calldata: String,
    /// the issuer clone address (documentStore) this record anchors to.
    pub issuer_addr: String,
    pub status: RecordStatus,
    pub tx_hash: Option<String>,
    pub confirmed_tx_hash: Option<String>,
    pub signer_address: Option<String>,
    pub signing_mode: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordStatus {
    Prepared,
    Confirming,
    Issued,
    Revoked,
}

/// A verifier session (impl §3.9).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifySession {
    pub session_id: String,
    pub relayer: String,
    pub purpose: String,
    pub record_type: String,
    pub mode: String, // "normal" | "zk"
    pub challenge: String,
    pub status: String, // "pending" | "recorded"
    pub tx_hash: Option<String>,
    /// the consumed verification nullifier (set on `recorded`, primarily the ZK path).
    #[serde(default)]
    pub nullifier: Option<String>,
}

// --------------------------------------------------------------------------------------------
// DOG_PROFILE (SBT) issuance — pet record + owner identity + the device-bind session.
// The vet ISSUES dog tags: the operator starts a session (allocating a dogTagId + a one-time QR
// token); the device scans the QR, posts its wallet + a signature, and the vet mints the SBT.
// Pet record structs are ported from the admin stack (stacks/admin/api/src/store.rs).
// --------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Microchip {
    pub code: String,
    pub standard: String,
    #[serde(rename = "implantDate", default)]
    pub implant_date: String,
    #[serde(rename = "bodyLocation", default)]
    pub body_location: String,
}

/// The owner's official identity, entered by the vet operator at session-start and signed into the
/// DOG_PROFILE `credentialSubject.ownerIdentity`. The schema requires the keys present as strings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerIdentity {
    pub country_of_identification: String,
    pub identification: String,
    pub name: String,
}

/// One dated, unit-bearing weight measurement (DOG_PROFILE `weightHistory[i]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WeightEntry {
    pub unit: String,
    /// decimal string (e.g. "22.7") — NEVER a float (precision/leading-zero loss).
    pub value: String,
    #[serde(rename = "measuredOn")]
    pub measured_on: String,
}

/// Optional DOG_PROFILE identity fields. All optional on input; `build_profile_vc` fills defaults.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PetProfile {
    #[serde(default)]
    pub species: Option<String>,
    #[serde(rename = "breedVbo", default)]
    pub breed_vbo: Option<String>,
    #[serde(rename = "breedLabel", default)]
    pub breed_label: Option<String>,
    #[serde(default)]
    pub sex: Option<String>,
    #[serde(rename = "neuterStatus", default)]
    pub neuter_status: Option<String>,
    #[serde(rename = "dateOfBirth", default)]
    pub date_of_birth: Option<String>,
    #[serde(rename = "weightHistory", default)]
    pub weight_history: Vec<WeightEntry>,
}

/// A VET-side DOG_PROFILE issuance session. Created at `POST /profiles/issue/session/start` with a
/// fresh one-time QR token; consumed at `POST /profiles/issue/bind` when the device posts its wallet.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileIssueSession {
    /// stable id the portal polls (`GET /profiles/issue/session/{id}`). NOT the one-time QR token.
    pub session_id: String,
    /// the allocated non-personal dogTagId (decimal string).
    pub dog_tag_id: String,
    pub owner_identity: OwnerIdentity,
    /// the pet record: { name, microchip, profile fields } as posted by the operator.
    pub pet_name: String,
    pub microchip: Microchip,
    pub profile: PetProfile,
    /// "pending" -> "bound".
    pub status: String,
    pub created_at: u64,
    /// set on bind: the device wallet the SBT was minted to.
    #[serde(default)]
    pub wallet_address: Option<String>,
    /// set on bind: the DOG_PROFILE merkle root (== SBT profileRoot[dogTagId]).
    #[serde(default)]
    pub root: Option<String>,
    /// set on bind: the mint txHash.
    #[serde(default)]
    pub tx_hash: Option<String>,
}

/// Persisted per-issuer settings (impl §3.8).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerSettings {
    pub signing_mode: String, // "wallet" | "backend"
}

impl Default for IssuerSettings {
    fn default() -> Self {
        IssuerSettings {
            signing_mode: "backend".to_string(),
        }
    }
}

/// Keystore metadata — addresses + labels ONLY. The encrypted seed is held separately.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct KeystoreMeta {
    /// derived accounts: index -> (address, label)
    pub accounts: Vec<AccountMeta>,
    pub state: String, // "uninitialized" | "initialized"
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountMeta {
    pub index: u32,
    pub address: String,
    pub label: String,
}

/// The custody blob: the age-encrypted (scrypt passphrase) BIP-39 seed/mnemonic + meta.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CustodyBlob {
    /// age-encrypted ciphertext (armored).
    pub encrypted_seed: Vec<u8>,
    pub meta: KeystoreMeta,
}

// --------------------------------------------------------------------------------------------
// Calendar sync + appointment replica (Phase 7, impl §3.6 / §3.7).
// --------------------------------------------------------------------------------------------

/// The business-side appointment REPLICA. The central backend is the system-of-record; the business
/// keeps an idempotent replica keyed by `appointment_id` + central-assigned `rev` (NEVER bumped
/// locally — the business is not a rev allocator).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApptReplica {
    pub appointment_id: String,
    #[serde(rename = "businessId")]
    pub business_id: String,
    #[serde(rename = "dogTagId")]
    pub dog_tag_id: String,
    pub slot: String,
    /// central-assigned monotonic revision. Apply-if-newer; an older rev arriving is `409 stale_rev`.
    pub rev: u64,
    pub state: String, // REQUESTED | CONFIRMED | DECLINED | CANCELLED | COMPLETED | NO_SHOW
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
}

/// One row of the `gcal_event_map` mapping table (appointmentId <-> googleEventId, etag, rev, dir).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GcalEventMap {
    pub appointment_id: String,
    pub google_event_id: String,
    /// the etag Google returned for OUR last write — the PRIMARY echo discriminator (§13.3).
    pub etag: String,
    /// the appointment rev this mirror reflects.
    pub rev: u64,
    /// "out" (platform -> google) | "in" (google -> platform, e.g. external busy block).
    pub direction: String,
}

/// The `gcal_sync_state`: the persisted incremental `syncToken` + watch channel identifiers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GcalSyncState {
    pub sync_token: Option<String>,
    pub channel_id: Option<String>,
    pub resource_id: Option<String>,
    /// unix seconds the watch channel was (re)created — the ~6-day renewal cron reads this.
    pub channel_created_at: u64,
    /// the stored Google refresh token (opaque/encrypted at rest in production).
    pub refresh_token: Option<String>,
}

/// The persistence trait. All methods async so MongoStore is a drop-in.
#[async_trait]
pub trait Store: Send + Sync {
    // ---- records ----
    async fn put_record(&self, r: Record);
    async fn get_record(&self, id: &str) -> Option<Record>;
    async fn update_record(&self, r: Record);
    /// true if any record currently has status == prepared.
    async fn has_prepared(&self) -> bool;
    /// idempotency lookup: record already confirmed at this txHash.
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record>;

    // ---- verify sessions ----
    async fn put_session(&self, s: VerifySession);
    async fn get_session(&self, id: &str) -> Option<VerifySession>;
    async fn update_session(&self, s: VerifySession);

    // ---- jwt jti (one-time) ----
    /// Atomic consume: returns true if the jti was unused (now consumed), false if already used.
    async fn consume_jti(&self, jti: &str) -> bool;

    // ---- share tokens (short one-time QR token -> record) ----
    /// Store a short one-time share token mapping to `record_id`, expiring at unix-seconds `exp`.
    async fn put_share_token(&self, token: &str, record_id: &str, exp: u64);
    /// Atomically REMOVE the token (one-time consume) and return its `record_id` iff it exists and
    /// has not expired. A missing/expired token returns `None` (and is purged if expired).
    async fn take_share_token(&self, token: &str) -> Option<String>;

    // ---- export tokens (short one-time EXPORT QR token -> verify session) ----
    /// Store a short one-time export token mapping to `session_id`, expiring at unix-seconds `exp`.
    /// Mirrors the share-token pattern but resolves to a verify (export) session instead of a record.
    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64);
    /// NON-consuming lookup: return the export token's `session_id` iff it exists and has not
    /// expired. Used by `GET /x/{token}` (resolve) and the status poll — the token is NOT consumed
    /// here (consume happens only on consent submit). An expired token returns `None`.
    async fn peek_export_token(&self, token: &str) -> Option<String>;
    /// Atomically REMOVE the export token (one-time consume) and return its `session_id` iff it
    /// exists and has not expired. Used by the consent SUBMIT for replay protection.
    async fn take_export_token(&self, token: &str) -> Option<String>;

    // ---- DOG_PROFILE issuance: dogTagId counter + bind sessions + one-time bind tokens ----
    /// Allocate the next non-personal dogTagId (atomic monotonic counter). NEVER a hash of the
    /// microchip. Returns a fresh integer each call.
    async fn next_dog_tag_id(&self) -> u64;
    /// Store/replace a profile-issue session keyed by `session_id`.
    async fn put_profile_session(&self, s: ProfileIssueSession);
    /// Non-consuming lookup by `session_id` (the portal status poll).
    async fn get_profile_session(&self, session_id: &str) -> Option<ProfileIssueSession>;
    async fn update_profile_session(&self, s: ProfileIssueSession);
    /// Store a one-time bind token mapping to `session_id`, expiring at unix-seconds `exp`.
    async fn put_bind_token(&self, token: &str, session_id: &str, exp: u64);
    /// NON-consuming lookup: the bind token's `session_id` iff present and unexpired (`GET /p/{token}`).
    async fn peek_bind_token(&self, token: &str) -> Option<String>;
    /// Atomically REMOVE the bind token (one-time consume) and return its `session_id` iff present and
    /// unexpired. Used by `POST /profiles/issue/bind` for replay protection.
    async fn take_bind_token(&self, token: &str) -> Option<String>;

    // ---- issuer settings ----
    async fn get_settings(&self) -> IssuerSettings;
    async fn put_settings(&self, s: IssuerSettings);

    // ---- custody ----
    async fn get_custody(&self) -> Option<CustodyBlob>;
    async fn put_custody(&self, blob: CustodyBlob);

    // ---- operator sessions (bearer tokens) ----
    async fn put_op_session(&self, token: String);
    async fn has_op_session(&self, token: &str) -> bool;

    // ---- imported client cache (import/pull) ----
    async fn upsert_client_cache(&self, dog_tag_id: String, doc: serde_json::Value);
    async fn get_client_cache(&self, dog_tag_id: &str) -> Option<serde_json::Value>;

    // ---- appointment replica (Phase 7, §3.7) ----
    async fn get_appt(&self, id: &str) -> Option<ApptReplica>;
    async fn put_appt(&self, a: ApptReplica);
    async fn appts_updated_since(&self, since: u64) -> Vec<ApptReplica>;
    /// Idempotency-Key dedupe: true if newly recorded (proceed), false if already seen (replay).
    async fn record_idempotency_key(&self, key: &str) -> bool;

    // ---- gcal mapping table + sync state (Phase 7, §3.6) ----
    async fn put_gcal_map(&self, m: GcalEventMap);
    async fn get_gcal_map_by_appt(&self, appointment_id: &str) -> Option<GcalEventMap>;
    async fn get_gcal_map_by_event(&self, google_event_id: &str) -> Option<GcalEventMap>;
    async fn all_gcal_maps(&self) -> Vec<GcalEventMap>;
    async fn delete_gcal_map_by_event(&self, google_event_id: &str);
    /// Wipe the ENTIRE gcal mirror (mapping table) — called on an HTTP-410 full resync.
    async fn wipe_gcal_mirror(&self);
    async fn get_sync_state(&self) -> GcalSyncState;
    async fn put_sync_state(&self, s: GcalSyncState);
}

// --------------------------------------------------------------------------------------------
// MemStore — Arc<RwLock<...>>; used by tests (no live Mongo required).
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    records: HashMap<String, Record>,
    sessions: HashMap<String, VerifySession>,
    jtis: std::collections::HashSet<String>,
    /// short one-time share tokens: token -> (record_id, exp unix-seconds).
    share_tokens: HashMap<String, (String, u64)>,
    /// short one-time EXPORT tokens: token -> (session_id, exp unix-seconds).
    export_tokens: HashMap<String, (String, u64)>,
    /// DOG_PROFILE issuance: monotonic dogTagId counter.
    dog_tag_seq: u64,
    /// profile-issue sessions keyed by session_id.
    profile_sessions: HashMap<String, ProfileIssueSession>,
    /// one-time bind tokens: token -> (session_id, exp unix-seconds).
    bind_tokens: HashMap<String, (String, u64)>,
    settings: Option<IssuerSettings>,
    custody: Option<CustodyBlob>,
    op_sessions: std::collections::HashSet<String>,
    client_cache: HashMap<String, serde_json::Value>,
    // Phase 7
    appts: HashMap<String, ApptReplica>,
    idempotency_keys: std::collections::HashSet<String>,
    gcal_maps: HashMap<String, GcalEventMap>, // keyed by google_event_id
    sync_state: GcalSyncState,
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
    async fn put_record(&self, r: Record) {
        self.inner
            .write()
            .unwrap()
            .records
            .insert(r.record_id.clone(), r);
    }
    async fn get_record(&self, id: &str) -> Option<Record> {
        self.inner.read().unwrap().records.get(id).cloned()
    }
    async fn update_record(&self, r: Record) {
        self.inner
            .write()
            .unwrap()
            .records
            .insert(r.record_id.clone(), r);
    }
    async fn has_prepared(&self) -> bool {
        self.inner
            .read()
            .unwrap()
            .records
            .values()
            .any(|r| r.status == RecordStatus::Prepared)
    }
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record> {
        self.inner
            .read()
            .unwrap()
            .records
            .values()
            .find(|r| r.confirmed_tx_hash.as_deref() == Some(tx_hash))
            .cloned()
    }

    async fn put_session(&self, s: VerifySession) {
        self.inner
            .write()
            .unwrap()
            .sessions
            .insert(s.session_id.clone(), s);
    }
    async fn get_session(&self, id: &str) -> Option<VerifySession> {
        self.inner.read().unwrap().sessions.get(id).cloned()
    }
    async fn update_session(&self, s: VerifySession) {
        self.inner
            .write()
            .unwrap()
            .sessions
            .insert(s.session_id.clone(), s);
    }

    async fn consume_jti(&self, jti: &str) -> bool {
        // atomic under the write lock: insert returns true iff newly inserted.
        self.inner.write().unwrap().jtis.insert(jti.to_string())
    }

    async fn put_share_token(&self, token: &str, record_id: &str, exp: u64) {
        self.inner
            .write()
            .unwrap()
            .share_tokens
            .insert(token.to_string(), (record_id.to_string(), exp));
    }
    async fn take_share_token(&self, token: &str) -> Option<String> {
        // atomic remove under the write lock == one-time consume.
        let mut inner = self.inner.write().unwrap();
        let (record_id, exp) = inner.share_tokens.remove(token)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // expired tokens are consumed-on-read and treated as missing.
        if now > exp {
            None
        } else {
            Some(record_id)
        }
    }

    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64) {
        self.inner
            .write()
            .unwrap()
            .export_tokens
            .insert(token.to_string(), (session_id.to_string(), exp));
    }
    async fn peek_export_token(&self, token: &str) -> Option<String> {
        let inner = self.inner.read().unwrap();
        let (session_id, exp) = inner.export_tokens.get(token)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > *exp {
            None
        } else {
            Some(session_id.clone())
        }
    }
    async fn take_export_token(&self, token: &str) -> Option<String> {
        // atomic remove under the write lock == one-time consume.
        let mut inner = self.inner.write().unwrap();
        let (session_id, exp) = inner.export_tokens.remove(token)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            Some(session_id)
        }
    }

    async fn next_dog_tag_id(&self) -> u64 {
        let mut g = self.inner.write().unwrap();
        g.dog_tag_seq += 1;
        g.dog_tag_seq
    }
    async fn put_profile_session(&self, s: ProfileIssueSession) {
        self.inner
            .write()
            .unwrap()
            .profile_sessions
            .insert(s.session_id.clone(), s);
    }
    async fn get_profile_session(&self, session_id: &str) -> Option<ProfileIssueSession> {
        self.inner
            .read()
            .unwrap()
            .profile_sessions
            .get(session_id)
            .cloned()
    }
    async fn update_profile_session(&self, s: ProfileIssueSession) {
        self.inner
            .write()
            .unwrap()
            .profile_sessions
            .insert(s.session_id.clone(), s);
    }
    async fn put_bind_token(&self, token: &str, session_id: &str, exp: u64) {
        self.inner
            .write()
            .unwrap()
            .bind_tokens
            .insert(token.to_string(), (session_id.to_string(), exp));
    }
    async fn peek_bind_token(&self, token: &str) -> Option<String> {
        let inner = self.inner.read().unwrap();
        let (session_id, exp) = inner.bind_tokens.get(token)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > *exp {
            None
        } else {
            Some(session_id.clone())
        }
    }
    async fn take_bind_token(&self, token: &str) -> Option<String> {
        // atomic remove under the write lock == one-time consume.
        let mut inner = self.inner.write().unwrap();
        let (session_id, exp) = inner.bind_tokens.remove(token)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            Some(session_id)
        }
    }

    async fn get_settings(&self) -> IssuerSettings {
        self.inner
            .read()
            .unwrap()
            .settings
            .clone()
            .unwrap_or_default()
    }
    async fn put_settings(&self, s: IssuerSettings) {
        self.inner.write().unwrap().settings = Some(s);
    }

    async fn get_custody(&self) -> Option<CustodyBlob> {
        self.inner.read().unwrap().custody.clone()
    }
    async fn put_custody(&self, blob: CustodyBlob) {
        self.inner.write().unwrap().custody = Some(blob);
    }

    async fn put_op_session(&self, token: String) {
        self.inner.write().unwrap().op_sessions.insert(token);
    }
    async fn has_op_session(&self, token: &str) -> bool {
        self.inner.read().unwrap().op_sessions.contains(token)
    }

    async fn upsert_client_cache(&self, dog_tag_id: String, doc: serde_json::Value) {
        self.inner
            .write()
            .unwrap()
            .client_cache
            .insert(dog_tag_id, doc);
    }
    async fn get_client_cache(&self, dog_tag_id: &str) -> Option<serde_json::Value> {
        self.inner
            .read()
            .unwrap()
            .client_cache
            .get(dog_tag_id)
            .cloned()
    }

    // ---- appointment replica ----
    async fn get_appt(&self, id: &str) -> Option<ApptReplica> {
        self.inner.read().unwrap().appts.get(id).cloned()
    }
    async fn put_appt(&self, a: ApptReplica) {
        self.inner
            .write()
            .unwrap()
            .appts
            .insert(a.appointment_id.clone(), a);
    }
    async fn appts_updated_since(&self, since: u64) -> Vec<ApptReplica> {
        let mut v: Vec<ApptReplica> = self
            .inner
            .read()
            .unwrap()
            .appts
            .values()
            .filter(|a| a.updated_at >= since)
            .cloned()
            .collect();
        v.sort_by_key(|a| a.updated_at);
        v
    }
    async fn record_idempotency_key(&self, key: &str) -> bool {
        // atomic under the write lock: insert returns true iff newly inserted.
        self.inner
            .write()
            .unwrap()
            .idempotency_keys
            .insert(key.to_string())
    }

    // ---- gcal mapping + sync state ----
    async fn put_gcal_map(&self, m: GcalEventMap) {
        self.inner
            .write()
            .unwrap()
            .gcal_maps
            .insert(m.google_event_id.clone(), m);
    }
    async fn get_gcal_map_by_appt(&self, appointment_id: &str) -> Option<GcalEventMap> {
        self.inner
            .read()
            .unwrap()
            .gcal_maps
            .values()
            .find(|m| m.appointment_id == appointment_id)
            .cloned()
    }
    async fn get_gcal_map_by_event(&self, google_event_id: &str) -> Option<GcalEventMap> {
        self.inner
            .read()
            .unwrap()
            .gcal_maps
            .get(google_event_id)
            .cloned()
    }
    async fn all_gcal_maps(&self) -> Vec<GcalEventMap> {
        self.inner
            .read()
            .unwrap()
            .gcal_maps
            .values()
            .cloned()
            .collect()
    }
    async fn delete_gcal_map_by_event(&self, google_event_id: &str) {
        self.inner
            .write()
            .unwrap()
            .gcal_maps
            .remove(google_event_id);
    }
    async fn wipe_gcal_mirror(&self) {
        self.inner.write().unwrap().gcal_maps.clear();
    }
    async fn get_sync_state(&self) -> GcalSyncState {
        self.inner.read().unwrap().sync_state.clone()
    }
    async fn put_sync_state(&self, s: GcalSyncState) {
        self.inner.write().unwrap().sync_state = s;
    }
}
