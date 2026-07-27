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
///
/// The record bundles the credential data with its **immutable on-chain proof** — the anchoring tx
/// hash, the block it mined into, the DogTagIssuer clone (contract) address, and a ready-to-click
/// block-explorer link — so the operator can always trace a record back to the chain and re-verify
/// it. On a metadata update the on-chain-derived fields are never mutated; only `label`/`notes`
/// (off-chain metadata) and the invalidation status are editable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub record_id: String,
    pub record_type: String,
    pub dog_tag_id: String,
    /// The wrapped document (dogtag-standard WrappedDoc), serialized. Carries the anchored document
    /// hash — IMMUTABLE.
    pub wrapped_doc: serde_json::Value,
    /// The single Poseidon root R (`0x..` hex32) == doc.signature.merkleRoot. IMMUTABLE.
    pub root: String,
    /// hex calldata for issue(root), pinned at prepare time.
    pub prepared_calldata: String,
    /// the issuer clone (contract) address (documentStore) this record anchors to. IMMUTABLE.
    /// == the `protocol` block's `issuerClone` (M7 §4.2).
    pub issuer_addr: String,
    /// M7 provenance mirror (§4.2), populated from the `WrappedDoc.protocol` block - persisted BESIDE
    /// `R`, never inside it. IMMUTABLE once set. `chain_id`/`protocol_version`/`verification_registry`
    /// are known at prepare; `issuer_signer` is the on-chain `clone.issuedBy[R]`, learned at confirm
    /// (== the `signer_address` derived from the `RootIssued` log). `Option`/defaulted so pre-M7 rows
    /// still load.
    #[serde(default)]
    pub chain_id: Option<u64>,
    #[serde(default)]
    pub protocol_version: Option<String>,
    #[serde(default)]
    pub verification_registry: Option<String>,
    /// The signer that issued (== `clone.issuedBy[R]`). Mirrors `signer_address` (which is set from the
    /// same `RootIssued` log at confirm); kept as a distinct provenance column so all three stacks
    /// expose a uniform `issuer_signer`, and to decouple the provenance mirror from the signing-flow field.
    #[serde(default)]
    pub issuer_signer: Option<String>,
    pub status: RecordStatus,
    pub tx_hash: Option<String>,
    pub confirmed_tx_hash: Option<String>,
    pub signer_address: Option<String>,
    pub signing_mode: Option<String>,
    /// The block number the anchoring (confirmed) tx mined into. IMMUTABLE once set.
    #[serde(default)]
    pub block_number: Option<u64>,
    /// Ready-to-click block-explorer link for the anchoring tx (`https://explorer.roax.net/tx/<hash>`).
    #[serde(default)]
    pub explorer_url: Option<String>,
    /// Unix seconds the record was first prepared.
    #[serde(default)]
    pub created_at: u64,
    /// Unix seconds the record was last touched (issuance, metadata edit, invalidation).
    #[serde(default)]
    pub updated_at: u64,
    /// Operator-editable off-chain label (never anchored).
    #[serde(default)]
    pub label: Option<String>,
    /// Operator-editable off-chain notes (never anchored).
    #[serde(default)]
    pub notes: Option<String>,
    /// Set when revoked on-chain: the revoke tx hash + its block + explorer link. IMMUTABLE once set.
    #[serde(default)]
    pub revoked_tx_hash: Option<String>,
    #[serde(default)]
    pub revoked_block_number: Option<u64>,
    #[serde(default)]
    pub revoke_explorer_url: Option<String>,
    /// Unix seconds the record was invalidated (revoked or expired), if it has been.
    #[serde(default)]
    pub invalidated_at: Option<u64>,
    /// Optional human reason for the invalidation.
    #[serde(default)]
    pub invalidation_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordStatus {
    Prepared,
    Confirming,
    Issued,
    Revoked,
    /// Marked expired off-chain (validity window lapsed) — the on-chain anchor is untouched, the
    /// record stays visible + verifiable. A non-destructive state change, never a row removal.
    Expired,
}

/// A verifier session (impl §3.9).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifySession {
    pub session_id: String,
    pub relayer: String,
    pub purpose: String,
    pub record_type: String,
    pub challenge: String,
    pub status: String, // "pending" | "recording" | "recorded" | "error"
    pub tx_hash: Option<String>,
    /// the consumed verification nullifier (set on `recorded`, primarily the ZK path).
    #[serde(default)]
    pub nullifier: Option<String>,
    /// Unix seconds the verification session/audit row was created.
    #[serde(default)]
    pub created_at: u64,
    /// Unix seconds the verification session/audit row last changed state.
    #[serde(default)]
    pub updated_at: u64,
    /// D1: the identity-leaf keyPaths the owner chose to REVEAL alongside this consent proof
    /// (verified against the anchored `R` before recording). KeyPaths only - the disclosed VALUES
    /// are shown to the verifying operator, never stored. Empty when nothing was disclosed.
    #[serde(default)]
    pub disclosed_key_paths: Vec<String>,
    /// OPTIONAL business context: the shop [`Appointment`] this verification was started FROM, plus
    /// the client/pet it resolved to. Set at session start when the operator starts the verification
    /// from an appointment; `None` for an ad-hoc verification. These NEVER reach the owner's phone —
    /// `GET /x/{token}` deliberately does not expose them (they are the shop's own customer records).
    /// `#[serde(default)]` so rows written before this field existed still deserialize.
    #[serde(default)]
    pub appointment_id: Option<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub pet_id: Option<String>,
}

