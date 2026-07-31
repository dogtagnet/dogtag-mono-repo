//! The SHOP's own business surfaces: clients, appointments, and the central verification history.
//!
//! This is the layer that turns the portal from a bare verification tool into the application a
//! groomer actually works in. The primary flow the operator follows is:
//!
//!   create a [`Client`] -> book an [`Appointment`] for them -> start the verification FROM that
//!   appointment -> the result lands in the verification history, joined to both.
//!
//! WHAT THIS MODULE DOES NOT TOUCH. The ZK consent mechanics are untouched: this module adds
//! business context AROUND a verification and never participates in the proof. Concretely,
//! [`start_log`] / [`attach_evidence`] / [`finish_log`] are pure store writes invoked from
//! `verify.rs` at points where the verify session's own status already changed — they cannot affect
//! whether or how a verification is recorded on chain, and a failure to write one never fails a
//! verification.
//!
//! PRIVACY. A verification row stores only the public on-chain facts plus the keyPaths the owner
//! chose to disclose (see [`VerificationLog`]) — never their values. The owner's `subject` wallet is
//! deliberately NOT stored, and the disclosed keyPath list stays EMPTY on an ordinary owner-hidden
//! verification because nothing was disclosed. The client/appointment linkage lives only in the
//! shop's own store — `GET /x/{token}`, which any scanner of the QR can read, still returns exactly
//! what it returned before.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::auth::now;
use crate::microchip::{MicrochipCheck, NotComparable};
use crate::store::{
    Appointment, AppointmentQuery, Client, ClientPet, ClientQuery, Page, PetQuery, PetRow, Store,
    StoreReadError, VerificationLog, VerificationQuery, VerifySession, APPOINTMENT_STATES,
    VERIFICATION_STATES,
};

type Resp = (StatusCode, Json<Value>);
fn ok(v: Value) -> Resp {
    (StatusCode::OK, Json(v))
}
fn err(code: StatusCode, msg: &str) -> Resp {
    (code, Json(json!({ "error": msg })))
}

// ============================================================================================
// JSON projections — the wire shape the portal consumes.
// ============================================================================================

fn pet_json(p: &ClientPet) -> Value {
    json!({
        "petId": p.pet_id,
        "name": p.name,
        "species": p.species,
        "breed": p.breed,
        "sex": p.sex,
        "dateOfBirth": p.date_of_birth,
        "notes": p.notes,
        "dogTagId": p.dog_tag_id,
        // `null` when the shop has none. Deliberately NOT accompanied by a `microchipCheck` on this
        // projection: a list row carries no verdict, so an absent key here can never be mistaken for
        // "checked and fine". The verdict is emitted only by the routes that WRITE the binding.
        "microchipCode": p.microchip_code,
    })
}

/// One row of the pets collection: the pet's own fields plus the owner it belongs to.
///
/// `clientId`/`clientName` are what make the pet -> owner half of the round trip a plain link instead
/// of a second fetch, and they are why the pets list can render an owner column without an N+1 join.
fn pet_row_json(r: &PetRow) -> Value {
    let mut v = pet_json(&r.pet);
    v["clientId"] = json!(r.client_id);
    v["clientName"] = json!(r.client_name);
    v["updatedAt"] = json!(r.owner_updated_at);
    v
}

fn client_json(c: &Client) -> Value {
    json!({
        "clientId": c.client_id,
        "name": c.name,
        "email": c.email,
        "phone": c.phone,
        "address": c.address,
        "notes": c.notes,
        "pets": c.pets.iter().map(pet_json).collect::<Vec<_>>(),
        "createdAt": c.created_at,
        "updatedAt": c.updated_at,
    })
}

fn appointment_json(a: &Appointment) -> Value {
    json!({
        "appointmentId": a.appointment_id,
        "clientId": a.client_id,
        "clientName": a.client_name,
        "petId": a.pet_id,
        "petName": a.pet_name,
        "service": a.service,
        "startAt": a.start_at,
        "endAt": a.end_at,
        "status": a.status,
        "notes": a.notes,
        "groomer": a.groomer,
        "createdAt": a.created_at,
        "updatedAt": a.updated_at,
        // Provenance, so the portal can label an imported booking as imported (and as UNASSIGNED
        // while its `clientId` is still empty) rather than presenting it as one the shop entered.
        "source": a.source,
        "externalUid": a.external_uid,
    })
}

fn verification_json(v: &VerificationLog) -> Value {
    json!({
        "verificationId": v.verification_id,
        "appointmentId": v.appointment_id,
        "clientId": v.client_id,
        "clientName": v.client_name,
        "petId": v.pet_id,
        "petName": v.pet_name,
        "purpose": v.purpose,
        "recordType": v.record_type,
        "status": v.status,
        "txHash": v.tx_hash,
        "nullifier": v.nullifier,
        "dogTagId": v.dog_tag_id,
        "disclosedKeyPaths": v.disclosed_key_paths,
        "createdAt": v.created_at,
        "updatedAt": v.updated_at,
    })
}

/// Wrap a page as `{ rows, total, limit, offset }` so the portal can render a pager without
/// having to know the collection size.
fn page_json<T>(p: Page<T>, limit: usize, offset: usize, project: impl Fn(&T) -> Value) -> Value {
    json!({
        "rows": p.rows.iter().map(project).collect::<Vec<_>>(),
        "total": p.total,
        "limit": crate::store::clamp_limit(limit),
        "offset": offset,
    })
}

// ============================================================================================
// request bodies / query strings
// ============================================================================================

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PetBody {
    /// omitted on create; supplied to keep a pet's identity (and its appointment links) across edits.
    #[serde(default)]
    pet_id: Option<String>,
    name: String,
    #[serde(default)]
    species: String,
    #[serde(default)]
    breed: String,
    #[serde(default)]
    sex: String,
    #[serde(default)]
    date_of_birth: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    dog_tag_id: Option<String>,
    /// The animal's microchip code, if the shop has one. Absent is normal — see
    /// [`ClientPet::microchip_code`].
    #[serde(default)]
    microchip_code: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClientBody {
    name: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    phone: String,
    #[serde(default)]
    address: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    pets: Vec<PetBody>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AppointmentBody {
    client_id: String,
    #[serde(default)]
    pet_id: Option<String>,
    #[serde(default)]
    service: String,
    start_at: u64,
    #[serde(default)]
    end_at: u64,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    groomer: String,
}

#[derive(Deserialize, Default)]
struct ListQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(rename = "clientId", default)]
    client_id: Option<String>,
    #[serde(rename = "petId", default)]
    pet_id: Option<String>,
    #[serde(rename = "appointmentId", default)]
    appointment_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    from: Option<u64>,
    #[serde(default)]
    to: Option<u64>,
}

impl ListQuery {
    fn limit(&self) -> usize {
        self.limit.unwrap_or(0)
    }
    fn offset(&self) -> usize {
        self.offset.unwrap_or(0)
    }
    /// Normalize an absent-or-blank query-string filter to `None`, so `?status=` never filters on
    /// the empty string and silently returns nothing.
    fn opt(v: &Option<String>) -> Option<String> {
        v.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    }
}

// ============================================================================================
// clients
// ============================================================================================

async fn list_clients(State(st): State<AppState>, headers: HeaderMap, Query(q): Query<ListQuery>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let page = st
        .store
        .list_clients(&ClientQuery {
            q: ListQuery::opt(&q.q),
            limit: q.limit(),
            offset: q.offset(),
        })
        .await;
    ok(page_json(page, q.limit(), q.offset(), client_json))
}

async fn create_client(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClientBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    if body.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name is required");
    }
    let pets: Vec<ClientPet> = body.pets.iter().map(build_pet).collect();
    if let Some(conflict) = reject_foreign_pet_ids(&st.store, &pets, None).await {
        return conflict;
    }
    if let Some(conflict) = reject_dog_tag_conflicts(&st.store, &pets, None).await {
        return conflict;
    }
    if let Some(conflict) = reject_microchip_mismatches(&st.store, &pets).await {
        return conflict;
    }
    let ts = now();
    let mut c = Client {
        client_id: uuid::Uuid::new_v4().to_string(),
        name: body.name.trim().to_string(),
        email: body.email.trim().to_string(),
        phone: body.phone.trim().to_string(),
        address: body.address.trim().to_string(),
        notes: body.notes,
        pets,
        created_at: ts,
        updated_at: ts,
        search_key: String::new(),
    };
    c.rebuild_search_key();
    st.store.put_client(c.clone()).await;
    (StatusCode::CREATED, Json(client_json(&c)))
}

