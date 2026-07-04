//! Production `MongoStore` (behind the `mongo` feature). Mongo is internal to the compose network
//! only (never published to the host) — see `stacks/indexer/docker-compose.yml`.
//!
//! Two collections back the `Store` trait: `events` (the flattened non-PII log index, keyed by the
//! idempotency id `txHash:logIndex`) and `cursor` (a single resume-cursor doc). The high-selectivity
//! equality/range predicates are pushed into the Mongo query; the caller's scope predicate + exact
//! `EventQuery::matches` + pagination are applied in Rust (identical semantics to `MemStore`).

use async_trait::async_trait;
use mongodb::bson::{doc, Document};
use mongodb::options::IndexOptions;
use mongodb::{Client, Collection, Database, IndexModel};

use crate::events::IndexedEvent;
use crate::scope::Scope;
use crate::store::{sort_newest_first, Cursor, EventQuery, Store};

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
        // Unique index on the idempotency id so a re-scan can never persist a duplicate.
        let idx = IndexModel::builder()
            .keys(doc! { "id": 1 })
            .options(IndexOptions::builder().unique(true).build())
            .build();
        store.events().create_index(idx).await?;
        // Feed sort key.
        let sort_idx = IndexModel::builder()
            .keys(doc! { "blockNumber": -1i32, "logIndex": -1i32 })
            .build();
        store.events().create_index(sort_idx).await?;
        Ok(store)
    }

    fn events(&self) -> Collection<IndexedEvent> {
        self.db.collection("events")
    }
    fn cursor_col(&self) -> Collection<Document> {
        self.db.collection("cursor")
    }
}

/// Build the Mongo pre-filter from the storable predicates (exact matches on lowercased fields +
/// timestamp range). The scope predicate and `EventQuery::matches` are re-applied in Rust for exactness.
fn to_filter_doc(q: &EventQuery) -> Document {
    let mut d = Document::new();
    if let Some(t) = q.event_type {
        if let Ok(mongodb::bson::Bson::String(s)) = mongodb::bson::to_bson(&t) {
            d.insert("type", s);
        }
    }
    if let Some(s) = &q.signer {
        d.insert("actor", s);
    }
    if let Some(c) = &q.issuer {
        d.insert("clone", c);
    }
    if let Some(rt) = &q.record_type {
        d.insert("recordType", rt);
    }
    if let Some(r) = &q.root {
        d.insert("root", r);
    }
    if let Some(dt) = &q.dog_tag_id {
        d.insert("dogTagId", dt);
    }
    let mut ts = Document::new();
    if let Some(s) = q.since {
        ts.insert("$gte", s as i64);
    }
    if let Some(u) = q.until {
        ts.insert("$lte", u as i64);
    }
    if !ts.is_empty() {
        d.insert("blockTimestamp", ts);
    }
    d
}

#[async_trait]
impl Store for MongoStore {
    async fn upsert_events(&self, events: &[IndexedEvent]) {
        for ev in events {
            let _ = self
                .events()
                .replace_one(doc! { "id": &ev.id }, ev)
                .upsert(true)
                .await;
        }
    }

    async fn query_events(&self, q: &EventQuery, scope: &Scope) -> (Vec<IndexedEvent>, usize) {
        use futures::TryStreamExt;
        let filter = to_filter_doc(q);
        let cur = match self
            .events()
            .find(filter)
            .sort(doc! { "blockNumber": -1i32, "logIndex": -1i32 })
            .await
        {
            Ok(c) => c,
            Err(_) => return (Vec::new(), 0),
        };
        let collected: Vec<IndexedEvent> = cur.try_collect().await.unwrap_or_default();
        // Re-apply exact filter + scope, then paginate (mirrors MemStore). A plain loop keeps the
        // `admit` HRTB predicate happy.
        let mut all: Vec<IndexedEvent> = Vec::new();
        for e in collected {
            if scope.admits(&e) && q.matches(&e) {
                all.push(e);
            }
        }
        sort_newest_first(&mut all);
        let total = all.len();
        let page = all.into_iter().skip(q.offset).take(q.limit).collect();
        (page, total)
    }

    async fn delete_from_block(&self, from_block: u64) -> usize {
        match self
            .events()
            .delete_many(doc! { "blockNumber": { "$gte": from_block as i64 } })
            .await
        {
            Ok(r) => r.deleted_count as usize,
            Err(_) => 0,
        }
    }

    async fn get_cursor(&self) -> Cursor {
        match self.cursor_col().find_one(doc! { "_id": "cursor" }).await {
            Ok(Some(d)) => Cursor {
                last_block: d.get_i64("lastBlock").ok().map(|v| v as u64),
                last_block_hash: d.get_str("lastBlockHash").ok().map(|s| s.to_string()),
            },
            _ => Cursor::default(),
        }
    }

    async fn set_cursor(&self, cursor: Cursor) {
        let mut d = doc! { "_id": "cursor" };
        if let Some(b) = cursor.last_block {
            d.insert("lastBlock", b as i64);
        }
        if let Some(h) = cursor.last_block_hash {
            d.insert("lastBlockHash", h);
        }
        let _ = self
            .cursor_col()
            .replace_one(doc! { "_id": "cursor" }, d)
            .upsert(true)
            .await;
    }
}