// --------------------------------------------------------------------------------------------
// DOG_PROFILE issuance — pet record + owner identity + the device-bind session.
// The vet ISSUES dog tags: the operator starts a session (allocating a dogTagId + a one-time QR
// token); the device scans the QR, posts its owner-hidden root, and the vet anchors + mints it.
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

/// The owner's official identity, entered by the vet operator at session-start and committed into
/// `R` as hidden, selectively-disclosable `owner.identity.*` attribute leaves (D1). The schema
/// requires the keys present as strings.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerIdentity {
    pub country_of_identification: String,
    pub identification: String,
    pub name: String,
}

/// One D1 identity attribute leaf the DEVICE must fold into `R`: `{keyPath, salt, value}` with the
/// salt generated fresh by the VET at session-start (16 random bytes). The vet retains this triple
/// on the session row - it is what the bind-time attestation-integrity gate recomputes the leaf
/// from - and hands the same triple to the device via `/p/<token>` so the device can fold the leaf
/// and later disclose it. High-entropy issuer salts are what keep low-entropy identity values
/// (e.g. a country, ~200 possibilities) safe inside the public root.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityLeaf {
    /// `owner.identity.*` - the sanctioned identity namespace (never a reserved keyPath).
    pub key_path: String,
    /// `0x` + 32 hex chars (16 bytes).
    pub salt_hex: String,
    /// TypeTag byte (2 = String for every v1 identity attribute).
    pub tag: u8,
    pub value: String,
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

/// Optional DOG_PROFILE pet fields collected at session start.
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
/// fresh one-time QR token; consumed at `POST /profiles/issue/custodial-bind` when the device posts
/// its owner-hidden root.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfileIssueSession {
    /// stable id the portal polls (`GET /profiles/issue/session/{id}`). NOT the one-time QR token.
    pub session_id: String,
    /// the allocated non-personal dogTagId (decimal string).
    pub dog_tag_id: String,
    pub owner_identity: OwnerIdentity,
    /// D1: the identity attribute leaves this session commits into `R` - one per non-blank
    /// [`OwnerIdentity`] field, salted fresh at session-start. Empty when the operator collected no
    /// identity (the bind then runs without the integrity gate). `#[serde(default)]` keeps
    /// pre-D1 session rows decodable.
    #[serde(default)]
    pub identity_leaves: Vec<IdentityLeaf>,
    /// the pet record: { name, microchip, profile fields } as posted by the operator.
    pub pet_name: String,
    pub microchip: Microchip,
    pub profile: PetProfile,
    /// "pending" -> "bound".
    pub status: String,
    pub created_at: u64,
    /// set on bind: the DOG_PROFILE merkle root (== SBT profileRoot[dogTagId]).
    #[serde(default)]
    pub root: Option<String>,
    /// set on bind: the mint txHash.
    #[serde(default)]
    pub tx_hash: Option<String>,
    /// set on bind: the owner-hidden protocol version used by `mintCustodial` + `issue(R)`.
    #[serde(default)]
    pub protocol_version: Option<String>,
}

/// Persisted per-issuer settings (impl §3.8).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssuerSettings {
    pub signing_mode: String, // "wallet" | "backend"
    /// The BEARER SECRET in the published `.ics` subscription URL — anyone holding it can read the
    /// shop's whole schedule, so it is treated as a credential: minted from a CSPRNG, never derived
    /// from anything guessable, and revoked by clearing it (which instantly 404s the feed). `None`
    /// (the default) means no feed has ever been published for this deployment.
    #[serde(rename = "icsFeedToken", default)]
    pub ics_feed_token: Option<String>,
}