/// Build a stored pet from a request body, minting a `pet_id` when the caller did not supply one.
/// An edit that echoes the existing `petId` keeps the pet's identity, so appointments and
/// verifications already pointing at it stay linked.
fn build_pet(p: &PetBody) -> ClientPet {
    ClientPet {
        pet_id: p
            .pet_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: p.name.trim().to_string(),
        species: p.species.trim().to_string(),
        breed: p.breed.trim().to_string(),
        sex: p.sex.trim().to_string(),
        date_of_birth: p.date_of_birth.trim().to_string(),
        notes: p.notes.clone(),
        // TRIMMED, like every other field here and like `link_pet_dogtag`. The tag is compared as an
        // exact string by both `try_find_pets_by_dog_tag` and the held-document cache lookup, so storing
        // a pasted " 4" would silently defeat the one-pet-per-tag guard AND miss the credential
        // filed under "4" - one stray character reintroducing two separate bugs.
        dog_tag_id: p
            .dog_tag_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        // Trimmed and blank-normalized to `None` for the same reason: the microchip is compared as an
        // exact string against a credential leaf, so a pasted " 985…" would report a mismatch between
        // a code and itself — a refusal the operator cannot see the cause of and cannot clear.
        microchip_code: p
            .microchip_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

async fn get_client(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    match st.store.get_client(&id).await {
        Some(c) => ok(client_json(&c)),
        None => err(StatusCode::NOT_FOUND, "client not found"),
    }
}

async fn update_client(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ClientBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let existing = match st.store.get_client(&id).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "client not found"),
    };
    if body.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name is required");
    }
    let pets: Vec<ClientPet> = body.pets.iter().map(build_pet).collect();
    if let Some(conflict) =
        reject_foreign_pet_ids(&st.store, &pets, Some(&existing.client_id)).await
    {
        return conflict;
    }
    if let Some(conflict) =
        reject_dog_tag_conflicts(&st.store, &pets, Some(&existing.client_id)).await
    {
        return conflict;
    }
    if let Some(conflict) = reject_orphaning_pet_removals(&st.store, &existing.pets, &pets).await {
        return conflict;
    }
    if let Some(conflict) = reject_microchip_mismatches(&st.store, &pets).await {
        return conflict;
    }
    let mut c = Client {
        client_id: existing.client_id,
        name: body.name.trim().to_string(),
        email: body.email.trim().to_string(),
        phone: body.phone.trim().to_string(),
        address: body.address.trim().to_string(),
        notes: body.notes,
        pets,
        created_at: existing.created_at,
        updated_at: now(),
        search_key: String::new(),
    };
    c.rebuild_search_key();
    st.store.put_client(c.clone()).await;
    // Keep the appointments' denormalized client/pet names in step with the edit, so the calendar
    // and the verification history do not keep showing the pre-rename label.
    resync_client_labels(&st.store, &c).await;
    ok(client_json(&c))
}

/// After a client edit, refresh the denormalized `clientName`/`petName` the shop's own rows carry —
/// the appointments the calendar renders and the SETTLED verification-history rows — rebuilding each
/// row's search key so the client is findable under the new name.
///
/// An IN-FLIGHT verification is deliberately left alone. Every store write is a whole-document
/// replace, so a rename racing the verify leg's detached settle would write a stale copy back and
/// lose the row's `recorded` status, txHash and nullifier — permanently, since nothing re-runs the
/// settle. The two writers are therefore split by row state rather than by a lock: while a row is
/// non-terminal the verify leg owns it outright, and [`finish_log`] re-reads the client at settle so
/// a rename that landed mid-flight still lands on the row.
///
/// Both passes are bounded to one page: a single client's bookings and checks, never the whole
/// collection.
async fn resync_client_labels(store: &Arc<dyn Store>, c: &Client) {
    // A row naming NO pet has no name to carry; a row whose pet no longer resolves KEEPS the name it
    // has. Those two cases used to share `unwrap_or_default()`, which blanked the second — and an
    // unresolvable pet is not evidence the stored name was wrong, exactly as an unreadable client is
    // not (see `refresh_client_labels`). `reject_orphaning_pet_removals` should now make the second
    // case unreachable for any pet a row points at; this is the belt to that guard's braces, and it
    // must not degrade the label it is defending.
    let pet_name_of = |pet_id: Option<&String>, current: &str| match pet_id {
        None => String::new(),
        Some(pid) => c
            .pets
            .iter()
            .find(|p| &p.pet_id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| current.to_string()),
    };

    let appts = store
        .list_appointments(&AppointmentQuery {
            client_id: Some(c.client_id.clone()),
            limit: crate::store::MAX_PAGE,
            ..Default::default()
        })
        .await;
    for mut a in appts.rows {
        let pet_name = pet_name_of(a.pet_id.as_ref(), &a.pet_name);
        if a.client_name == c.name && a.pet_name == pet_name {
            continue;
        }
        a.client_name = c.name.clone();
        a.pet_name = pet_name;
        a.rebuild_search_key();
        a.updated_at = now();
        store.put_appointment(a).await;
    }

    let verifs = store
        .list_verification_logs(&VerificationQuery {
            client_id: Some(c.client_id.clone()),
            limit: crate::store::MAX_PAGE,
            ..Default::default()
        })
        .await;
    for mut v in verifs.rows {
        if !v.is_terminal() {
            continue;
        }
        let pet_name = pet_name_of(v.pet_id.as_ref(), &v.pet_name);
        if v.client_name == c.name && v.pet_name == pet_name {
            continue;
        }
        v.client_name = c.name.clone();
        v.pet_name = pet_name;
        v.rebuild_search_key();
        v.updated_at = now();
        store.put_verification_log(v).await;
    }
}

async fn delete_client(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    // Refuse to orphan bookings: a client with appointments must have them removed first, so the
    // verification history never points at a client row that no longer exists.
    let appts = st
        .store
        .list_appointments(&AppointmentQuery {
            client_id: Some(id.clone()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if appts.total > 0 {
        return err(
            StatusCode::CONFLICT,
            "client has appointments; delete or reassign them first",
        );
    }
    if st.store.delete_client(&id).await {
        ok(json!({ "deleted": true }))
    } else {
        err(StatusCode::NOT_FOUND, "client not found")
    }
}

// ============================================================================================
// pets — addressed in their own right, still stored inside their owner
// ============================================================================================
//
// A pet is reachable BOTH ways round: `/pets?clientId=` lists one owner's pets, and every pet row
// carries `clientId`/`clientName` so the pet names its owner. That symmetry is the point — an
// operator with a pet in front of them and no idea who brought it needs to get to the owner, and an
// operator on the phone to an owner needs to get to the pet.
//
// Every write here goes through [`mutate_pet`], which patches the ONE addressed pet inside its
// client. That is deliberately unlike `PUT /clients/{id}`, which replaces the whole `pets` array and
// therefore deletes any pet the caller omits.

/// Fields a pet write accepts. Absent fields are left as they are on an edit — a pet route must not
/// blank a field the caller simply did not mention, which is the failure mode a whole-document
/// replace has by construction.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PetPatchBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    species: Option<String>,
    #[serde(default)]
    breed: Option<String>,
    #[serde(default)]
    sex: Option<String>,
    #[serde(default)]
    date_of_birth: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    /// The animal's microchip code. ABSENT leaves it alone (like every other field here); an explicit
    /// blank CLEARS it, which is the only way to correct a code typed onto the wrong pet.
    #[serde(default)]
    microchip_code: Option<String>,
}

/// Body of `POST /pets` — a pet must be created UNDER an owner, so `clientId` is required.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PetCreateBody {
    client_id: String,
    #[serde(flatten)]
    pet: PetBody,
}

/// Body of `POST /pets/{id}/dogtag` — recording which tag this pet holds.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PetDogTagBody {
    dog_tag_id: String,
}

/// The two store reads every pet-uniqueness guard depends on, as a FALLIBLE seam.
///
/// The guards are the one place where "the store said nothing holds this" and "the store did not
/// answer" must not be the same value: collapsing them lets a driver fault admit the duplicate they
/// exist to refuse, silently, with no operator-visible sign. Narrowing that dependency to two
/// methods is also what makes the refusal testable — a stub implementing this is two functions,
/// where a stub implementing [`Store`] would be nearly sixty.
#[async_trait::async_trait]
trait PetLookup {
    async fn lookup_pet(&self, pet_id: &str) -> Result<Option<PetRow>, StoreReadError>;
    async fn lookup_pets_by_tag(&self, tag: &str) -> Result<Vec<PetRow>, StoreReadError>;
}

