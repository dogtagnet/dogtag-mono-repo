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
use crate::store::{
    Appointment, AppointmentQuery, Client, ClientPet, ClientQuery, Page, PetQuery, PetRow, Store,
    VerificationLog, VerificationQuery, VerifySession, APPOINTMENT_STATES, VERIFICATION_STATES,
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
    if let Some(conflict) = reject_dog_tag_conflicts(&st.store, &pets, None).await {
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
        // exact string by both `find_pets_by_dog_tag` and the held-document cache lookup, so storing
        // a pasted " 4" would silently defeat the one-pet-per-tag guard AND miss the credential
        // filed under "4" - one stray character reintroducing two separate bugs.
        dog_tag_id: p
            .dog_tag_id
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
        reject_dog_tag_conflicts(&st.store, &pets, Some(&existing.client_id)).await
    {
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
    let pet_name_of = |pet_id: Option<&String>| {
        pet_id
            .and_then(|pid| c.pets.iter().find(|p| &p.pet_id == pid))
            .map(|p| p.name.clone())
            .unwrap_or_default()
    };

    let appts = store
        .list_appointments(&AppointmentQuery {
            client_id: Some(c.client_id.clone()),
            limit: crate::store::MAX_PAGE,
            ..Default::default()
        })
        .await;
    for mut a in appts.rows {
        let pet_name = pet_name_of(a.pet_id.as_ref());
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
        let pet_name = pet_name_of(v.pet_id.as_ref());
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
async fn conflicting_pet(store: &Arc<dyn Store>, tag: &str, self_pet_id: &str) -> Option<PetRow> {
    store
        .find_pets_by_dog_tag(tag)
        .await
        .into_iter()
        .find(|r| r.pet.pet_id != self_pet_id)
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
///    of them stored yet - a case no lookup against the store could see. That check is
///    unconditional: two pets carrying one tag in a single write is the merge itself arriving,
///    whichever of them held it before.
///  - What is validated is the DELTA, not the state. A pet whose STORED record already carries the
///    tag it is submitting is grandfathered, because this rule postdates the data: a store written
///    before it can already hold two pets on one tag, and without this an operator fixing a phone
///    number would be refused over a DogTag they never touched, on a conflict they cannot resolve
///    from that form. Only a tag that is NEW or CHANGED for a pet is a claim, and every claim is
///    still checked - so no fresh duplicate can be introduced by any route.
async fn reject_dog_tag_conflicts(
    store: &Arc<dyn Store>,
    pets: &[ClientPet],
    client_id: Option<&str>,
) -> Option<Resp> {
    for (i, p) in pets.iter().enumerate() {
        let Some(tag) = p.dog_tag_id.as_deref() else {
            continue;
        };
        if let Some(twin) = pets[..i].iter().find(|q| q.dog_tag_id.as_deref() == Some(tag)) {
            return Some(err(
                StatusCode::CONFLICT,
                &format!(
                    "DogTag {tag} is listed on two pets in this request ({} and {}). A tag identifies one animal, so give each pet its own id.",
                    twin.name, p.name
                ),
            ));
        }
        let held = store.find_pets_by_dog_tag(tag).await;
        let Some(other) = held
            .into_iter()
            .find(|r| client_id.is_none_or(|id| r.client_id != id))
        else {
            continue;
        };
        // Reached only when the write WOULD be refused, so the grandfather lookup costs a store
        // read on the rare rejecting path rather than on every tagged pet of every client write.
        let unchanged = store
            .get_pet(&p.pet_id)
            .await
            .is_some_and(|stored| stored.pet.dog_tag_id.as_deref() == Some(tag));
        if unchanged {
            continue;
        }
        return Some(dog_tag_conflict(tag, &other));
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
        if let Some(other) = conflicting_pet(&st.store, tag, &pet.pet_id).await {
            return dog_tag_conflict(tag, &other);
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
    if st.store.get_pet(&id).await.is_none() {
        return err(StatusCode::NOT_FOUND, "pet not found");
    }
    if let Some(other) = conflicting_pet(&st.store, &tag, &id).await {
        return dog_tag_conflict(&tag, &other);
    }
    match mutate_pet(&st.store, &id, |p| p.dog_tag_id = Some(tag)).await {
        Some(r) => ok(pet_row_json(&r)),
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
    // this pet must not be left pointing at a pet row that no longer exists.
    let appts = st
        .store
        .list_appointments(&AppointmentQuery {
            pet_id: Some(id.clone()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if appts.total > 0 {
        return err(
            StatusCode::CONFLICT,
            "pet has appointments; delete or reassign them first",
        );
    }
    let verifs = st
        .store
        .list_verification_logs(&VerificationQuery {
            pet_id: Some(id.clone()),
            limit: 1,
            ..Default::default()
        })
        .await;
    if verifs.total > 0 {
        return err(
            StatusCode::CONFLICT,
            "pet has verifications; its history must be kept",
        );
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