impl Default for IssuerSettings {
    fn default() -> Self {
        IssuerSettings {
            signing_mode: "backend".to_string(),
            ics_feed_token: None,
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

// --------------------------------------------------------------------------------------------
// SHOP CRM — the business's OWN customer/booking records (clients, appointments) plus the central
// verification history. Distinct from the Phase-7 [`ApptReplica`], which mirrors CENTRAL-owned
// cross-business bookings and is rev-allocated by central; these rows are the shop's own
// system-of-record and are created/edited by the operator in the portal.
// --------------------------------------------------------------------------------------------

/// One pet belonging to a [`Client`]. Embedded in the client document (a pet has no life of its own —
/// it is always reached through its owner), so a client read is a single lookup.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClientPet {
    #[serde(rename = "petId")]
    pub pet_id: String,
    pub name: String,
    #[serde(default)]
    pub species: String,
    #[serde(default)]
    pub breed: String,
    #[serde(default)]
    pub sex: String,
    #[serde(rename = "dateOfBirth", default)]
    pub date_of_birth: String,
    #[serde(default)]
    pub notes: String,
    /// OPTIONAL DogTag id the owner's pet holds (the opaque on-chain id), recorded by the operator so
    /// the shop can tell WHICH pet a verification was about. Not required — a client may have no tag.
    #[serde(rename = "dogTagId", default)]
    pub dog_tag_id: Option<String>,
}

/// A CUSTOMER of the shop: the owner's contact particulars plus their pets. This is business contact
/// data the customer gave the shop directly — it is NOT, and must never be populated from, the
/// owner identity the DogTag protocol deliberately withholds from a verifier.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Client {
    #[serde(rename = "clientId")]
    pub client_id: String,
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub pets: Vec<ClientPet>,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
    /// Lowercased concatenation of every searchable field (name/email/phone/pet names/tag ids), so a
    /// free-text needle scans ONE field instead of N — and so the browser never has to pull the
    /// collection to filter it.
    ///
    /// It is NOT an indexed lookup: the match is an UNANCHORED substring, which no B-tree index can
    /// serve as a bounded seek. The denormalization narrows the scan, it does not remove it.
    #[serde(rename = "searchKey", default)]
    pub search_key: String,
}

/// One row of the PETS collection: a [`ClientPet`] together with the owner it belongs to.
///
/// A pet is *stored* embedded in its client document, but the portal addresses pets in their own
/// right (`/pets`, `/pets/{petId}`), so every pet read carries the owner's id and name denormalized —
/// the same denormalization [`Appointment::client_name`] uses, and for the same reason: a pet list
/// must render, and link to each owner, without an N+1 client join.
///
/// `owner_updated_at` is the OWNING CLIENT's `updated_at`. A pet has no timestamp of its own (it is
/// part of the client document, so every pet edit bumps the client), and it is carried here because
/// it is the field the collection is ordered by — offset paging is only coherent over a TOTAL order,
/// and the store has to be able to reproduce that same order on the next page request.
///
/// Deliberately not `Serialize`/`Deserialize`: this is an internal projection assembled by each
/// store, never a persisted document.
#[derive(Clone, Debug, Default)]
pub struct PetRow {
    pub pet: ClientPet,
    pub client_id: String,
    pub client_name: String,
    pub owner_updated_at: u64,
}

impl PetRow {
    /// The lowercased text a `?q=` needle is matched against: the pet's own searchable fields plus
    /// the OWNER's name, so an operator who only remembers "the poodle that belongs to Tan" finds
    /// the pet from either half of what they remember.
    ///
    /// Computed per row rather than stored. [`Client::search_key`] cannot serve a pet query: it
    /// concatenates EVERY pet of the client, so a needle matching one pet would match all of its
    /// siblings — on a client with two pets, searching one by name would return both.
    pub fn search_key(&self) -> String {
        [
            self.pet.name.as_str(),
            self.pet.species.as_str(),
            self.pet.breed.as_str(),
            self.pet.sex.as_str(),
            self.pet.dog_tag_id.as_deref().unwrap_or(""),
            self.client_name.as_str(),
        ]
        .join(" ")
        .to_lowercase()
    }
}

impl Client {
    /// Build the [`PetRow`] projection for one of this client's pets.
    pub fn pet_row(&self, p: &ClientPet) -> PetRow {
        PetRow {
            pet: p.clone(),
            client_id: self.client_id.clone(),
            client_name: self.name.clone(),
            owner_updated_at: self.updated_at,
        }
    }

    /// Recompute [`Client::search_key`] from the current field values. Call after every mutation.
    pub fn rebuild_search_key(&mut self) {
        let mut parts = vec![
            self.name.clone(),
            self.email.clone(),
            self.phone.clone(),
            self.address.clone(),
        ];
        for p in &self.pets {
            parts.push(p.name.clone());
            parts.push(p.breed.clone());
            if let Some(d) = &p.dog_tag_id {
                parts.push(d.clone());
            }
        }
        self.search_key = parts.join(" ").to_lowercase();
    }
}

/// A booking in the shop's calendar: which client + pet, what service, when, and its lifecycle state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Appointment {
    #[serde(rename = "appointmentId")]
    pub appointment_id: String,
    /// The shop client this booking belongs to, or EMPTY for an UNASSIGNED booking.
    ///
    /// Every booking made in the portal carries a real client id — the create/update routes reject a
    /// `clientId` that does not resolve. Empty is reserved for a booking that arrived from OUTSIDE
    /// the shop's own directory, namely an `.ics` import: a calendar invite carries a summary and a
    /// slot, not a DogTag client, and inventing a placeholder client to satisfy the column would put
    /// fabricated rows in the customer directory. The portal renders an empty id as "Unassigned" and
    /// the operator links a real client by editing the booking.
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "petId", default)]
    pub pet_id: Option<String>,
    /// free-text service label (e.g. "Full groom", "Bath & brush").
    #[serde(default)]
    pub service: String,
    /// UNIX SECONDS, not an ISO string: the calendar's day/week queries are range scans over this
    /// field, so it has to be numerically ordered and indexable.
    #[serde(rename = "startAt")]
    pub start_at: u64,
    #[serde(rename = "endAt")]
    pub end_at: u64,
    /// scheduled | confirmed | in_progress | completed | cancelled | no_show
    pub status: String,
    #[serde(default)]
    pub notes: String,
    /// the staff member assigned (free text; the shop's own roster is out of scope).
    #[serde(default)]
    pub groomer: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
    /// denormalized client name so the list/calendar renders without an N+1 client join.
    #[serde(rename = "clientName", default)]
    pub client_name: String,
    #[serde(rename = "petName", default)]
    pub pet_name: String,
    /// see [`Client::search_key`].
    #[serde(rename = "searchKey", default)]
    pub search_key: String,
    /// Where this booking came from. `None`/absent -> booked in the portal; `Some("ics")` -> created
    /// by an `.ics` import. Kept so the portal can label an imported booking honestly rather than
    /// presenting it as one the shop entered.
    #[serde(default)]
    pub source: Option<String>,
    /// The ORIGINATING calendar's `UID` for an imported booking (RFC 5545 §3.8.4.7), which is what
    /// makes a re-import of the same file idempotent instead of duplicating every event. Also
    /// re-emitted as the `UID` of the outbound feed, so an appointment that came in from a calendar
    /// and goes back out to one keeps a single identity across the round trip.
    #[serde(rename = "externalUid", default)]
    pub external_uid: Option<String>,
}

