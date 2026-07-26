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
    Appointment, AppointmentQuery, Client, ClientPet, ClientQuery, Page, Store, VerificationLog,
    VerificationQuery, VerifySession, APPOINTMENT_STATES,
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
    let ts = now();
    let mut c = Client {
        client_id: uuid::Uuid::new_v4().to_string(),
        name: body.name.trim().to_string(),
        email: body.email.trim().to_string(),
        phone: body.phone.trim().to_string(),
        address: body.address.trim().to_string(),
        notes: body.notes,
        pets: body.pets.iter().map(build_pet).collect(),
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
        dog_tag_id: p.dog_tag_id.clone().filter(|s| !s.trim().is_empty()),
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
    let mut c = Client {
        client_id: existing.client_id,
        name: body.name.trim().to_string(),
        email: body.email.trim().to_string(),
        phone: body.phone.trim().to_string(),
        address: body.address.trim().to_string(),
        notes: body.notes,
        pets: body.pets.iter().map(build_pet).collect(),
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

/// After a client edit, refresh the denormalized `clientName`/`petName` on that client's
/// appointments (bounded to one page — a single client's booking list, not the whole collection).
async fn resync_client_labels(store: &Arc<dyn Store>, c: &Client) {
    let page = store
        .list_appointments(&AppointmentQuery {
            client_id: Some(c.client_id.clone()),
            limit: crate::store::MAX_PAGE,
            ..Default::default()
        })
        .await;
    for mut a in page.rows {
        let pet_name = a
            .pet_id
            .as_ref()
            .and_then(|pid| c.pets.iter().find(|p| &p.pet_id == pid))
            .map(|p| p.name.clone())
            .unwrap_or_default();
        if a.client_name == c.name && a.pet_name == pet_name {
            continue;
        }
        a.client_name = c.name.clone();
        a.pet_name = pet_name;
        a.rebuild_search_key();
        a.updated_at = now();
        store.put_appointment(a).await;
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
    let page = st
        .store
        .list_verification_logs(&VerificationQuery {
            q: ListQuery::opt(&q.q),
            client_id: ListQuery::opt(&q.client_id),
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
        client_id: Some(a.client_id),
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
    v.updated_at = ts;
    v.rebuild_search_key();
    store.put_verification_log(v).await;
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
        .route("/appointments", get(list_appointments).post(create_appointment))
        .route(
            "/appointments/:id",
            get(get_appointment).put(update_appointment).delete(delete_appointment),
        )
        .route("/verifications", get(list_verifications))
        .route("/verifications/:id", get(get_verification))
        // POST-only alias kept out of the resource paths: nothing here mutates a verification, the
        // verify leg owns that.
        .route("/verifications/search", post(list_verifications_post))
}

/// `POST /verifications/search` — the same filter surface as the GET, for callers that would rather
/// send a JSON body than a query string (long free-text needles, many filters).
async fn list_verifications_post(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(q): Json<ListQuery>,
) -> Resp {
    list_verifications(State(st), headers, Query(q)).await
}