#[async_trait::async_trait]
impl<S: Store + ?Sized> PetLookup for Arc<S> {
    async fn lookup_pet(&self, pet_id: &str) -> Result<Option<PetRow>, StoreReadError> {
        self.try_get_pet(pet_id).await
    }
    async fn lookup_pets_by_tag(&self, tag: &str) -> Result<Vec<PetRow>, StoreReadError> {
        self.try_find_pets_by_dog_tag(tag).await
    }
}

/// The 503 for a write whose uniqueness could not be CHECKED.
///
/// Refusing is the only safe answer: the guards' whole job is to keep two animals off one tag or one
/// pet id, and admitting a write nobody could validate is exactly the merge they prevent. Nothing has
/// been written at the point every caller reaches this, so the operator loses only the retry.
fn unverifiable(e: StoreReadError) -> Resp {
    tracing::warn!(error = %e, "pet uniqueness guard could not read the store; refusing the write");
    err(
        StatusCode::SERVICE_UNAVAILABLE,
        "The pet records could not be read, so this save could not be checked against the pets already on file. Nothing was changed — try again.",
    )
}

// ---- the microchip cross-check, at every place a tag↔pet binding is written --------------------

/// Read the shop's held credential for `tag` and cross-check its microchip against `pet_code`.
///
/// The decision itself is [`crate::microchip::compare`]; this is the I/O half. It is deliberately
/// ONE helper rather than a rule restated per route, because the binding is written from five places
/// and a guard covering only some of them reads as an enforced invariant while the main way in stays
/// open — the exact reasoning that already made [`reject_dog_tag_conflicts`] the twin of
/// [`conflicting_pet`].
///
/// Never `Err`: an unreadable store is a `NotComparable` REASON, not a refusal. That is the opposite
/// of [`unverifiable`], and the difference is what the two guards are protecting. Uniqueness is a
/// safety invariant whose whole job is to keep two animals off one tag, so a check that could not run
/// must refuse the write. This one is *evidence*: it can only ever tighten a link the operator was
/// already entitled to make, so failing closed here would refuse ordinary work — most loudly for the
/// commonest case of all, a pet with no microchip — and teach operators to route around it.
///
/// STATED ASSUMPTION: the leaf is read from the CACHED document without re-running `check_integrity`.
/// That is sound because `POST /import/pull` verifies before filing and nothing else writes that
/// cache, so a document only gets in by passing every pillar. It does mean the tamper-evidence this
/// check rests on is the import's, not this read's: a cache altered AT REST could fake a match. Left
/// as recorded rather than closed because re-verifying here would be an offline Merkle recompute per
/// link — cheap enough to add if the store's at-rest integrity ever stops being assumed, which is the
/// condition to revisit it under, not the cost.
async fn microchip_check(
    store: &Arc<dyn Store>,
    tag: &str,
    pet_code: Option<&str>,
) -> MicrochipCheck {
    let tag = tag.trim();
    if !crate::microchip::is_handle_form(tag) {
        return MicrochipCheck::NotComparable(NotComparable::CannotLookUpByFieldElement);
    }
    let held = match store.try_get_client_cache(tag).await {
        Ok(Some(v)) => v,
        Ok(None) => return MicrochipCheck::NotComparable(NotComparable::NoCredentialHeld),
        Err(e) => return MicrochipCheck::NotComparable(NotComparable::CouldNotRead(e.to_string())),
    };
    // A cached document that will not parse is a failure to READ, never an absence: reporting it as
    // "this shop holds no credential" would state a fact about the shop's records on the strength of
    // a document it is holding and could not open.
    let doc: dogtag_standard::wrap::WrappedDoc = match serde_json::from_value(held) {
        Ok(d) => d,
        Err(e) => {
            return MicrochipCheck::NotComparable(NotComparable::CredentialUnreadable(format!(
                "the held credential is not a readable wrapped document: {e}"
            )))
        }
    };
    crate::microchip::compare(pet_code, &crate::microchip::credential_microchip(&doc))
}

/// The 409 for a link whose microchip cross-check says the credential describes a DIFFERENT animal.
///
/// 409 CONFLICT, the same code (and shape) as [`dog_tag_conflict`], because it is the same kind of
/// answer: the write is well-formed and the request is authorized — it just contradicts evidence
/// already on file. Deliberately NOT `import_pull`'s 422, which means "this credential did not
/// verify". The credential here is genuine and is not being accused of anything.
///
/// The structured verdict rides ALONG WITH the message, for the reason `a_refused_import_reports_why`
/// exists: a bare sentence leaves a client unable to distinguish this refusal from any other 409, and
/// gives it two vocabularies for one check.
fn microchip_conflict(check: &MicrochipCheck) -> Resp {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": check.refusal_message().unwrap_or_default(),
            "microchipCheck": check.to_json(),
        })),
    )
}

/// The same cross-check for the WHOLE-DOCUMENT client routes, over every pet in the payload.
///
/// The twin of [`reject_dog_tag_conflicts`], and present for the identical reason: `POST /clients`
/// and `PUT /clients/{id}` carry pets inline, so guarding only the pet routes would leave the
/// mis-link reachable through the adjacent route with the same fields.
///
/// Only a pet carrying BOTH a tag and a microchip can be refused; every other combination is
/// `NotComparable` and passes through untouched.
async fn reject_microchip_mismatches(
    store: &Arc<dyn Store>,
    pets: &[ClientPet],
) -> Option<Resp> {
    for p in pets {
        let (Some(tag), Some(code)) = (
            p.dog_tag_id.as_deref().map(str::trim).filter(|s| !s.is_empty()),
            p.microchip_code.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        ) else {
            continue;
        };
        let check = microchip_check(store, tag, Some(code)).await;
        if check.refuses() {
            return Some(microchip_conflict(&check));
        }
    }
    None
}

/// The pet, if any, that already holds `tag` and is not `self_pet_id`.
///
/// One tag, at most one pet - the tag is the key every check and every on-chain lookup is keyed by,
/// so two pets sharing one would show each other's held credential, each other's on-chain history,
/// and both would answer a `?q=<tag>` search: two animals' records silently merged, with a mistyped
/// digit the realistic way in. Excluding `self_pet_id` is what keeps re-recording a tag a pet already
/// holds idempotent rather than a self-conflict.
///
/// For the SURGICAL pet routes ([`create_pet`], [`link_pet_dogtag`]), which write one pet. The
/// whole-document client routes need [`reject_dog_tag_conflicts`] instead, because their payload
/// replaces the owner's entire pet list.
async fn conflicting_pet<L: PetLookup + ?Sized>(
    store: &L,
    tag: &str,
    self_pet_id: &str,
) -> Result<Option<PetRow>, StoreReadError> {
    Ok(store
        .lookup_pets_by_tag(tag)
        .await?
        .into_iter()
        .find(|r| r.pet.pet_id != self_pet_id))
}