/// The lifecycle states an [`Appointment`] may hold. Anything else is rejected at the route.
pub const APPOINTMENT_STATES: &[&str] = &[
    "scheduled",
    "confirmed",
    "in_progress",
    "completed",
    "cancelled",
    "no_show",
];

impl Appointment {
    pub fn rebuild_search_key(&mut self) {
        self.search_key = [
            self.client_name.as_str(),
            self.pet_name.as_str(),
            self.service.as_str(),
            self.groomer.as_str(),
            self.notes.as_str(),
        ]
        .join(" ")
        .to_lowercase();
    }
}

/// The shop's CENTRAL, permanent record of a verification it performed — the searchable history
/// behind "All verifications", joined to the appointment + client when the operator started it from
/// one.
///
/// PRIVACY BOUNDARY. This row holds only (a) the PUBLIC verification facts that are already on chain
/// (purpose, recordType, txHash, the consumed nullifier, the opaque dogTagId) and (b) the keyPaths
/// the owner explicitly chose to disclose — never their VALUES, mirroring
/// [`VerifySession::disclosed_key_paths`]. It deliberately does NOT store the owner's `subject`
/// wallet: though derivable from the tx, persisting it here would create a client -> wallet linkage
/// inside the shop's own database that the protocol goes out of its way not to hand a verifier.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VerificationLog {
    /// == the verify session id (one session == one verification).
    #[serde(rename = "verificationId")]
    pub verification_id: String,
    #[serde(rename = "appointmentId", default)]
    pub appointment_id: Option<String>,
    #[serde(rename = "clientId", default)]
    pub client_id: Option<String>,
    #[serde(rename = "petId", default)]
    pub pet_id: Option<String>,
    pub purpose: String,
    #[serde(rename = "recordType")]
    pub record_type: String,
    /// pending | recording | recorded | error — mirrors the verify session's own status.
    pub status: String,
    #[serde(rename = "txHash", default)]
    pub tx_hash: Option<String>,
    #[serde(default)]
    pub nullifier: Option<String>,
    /// the opaque on-chain dogTagId the consent was bound to (a public verification fact).
    #[serde(rename = "dogTagId", default)]
    pub dog_tag_id: Option<String>,
    /// D1: the identity-leaf keyPaths the owner chose to REVEAL for this verification, mirrored from
    /// [`VerifySession::disclosed_key_paths`]. EMPTY when the owner disclosed nothing, which is the
    /// ordinary owner-hidden case — that emptiness IS the privacy guarantee and is never backfilled.
    #[serde(rename = "disclosedKeyPaths", default)]
    pub disclosed_key_paths: Vec<String>,
    /// denormalized for the list view (avoids an N+1 join per row).
    #[serde(rename = "clientName", default)]
    pub client_name: String,
    #[serde(rename = "petName", default)]
    pub pet_name: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    #[serde(rename = "updatedAt")]
    pub updated_at: u64,
    /// see [`Client::search_key`].
    #[serde(rename = "searchKey", default)]
    pub search_key: String,
}

/// The statuses a [`VerificationLog`] may hold — the verify session's own lifecycle, mirrored. The
/// list route rejects anything else, so a typo'd `?status=` filter is a request error rather than an
/// empty page indistinguishable from an empty history.
pub const VERIFICATION_STATES: &[&str] = &["pending", "recording", "recorded", "error"];

/// The subset of [`VERIFICATION_STATES`] a row can never leave again. This is the OWNERSHIP boundary
/// for the row: while a verification is in flight the verify leg is its sole writer, and only once it
/// has settled may anything else (a client rename resyncing labels) rewrite it. Every write is a
/// whole-document replace, so two writers on one row is a lost update.
pub const VERIFICATION_TERMINAL_STATES: &[&str] = &["recorded", "error"];

