//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal-only in prod (impl
//! architecture: not internet-exposed). The jti collection uses a UNIQUE index so `consume_jti` is
//! atomic (insert-or-fail) — the one-time guarantee for record/verify share JWTs (§11.4).

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
use mongodb::{Client, Collection, Database, IndexModel};

use crate::store::{
    ApptReplica, CustodyBlob, GcalEventMap, GcalSyncState, IssuerSettings, KeystoreMeta,
    ProfileIssueSession, Record, Store, VerifySession,
};

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    /// Connect and ensure the unique jti index exists.
    pub async fn connect(uri: &str, db_name: &str) -> Result<Self, mongodb::error::Error> {
        let client = Client::with_uri_str(uri).await?;
        let db = client.database(db_name);
        let jti: Collection<Document> = db.collection("jwt_jti");
        let idx = IndexModel::builder()
            .keys(doc! { "jti": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build();
        jti.create_index(idx).await?;
        Ok(MongoStore { db })
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
        self.records().find_one(doc! { "record_id": id }).await.ok().flatten()
    }
    async fn update_record(&self, r: Record) {
        self.put_record(r).await;
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
        self.sessions().find_one(doc! { "session_id": id }).await.ok().flatten()
    }
    async fn update_session(&self, s: VerifySession) {
        self.put_session(s).await;
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
        let d = coll.find_one_and_delete(doc! { "token": token }).await.ok().flatten()?;
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
        let d = coll.find_one(doc! { "token": token }).await.ok().flatten()?;
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
    async fn take_export_token(&self, token: &str) -> Option<String> {
        // find_one_and_delete is atomic == one-time consume; then enforce expiry on the read.
        let coll: Collection<Document> = self.db.collection("export_tokens");
        let d = coll.find_one_and_delete(doc! { "token": token }).await.ok().flatten()?;
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
            .find_one_and_update(doc! { "_id": "dog_tag_id" }, doc! { "$inc": { "seq": 1i64 } })
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
        coll.find_one(doc! { "session_id": session_id }).await.ok().flatten()
    }
    async fn update_profile_session(&self, s: ProfileIssueSession) {
        self.put_profile_session(s).await;
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
        let d = coll.find_one(doc! { "token": token }).await.ok().flatten()?;
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
        let d = coll.find_one_and_delete(doc! { "token": token }).await.ok().flatten()?;
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
        self.custody().find_one(doc! { "_id": "singleton" }).await.ok().flatten()
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
        coll.find_one(doc! { "token": token }).await.ok().flatten().is_some()
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
        let d = coll.find_one(doc! { "dog_tag_id": dog_tag_id }).await.ok().flatten()?;
        d.get("doc").and_then(|b| mongodb::bson::from_bson(b.clone()).ok())
    }

    // ---- appointment replica (Phase 7) ----
    async fn get_appt(&self, id: &str) -> Option<ApptReplica> {
        let coll: Collection<ApptReplica> = self.db.collection("appt_replica");
        coll.find_one(doc! { "appointment_id": id }).await.ok().flatten()
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
        if let Ok(mut cur) = coll.find(doc! { "updatedAt": { "$gte": since as i64 } }).await {
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
        coll.find_one(doc! { "appointment_id": appointment_id }).await.ok().flatten()
    }
    async fn get_gcal_map_by_event(&self, google_event_id: &str) -> Option<GcalEventMap> {
        let coll: Collection<GcalEventMap> = self.db.collection("gcal_event_map");
        coll.find_one(doc! { "google_event_id": google_event_id }).await.ok().flatten()
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
        let _ = coll.delete_one(doc! { "google_event_id": google_event_id }).await;
    }
    async fn wipe_gcal_mirror(&self) {
        let coll: Collection<Document> = self.db.collection("gcal_event_map");
        let _ = coll.delete_many(doc! {}).await;
    }
    async fn get_sync_state(&self) -> GcalSyncState {
        let coll: Collection<GcalSyncState> = self.db.collection("gcal_sync_state");
        coll.find_one(doc! { "_id": "singleton" }).await.ok().flatten().unwrap_or_default()
    }
    async fn put_sync_state(&self, s: GcalSyncState) {
        let coll: Collection<GcalSyncState> = self.db.collection("gcal_sync_state");
        let _ = coll.replace_one(doc! { "_id": "singleton" }, &s).upsert(true).await;
    }
}

// keep KeystoreMeta referenced so the import is meaningful across feature configs.
#[allow(dead_code)]
fn _meta_ref(_m: &KeystoreMeta) {}