/// The same one-pet-per-tag rule, for the WHOLE-DOCUMENT client routes.
///
/// `POST /clients` and `PUT /clients/{id}` carry pets inline - and the groomer's client form has a
/// DogTag field, so this is the likelier place an operator types one than the pet page is. A guard
/// covering only the pet routes would be worse than none: it reads as an enforced invariant while
/// the main way in stays open, so every downstream reader trusts something untrue.
///
/// The set after the write is (every OTHER client's pets) + (this payload's pets), so that is what
/// is checked, and it is checked BEFORE anything is written so a rejected pet never leaves a
/// half-applied client. Three consequences fall out of taking the payload as the client's complete
/// new pet list rather than diffing it:
///
///  - Stored pets of `client_id` are skipped - the payload REPLACES them. That is what makes the
///    ordinary edit (echo every petId, tags unchanged) a non-conflict, and it also lets a tag move
///    between the owner's own pets in a single save.
///  - The payload is checked against ITSELF, since two of its pets can carry one tag with neither
///    of them stored yet - a case no lookup against the store could see. That twin check is
///    grandfathered on exactly the same DELTA terms as the cross-client one, and by `&&`: a pair is
///    exempt only when BOTH pets' stored records already carry the tag under this same owner. If
///    even one of them is claiming it now, that is the merge arriving and it is still refused. The
///    same-owner pre-existing pair has to be exempt for the same reason the cross-client one is -
///    otherwise a client whose two pets already share a tag can never be saved again, so an operator
///    fixing a phone number is refused over a duplicate they did not create and cannot clear from
///    that form, which no longer has a DogTag field at all.
///  - What is validated is the DELTA, not the state. A pet whose STORED record already carries the
///    tag it is submitting is grandfathered, because this rule postdates the data: a store written
///    before it can already hold two pets on one tag, and without this an operator fixing a phone
///    number would be refused over a DogTag they never touched, on a conflict they cannot resolve
///    from that form. Only a tag that is NEW or CHANGED for a pet is a claim, and every claim is
///    still checked - so no fresh duplicate can be introduced by any route.
///
/// That exemption is bounded by OWNERSHIP, and has to be: `build_pet` honours a caller-supplied
/// `petId` on both client routes (deliberately - echoing it is what preserves a pet's identity and
/// the appointment and verification rows pointing at it). A payload naming ANOTHER client's petId
/// would otherwise find that stranger's pet already holding the tag and read its own claim as
/// unchanged, grafting a second pet onto one petId AND one tag. So the stored pet must belong to the
/// client being written, and a create grandfathers nothing at all - every pet in a create payload is
/// new by definition, so it has no prior record to be unchanged from.
/// Every pet id in the payload must address an animal this write is allowed to address.
///
/// `build_pet` mints a `pet_id` only when the caller omits one, and echoing an existing id is what
/// preserves a pet's identity across an edit - the appointment and verification rows point at it - so
/// the client routes are the one place a pet id arrives from OUTSIDE. Unchecked, a payload can name
/// another client's pet id and seat a second animal under it, and `pet_id` is an ADDRESS: `get_pet`
/// resolves it to whichever client the store reaches first (`HashMap` order in `MemStore`, the first
/// matching document in Mongo), so `GET`/`DELETE /pets/{id}` and every [`mutate_pet`] write would
/// then act on an arbitrary one of the two. An operator linking a tag from the pet page could tag the
/// WRONG ANIMAL - and which animal a credential belongs to is the one thing this product asserts.
///
/// Deliberately INDEPENDENT of `dog_tag_id`: [`reject_dog_tag_conflicts`] skips an untagged pet at
/// its first guard, so folding this into that loop would leave the untagged payload - the whole gap -
/// unchecked.
///
/// Two payload shapes still pass, and must: an id that resolves to nothing (a caller-supplied id for
/// a genuinely new pet, which `build_pet` already tolerates), and an id belonging to the client being
/// written, which is the ordinary echo.
async fn reject_foreign_pet_ids<L: PetLookup + ?Sized>(
    store: &L,
    pets: &[ClientPet],
    client_id: Option<&str>,
) -> Option<Resp> {
    for (i, p) in pets.iter().enumerate() {
        if let Some(twin) = pets[..i].iter().find(|q| q.pet_id == p.pet_id) {
            return Some(err(
                StatusCode::CONFLICT,
                &format!(
                    "Pet id {} is listed on two pets in this request ({} and {}). A pet id addresses one animal, so give each pet its own.",
                    p.pet_id, twin.name, p.name
                ),
            ));
        }
        let stored = match store.lookup_pet(&p.pet_id).await {
            Ok(v) => v,
            Err(e) => return Some(unverifiable(e)),
        };
        let Some(stored) = stored else {
            continue;
        };
        if client_id.is_some_and(|owner| stored.client_id == owner) {
            continue;
        }
        return Some(err(
            StatusCode::CONFLICT,
            &format!(
                "Pet id {} already belongs to {} ({}). A pet id addresses one animal, so it cannot be reused on another owner's record.",
                p.pet_id, stored.pet.name, stored.client_name
            ),
        ));
    }
    None
}

async fn reject_dog_tag_conflicts<L: PetLookup + ?Sized>(
    store: &L,
    pets: &[ClientPet],
    client_id: Option<&str>,
) -> Option<Resp> {
    for (i, p) in pets.iter().enumerate() {
        let Some(tag) = p.dog_tag_id.as_deref() else {
            continue;
        };
        // Memoised across the two branches below, both of which need this pet's own verdict, so the
        // grandfather lookup still costs at most ONE store read per tagged pet - and only on a path
        // that would otherwise refuse.
        let mut mine: Option<bool> = None;
        if let Some(twin) = pets[..i].iter().find(|q| q.dog_tag_id.as_deref() == Some(tag)) {
            let p_unchanged = match tag_is_unchanged(store, client_id, p, tag).await {
                Ok(v) => v,
                Err(e) => return Some(unverifiable(e)),
            };
            mine = Some(p_unchanged);
            // Short-circuits: with this pet claiming the tag afresh the pair is refused regardless
            // of the twin, so the twin's read is never paid for.
            let pair_predates_the_rule = p_unchanged
                && match tag_is_unchanged(store, client_id, twin, tag).await {
                    Ok(v) => v,
                    Err(e) => return Some(unverifiable(e)),
                };
            if !pair_predates_the_rule {
                return Some(err(
                    StatusCode::CONFLICT,
                    &format!(
                        "DogTag {tag} is listed on two pets in this request ({} and {}). A tag identifies one animal — open the pet that should not have it and unlink the tag from its own page.",
                        twin.name, p.name
                    ),
                ));
            }
        }
        let held = match store.lookup_pets_by_tag(tag).await {
            Ok(v) => v,
            Err(e) => return Some(unverifiable(e)),
        };
        // Spelled as a `match` rather than `Option::is_none_or`, which is stable only since 1.82
        // while this workspace's MSRV is 1.80 — clippy's `incompatible_msrv` flags it.
        let Some(other) = held.into_iter().find(|r| match client_id {
            Some(id) => r.client_id != id,
            None => true,
        }) else {
            continue;
        };
        let unchanged = match mine {
            Some(v) => v,
            None => match tag_is_unchanged(store, client_id, p, tag).await {
                Ok(v) => v,
                Err(e) => return Some(unverifiable(e)),
            },
        };
        if unchanged {
            continue;
        }
        return Some(dog_tag_conflict(tag, &other));
    }
    None
}

/// Does the STORE already record this payload pet as holding `tag`, under the client being written?
///
/// The grandfather predicate, in one place because both branches of [`reject_dog_tag_conflicts`] ask
/// it and a second copy is how the twin branch came to be unconditional in the first place. Bound to
/// ownership deliberately: `build_pet` honours a caller-supplied `petId`, so a payload naming ANOTHER
/// client's pet would otherwise find that stranger's record already holding the tag and read its own
/// claim as unchanged. A create (`client_id == None`) grandfathers nothing — every pet in a create
/// payload is new by definition, so it has no prior record to be unchanged from.
async fn tag_is_unchanged<L: PetLookup + ?Sized>(
    store: &L,
    client_id: Option<&str>,
    pet: &ClientPet,
    tag: &str,
) -> Result<bool, StoreReadError> {
    let Some(owner) = client_id else {
        return Ok(false);
    };
    Ok(store.lookup_pet(&pet.pet_id).await?.is_some_and(|stored| {
        stored.client_id == owner && stored.pet.dog_tag_id.as_deref() == Some(tag)
    }))
}