impl VerificationLog {
    /// True once the verify leg has settled this row and will never write it again.
    pub fn is_terminal(&self) -> bool {
        VERIFICATION_TERMINAL_STATES.contains(&self.status.as_str())
    }

    pub fn rebuild_search_key(&mut self) {
        let mut parts = vec![
            self.purpose.clone(),
            self.record_type.clone(),
            self.status.clone(),
            self.client_name.clone(),
            self.pet_name.clone(),
        ];
        if let Some(t) = &self.tx_hash {
            parts.push(t.clone());
        }
        if let Some(d) = &self.dog_tag_id {
            parts.push(d.clone());
        }
        self.search_key = parts.join(" ").to_lowercase();
    }
}

/// How many rows a list query may return at most, whatever the caller asks for. Bounds the response
/// so a large collection can never be shipped to the browser in one page.
pub const MAX_PAGE: usize = 200;
pub const DEFAULT_PAGE: usize = 50;

/// A page of results plus the TOTAL number of matches (for the pager), as every list query returns.
#[derive(Clone, Debug)]
pub struct Page<T> {
    pub rows: Vec<T>,
    pub total: u64,
}

/// Free-text + paging shared by every list query.
#[derive(Clone, Debug, Default)]
pub struct ClientQuery {
    /// free-text needle, matched (lowercased) against [`Client::search_key`].
    pub q: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

/// Free-text + owner filter + paging for the PETS collection.
#[derive(Clone, Debug, Default)]
pub struct PetQuery {
    /// free-text needle, matched (lowercased) against [`PetRow::search_key`].
    pub q: Option<String>,
    /// restrict to one owner's pets — what the client detail page and the owner filter use.
    pub client_id: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, Default)]
pub struct AppointmentQuery {
    pub q: Option<String>,
    pub client_id: Option<String>,
    pub pet_id: Option<String>,
    pub status: Option<String>,
    /// inclusive lower / exclusive upper bound on `start_at` (unix seconds) — the calendar's window.
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Clone, Debug, Default)]
pub struct VerificationQuery {
    pub q: Option<String>,
    pub client_id: Option<String>,
    /// Restrict to one PET's verifications. A client may bring several pets and each holds its own
    /// DogTag, so "this client's checks" is not the same question as "this pet's checks" — the pet
    /// detail page needs the narrower one.
    pub pet_id: Option<String>,
    pub appointment_id: Option<String>,
    pub status: Option<String>,
    pub purpose: Option<String>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub limit: usize,
    pub offset: usize,
}

/// Clamp a caller-supplied limit into `1..=MAX_PAGE`, defaulting a 0/absent limit to [`DEFAULT_PAGE`].
pub fn clamp_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_PAGE
    } else {
        limit.min(MAX_PAGE)
    }
}

/// A store read that did not resolve — the driver errored, not "the row is absent".
///
/// Deliberately one opaque string rather than an error taxonomy: exactly two reads surface it (the
/// pet-uniqueness guards' lookups), and their only decision is refuse-vs-proceed. It carries the
/// driver's own text for the log, never for the operator, whose 503 says what to do instead.
#[derive(Debug, Clone)]
pub struct StoreReadError(pub String);

impl std::fmt::Display for StoreReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The persistence trait. All methods async so MongoStore is a drop-in.
#[async_trait]
pub trait Store: Send + Sync {
    // ---- records ----
    async fn put_record(&self, r: Record);
    async fn get_record(&self, id: &str) -> Option<Record>;
    async fn update_record(&self, r: Record);
    /// List all records, most-recently-created first. Surfaces revoked/expired history too (never
    /// filtered) so the operator can trace every credential this device ever issued.
    async fn list_records(&self) -> Vec<Record>;
    /// true if any record currently has status == prepared.
    async fn has_prepared(&self) -> bool;
    /// idempotency lookup: record already confirmed at this txHash.
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record>;

    // ---- verify sessions ----
    async fn put_session(&self, s: VerifySession);
    async fn get_session(&self, id: &str) -> Option<VerifySession>;
    async fn update_session(&self, s: VerifySession);
    /// List verifier sessions as a durable audit trail, most-recently-created first.
    async fn list_sessions(&self) -> Vec<VerifySession>;

    // ---- jwt jti (one-time) ----
    /// Atomic consume: returns true if the jti was unused (now consumed), false if already used.
    async fn consume_jti(&self, jti: &str) -> bool;

    // ---- share tokens (short one-time QR token -> record) ----
    /// Store a short one-time share token mapping to `record_id`, expiring at unix-seconds `exp`.
    async fn put_share_token(&self, token: &str, record_id: &str, exp: u64);
    /// Atomically REMOVE the token (one-time consume) and return its `record_id` iff it exists and
    /// has not expired. A missing/expired token returns `None` (and is purged if expired).
    async fn take_share_token(&self, token: &str) -> Option<String>;

