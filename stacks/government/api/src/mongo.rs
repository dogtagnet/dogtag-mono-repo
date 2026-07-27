//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal to the compose network
//! only (never published to the host) — see `stacks/government/docker-compose.yml`.
//!
//! Collections mirror the `Store` trait: `credentials` (issued government credentials, keyed by the
//! anchored root), `verifications` (the authority's on-chain verification audit log), `share_tokens`
//! and `export_tokens` (the short-lived phone-handoff QR tokens) and `verify_sessions` (owner-hidden
//! verify sessions). The token collections mirror vet-api's (`stacks/vet/api/src/mongo.rs`), including
//! `find_one_and_delete` as the atomic one-time consume.

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
use mongodb::{Client, Collection, Database, IndexModel};

use crate::store::{IssuedCredential, Store, VerificationRecord, VerifySession};

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    pub async fn connect(uri: &str, db_name: &str) -> Result<Self, mongodb::error::Error> {
        let client = Client::with_uri_str(uri).await?;
        let db = client.database(db_name);
        // Ping fail-closed so a misconfigured URI refuses to boot rather than silently degrading.
        db.run_command(doc! { "ping": 1 }).await?;
        let store = MongoStore { db };
        // Unique index on the public receipt handle so the `/r/:receiptId` lookup is O(1) and a
        // duplicate id can never be persisted (sparse: legacy rows without a receiptId are exempt).
        let idx = IndexModel::builder()
            .keys(doc! { "receiptId": 1 })
            .options(IndexOptions::builder().unique(true).sparse(true).build())
            .build();
        store.credentials().create_index(idx).await?;
        Ok(store)
    }

    fn credentials(&self) -> Collection<IssuedCredential> {
        self.db.collection("credentials")
    }
    fn verifications(&self) -> Collection<VerificationRecord> {
        self.db.collection("verifications")
    }
    fn sessions(&self) -> Collection<VerifySession> {
        self.db.collection("verify_sessions")
    }
}

#[async_trait]
impl Store for MongoStore {
    async fn put_credential(&self, cred: IssuedCredential) {
        let _ = self
            .credentials()
            .replace_one(doc! { "root": &cred.root }, &cred)
            .upsert(true)
            .await;
    }
    async fn get_credential(&self, root: &str) -> Option<IssuedCredential> {
        self.credentials()
            .find_one(doc! { "root": root })
            .await
            .ok()
            .flatten()
    }
    async fn get_credential_by_receipt_id(&self, receipt_id: &str) -> Option<IssuedCredential> {
        self.credentials()
            .find_one(doc! { "receiptId": receipt_id })
            .await
            .ok()
            .flatten()
    }
    async fn list_credentials(&self) -> Vec<IssuedCredential> {
        use futures::TryStreamExt;
        match self.credentials().find(doc! {}).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }
    async fn put_verification(&self, rec: VerificationRecord) {
        let _ = self.verifications().insert_one(&rec).await;
    }
    async fn list_verifications(&self) -> Vec<VerificationRecord> {
        use futures::TryStreamExt;
        match self.verifications().find(doc! {}).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    async fn put_share_token(&self, token: &str, root: &str, exp: u64) {
        let coll: Collection<Document> = self.db.collection("share_tokens");
        let _ = coll
            .replace_one(
                doc! { "token": token },
                doc! { "token": token, "root": root, "exp": exp as i64 },
            )
            .upsert(true)
            .await;
    }
    async fn take_share_token(&self, token: &str) -> Option<String> {
        // find_one_and_delete is atomic == one-time consume; expiry is enforced on the read, so an
        // expired token is purged and reported as missing.
        let coll: Collection<Document> = self.db.collection("share_tokens");
        let d = coll
            .find_one_and_delete(doc! { "token": token })
            .await
            .ok()
            .flatten()?;
        if now_secs() > d.get_i64("exp").unwrap_or(0) as u64 {
            None
        } else {
            d.get_str("root").ok().map(|s| s.to_string())
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
        let coll: Collection<Document> = self.db.collection("export_tokens");
        let d = coll.find_one(doc! { "token": token }).await.ok().flatten()?;
        if now_secs() > d.get_i64("exp").unwrap_or(0) as u64 {
            None
        } else {
            d.get_str("session_id").ok().map(|s| s.to_string())
        }
    }

    async fn put_session(&self, s: VerifySession) {
        let _ = self
            .sessions()
            .replace_one(doc! { "sessionId": &s.session_id }, &s)
            .upsert(true)
            .await;
    }
    async fn get_session(&self, id: &str) -> Option<VerifySession> {
        self.sessions()
            .find_one(doc! { "sessionId": id })
            .await
            .ok()
            .flatten()
    }
    async fn update_session(&self, s: VerifySession) {
        self.put_session(s).await;
    }
    async fn list_sessions(&self) -> Vec<VerifySession> {
        use futures::TryStreamExt;
        let mut v: Vec<VerifySession> = match self.sessions().find(doc! {}).await {
            Ok(cur) => cur.try_collect().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        v.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        v
    }
}

/// Wall-clock seconds — the expiry basis for both token collections.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