/// Why this pet cannot be removed from the shop's records, if it cannot.
///
/// The single definition of "removing this pet would orphan history", shared by BOTH routes that can
/// drop a pet: the surgical [`delete_pet`] and the whole-document [`update_client`]. It has to be
/// shared. An appointment and a verification row carry the pet's `petId`, and the verification
/// history is permanent evidence of a check this shop performed - once the pet it names is gone the
/// row keeps an id that resolves to nothing, so `GET /pets/{id}` 404s and the operator can no longer
/// reach the animal a recorded check concerned. A guard on only one of the two routes reads as an
/// enforced invariant while the other route quietly breaks it.
async fn pet_removal_blocker(store: &Arc<dyn Store>, pet_id: &str) -> Option<&'static str> {
    let appts = store
        .list_appointments(&AppointmentQuery {
            pet_id: Some(pet_id.to_string()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if appts.total > 0 {
        return Some("pet has appointments; delete or reassign them first");
    }
    let verifs = store
        .list_verification_logs(&VerificationQuery {
            pet_id: Some(pet_id.to_string()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if verifs.total > 0 {
        return Some("pet has verifications; its history must be kept");
    }
    None
}

/// The orphan guard for the WHOLE-DOCUMENT client routes.
///
/// `PUT /clients/{id}` replaces `Client.pets` outright, so a payload that simply omits a pet DELETES
/// it - including a pet [`delete_pet`] would refuse to delete. The client form has a per-pet "Remove
/// pet" button, so this is one click, not a hand-written request.
///
/// A DROP is a stored `petId` absent from the payload. That deliberately also catches a payload that
/// re-sends a pet WITHOUT its `petId`: `build_pet` then mints a fresh one, which severs the same link
/// just as thoroughly as omitting the pet, and leaves a second copy of the animal behind to hide it.
///
/// Checked BEFORE anything is written, like every other guard here, so a refusal never leaves a
/// half-applied client. Additions and edits are untouched - only a removal can orphan a row.
async fn reject_orphaning_pet_removals(
    store: &Arc<dyn Store>,
    stored: &[ClientPet],
    payload: &[ClientPet],
) -> Option<Resp> {
    for pet in stored {
        if payload.iter().any(|p| p.pet_id == pet.pet_id) {
            continue;
        }
        let Some(blocker) = pet_removal_blocker(store, &pet.pet_id).await else {
            continue;
        };
        let label = if pet.name.trim().is_empty() { &pet.pet_id } else { &pet.name };
        return Some(err(
            StatusCode::CONFLICT,
            &format!("{label} cannot be removed from this client: {blocker}."),
        ));
    }
    None
}

/// The 409 for a tag another pet already holds. NAMES that pet and its owner, because that is what
/// tells a typo apart from a genuine conflict - a bare "already linked" leaves the operator with no
/// way to find out which record to look at.
fn dog_tag_conflict(tag: &str, other: &PetRow) -> Resp {
    err(
        StatusCode::CONFLICT,
        &format!(
            "DogTag {tag} is already linked to {} ({}). Remove it from that pet first, or check the id.",
            other.pet.name, other.client_name
        ),
    )
}

/// Apply a mutation to ONE pet inside its owner's document, then persist the whole client.
///
/// Pets are stored embedded, so every pet write is a client write; this helper is the single place
/// that knows it. It re-reads the owner, mutates only the addressed pet in place, and writes the
/// document back — so the pet's SIBLINGS, and the appointment/verification rows pointing at them,
/// are untouched. It then rebuilds the owner's search key (the client's key includes its pets' names
/// and tag ids) and resyncs the denormalized labels the calendar and history carry.
///
/// `None` means no client holds a pet with this id, which the routes turn into a 404.
async fn mutate_pet(
    store: &Arc<dyn Store>,
    pet_id: &str,
    f: impl FnOnce(&mut ClientPet),
) -> Option<PetRow> {
    let row = store.get_pet(pet_id).await?;
    let mut c = store.get_client(&row.client_id).await?;
    {
        let p = c.pets.iter_mut().find(|p| p.pet_id == pet_id)?;
        f(p);
    }
    c.updated_at = now();
    c.rebuild_search_key();
    store.put_client(c.clone()).await;
    resync_client_labels(store, &c).await;
    c.pets.iter().find(|p| p.pet_id == pet_id).map(|p| c.pet_row(p))
}

async fn list_pets(State(st): State<AppState>, headers: HeaderMap, Query(q): Query<ListQuery>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let page = st
        .store
        .list_pets(&PetQuery {
            q: ListQuery::opt(&q.q),
            client_id: ListQuery::opt(&q.client_id),
            limit: q.limit(),
            offset: q.offset(),
        })
        .await;
    ok(page_json(page, q.limit(), q.offset(), pet_row_json))
}

async fn get_pet(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    match st.store.get_pet(&id).await {
        Some(r) => ok(pet_row_json(&r)),
        None => err(StatusCode::NOT_FOUND, "pet not found"),
    }
}

async fn create_pet(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PetCreateBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    if body.pet.name.trim().is_empty() {
        return err(StatusCode::BAD_REQUEST, "name is required");
    }
    let mut c = match st.store.get_client(body.client_id.trim()).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "client not found"),
    };
    // A supplied petId is ignored on create: `build_pet` would otherwise let a caller graft a second
    // pet onto an id that already exists elsewhere, and pet ids are what appointments point at.
    let mut pet = build_pet(&body.pet);
    pet.pet_id = uuid::Uuid::new_v4().to_string();
    // `PetBody` carries `dogTagId`, so create is a second way to attach a tag and has to enforce the
    // same one-pet-per-tag rule as [`link_pet_dogtag`]. Guarding only the link route would leave the
    // merge it prevents reachable through the adjacent route with the same field.
    if let Some(tag) = pet.dog_tag_id.as_deref() {
        match conflicting_pet(&st.store, tag, &pet.pet_id).await {
            Ok(Some(other)) => return dog_tag_conflict(tag, &other),
            Ok(None) => {}
            Err(e) => return unverifiable(e),
        }
        // ...and the same microchip cross-check, for the same reason the line above is here: create
        // carries both fields, so a guard on the link route alone would leave the mis-link reachable.
        let check = microchip_check(&st.store, tag, pet.microchip_code.as_deref()).await;
        if check.refuses() {
            return microchip_conflict(&check);
        }
    }
    c.pets.push(pet.clone());
    c.updated_at = now();
    c.rebuild_search_key();
    st.store.put_client(c.clone()).await;
    (StatusCode::CREATED, Json(pet_row_json(&c.pet_row(&pet))))
}

async fn update_pet(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PetPatchBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    // A pet with a blank name is unusable in every list that renders it, so an explicit blank is a
    // request error rather than something to silently accept.
    if body.name.as_ref().is_some_and(|n| n.trim().is_empty()) {
        return err(StatusCode::BAD_REQUEST, "name cannot be blank");
    }
    // The BINDING IS A PAIR, and this route moves the other half of it. Guarding only the places
    // that write a TAG would leave the whole defect reachable in two ordinary steps: link a tag while
    // the pet has no microchip (not comparable, correctly allowed), then set a microchip here that
    // belongs to a different animal, with nothing to refuse it. So the incoming code is checked
    // against the credential held for the tag the pet ALREADY carries.
    //
    // Only an explicitly supplied, non-blank code is a claim. An absent field changes nothing, and a
    // deliberate blank CLEARS the code — which must stay possible, because clearing a wrongly-typed
    // code is exactly how an operator gets out of a mismatch they cannot otherwise correct.
    if let Some(code) = body
        .microchip_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Reads the STORED tag, so a client whose portal predates this field cannot skip the check by
        // omitting one; the tag is not patchable from this route at all.
        //
        // THE FALLIBLE FORM, and it has to be. `get_pet` collapses an unreadable store to `None`,
        // which here would skip the whole `if let` arm and let the write land with no check and no
        // report — the fail-open this feature exists to close, reintroduced through the guard itself.
        // Unlike the CACHE read inside `microchip_check` (which reports `couldNotRead` and proceeds,
        // because that check is evidence and can only tighten), an unresolved PET read is the same
        // uniqueness-class failure `create_pet` and `link_pet_dogtag` already refuse one line away:
        // it is the read that decides WHICH tag is being checked at all.
        let tag = match st.store.try_get_pet(&id).await {
            Ok(row) => row
                .and_then(|r| r.pet.dog_tag_id)
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty()),
            Err(e) => return unverifiable(e),
        };
        if let Some(tag) = tag {
            let check = microchip_check(&st.store, &tag, Some(code)).await;
            if check.refuses() {
                return microchip_conflict(&check);
            }
        }
    }
    let set = |dst: &mut String, src: Option<String>| {
        if let Some(v) = src {
            *dst = v.trim().to_string();
        }
    };
    let updated = mutate_pet(&st.store, &id, |p| {
        set(&mut p.name, body.name);
        set(&mut p.species, body.species);
        set(&mut p.breed, body.breed);
        set(&mut p.sex, body.sex);
        set(&mut p.date_of_birth, body.date_of_birth);
        // Notes keep their whitespace: a multi-line handling note is not a label to trim.
        if let Some(n) = body.notes {
            p.notes = n;
        }
        if let Some(m) = body.microchip_code {
            let m = m.trim();
            p.microchip_code = (!m.is_empty()).then(|| m.to_string());
        }
    })
    .await;
    match updated {
        Some(r) => ok(pet_row_json(&r)),
        None => err(StatusCode::NOT_FOUND, "pet not found"),
    }
}

/// LINK a DogTag to this pet — a write to the SHOP's own record of which tag this pet holds.
///
/// This is not, and cannot be, an issuance: nothing is minted, no credential is created, and nothing
/// is written on chain. It records an id the operator read off the owner's app so that this shop can
/// tell which pet a verification concerned. It is freely reversible ([`unlink_pet_dogtag`]).
///
/// A tag already held by ANOTHER pet is refused with 409 rather than accepted. The tag is the key
/// every check and every on-chain lookup is keyed by, so two pets sharing one would show each other's
/// held credential, each other's on-chain history, and both answer a `?q=<tag>` search - two animals'
/// records silently merged, with a mistyped digit the realistic way in. The refusal NAMES the pet
/// already holding it, because that is what tells a typo apart from a genuine conflict. Re-linking a
/// tag to the pet that already has it stays idempotent: nothing is being merged.
///
/// # The microchip cross-check
///
/// That uniqueness rule keeps two pets off one tag; it says nothing about whether THIS tag belongs to
/// THIS animal. A credential carries `credentialSubject.microchip.code` as a Merkle leaf, so when the
/// shop holds one for the tag and has a microchip on the pet, the two are compared here — the moment
/// the credential and the local record meet — and a mismatch is refused ([`microchip_check`]).
///
/// The verdict is ALWAYS on the response, in all three states, so a client cannot read a missing key
/// as a check that passed. It is emitted here rather than on `GET /pets` for exactly that reason: a
/// list row carries no verdict, so its absence there is unambiguous.
async fn link_pet_dogtag(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<PetDogTagBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let tag = body.dog_tag_id.trim().to_string();
    if tag.is_empty() {
        return err(StatusCode::BAD_REQUEST, "dogTagId is required");
    }
    // Existence first, so an unknown pet reports 404 rather than a conflict with a pet it is not.
    let Some(row) = st.store.get_pet(&id).await else {
        return err(StatusCode::NOT_FOUND, "pet not found");
    };
    match conflicting_pet(&st.store, &tag, &id).await {
        Ok(Some(other)) => return dog_tag_conflict(&tag, &other),
        Ok(None) => {}
        Err(e) => return unverifiable(e),
    }
    let check = microchip_check(&st.store, &tag, row.pet.microchip_code.as_deref()).await;
    if check.refuses() {
        return microchip_conflict(&check);
    }
    match mutate_pet(&st.store, &id, |p| p.dog_tag_id = Some(tag)).await {
        Some(r) => {
            let mut v = pet_row_json(&r);
            v["microchipCheck"] = check.to_json();
            ok(v)
        }
        None => err(StatusCode::NOT_FOUND, "pet not found"),
    }
}

/// UNLINK the DogTag from this pet — clears the shop's own note of which tag the pet holds.
///
/// Deliberately NOT a revocation. The credential and the tag continue to exist, stay valid, and stay
/// verifiable by everyone else; all that changes is that this shop's record no longer says which pet
/// the tag belongs to. Re-linking restores it. On-chain revocation is a different act with a
/// different, permanent, publicly visible effect and does not live on this route.
async fn unlink_pet_dogtag(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    match mutate_pet(&st.store, &id, |p| p.dog_tag_id = None).await {
        Some(r) => ok(pet_row_json(&r)),
        None => err(StatusCode::NOT_FOUND, "pet not found"),
    }
}

/// The credential document(s) this shop HOLDS for the pet's DogTag.
///
/// `POST /import/pull` verifies a customer's credential and writes it to the per-tag document cache
/// (`upsert_client_cache`, keyed by the doc's own `credentialSubject.dogTagId` leaf - the short
/// operator-facing HANDLE). Nothing read that cache back, so an imported credential was stored and
/// then unreachable: the shop could accept a record and never see it again. This is the read side.
///
/// The lookup is an EXACT match on whatever the pet is linked by, and the link route accepts two
/// forms: that handle, or the full on-chain field element an explorer shows. Only the handle can
/// match here - handle -> field element is a Poseidon hash and cannot be inverted, so a
/// field-element link has no handle to look under. An empty list from such a pet therefore means
/// "this shop cannot perform the lookup", NOT "this shop holds nothing", and the caller must render
/// the two differently (`PetTagCredentials` does, off `resolveDogTagId(...).form`). Normalising the
/// two forms here is impossible, not merely unimplemented.
///
/// It returns the STORED DOCUMENT and no verdict. Whether a credential is currently valid is an
/// on-chain question whose answer changes after the import — a root can be revoked the next day — so
/// the caller re-checks it against the chain rather than trusting a verdict frozen at import time.
///
/// A LIST (of zero or one today, since the cache holds one document per tag) because "the credentials
/// of this pet" is plural in the domain: the shape must not have to change when the cache does.
async fn list_pet_credentials(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let row = match st.store.get_pet(&id).await {
        Some(r) => r,
        None => return err(StatusCode::NOT_FOUND, "pet not found"),
    };
    // No tag means no credential can be looked up — reported as an empty list with the reason, so the
    // caller does not read it as "this pet has no credentials".
    let Some(tag) = row.pet.dog_tag_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return ok(json!({ "dogTagId": null, "credentials": [] }));
    };
    let held = st.store.get_client_cache(tag).await;
    ok(json!({
        "dogTagId": tag,
        "credentials": held.map(|d| vec![d]).unwrap_or_default(),
    }))
}

