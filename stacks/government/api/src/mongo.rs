//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal to the compose network
//! only (never published to the host) — see `stacks/government/docker-compose.yml`.
//!
//! Two collections mirror the `Store` trait: `credentials` (issued government credentials, keyed by
//! the anchored root) and `verifications` (the authority's on-chain verification audit log).

use async_trait::async_trait;
use mongodb::bson::doc;
use mongodb::{Client, Collection, Database};

use crate::store::{IssuedCredential, Store, VerificationRecord};

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    pub async fn connect(uri: &str, db_name: &str) -> Result<Self, mongodb::error::Error> {
        let client = Client::with_uri_str(uri).await?;
        let db = client.database(db_name);
        // Ping fail-closed so a misconfigured URI refuses to boot rather than silently degrading.
        db.run_command(doc! { "ping": 1 }).await?;
        Ok(MongoStore { db })
    }

    fn credentials(&self) -> Collection<IssuedCredential> {
        self.db.collection("credentials")
    }
    fn verifications(&self) -> Collection<VerificationRecord> {
        self.db.collection("verifications")
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
}
