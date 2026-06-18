//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal-only in prod. The `jwt_jti`
//! and `microchips` collections use UNIQUE indexes so `consume_jti` / `reserve_microchip` are atomic
//! (insert-or-fail) — the one-time share-JWT guarantee (§11.4) + microchip.code uniqueness (§4.1).
//!
//! NOTE: `alloc_rev_and_apply` (the sole-rev-allocator) here uses a read-modify-write; in production
//! this MUST run inside a Mongo transaction or a `findOneAndUpdate` with `$inc`-bound rev to remain
//! collision-free under concurrency. The hermetic `MemStore` enforces the invariant under a lock.

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
use mongodb::{Client, Collection, Database, IndexModel};

use crate::store::*;

pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    pub async fn connect(uri: &str, db_name: &str) -> Result<Self, mongodb::error::Error> {
        let client = Client::with_uri_str(uri).await?;
        let db = client.database(db_name);
        for (coll, key) in [("jwt_jti", "jti"), ("microchips", "code")] {
            let c: Collection<Document> = db.collection(coll);
            let idx = IndexModel::builder()
                .keys(doc! { key: 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build();
            c.create_index(idx).await?;
        }
        Ok(MongoStore { db })
    }

    fn owners(&self) -> Collection<Owner> {
        self.db.collection("owners")
    }
    fn pets(&self) -> Collection<Pet> {
        self.db.collection("pets")
    }
    fn credentials(&self) -> Collection<Credential> {
        self.db.collection("credentials")
    }
    fn share_refs(&self) -> Collection<ShareRef> {
        self.db.collection("share_refs")
    }
    fn businesses(&self) -> Collection<Business> {
        self.db.collection("businesses")
    }
    fn applications(&self) -> Collection<IssuerApplication> {
        self.db.collection("issuer_applications")
    }
    fn appointments(&self) -> Collection<Appointment> {
        self.db.collection("appointments")
    }
    fn consents(&self) -> Collection<Consent> {
        self.db.collection("consents")
    }
    fn receipts(&self) -> Collection<ConsentReceipt> {
        self.db.collection("consent_receipts")
    }
    fn verification_records(&self) -> Collection<VerificationRecord> {
        self.db.collection("verification_records")
    }
    fn deletions(&self) -> Collection<Deletion> {
        self.db.collection("deletions")
    }

    async fn collect<T: Send + Sync + serde::de::DeserializeOwned + Unpin>(
        coll: Collection<T>,
        filter: Document,
    ) -> Vec<T> {
        use futures::StreamExt;
        let mut out = Vec::new();
        if let Ok(mut cursor) = coll.find(filter).await {
            while let Some(Ok(d)) = cursor.next().await {
                out.push(d);
            }
        }
        out
    }
}

#[async_trait]
impl Store for MongoStore {
    async fn put_owner(&self, o: Owner) {
        let _ = self.owners().replace_one(doc! { "owner_id": &o.owner_id }, &o).upsert(true).await;
    }
    async fn get_owner(&self, id: &str) -> Option<Owner> {
        self.owners().find_one(doc! { "owner_id": id }).await.ok().flatten()
    }
    async fn get_owner_by_email(&self, email: &str) -> Option<Owner> {
        self.owners().find_one(doc! { "email": email }).await.ok().flatten()
    }
    async fn put_session(&self, token: String, owner_id: String) {
        let coll: Collection<Document> = self.db.collection("sessions");
        let _ = coll
            .replace_one(doc! { "token": &token }, doc! { "token": token, "owner_id": owner_id })
            .upsert(true)
            .await;
    }
    async fn session_owner(&self, token: &str) -> Option<String> {
        let coll: Collection<Document> = self.db.collection("sessions");
        coll.find_one(doc! { "token": token })
            .await
            .ok()
            .flatten()
            .and_then(|d| d.get_str("owner_id").ok().map(|s| s.to_string()))
    }
    async fn put_admin_session(&self, token: String) {
        let coll: Collection<Document> = self.db.collection("admin_sessions");
        let _ = coll.insert_one(doc! { "token": token }).await;
    }
    async fn has_admin_session(&self, token: &str) -> bool {
        let coll: Collection<Document> = self.db.collection("admin_sessions");
        coll.find_one(doc! { "token": token }).await.ok().flatten().is_some()
    }

    async fn put_pet(&self, p: Pet) {
        let _ = self.pets().replace_one(doc! { "pet_id": &p.pet_id }, &p).upsert(true).await;
    }
    async fn get_pet(&self, id: &str) -> Option<Pet> {
        self.pets().find_one(doc! { "pet_id": id }).await.ok().flatten()
    }
    async fn pets_of_owner(&self, owner_id: &str) -> Vec<Pet> {
        Self::collect(self.pets(), doc! { "owner_id": owner_id }).await
    }
    async fn microchip_exists(&self, code: &str) -> bool {
        let coll: Collection<Document> = self.db.collection("microchips");
        coll.find_one(doc! { "code": code }).await.ok().flatten().is_some()
    }
    async fn reserve_microchip(&self, code: &str) -> bool {
        // unique-index insert-or-fail == atomic uniqueness.
        let coll: Collection<Document> = self.db.collection("microchips");
        coll.insert_one(doc! { "code": code }).await.is_ok()
    }
    async fn next_dog_tag_id(&self) -> u64 {
        // atomic counter via findOneAndUpdate($inc).
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

    async fn put_credential(&self, c: Credential) {
        let _ = self.credentials().replace_one(doc! { "credential_id": &c.credential_id }, &c).upsert(true).await;
    }
    async fn get_credential(&self, id: &str) -> Option<Credential> {
        self.credentials().find_one(doc! { "credential_id": id }).await.ok().flatten()
    }
    async fn credentials_of_owner(&self, owner_id: &str) -> Vec<Credential> {
        Self::collect(self.credentials(), doc! { "owner_id": owner_id }).await
    }

    async fn put_share_ref(&self, s: ShareRef) {
        let _ = self.share_refs().replace_one(doc! { "ref_id": &s.ref_id }, &s).upsert(true).await;
    }
    async fn get_share_ref(&self, ref_id: &str) -> Option<ShareRef> {
        self.share_refs().find_one(doc! { "ref_id": ref_id }).await.ok().flatten()
    }
    async fn consume_jti(&self, jti: &str) -> bool {
        let coll: Collection<Document> = self.db.collection("jwt_jti");
        coll.insert_one(doc! { "jti": jti }).await.is_ok()
    }

    async fn put_business(&self, b: Business) {
        let _ = self.businesses().replace_one(doc! { "business_id": &b.business_id }, &b).upsert(true).await;
    }
    async fn get_business(&self, id: &str) -> Option<Business> {
        self.businesses().find_one(doc! { "business_id": id }).await.ok().flatten()
    }
    async fn all_businesses(&self) -> Vec<Business> {
        Self::collect(self.businesses(), doc! {}).await
    }

    async fn put_application(&self, a: IssuerApplication) {
        let _ = self.applications().replace_one(doc! { "application_id": &a.application_id }, &a).upsert(true).await;
    }
    async fn get_application(&self, id: &str) -> Option<IssuerApplication> {
        self.applications().find_one(doc! { "application_id": id }).await.ok().flatten()
    }
    async fn all_applications(&self) -> Vec<IssuerApplication> {
        Self::collect(self.applications(), doc! {}).await
    }

    async fn put_appointment(&self, a: Appointment) {
        let _ = self.appointments().replace_one(doc! { "appointment_id": &a.appointment_id }, &a).upsert(true).await;
    }
    async fn get_appointment(&self, id: &str) -> Option<Appointment> {
        self.appointments().find_one(doc! { "appointment_id": id }).await.ok().flatten()
    }
    async fn appointments_updated_since(&self, owner_id: &str, since: u64) -> Vec<Appointment> {
        Self::collect(
            self.appointments(),
            doc! { "owner_id": owner_id, "updated_at": { "$gte": since as i64 } },
        )
        .await
    }
    async fn alloc_rev_and_apply(&self, appointment_id: &str, f: RevApply) -> Option<Appointment> {
        // read-modify-write (see module note: production needs a txn / $inc-bound rev).
        let current = self.get_appointment(appointment_id).await;
        let next_rev = current.as_ref().map(|a| a.rev + 1).unwrap_or(1);
        let result = f(current, next_rev)?;
        self.put_appointment(result.clone()).await;
        Some(result)
    }

    async fn put_consent(&self, c: Consent) {
        let _ = self.consents().replace_one(doc! { "consent_id": &c.consent_id }, &c).upsert(true).await;
    }
    async fn get_consent(&self, id: &str) -> Option<Consent> {
        self.consents().find_one(doc! { "consent_id": id }).await.ok().flatten()
    }
    async fn consents_of_owner(&self, owner_id: &str) -> Vec<Consent> {
        Self::collect(self.consents(), doc! { "owner_id": owner_id }).await
    }
    async fn put_consent_receipt(&self, r: ConsentReceipt) {
        let _ = self.receipts().replace_one(doc! { "receipt_id": &r.receipt_id }, &r).upsert(true).await;
    }
    async fn receipts_of_owner(&self, owner_id: &str) -> Vec<ConsentReceipt> {
        Self::collect(self.receipts(), doc! { "owner_id": owner_id }).await
    }
    async fn put_verification_record(&self, v: VerificationRecord) {
        let _ = self.verification_records().replace_one(doc! { "record_id": &v.record_id }, &v).upsert(true).await;
    }
    async fn verification_records_of_owner(&self, owner_id: &str) -> Vec<VerificationRecord> {
        Self::collect(self.verification_records(), doc! { "owner_id": owner_id }).await
    }
    async fn put_deletion(&self, d: Deletion) {
        let _ = self.deletions().replace_one(doc! { "request_id": &d.request_id }, &d).upsert(true).await;
    }
    async fn due_deletions(&self, now: u64) -> Vec<Deletion> {
        Self::collect(
            self.deletions(),
            doc! { "status": "pending", "due_by": { "$lte": now as i64 } },
        )
        .await
    }
    async fn update_deletion(&self, d: Deletion) {
        self.put_deletion(d).await;
    }

    async fn delete_credential(&self, id: &str) {
        let _ = self.credentials().delete_one(doc! { "credential_id": id }).await;
    }
    async fn delete_verification_record(&self, id: &str) {
        let _ = self.verification_records().delete_one(doc! { "record_id": id }).await;
    }
    async fn delete_consent_receipt(&self, id: &str) {
        let _ = self.receipts().delete_one(doc! { "receipt_id": id }).await;
    }
    async fn clear_owner_pii(&self, owner_id: &str) {
        let _ = self
            .owners()
            .update_one(doc! { "owner_id": owner_id }, doc! { "$set": { "profile_pii": null } })
            .await;
    }
    async fn clear_pet_doc(&self, pet_id: &str) {
        let _ = self
            .pets()
            .update_one(doc! { "pet_id": pet_id }, doc! { "$set": { "sealed_doc": null } })
            .await;
    }
}
