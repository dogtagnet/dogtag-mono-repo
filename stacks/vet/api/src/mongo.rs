//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal-only in prod (impl
//! architecture: not internet-exposed). The jti collection uses a UNIQUE index so `consume_jti` is
//! atomic (insert-or-fail) — the one-time guarantee for record/verify share JWTs (§11.4).

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
use mongodb::{Client, Collection, Database, IndexModel};

use crate::store::{
    CustodyBlob, IssuerSettings, KeystoreMeta, Record, Store, VerifySession,
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
}

// keep KeystoreMeta referenced so the import is meaningful across feature configs.
#[allow(dead_code)]
fn _meta_ref(_m: &KeystoreMeta) {}