    // ---- export tokens (short-lived EXPORT QR token -> verify session) ----
    /// Store a short-lived export token mapping to `session_id`, expiring at unix-seconds `exp`.
    /// Mirrors the share-token pattern but resolves to a verify (export) session instead of a record.
    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64);
    /// NON-consuming lookup: return the export token's `session_id` iff it exists and has not
    /// expired. Used by `GET /x/{token}` (resolve) and the status poll — the token is NOT consumed
    /// here; session status provides the replay guard. An expired token returns `None`.
    async fn peek_export_token(&self, token: &str) -> Option<String>;

    // ---- DOG_PROFILE issuance: dogTagId counter + bind sessions + one-time bind tokens ----
    /// Allocate the next non-personal dogTagId (atomic monotonic counter). NEVER a hash of the
    /// microchip. Returns a fresh integer each call.
    async fn next_dog_tag_id(&self) -> u64;
    /// Store/replace a profile-issue session keyed by `session_id`.
    async fn put_profile_session(&self, s: ProfileIssueSession);
    /// Non-consuming lookup by `session_id` (the portal status poll).
    async fn get_profile_session(&self, session_id: &str) -> Option<ProfileIssueSession>;
    async fn update_profile_session(&self, s: ProfileIssueSession);
    /// Every profile-issue session, newest first - the dog-tag mint half of the traceability join
    /// (`/trace/*`): a bound session's anchored root/tx ties this vet's own mint to its on-chain
    /// `RootIssued` event.
    async fn list_profile_sessions(&self) -> Vec<ProfileIssueSession>;
    /// Store a one-time bind token mapping to `session_id`, expiring at unix-seconds `exp`.
    async fn put_bind_token(&self, token: &str, session_id: &str, exp: u64);
    /// NON-consuming lookup: the bind token's `session_id` iff present and unexpired (`GET /p/{token}`).
    async fn peek_bind_token(&self, token: &str) -> Option<String>;
    /// Atomically REMOVE the bind token (one-time consume) and return its `session_id` iff present and
    /// unexpired. Used by `POST /profiles/issue/custodial-bind` for replay protection.
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

    // ---- shop CRM: clients ----
    /// Insert or replace a client (keyed by `client_id`).
    async fn put_client(&self, c: Client);
    async fn get_client(&self, id: &str) -> Option<Client>;
    /// Remove a client; `true` iff a row existed.
    async fn delete_client(&self, id: &str) -> bool;
    /// Search/paginate clients, newest-updated first. Filtering happens HERE (indexed), never in the
    /// browser: the caller gets one bounded page plus the total match count.
    async fn list_clients(&self, q: &ClientQuery) -> Page<Client>;

    // ---- shop CRM: pets (addressed in their own right; stored inside their owner) ----
    /// Search/filter/paginate PETS across every client, each row carrying its owner.
    ///
    /// This is a store method rather than a fold over [`Store::list_clients`] because neither the
    /// page nor the total can be derived from a page of clients: `total` must count PETS, and a
    /// client page boundary falls between clients, not between pets. Doing it in the caller would
    /// mean pulling the whole collection into memory to slice it — exactly what every other list
    /// query here refuses to do.
    async fn list_pets(&self, q: &PetQuery) -> Page<PetRow>;
    /// One pet by its `pet_id`, found without the caller having to know its owner.
    ///
    /// `Ok(None)` means the read SUCCEEDED and no client holds a pet with that id; `Err` means the
    /// read itself did not resolve. The uniqueness guards need those two apart — collapsing them
    /// makes a driver fault read as "no conflict" and admits exactly the duplicate they exist to
    /// refuse — so this is the fallible form and [`Store::get_pet`] is derived from it.
    async fn try_get_pet(&self, pet_id: &str) -> Result<Option<PetRow>, StoreReadError>;
    /// Every pet currently linked to `dog_tag_id`, across ALL owners.
    ///
    /// An EXACT match on the stored value, deliberately not the `?q=` needle: a needle over
    /// [`PetRow::search_key`] is a substring match against the pet's other fields and its owner's
    /// name too, so it would both over- and under-report. Exists so the link route can refuse to let
    /// two pets share one tag - the tag is the key every check and every on-chain lookup is keyed
    /// by, and a mistyped id that silently merges two animals' histories is far worse than an error.
    ///
    /// Fallible for the same reason as [`Store::try_get_pet`]: an empty result must mean "the tag is
    /// free", never "the store could not say".
    async fn try_find_pets_by_dog_tag(
        &self,
        dog_tag_id: &str,
    ) -> Result<Vec<PetRow>, StoreReadError>;

    /// [`Store::try_get_pet`] for readers that have nothing to do with an unreadable store: a
    /// collapsed error becomes `None`, which every caller of this form turns into a 404 that REFUSES
    /// the write. That is already fail-closed, so they do not need the distinction. The uniqueness
    /// guards do, and use the fallible form.
    async fn get_pet(&self, pet_id: &str) -> Option<PetRow> {
        self.try_get_pet(pet_id).await.ok().flatten()
    }

    // ---- shop CRM: appointments ----
    async fn put_appointment(&self, a: Appointment);
    async fn get_appointment(&self, id: &str) -> Option<Appointment>;
    async fn delete_appointment(&self, id: &str) -> bool;
    /// Search/filter/paginate appointments ordered by `start_at` ASC (calendar order).
    async fn list_appointments(&self, q: &AppointmentQuery) -> Page<Appointment>;
    /// Find the booking an `.ics` import previously created for `external_uid`, if any.
    ///
    /// This is the whole dedup mechanism for repeated imports: an event's `UID` is stable across
    /// exports of the same calendar, so re-uploading the same file finds the existing booking and
    /// updates it instead of creating a second copy. Indexed (unique-per-present-value in Mongo), so
    /// it is a seek and not a scan.
    async fn appointment_by_external_uid(&self, external_uid: &str) -> Option<Appointment>;

    // ---- shop CRM: verification history ----
    async fn put_verification_log(&self, v: VerificationLog);
    async fn get_verification_log(&self, id: &str) -> Option<VerificationLog>;
    /// Search/filter/paginate the verification history, newest-created first.
    async fn list_verification_logs(&self, q: &VerificationQuery) -> Page<VerificationLog>;
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
    // shop CRM
    clients: HashMap<String, Client>,
    appointments: HashMap<String, Appointment>,
    verification_logs: HashMap<String, VerificationLog>,
}