async fn delete_pet(State(st): State<AppState>, headers: HeaderMap, Path(id): Path<String>) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    // Refuse to orphan bookings, mirroring `delete_client`: an appointment or a verification naming
    // this pet must not be left pointing at a pet row that no longer exists. Shared with
    // `reject_orphaning_pet_removals` so the whole-document client route cannot admit what this one
    // refuses.
    if let Some(blocker) = pet_removal_blocker(&st.store, &id).await {
        return err(StatusCode::CONFLICT, blocker);
    }
    let row = match st.store.get_pet(&id).await {
        Some(r) => r,
        None => return err(StatusCode::NOT_FOUND, "pet not found"),
    };
    let mut c = match st.store.get_client(&row.client_id).await {
        Some(c) => c,
        None => return err(StatusCode::NOT_FOUND, "pet not found"),
    };
    c.pets.retain(|p| p.pet_id != id);
    c.updated_at = now();
    c.rebuild_search_key();
    st.store.put_client(c).await;
    ok(json!({ "deleted": true }))
}

// ============================================================================================
// appointments
// ============================================================================================

async fn list_appointments(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    if let Some(s) = ListQuery::opt(&q.status) {
        if !APPOINTMENT_STATES.contains(&s.as_str()) {
            return err(StatusCode::BAD_REQUEST, "unknown status filter");
        }
    }
    let page = st
        .store
        .list_appointments(&AppointmentQuery {
            q: ListQuery::opt(&q.q),
            client_id: ListQuery::opt(&q.client_id),
            pet_id: ListQuery::opt(&q.pet_id),
            status: ListQuery::opt(&q.status),
            from: q.from,
            to: q.to,
            limit: q.limit(),
            offset: q.offset(),
        })
        .await;
    ok(page_json(page, q.limit(), q.offset(), appointment_json))
}

/// Validate + resolve an appointment body against its client, returning the denormalized
/// (clientName, petName) or a request error.
async fn resolve_appointment_target(
    st: &AppState,
    body: &AppointmentBody,
) -> Result<(String, String), Resp> {
    let client = match st.store.get_client(&body.client_id).await {
        Some(c) => c,
        None => return Err(err(StatusCode::BAD_REQUEST, "clientId does not exist")),
    };
    let pet_name = match body.pet_id.as_ref().filter(|s| !s.trim().is_empty()) {
        Some(pid) => match client.pets.iter().find(|p| &p.pet_id == pid) {
            Some(p) => p.name.clone(),
            None => return Err(err(StatusCode::BAD_REQUEST, "petId does not belong to this client")),
        },
        None => String::new(),
    };
    Ok((client.name, pet_name))
}

/// Validate the time window + status shared by create and update.
fn validate_slot(body: &AppointmentBody) -> Option<Resp> {
    if body.start_at == 0 {
        return Some(err(StatusCode::BAD_REQUEST, "startAt is required"));
    }
    if body.end_at != 0 && body.end_at <= body.start_at {
        return Some(err(StatusCode::BAD_REQUEST, "endAt must be after startAt"));
    }
    if let Some(s) = &body.status {
        if !APPOINTMENT_STATES.contains(&s.as_str()) {
            return Some(err(StatusCode::BAD_REQUEST, "unknown status"));
        }
    }
    None
}

