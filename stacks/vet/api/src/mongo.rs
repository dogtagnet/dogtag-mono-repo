//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal-only in prod (impl
//! architecture: not internet-exposed). The jti collection uses a UNIQUE index so `consume_jti` is
//! atomic (insert-or-fail) — the one-time guarantee for record/verify share JWTs (§11.4).

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
// `Client` is aliased: the CRM entity `store::Client` (a shop customer) owns that name in this file.
use mongodb::{Client as MongoClient, Collection, Database, IndexModel};

use crate::store::{
    clamp_limit, Appointment, AppointmentQuery, ApptReplica, Client, ClientPet, ClientQuery,
    CustodyBlob, GcalEventMap, GcalSyncState, IssuerSettings, Page, PetQuery, PetRow,
    ProfileIssueSession, Record, Store, VerificationLog, VerificationQuery, VerifySession,
};

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    /// Connect and ensure the unique jti index exists, plus the CRM query indexes.
    pub async fn connect(uri: &str, db_name: &str) -> Result<Self, mongodb::error::Error> {
        let client = MongoClient::with_uri_str(uri).await?;
        let db = client.database(db_name);
        let jti: Collection<Document> = db.collection("jwt_jti");
        let idx = IndexModel::builder()
            .keys(doc! { "jti": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build();
        jti.create_index(idx).await?;
        let store = MongoStore { db };
        store.ensure_crm_indexes().await?;
        Ok(store)
    }

    /// Index the fields the CRM list queries filter and sort on — the equality keys plus the
    /// range/sort keys — so the ordinary list and calendar views stay bounded index seeks as the
    /// collections grow rather than collection scans.
    ///
    /// `searchKey` is the deliberate exception: [`search_filter`] emits an UNANCHORED `$regex`, which
    /// no B-tree index can serve as a bounded seek, so free-text search stays a scan. What the
    /// denormalized key buys is that the scan touches ONE field per row instead of N.
    async fn ensure_crm_indexes(&self) -> Result<(), mongodb::error::Error> {
        let unique = |keys: Document| {
            IndexModel::builder()
                .keys(keys)
                .options(IndexOptions::builder().unique(true).build())
                .build()
        };
        let plain = |keys: Document| IndexModel::builder().keys(keys).build();

        let clients: Collection<Document> = self.db.collection("crm_clients");
        clients.create_index(unique(doc! { "clientId": 1 })).await?;
        // searchKey is the single free-text field (scanned, never seeked — see above); updatedAt is
        // the list sort.
        clients.create_index(plain(doc! { "searchKey": 1 })).await?;
        clients.create_index(plain(doc! { "updatedAt": -1 })).await?;
        // A pet is addressed in its own right (`/pets/{petId}`) but stored inside its owner, so the
        // by-id lookup is a multikey seek into the embedded array rather than a top-level key.
        clients.create_index(plain(doc! { "pets.petId": 1 })).await?;

        let appts: Collection<Document> = self.db.collection("crm_appointments");
        appts.create_index(unique(doc! { "appointmentId": 1 })).await?;
        // startAt leads: it is BOTH the calendar's range filter and the list sort, so a compound
        // index with the equality filters after it serves both without a separate sort stage.
        appts.create_index(plain(doc! { "startAt": 1 })).await?;
        appts.create_index(plain(doc! { "clientId": 1, "startAt": 1 })).await?;
        appts.create_index(plain(doc! { "status": 1, "startAt": 1 })).await?;
        appts.create_index(plain(doc! { "searchKey": 1 })).await?;

        let verifs: Collection<Document> = self.db.collection("crm_verifications");
        verifs.create_index(unique(doc! { "verificationId": 1 })).await?;
        verifs.create_index(plain(doc! { "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "clientId": 1, "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "petId": 1, "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "appointmentId": 1, "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "status": 1, "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "purpose": 1, "createdAt": -1 })).await?;
        verifs.create_index(plain(doc! { "searchKey": 1 })).await?;
        Ok(())
    }

    fn crm_clients(&self) -> Collection<Client> {
        self.db.collection("crm_clients")
    }
    fn crm_appointments(&self) -> Collection<Appointment> {
        self.db.collection("crm_appointments")
    }
    fn crm_verifications(&self) -> Collection<VerificationLog> {
        self.db.collection("crm_verifications")
    }

    fn records(&self) -> Collection<Record> {
        self.db.collection("records")
    }
    fn sessions(&self) -> Collection<VerifySession> {
        self.db.collection("verify_sessions")
    }
    fn settings(&self) -> Collection<IssuerSettings> {
        self.db.collection("issuer_settings")
    }
    fn custody(&self) -> Collection<CustodyBlob> {
        self.db.collection("custody")
    }
}

#[async_trait]
impl Store for MongoStore {
    async fn put_record(&self, r: Record) {
        let _ = self
            .records()
            .replace_one(doc! { "record_id": &r.record_id }, &r)
            .upsert(true)
            .await;
    }
    async fn get_record(&self, id: &str) -> Option<Record> {
        self.records()
            .find_one(doc! { "record_id": id })
            .await
            .ok()
            .flatten()
    }
    async fn update_record(&self, r: Record) {
        self.put_record(r).await;
    }
    async fn list_records(&self) -> Vec<Record> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;
        // Most-recent first (created_at desc). Records with no created_at (legacy) sort last.
        let opts = FindOptions::builder().sort(doc! { "created_at": -1 }).build();
        match self.records().find(doc! {}).with_options(opts).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }
    async fn has_prepared(&self) -> bool {
        self.records()
            .find_one(doc! { "status": "prepared" })
            .await
            .ok()
            .flatten()
            .is_some()
    }
    async fn record_by_confirmed_tx(&self, tx_hash: &str) -> Option<Record> {
        self.records()
            .find_one(doc! { "confirmed_tx_hash": tx_hash })
            .await
            .ok()
            .flatten()
    }

    async fn put_session(&self, s: VerifySession) {
        let _ = self
            .sessions()
            .replace_one(doc! { "session_id": &s.session_id }, &s)
            .upsert(true)
            .await;
    }
    async fn get_session(&self, id: &str) -> Option<VerifySession> {
        self.sessions()
            .find_one(doc! { "session_id": id })
            .await
            .ok()
            .flatten()
    }
    async fn update_session(&self, s: VerifySession) {
        self.put_session(s).await;
    }
    async fn list_sessions(&self) -> Vec<VerifySession> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;
        // Most-recent first; legacy rows without created_at sort last.
        let opts = FindOptions::builder()
            .sort(doc! { "created_at": -1, "updated_at": -1, "session_id": -1 })
            .build();
        match self.sessions().find(doc! {}).with_options(opts).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    async fn consume_jti(&self, jti: &str) -> bool {
        // insert-or-fail against the unique index == atomic one-time consume.
        let coll: Collection<Document> = self.db.collection("jwt_jti");
        coll.insert_one(doc! { "jti": jti }).await.is_ok()
    }

    async fn put_share_token(&self, token: &str, record_id: &str, exp: u64) {
        let coll: Collection<Document> = self.db.collection("share_tokens");
        let _ = coll
            .replace_one(
                doc! { "token": token },
                doc! { "token": token, "record_id": record_id, "exp": exp as i64 },
            )
            .upsert(true)
            .await;
    }
    async fn take_share_token(&self, token: &str) -> Option<String> {
        // find_one_and_delete is atomic == one-time consume; then enforce expiry on the read.
        let coll: Collection<Document> = self.db.collection("share_tokens");
        let d = coll
            .find_one_and_delete(doc! { "token": token })
            .await
            .ok()
            .flatten()?;
        let exp = d.get_i64("exp").unwrap_or(0) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            d.get_str("record_id").ok().map(|s| s.to_string())
        }
    }

    async fn put_export_token(&self, token: &str, session_id: &str, exp: u64) {
        let coll: Collection<Document> = self.db.collection("export_tokens");
        let _ = coll
            .replace_one(
                doc! { "token": token },
                doc! { "token": token, "session_id": session_id, "exp": exp as i64 },
            )
            .upsert(true)
            .await;
    }
    async fn peek_export_token(&self, token: &str) -> Option<String> {
        // NON-consuming read; enforce expiry.
        let coll: Collection<Document> = self.db.collection("export_tokens");
        let d = coll
            .find_one(doc! { "token": token })
            .await
            .ok()
            .flatten()?;
        let exp = d.get_i64("exp").unwrap_or(0) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            d.get_str("session_id").ok().map(|s| s.to_string())
        }
    }
    async fn next_dog_tag_id(&self) -> u64 {
        // atomic counter via findOneAndUpdate($inc) (mirrors admin mongo.rs:145).
        use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
        let coll: Collection<Document> = self.db.collection("counters");
        let opts = FindOneAndUpdateOptions::builder()
            .upsert(true)
            .return_document(ReturnDocument::After)
            .build();
        let d = coll
            .find_one_and_update(
                doc! { "_id": "dog_tag_id" },
                doc! { "$inc": { "seq": 1i64 } },
            )
            .with_options(opts)
            .await
            .ok()
            .flatten();
        d.and_then(|d| d.get_i64("seq").ok()).unwrap_or(1) as u64
    }
    async fn put_profile_session(&self, s: ProfileIssueSession) {
        let coll: Collection<ProfileIssueSession> = self.db.collection("profile_sessions");
        let _ = coll
            .replace_one(doc! { "session_id": &s.session_id }, &s)
            .upsert(true)
            .await;
    }
    async fn get_profile_session(&self, session_id: &str) -> Option<ProfileIssueSession> {
        let coll: Collection<ProfileIssueSession> = self.db.collection("profile_sessions");
        coll.find_one(doc! { "session_id": session_id })
            .await
            .ok()
            .flatten()
    }
    async fn update_profile_session(&self, s: ProfileIssueSession) {
        self.put_profile_session(s).await;
    }
    async fn list_profile_sessions(&self) -> Vec<ProfileIssueSession> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;
        // Most-recent first, mirroring `list_sessions`.
        let coll: Collection<ProfileIssueSession> = self.db.collection("profile_sessions");
        let opts = FindOptions::builder()
            .sort(doc! { "created_at": -1, "session_id": -1 })
            .build();
        match coll.find(doc! {}).with_options(opts).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }
    async fn put_bind_token(&self, token: &str, session_id: &str, exp: u64) {
        let coll: Collection<Document> = self.db.collection("bind_tokens");
        let _ = coll
            .replace_one(
                doc! { "token": token },
                doc! { "token": token, "session_id": session_id, "exp": exp as i64 },
            )
            .upsert(true)
            .await;
    }
    async fn peek_bind_token(&self, token: &str) -> Option<String> {
        let coll: Collection<Document> = self.db.collection("bind_tokens");
        let d = coll
            .find_one(doc! { "token": token })
            .await
            .ok()
            .flatten()?;
        let exp = d.get_i64("exp").unwrap_or(0) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            d.get_str("session_id").ok().map(|s| s.to_string())
        }
    }
    async fn take_bind_token(&self, token: &str) -> Option<String> {
        // find_one_and_delete is atomic == one-time consume; then enforce expiry.
        let coll: Collection<Document> = self.db.collection("bind_tokens");
        let d = coll
            .find_one_and_delete(doc! { "token": token })
            .await
            .ok()
            .flatten()?;
        let exp = d.get_i64("exp").unwrap_or(0) as u64;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        if now > exp {
            None
        } else {
            d.get_str("session_id").ok().map(|s| s.to_string())
        }
    }

    async fn get_settings(&self) -> IssuerSettings {
        self.settings()
            .find_one(doc! { "_id": "singleton" })
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }
    async fn put_settings(&self, s: IssuerSettings) {
        let _ = self
            .settings()
            .replace_one(doc! { "_id": "singleton" }, &s)
            .upsert(true)
            .await;
    }

    async fn get_custody(&self) -> Option<CustodyBlob> {
        self.custody()
            .find_one(doc! { "_id": "singleton" })
            .await
            .ok()
            .flatten()
    }
    async fn put_custody(&self, blob: CustodyBlob) {
        let _ = self
            .custody()
            .replace_one(doc! { "_id": "singleton" }, &blob)
            .upsert(true)
            .await;
    }

    async fn put_op_session(&self, token: String) {
        let coll: Collection<Document> = self.db.collection("op_sessions");
        let _ = coll.insert_one(doc! { "token": token }).await;
    }
    async fn has_op_session(&self, token: &str) -> bool {
        let coll: Collection<Document> = self.db.collection("op_sessions");
        coll.find_one(doc! { "token": token })
            .await
            .ok()
            .flatten()
            .is_some()
    }

    async fn upsert_client_cache(&self, dog_tag_id: String, doc_v: serde_json::Value) {
        let coll: Collection<Document> = self.db.collection("client_cache");
        let bson = mongodb::bson::to_bson(&doc_v).unwrap_or(mongodb::bson::Bson::Null);
        let _ = coll
            .replace_one(
                doc! { "dog_tag_id": &dog_tag_id },
                doc! { "dog_tag_id": &dog_tag_id, "doc": bson },
            )
            .upsert(true)
            .await;
    }
    async fn get_client_cache(&self, dog_tag_id: &str) -> Option<serde_json::Value> {
        let coll: Collection<Document> = self.db.collection("client_cache");
        let d = coll
            .find_one(doc! { "dog_tag_id": dog_tag_id })
            .await
            .ok()
            .flatten()?;
        d.get("doc")
            .and_then(|b| mongodb::bson::from_bson(b.clone()).ok())
    }

    // ---- appointment replica (Phase 7) ----
    async fn get_appt(&self, id: &str) -> Option<ApptReplica> {
        let coll: Collection<ApptReplica> = self.db.collection("appt_replica");
        coll.find_one(doc! { "appointment_id": id })
            .await
            .ok()
            .flatten()
    }
    async fn put_appt(&self, a: ApptReplica) {
        let coll: Collection<ApptReplica> = self.db.collection("appt_replica");
        let _ = coll
            .replace_one(doc! { "appointment_id": &a.appointment_id }, &a)
            .upsert(true)
            .await;
    }
    async fn appts_updated_since(&self, since: u64) -> Vec<ApptReplica> {
        let coll: Collection<ApptReplica> = self.db.collection("appt_replica");
        let mut out = Vec::new();
        if let Ok(mut cur) = coll
            .find(doc! { "updatedAt": { "$gte": since as i64 } })
            .await
        {
            use futures::StreamExt;
            while let Some(Ok(a)) = cur.next().await {
                out.push(a);
            }
        }
        out
    }
    async fn record_idempotency_key(&self, key: &str) -> bool {
        // unique index gives atomic insert-or-fail (mirrors consume_jti).
        let coll: Collection<Document> = self.db.collection("idempotency_keys");
        let idx = IndexModel::builder()
            .keys(doc! { "key": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build();
        let _ = coll.create_index(idx).await;
        coll.insert_one(doc! { "key": key }).await.is_ok()
    }

    // ---- gcal mapping + sync state ----
    async fn put_gcal_map(&self, m: GcalEventMap) {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        let _ = coll
            .replace_one(doc! { "google_event_id": &m.google_event_id }, &m)
            .upsert(true)
            .await;
    }
    async fn get_gcal_map_by_appt(&self, appointment_id: &str) -> Option<GcalEventMap> {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        coll.find_one(doc! { "appointment_id": appointment_id })
            .await
            .ok()
            .flatten()
    }
    async fn get_gcal_map_by_event(&self, google_event_id: &str) -> Option<GcalEventMap> {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        coll.find_one(doc! { "google_event_id": google_event_id })
            .await
            .ok()
            .flatten()
    }
    async fn all_gcal_maps(&self) -> Vec<GcalEventMap> {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        let mut out = Vec::new();
        if let Ok(mut cur) = coll.find(doc! {}).await {
            use futures::StreamExt;
            while let Some(Ok(m)) = cur.next().await {
                out.push(m);
            }
        }
        out
    }
    async fn delete_gcal_map_by_event(&self, google_event_id: &str) {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        let _ = coll
            .delete_one(doc! { "google_event_id": google_event_id })
            .await;
    }
    async fn wipe_gcal_mirror(&self) {
        let coll: Collection<Document> = self.db.collection("gcal_event_map");
        let _ = coll.delete_many(doc! {}).await;
    }
    async fn get_sync_state(&self) -> GcalSyncState {
        let coll: Collection<GcalSyncState> = self.db.collection("gcal_sync_state");
        coll.find_one(doc! { "_id": "singleton" })
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }
    async fn put_sync_state(&self, s: GcalSyncState) {
        let coll: Collection<GcalSyncState> = self.db.collection("gcal_sync_state");
        let _ = coll
            .replace_one(doc! { "_id": "singleton" }, &s)
            .upsert(true)
            .await;
    }

    // ---- shop CRM: clients ----
    //
    // KNOWN GAP (deliberately not fixed here). Like every other `Store` write in this file, the CRM
    // writes below discard the driver result, so a transient Mongo failure (unreachable, expired
    // auth, replica-set step-down) still lets the route answer 201/200. The trait returns unit
    // repo-wide; surfacing the error means widening `Store` to `Result`, which is its own change.
    async fn put_client(&self, c: Client) {
        let _ = self
            .crm_clients()
            .replace_one(doc! { "clientId": &c.client_id }, &c)
            .upsert(true)
            .await;
    }
    async fn get_client(&self, id: &str) -> Option<Client> {
        self.crm_clients().find_one(doc! { "clientId": id }).await.ok().flatten()
    }
    async fn delete_client(&self, id: &str) -> bool {
        self.crm_clients()
            .delete_one(doc! { "clientId": id })
            .await
            .map(|r| r.deleted_count > 0)
            .unwrap_or(false)
    }
    async fn list_clients(&self, q: &ClientQuery) -> Page<Client> {
        let filter = merge(vec![search_filter(q.q.as_deref())]);
        let coll = self.crm_clients();
        let total = coll.count_documents(filter.clone()).await.unwrap_or(0);
        let rows = collect_page(
            coll.find(filter)
                .sort(doc! { "updatedAt": -1, "clientId": 1 })
                .skip(q.offset as u64)
                .limit(clamp_limit(q.limit) as i64),
        )
        .await;
        Page { rows, total }
    }

    // ---- shop CRM: pets ----
    ///
    /// `$unwind` is what makes this a PETS query rather than a clients query: it emits one document
    /// per pet, so the `$match`/`$sort`/`$skip`/`$limit` after it filter, order and page PETS. That is
    /// the whole reason this cannot be folded over `list_clients` — `total` has to count pets, and a
    /// client page boundary falls between clients, not between pets.
    async fn list_pets(&self, q: &PetQuery) -> Page<PetRow> {
        let mut pipeline: Vec<Document> = Vec::new();
        // Narrow BEFORE the unwind when an owner is given, so the index on `clientId` still applies
        // and one client's pets never require expanding the whole collection.
        if let Some(owner) = eq_filter("clientId", q.client_id.as_deref()) {
            pipeline.push(doc! { "$match": owner });
        }
        pipeline.push(doc! { "$unwind": "$pets" });
        // The per-pet needle, mirroring `PetRow::search_key`: the pet's own fields plus the OWNER's
        // name. `$ifNull` on every part is load-bearing — `$concat` returns null if ANY argument is
        // null, so one absent `dogTagId` would otherwise blank the whole key and make the pet
        // unsearchable by any term at all.
        pipeline.push(doc! { "$addFields": {
            "petSearchKey": { "$toLower": { "$concat": [
                { "$ifNull": ["$pets.name", ""] }, " ",
                { "$ifNull": ["$pets.species", ""] }, " ",
                { "$ifNull": ["$pets.breed", ""] }, " ",
                { "$ifNull": ["$pets.sex", ""] }, " ",
                { "$ifNull": ["$pets.dogTagId", ""] }, " ",
                { "$ifNull": ["$name", ""] },
            ] } },
        } });
        if let Some(needle) = field_search_filter("petSearchKey", q.q.as_deref()) {
            pipeline.push(doc! { "$match": needle });
        }
        // Same total order as MemStore. `pets.petId` last is load-bearing: siblings share their
        // owner's `updatedAt` exactly, so without it two page requests could order them differently
        // and the pager would repeat some rows while skipping others.
        pipeline.push(doc! { "$sort": { "updatedAt": -1, "clientId": 1, "pets.petId": 1 } });
        // One round trip for both halves: `$facet` runs the page and the count over the same
        // post-filter stream, so `total` can never disagree with the rows it describes.
        pipeline.push(doc! { "$facet": {
            "rows": [
                { "$skip": q.offset as i64 },
                { "$limit": clamp_limit(q.limit) as i64 },
            ],
            "total": [ { "$count": "n" } ],
        } });

        let coll: Collection<Document> = self.db.collection("crm_clients");
        let facet = match coll.aggregate(pipeline).await {
            Ok(mut cur) => {
                use futures::StreamExt;
                match cur.next().await {
                    Some(Ok(d)) => d,
                    _ => return Page { rows: Vec::new(), total: 0 },
                }
            }
            Err(_) => return Page { rows: Vec::new(), total: 0 },
        };
        let total = facet
            .get_array("total")
            .ok()
            .and_then(|a| a.first().cloned())
            .and_then(|v| v.as_document().and_then(|d| d.get_i32("n").ok()).map(|n| n as u64))
            .unwrap_or(0);
        let rows = facet
            .get_array("rows")
            .map(|a| a.iter().filter_map(|v| v.as_document().and_then(pet_row_from_unwound)).collect())
            .unwrap_or_default();
        Page { rows, total }
    }

    async fn get_pet(&self, pet_id: &str) -> Option<PetRow> {
        // An elemMatch-free query is correct here: `pets.petId` is unique across the collection (it
        // is a minted uuid), so the matching client holds exactly one pet with this id.
        let c: Client = self
            .crm_clients()
            .find_one(doc! { "pets.petId": pet_id })
            .await
            .ok()
            .flatten()?;
        let p = c.pets.iter().find(|p| p.pet_id == pet_id)?;
        Some(c.pet_row(p))
    }

    // ---- shop CRM: appointments ----
    async fn put_appointment(&self, a: Appointment) {
        let _ = self
            .crm_appointments()
            .replace_one(doc! { "appointmentId": &a.appointment_id }, &a)
            .upsert(true)
            .await;
    }
    async fn get_appointment(&self, id: &str) -> Option<Appointment> {
        self.crm_appointments()
            .find_one(doc! { "appointmentId": id })
            .await
            .ok()
            .flatten()
    }
    async fn delete_appointment(&self, id: &str) -> bool {
        self.crm_appointments()
            .delete_one(doc! { "appointmentId": id })
            .await
            .map(|r| r.deleted_count > 0)
            .unwrap_or(false)
    }
    async fn list_appointments(&self, q: &AppointmentQuery) -> Page<Appointment> {
        let filter = merge(vec![
            search_filter(q.q.as_deref()),
            eq_filter("clientId", q.client_id.as_deref()),
            eq_filter("petId", q.pet_id.as_deref()),
            eq_filter("status", q.status.as_deref()),
            // [from, to): inclusive lower, exclusive upper, matching MemStore.
            range_filter("startAt", q.from, q.to),
        ]);
        let coll = self.crm_appointments();
        let total = coll.count_documents(filter.clone()).await.unwrap_or(0);
        let rows = collect_page(
            coll.find(filter)
                .sort(doc! { "startAt": 1, "appointmentId": 1 })
                .skip(q.offset as u64)
                .limit(clamp_limit(q.limit) as i64),
        )
        .await;
        Page { rows, total }
    }

    // ---- shop CRM: verification history ----
    async fn put_verification_log(&self, v: VerificationLog) {
        let _ = self
            .crm_verifications()
            .replace_one(doc! { "verificationId": &v.verification_id }, &v)
            .upsert(true)
            .await;
    }
    async fn get_verification_log(&self, id: &str) -> Option<VerificationLog> {
        self.crm_verifications()
            .find_one(doc! { "verificationId": id })
            .await
            .ok()
            .flatten()
    }
    async fn list_verification_logs(&self, q: &VerificationQuery) -> Page<VerificationLog> {
        let filter = merge(vec![
            search_filter(q.q.as_deref()),
            eq_filter("clientId", q.client_id.as_deref()),
            eq_filter("petId", q.pet_id.as_deref()),
            eq_filter("appointmentId", q.appointment_id.as_deref()),
            eq_filter("status", q.status.as_deref()),
            eq_filter("purpose", q.purpose.as_deref()),
            range_filter("createdAt", q.from, q.to),
        ]);
        let coll = self.crm_verifications();
        let total = coll.count_documents(filter.clone()).await.unwrap_or(0);
        let rows = collect_page(
            coll.find(filter)
                .sort(doc! { "createdAt": -1, "verificationId": 1 })
                .skip(q.offset as u64)
                .limit(clamp_limit(q.limit) as i64),
        )
        .await;
        Page { rows, total }
    }
}

// --------------------------------------------------------------------------------------------
// CRM query helpers — build the same filter semantics MemStore implements.
// --------------------------------------------------------------------------------------------

/// Combine the non-empty filter clauses into one document (an `$and` when more than one applies).
fn merge(clauses: Vec<Option<Document>>) -> Document {
    let present: Vec<Document> = clauses.into_iter().flatten().collect();
    match present.len() {
        0 => doc! {},
        1 => present.into_iter().next().unwrap(),
        _ => doc! { "$and": present },
    }
}

/// Free-text search over the row's `searchKey`: EVERY whitespace-separated term must be a
/// case-insensitive substring, so multiple terms narrow the result set (mirrors MemStore).
/// The needle is regex-escaped — an operator typing `.` or `(` must not alter the match semantics.
///
/// The regex is UNANCHORED by design (an operator searching "lim" must find "Alice Lim"), so this
/// clause is a scan over `searchKey` and the index on that field cannot bound it.
fn search_filter(q: Option<&str>) -> Option<Document> {
    field_search_filter("searchKey", q)
}

/// [`search_filter`] against an arbitrary field, for the pets pipeline — whose needle field
/// (`petSearchKey`) is computed per pet by an `$addFields` stage rather than stored on the document.
/// One implementation so pet search cannot drift from client/appointment search in its escaping or
/// its every-term-must-match semantics.
fn field_search_filter(field: &str, q: Option<&str>) -> Option<Document> {
    let needle = q?.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let clauses: Vec<Document> = needle
        .split_whitespace()
        .map(|term| doc! { field: { "$regex": regex_escape(term) } })
        .collect();
    if clauses.is_empty() {
        None
    } else {
        Some(doc! { "$and": clauses })
    }
}

/// Map one `$unwind`-ed client document (exactly one pet in `pets`) to a [`PetRow`].
///
/// Returns `None` for a row whose embedded pet will not deserialize, matching [`collect_page`]'s
/// rule: one malformed legacy document must not blank the operator's whole list.
fn pet_row_from_unwound(d: &Document) -> Option<PetRow> {
    let pet: ClientPet = d
        .get_document("pets")
        .ok()
        .and_then(|p| mongodb::bson::from_document(p.clone()).ok())?;
    Some(PetRow {
        pet,
        client_id: d.get_str("clientId").unwrap_or_default().to_string(),
        client_name: d.get_str("name").unwrap_or_default().to_string(),
        // `updatedAt` is written as an i64; a legacy row that lacks it sorts last rather than failing.
        owner_updated_at: d.get_i64("updatedAt").unwrap_or(0) as u64,
    })
}

/// Escape the PCRE metacharacters so a user-typed needle is matched literally.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if r".^$*+?()[]{}|\/-".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// An equality clause, or `None` when the filter is absent/empty.
fn eq_filter(field: &str, value: Option<&str>) -> Option<Document> {
    let v = value?;
    if v.is_empty() {
        return None;
    }
    Some(doc! { field: v })
}

/// A `[from, to)` half-open range clause over a numeric field, or `None` when neither bound is set.
fn range_filter(field: &str, from: Option<u64>, to: Option<u64>) -> Option<Document> {
    let mut bounds = Document::new();
    if let Some(f) = from {
        bounds.insert("$gte", f as i64);
    }
    if let Some(t) = to {
        bounds.insert("$lt", t as i64);
    }
    if bounds.is_empty() {
        None
    } else {
        Some(doc! { field: bounds })
    }
}

/// Drain a find cursor into a Vec, dropping rows that fail to deserialize rather than failing the
/// whole page (a single malformed legacy document must not blank the operator's list).
async fn collect_page<T: Send + Sync + serde::de::DeserializeOwned>(
    find: mongodb::action::Find<'_, T>,
) -> Vec<T> {
    let mut out = Vec::new();
    if let Ok(mut cur) = find.await {
        use futures::StreamExt;
        while let Some(next) = cur.next().await {
            if let Ok(row) = next {
                out.push(row);
            }
        }
    }
    out
}