/// Apply a free-text needle to a `search_key`: an empty/whitespace needle matches everything;
/// otherwise EVERY whitespace-separated term must appear, so "rex smith" narrows rather than widens.
fn search_matches(search_key: &str, needle: Option<&String>) -> bool {
    match needle.map(|s| s.trim().to_lowercase()) {
        None => true,
        Some(n) if n.is_empty() => true,
        Some(n) => n.split_whitespace().all(|term| search_key.contains(term)),
    }
}

/// Take one bounded page out of an already-sorted match set, returning it with the total count.
fn paginate<T: Clone>(matched: Vec<T>, limit: usize, offset: usize) -> Page<T> {
    let total = matched.len() as u64;
    let rows = matched.into_iter().skip(offset).take(clamp_limit(limit)).collect();
    Page { rows, total }
}

/// An equality filter: an absent filter matches everything, a present one must equal the row's value.
/// (Spelled out rather than `Option::is_none_or`, which postdates the workspace MSRV of 1.80.)
fn opt_eq<T: PartialEq>(want: &Option<T>, got: &T) -> bool {
    match want {
        None => true,
        Some(w) => w == got,
    }
}

/// Same, for a nullable column: a present filter never matches a row whose value is absent.
fn opt_eq_nullable<T: PartialEq>(want: &Option<T>, got: &Option<T>) -> bool {
    match (want, got) {
        (None, _) => true,
        (Some(w), Some(x)) => w == x,
        (Some(_), None) => false,
    }
}

/// A `>=` lower bound that an absent filter always satisfies.
fn at_least(bound: Option<u64>, value: u64) -> bool {
    match bound {
        None => true,
        Some(b) => value >= b,
    }
}