/// Default a missing `endAt` to a one-hour slot — the operator books a start time and a service,
/// not an interval, in the common case.
const DEFAULT_SLOT_SECS: u64 = 3600;

async fn create_appointment(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<AppointmentBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    if let Some(e) = validate_slot(&body) {
        return e;
    }
    let (client_name, pet_name) = match resolve_appointment_target(&st, &body).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ts = now();
    let mut a = Appointment {
        appointment_id: uuid::Uuid::new_v4().to_string(),
        client_id: body.client_id.clone(),
        pet_id: body.pet_id.clone().filter(|s| !s.trim().is_empty()),
        service: body.service.trim().to_string(),
        start_at: body.start_at,
        end_at: if body.end_at == 0 { body.start_at + DEFAULT_SLOT_SECS } else { body.end_at },
        status: body.status.clone().unwrap_or_else(|| "scheduled".to_string()),
        notes: body.notes.clone(),
        groomer: body.groomer.trim().to_string(),
        created_at: ts,
        updated_at: ts,
        client_name,
        pet_name,
        search_key: String::new(),
        // Booked in the portal, so it has no external calendar identity. Only `.ics` import sets
        // these (see `calendar_ics::new_from_import`).
        source: None,
        external_uid: None,
    };
    a.rebuild_search_key();
    st.store.put_appointment(a.clone()).await;
    (StatusCode::CREATED, Json(appointment_json(&a)))
}

async fn get_appointment(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let a = match st.store.get_appointment(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "appointment not found"),
    };
    // Include this appointment's verifications so the detail view is one round trip.
    let verifs = st
        .store
        .list_verification_logs(&VerificationQuery {
            appointment_id: Some(id),
            limit: crate::store::MAX_PAGE,
            ..Default::default()
        })
        .await;
    let mut out = appointment_json(&a);
    out.as_object_mut().unwrap().insert(
        "verifications".to_string(),
        json!(verifs.rows.iter().map(verification_json).collect::<Vec<_>>()),
    );
    ok(out)
}

async fn update_appointment(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<AppointmentBody>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    let existing = match st.store.get_appointment(&id).await {
        Some(a) => a,
        None => return err(StatusCode::NOT_FOUND, "appointment not found"),
    };
    if let Some(e) = validate_slot(&body) {
        return e;
    }
    let (client_name, pet_name) = match resolve_appointment_target(&st, &body).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut a = Appointment {
        appointment_id: existing.appointment_id,
        client_id: body.client_id.clone(),
        pet_id: body.pet_id.clone().filter(|s| !s.trim().is_empty()),
        service: body.service.trim().to_string(),
        start_at: body.start_at,
        end_at: if body.end_at == 0 { body.start_at + DEFAULT_SLOT_SECS } else { body.end_at },
        status: body.status.clone().unwrap_or(existing.status),
        notes: body.notes.clone(),
        groomer: body.groomer.trim().to_string(),
        created_at: existing.created_at,
        updated_at: now(),
        client_name,
        pet_name,
        search_key: String::new(),
        // An edit never rewrites the booking's ORIGIN. Dropping `external_uid` here would orphan an
        // imported booking from its source event, so the next re-import of the same file would
        // create a duplicate instead of updating this row.
        source: existing.source,
        external_uid: existing.external_uid,
    };
    a.rebuild_search_key();
    st.store.put_appointment(a.clone()).await;
    ok(appointment_json(&a))
}

async fn delete_appointment(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    // A recorded verification is permanent evidence — deleting the booking it hung off would leave
    // the history pointing at nothing, so cancel the appointment instead.
    let verifs = st
        .store
        .list_verification_logs(&VerificationQuery {
            appointment_id: Some(id.clone()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if verifs.total > 0 {
        return err(
            StatusCode::CONFLICT,
            "appointment has verifications; set status=cancelled instead of deleting",
        );
    }
    if st.store.delete_appointment(&id).await {
        ok(json!({ "deleted": true }))
    } else {
        err(StatusCode::NOT_FOUND, "appointment not found")
    }
}

// ============================================================================================
// verification history ("All verifications")
// ============================================================================================

async fn list_verifications(
    State(st): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    // A typo'd status must be a request error, not a 200 with an empty page — the operator cannot
    // tell that apart from a genuinely empty history.
    if let Some(s) = ListQuery::opt(&q.status) {
        if !VERIFICATION_STATES.contains(&s.as_str()) {
            return err(StatusCode::BAD_REQUEST, "unknown status filter");
        }
    }
    let page = st
        .store
        .list_verification_logs(&VerificationQuery {
            q: ListQuery::opt(&q.q),
            client_id: ListQuery::opt(&q.client_id),
            // `petId` was parsed off the query string but never applied, so `?petId=` silently
            // returned the UNFILTERED history — a caller asking for one pet's checks got every
            // pet's, which reads as "this pet was verified" about verifications that were not its.
            pet_id: ListQuery::opt(&q.pet_id),
            appointment_id: ListQuery::opt(&q.appointment_id),
            status: ListQuery::opt(&q.status),
            purpose: ListQuery::opt(&q.purpose),
            from: q.from,
            to: q.to,
            limit: q.limit(),
            offset: q.offset(),
        })
        .await;
    ok(page_json(page, q.limit(), q.offset(), verification_json))
}

async fn get_verification(
    State(st): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Resp {
    if let Err(e) = crate::routes::require_operator(&st, &headers).await {
        return e;
    }
    match st.store.get_verification_log(&id).await {
        Some(v) => ok(verification_json(&v)),
        None => err(StatusCode::NOT_FOUND, "verification not found"),
    }
}

// ============================================================================================
// verification <-> appointment linkage, called from the verify leg
// ============================================================================================

/// Resolve the `appointmentId` an operator started a verification from into the (appointment,
/// client, pet) context to hang on the session. `Err` when the id does not exist — the operator
/// must not silently get an unlinked verification when they asked for a linked one.
pub async fn resolve_session_context(
    store: &Arc<dyn Store>,
    appointment_id: &str,
) -> Result<SessionContext, String> {
    let a = store
        .get_appointment(appointment_id)
        .await
        .ok_or_else(|| "appointmentId does not exist".to_string())?;
    Ok(SessionContext {
        appointment_id: a.appointment_id,
        // An `.ics`-imported booking has no client yet (see [`Appointment::client_id`]). Carrying an
        // EMPTY client id onto the verification would make the history filterable by a client that
        // does not exist and link the row to nothing — `None` is what "no client" means here.
        client_id: Some(a.client_id).filter(|c| !c.is_empty()),
        pet_id: a.pet_id,
        client_name: a.client_name,
        pet_name: a.pet_name,
    })
}

/// The business context a verification session carries when started from an appointment.
pub struct SessionContext {
    pub appointment_id: String,
    pub client_id: Option<String>,
    pub pet_id: Option<String>,
    pub client_name: String,
    pub pet_name: String,
}

/// Open the history row for a freshly started verification session (status mirrors the session's
/// own `pending`), so an in-flight verification is already visible in "All verifications" and an
/// abandoned one leaves a trace rather than vanishing.
pub async fn start_log(store: &Arc<dyn Store>, s: &VerifySession, ctx: Option<&SessionContext>) {
    let ts = now();
    let mut v = VerificationLog {
        verification_id: s.session_id.clone(),
        appointment_id: s.appointment_id.clone(),
        client_id: s.client_id.clone(),
        pet_id: s.pet_id.clone(),
        purpose: s.purpose.clone(),
        record_type: s.record_type.clone(),
        status: s.status.clone(),
        tx_hash: None,
        nullifier: None,
        dog_tag_id: None,
        disclosed_key_paths: Vec::new(),
        client_name: ctx.map(|c| c.client_name.clone()).unwrap_or_default(),
        pet_name: ctx.map(|c| c.pet_name.clone()).unwrap_or_default(),
        created_at: ts,
        updated_at: ts,
        search_key: String::new(),
    };
    v.rebuild_search_key();
    store.put_verification_log(v).await;
}

/// Attach the verification EVIDENCE once a consent submission has passed validation: the opaque
/// dogTagId the consent was bound to, and the keyPaths the owner chose to disclose.
///
/// `disclosed_key_paths` is EMPTY on an ordinary owner-hidden verification and that is the correct,
/// intended value — the owner revealed nothing, so there is nothing to record. It is never
/// back-filled from any other source, and the disclosed VALUES are never stored at all (they are
/// shown to the verifying operator only), mirroring `VerifySession::disclosed_key_paths`.
///
/// Also mirrors the session's current status, so a COLD submit — one with no operator-started
/// session, hence no row from [`start_log`] — still lands in "All verifications" as it happens
/// rather than only once it settles.
pub async fn attach_evidence(
    store: &Arc<dyn Store>,
    s: &VerifySession,
    dog_tag_id: Option<String>,
    disclosed_key_paths: Vec<String>,
) {
    let ts = now();
    let mut v = store
        .get_verification_log(&s.session_id)
        .await
        .unwrap_or_else(|| VerificationLog {
            verification_id: s.session_id.clone(),
            appointment_id: s.appointment_id.clone(),
            client_id: s.client_id.clone(),
            pet_id: s.pet_id.clone(),
            purpose: s.purpose.clone(),
            record_type: s.record_type.clone(),
            created_at: ts,
            ..Default::default()
        });
    v.status = s.status.clone();
    v.dog_tag_id = dog_tag_id;
    v.disclosed_key_paths = disclosed_key_paths;
    v.updated_at = ts;
    v.rebuild_search_key();
    store.put_verification_log(v).await;
}

/// Record a verification's terminal outcome. Called from EVERY site where the verify session
/// reaches a new status — both arms of the detached broadcast — so the history never disagrees
/// with the session. Idempotent: re-running it with the same session is a no-op beyond the
/// timestamp.
///
/// `tx_hash` follows the session's own convention: on `error` the session's `tx_hash` field holds
/// the failure message rather than a hash, and this row mirrors that verbatim so the operator sees
/// the same diagnostic the portal shows.
///
/// This is also the HANDOFF point for the row's denormalized client labels: [`resync_client_labels`]
/// refuses to touch a row while it is in flight, so a rename that landed mid-verification is picked
/// up here instead, by re-reading the client as the row settles.
pub async fn finish_log(
    store: &Arc<dyn Store>,
    s: &VerifySession,
    status: &str,
    tx_hash: Option<String>,
    nullifier: Option<String>,
) {
    let ts = now();
    let mut v = match store.get_verification_log(&s.session_id).await {
        Some(v) => v,
        // Defensive: a verification whose row was never opened (e.g. a session created before this
        // feature shipped) still gets one, so "All verifications" is genuinely complete.
        None => VerificationLog {
            verification_id: s.session_id.clone(),
            appointment_id: s.appointment_id.clone(),
            client_id: s.client_id.clone(),
            pet_id: s.pet_id.clone(),
            purpose: s.purpose.clone(),
            record_type: s.record_type.clone(),
            created_at: ts,
            ..Default::default()
        },
    };
    v.status = status.to_string();
    v.tx_hash = tx_hash;
    if nullifier.is_some() {
        v.nullifier = nullifier;
    }
    refresh_client_labels(store, &mut v).await;
    v.updated_at = ts;
    v.rebuild_search_key();
    store.put_verification_log(v).await;
}

/// Re-read the client a history row points at and refresh its denormalized labels from the CURRENT
/// record.
///
/// An ad-hoc verification (no client) and a client that can no longer be read both leave the row's
/// existing labels untouched: an unreadable client is not evidence that the names were wrong, so
/// blanking them would destroy context rather than correct it.
async fn refresh_client_labels(store: &Arc<dyn Store>, v: &mut VerificationLog) {
    let client_id = match v.client_id.as_deref() {
        Some(id) => id,
        None => return,
    };
    let c = match store.get_client(client_id).await {
        Some(c) => c,
        None => return,
    };
    v.pet_name = v
        .pet_id
        .as_ref()
        .and_then(|pid| c.pets.iter().find(|p| &p.pet_id == pid))
        .map(|p| p.name.clone())
        .unwrap_or_default();
    v.client_name = c.name;
}

// ============================================================================================
// router
// ============================================================================================

/// The shop CRM routes (operator-gated). Mounted for every deployment role: unlike issuance, a
/// client/appointment book is not role-specific, and the vet portal simply does not call them yet.
pub fn crm_router() -> Router<AppState> {
    Router::new()
        .route("/clients", get(list_clients).post(create_client))
        .route(
            "/clients/:id",
            get(get_client).put(update_client).delete(delete_client),
        )
        // Pets are a collection of their own — searchable and addressable without going through the
        // owner — while still being STORED inside the client document. See the pets section above.
        .route("/pets", get(list_pets).post(create_pet))
        .route("/pets/:id", get(get_pet).put(update_pet).delete(delete_pet))
        // LINK / UNLINK the shop's record of which tag this pet holds. Neither touches the chain:
        // `DELETE` here is a local disassociation, NOT a revocation. See the handler docs.
        .route("/pets/:id/dogtag", post(link_pet_dogtag).delete(unlink_pet_dogtag))
        // The credentials this shop holds for the pet's tag — the read side of `POST /import/pull`.
        .route("/pets/:id/credentials", get(list_pet_credentials))
        .route("/appointments", get(list_appointments).post(create_appointment))
        .route(
            "/appointments/:id",
            get(get_appointment).put(update_appointment).delete(delete_appointment),
        )
        .route("/verifications", get(list_verifications))
        .route("/verifications/:id", get(get_verification))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A [`PetLookup`] whose every read fails, standing in for a store that cannot be reached.
    ///
    /// This is the whole point of narrowing the guards' dependency to two methods: a stub for the
    /// full [`Store`] trait would be nearly sixty. It proves the GUARDS refuse rather than admit a
    /// write they could not check. It does NOT prove `MongoStore` reports its driver errors instead
    /// of swallowing them — that impl is behind `#[cfg(feature = "mongo")]`, which `cargo test` does
    /// not compile, so `cargo check -p vet-api --features mongo` is the only signal on that side.
    struct UnreachableStore;

    #[async_trait::async_trait]
    impl PetLookup for UnreachableStore {
        async fn lookup_pet(&self, _pet_id: &str) -> Result<Option<PetRow>, StoreReadError> {
            Err(StoreReadError("connection reset".into()))
        }
        async fn lookup_pets_by_tag(&self, _tag: &str) -> Result<Vec<PetRow>, StoreReadError> {
            Err(StoreReadError("connection reset".into()))
        }
    }

    fn pet(pet_id: &str, name: &str, tag: Option<&str>) -> ClientPet {
        ClientPet {
            pet_id: pet_id.to_string(),
            name: name.to_string(),
            species: "dog".into(),
            breed: String::new(),
            sex: String::new(),
            date_of_birth: String::new(),
            notes: String::new(),
            dog_tag_id: tag.map(str::to_string),
            // These guards are about pet ids and tag uniqueness; the microchip cross-check is a
            // separate rule with its own tests (`crate::microchip` and `tests/pets.rs`).
            microchip_code: None,
        }
    }

    fn status(r: &Option<Resp>) -> Option<StatusCode> {
        r.as_ref().map(|(s, _)| *s)
    }

    #[tokio::test]
    async fn a_tag_conflict_that_cannot_be_read_refuses_the_write() {
        // "No pet holds this tag" and "the store did not answer" must not be the same answer: the
        // second one admitting the write is exactly the two-animals-on-one-tag merge the guard is
        // for, arriving silently.
        let out = reject_dog_tag_conflicts(&UnreachableStore, &[pet("p1", "Rex", Some("4"))], None)
            .await;
        assert_eq!(status(&out), Some(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn a_pet_id_ownership_check_that_cannot_be_read_refuses_the_write() {
        let out =
            reject_foreign_pet_ids(&UnreachableStore, &[pet("p1", "Rex", None)], Some("c1")).await;
        assert_eq!(status(&out), Some(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn the_grandfather_lookup_cannot_be_assumed_absent_either() {
        // The exemption is decided by a store read too, so an unreadable store must not be allowed
        // to resolve as "not grandfathered" NOR as "grandfathered" — both are guesses about state
        // nobody could see. It refuses.
        let out = reject_dog_tag_conflicts(
            &UnreachableStore,
            &[pet("p1", "Rex", Some("4")), pet("p2", "Milo", Some("4"))],
            Some("c1"),
        )
        .await;
        assert_eq!(status(&out), Some(StatusCode::SERVICE_UNAVAILABLE));
    }

    #[tokio::test]
    async fn an_unreadable_store_never_reports_a_conflicting_pet() {
        // The surgical pet routes read through the same seam, so their `Ok(None)` must mean the tag
        // is free and never "the lookup failed".
        assert!(conflicting_pet(&UnreachableStore, "4", "p1").await.is_err());
    }
}