/// A `<` upper bound (exclusive) that an absent filter always satisfies.
fn below(bound: Option<u64>, value: u64) -> bool {
    match bound {
        None => true,
        Some(b) => value < b,
    }
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
    async fn list_records(&self) -> Vec<Record> {
        let mut v: Vec<Record> = self
            .inner
            .read()
            .unwrap()
            .records
            .values()
            .cloned()
            .collect();
        // Most-recent first; created_at is the primary key, record_id breaks ties deterministically.
        v.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.record_id.cmp(&a.record_id))
        });
        v
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
    async fn list_sessions(&self) -> Vec<VerifySession> {
        let mut v: Vec<VerifySession> = self
            .inner
            .read()
            .unwrap()
            .sessions
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        v
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
    async fn list_profile_sessions(&self) -> Vec<ProfileIssueSession> {
        let mut v: Vec<ProfileIssueSession> = self
            .inner
            .read()
            .unwrap()
            .profile_sessions
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        v
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

    // ---- shop CRM: clients ----
    async fn put_client(&self, c: Client) {
        self.inner.write().unwrap().clients.insert(c.client_id.clone(), c);
    }
    async fn get_client(&self, id: &str) -> Option<Client> {
        self.inner.read().unwrap().clients.get(id).cloned()
    }
    async fn delete_client(&self, id: &str) -> bool {
        self.inner.write().unwrap().clients.remove(id).is_some()
    }
    async fn list_clients(&self, q: &ClientQuery) -> Page<Client> {
        let g = self.inner.read().unwrap();
        let mut matched: Vec<Client> = g
            .clients
            .values()
            .filter(|c| search_matches(&c.search_key, q.q.as_ref()))
            .cloned()
            .collect();
        // newest-updated first; client_id breaks ties so paging is stable.
        matched.sort_by(|a, b| {
            b.updated_at.cmp(&a.updated_at).then_with(|| a.client_id.cmp(&b.client_id))
        });
        paginate(matched, q.limit, q.offset)
    }

    // ---- shop CRM: pets ----
    async fn list_pets(&self, q: &PetQuery) -> Page<PetRow> {
        let g = self.inner.read().unwrap();
        let mut matched: Vec<PetRow> = g
            .clients
            .values()
            .filter(|c| opt_eq(&q.client_id, &c.client_id))
            .flat_map(|c| c.pets.iter().map(move |p| c.pet_row(p)))
            .filter(|r| search_matches(&r.search_key(), q.q.as_ref()))
            .collect();
        // Newest-updated owner first, then client_id, then pet_id. All three keys are needed: pets
        // sharing an owner share its `updated_at` exactly, so without the pet_id tiebreak two pages
        // of the same result set could order those siblings differently and the pager would both
        // repeat and skip rows.
        matched.sort_by(|a, b| {
            b.owner_updated_at
                .cmp(&a.owner_updated_at)
                .then_with(|| a.client_id.cmp(&b.client_id))
                .then_with(|| a.pet.pet_id.cmp(&b.pet.pet_id))
        });
        paginate(matched, q.limit, q.offset)
    }

    // An in-memory map cannot fail to be read, so both fallible reads are always `Ok` here. The
    // distinction they carry is real only for `MongoStore`.
    async fn try_get_pet(&self, pet_id: &str) -> Result<Option<PetRow>, StoreReadError> {
        let g = self.inner.read().unwrap();
        Ok(g.clients
            .values()
            .find_map(|c| c.pets.iter().find(|p| p.pet_id == pet_id).map(|p| c.pet_row(p))))
    }

    async fn try_find_pets_by_dog_tag(
        &self,
        dog_tag_id: &str,
    ) -> Result<Vec<PetRow>, StoreReadError> {
        let g = self.inner.read().unwrap();
        let mut out: Vec<PetRow> = g
            .clients
            .values()
            .flat_map(|c| {
                c.pets
                    .iter()
                    .filter(|p| p.dog_tag_id.as_deref() == Some(dog_tag_id))
                    .map(move |p| c.pet_row(p))
            })
            .collect();
        // Deterministic, so an error message naming "the pet that already holds this tag" names the
        // same one on every call rather than whichever the hash map happened to yield first.
        out.sort_by(|a, b| a.pet.pet_id.cmp(&b.pet.pet_id));
        Ok(out)
    }

    // ---- shop CRM: appointments ----
    async fn put_appointment(&self, a: Appointment) {
        self.inner.write().unwrap().appointments.insert(a.appointment_id.clone(), a);
    }
    async fn get_appointment(&self, id: &str) -> Option<Appointment> {
        self.inner.read().unwrap().appointments.get(id).cloned()
    }
    async fn delete_appointment(&self, id: &str) -> bool {
        self.inner.write().unwrap().appointments.remove(id).is_some()
    }
    async fn list_appointments(&self, q: &AppointmentQuery) -> Page<Appointment> {
        let g = self.inner.read().unwrap();
        let mut matched: Vec<Appointment> = g
            .appointments
            .values()
            .filter(|a| search_matches(&a.search_key, q.q.as_ref()))
            .filter(|a| opt_eq(&q.client_id, &a.client_id))
            .filter(|a| opt_eq_nullable(&q.pet_id, &a.pet_id))
            .filter(|a| opt_eq(&q.status, &a.status))
            // [from, to): inclusive lower, exclusive upper — adjacent calendar windows never
            // double-count an appointment that starts exactly on the boundary.
            .filter(|a| at_least(q.from, a.start_at))
            .filter(|a| below(q.to, a.start_at))
            .cloned()
            .collect();
        // calendar order: earliest first.
        matched.sort_by(|a, b| {
            a.start_at.cmp(&b.start_at).then_with(|| a.appointment_id.cmp(&b.appointment_id))
        });
        paginate(matched, q.limit, q.offset)
    }
    async fn appointment_by_external_uid(&self, external_uid: &str) -> Option<Appointment> {
        if external_uid.is_empty() {
            return None;
        }
        self.inner
            .read()
            .unwrap()
            .appointments
            .values()
            .find(|a| a.external_uid.as_deref() == Some(external_uid))
            .cloned()
    }

    // ---- shop CRM: verification history ----
    async fn put_verification_log(&self, v: VerificationLog) {
        self.inner
            .write()
            .unwrap()
            .verification_logs
            .insert(v.verification_id.clone(), v);
    }
    async fn get_verification_log(&self, id: &str) -> Option<VerificationLog> {
        self.inner.read().unwrap().verification_logs.get(id).cloned()
    }
    async fn list_verification_logs(&self, q: &VerificationQuery) -> Page<VerificationLog> {
        let g = self.inner.read().unwrap();
        let mut matched: Vec<VerificationLog> = g
            .verification_logs
            .values()
            .filter(|v| search_matches(&v.search_key, q.q.as_ref()))
            .filter(|v| opt_eq_nullable(&q.client_id, &v.client_id))
            .filter(|v| opt_eq_nullable(&q.pet_id, &v.pet_id))
            .filter(|v| opt_eq_nullable(&q.appointment_id, &v.appointment_id))
            .filter(|v| opt_eq(&q.status, &v.status))
            .filter(|v| opt_eq(&q.purpose, &v.purpose))
            .filter(|v| at_least(q.from, v.created_at))
            .filter(|v| below(q.to, v.created_at))
            .cloned()
            .collect();
        // newest first.
        matched.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.verification_id.cmp(&b.verification_id))
        });
        paginate(matched, q.limit, q.offset)
    }
}
