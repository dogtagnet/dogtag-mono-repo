//! `Store` — the indexer's queryable, non-PII event index + resume cursor.
//!
//! `MemStore` (default) is ephemeral (demo/local + tests). `MongoStore` (feature `mongo`) persists to
//! the `indexerdata` volume. Both back the exact same trait so the ingest loop + query API are
//! storage-agnostic. Nothing personal is ever written here — only the flattened chain events + the
//! scan cursor (arch doctrine §4: "No PII in the index").

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::events::{EventType, Finality, IndexedEvent};
use crate::scope::Scope;

/// The resume cursor: the highest **finalized** (immutable) block + its hash. Blocks at or below it
/// are settled and never re-scanned; the ingest loop always re-derives the pending range above it
/// from the canonical chain. Persisted so a restart resumes from the finalized watermark instead of
/// re-scanning genesis.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cursor {
    /// The highest finalized block number whose logs are fully indexed. A fresh index reports `None`.
    #[serde(rename = "lastFinalized", skip_serializing_if = "Option::is_none")]
    pub last_finalized: Option<u64>,
    /// The block hash of `last_finalized` at index time — a defensive guard: if a supposedly-final
    /// block's hash ever changes (only possible under the confirmations-depth fallback, never under
    /// true finality) the loop rewinds the watermark.
    #[serde(rename = "lastFinalizedHash", skip_serializing_if = "Option::is_none")]
    pub last_finalized_hash: Option<String>,
}

/// Server-side query filters. Every field is an AND-narrowing predicate applied *after* the caller's
/// [`crate::scope::Scope`] ceiling — so a filter can only shrink a result set, never widen it.
#[derive(Clone, Debug, Default)]
pub struct EventQuery {
    pub event_type: Option<EventType>,
    /// An in-scope signer address (the acting `by`/`signer`/`relayer`). Lowercased by the caller.
    pub signer: Option<String>,
    /// An in-scope `DogTagIssuer` clone address. Lowercased by the caller.
    pub issuer: Option<String>,
    /// keccak256(recordType) key.
    pub record_type: Option<String>,
    pub root: Option<String>,
    pub dog_tag_id: Option<String>,
    /// Finality lifecycle filter (`finalized` vs `pending`).
    pub finality: Option<Finality>,
    /// Inclusive lower bound on `block_timestamp` (Unix seconds).
    pub since: Option<u64>,
    /// Inclusive upper bound on `block_timestamp` (Unix seconds).
    pub until: Option<u64>,
    /// Page size (defaults applied by the route layer).
    pub limit: usize,
    pub offset: usize,
}

impl EventQuery {
    /// Does `ev` satisfy every set predicate? (Scope admission is checked separately, before this.)
    pub fn matches(&self, ev: &IndexedEvent) -> bool {
        if let Some(t) = self.event_type {
            if ev.event_type != t {
                return false;
            }
        }
        if let Some(s) = &self.signer {
            if ev.actor.as_deref().map(|a| a.to_ascii_lowercase()) != Some(s.clone()) {
                return false;
            }
        }
        if let Some(c) = &self.issuer {
            if ev.clone.as_deref().map(|a| a.to_ascii_lowercase()) != Some(c.clone()) {
                return false;
            }
        }
        if let Some(rt) = &self.record_type {
            if ev.record_type.as_deref().map(|s| s.to_ascii_lowercase())
                != Some(rt.to_ascii_lowercase())
            {
                return false;
            }
        }
        if let Some(r) = &self.root {
            if ev.root.as_deref().map(|s| s.to_ascii_lowercase()) != Some(r.to_ascii_lowercase()) {
                return false;
            }
        }
        if let Some(d) = &self.dog_tag_id {
            if ev.dog_tag_id.as_deref() != Some(d.as_str()) {
                return false;
            }
        }
        if let Some(f) = self.finality {
            if ev.finality != f {
                return false;
            }
        }
        if let Some(s) = self.since {
            if ev.block_timestamp < s {
                return false;
            }
        }
        if let Some(u) = self.until {
            if ev.block_timestamp > u {
                return false;
            }
        }
        true
    }
}

/// The storage surface the ingest loop + query API share.
#[async_trait]
pub trait Store: Send + Sync {
    /// Idempotently upsert a batch of events (keyed by `id = txHash:logIndex`). Re-indexing a range
    /// is a no-op rather than a duplicate. Ordering within the batch is irrelevant.
    async fn upsert_events(&self, events: &[IndexedEvent]);

    /// Return events matching `q` that fall within `scope`, **newest-first** (block_number desc, then
    /// log_index desc), with `q.offset`/`q.limit` applied *after* filtering. The total number of
    /// matches (pre-pagination) is returned alongside the page. Scope admission is enforced here so
    /// no store consumer can accidentally bypass it.
    async fn query_events(&self, q: &EventQuery, scope: &Scope) -> (Vec<IndexedEvent>, usize);

    /// Delete every event at or above `from_block` — the finalized-watermark rewind primitive (only
    /// exercised by the confirmations-depth fallback's defensive guard). Returns the count removed.
    async fn delete_from_block(&self, from_block: u64) -> usize;

    /// Delete every `pending` (non-finalized) event — the pending-range reconciliation primitive. The
    /// ingest loop drops all pending rows each tick and re-derives them from the canonical chain, so
    /// an orphaned pending event from a reorg simply disappears. Finalized rows are never touched.
    /// Returns the count removed.
    async fn delete_pending(&self) -> usize;

    /// Read the resume cursor.
    async fn get_cursor(&self) -> Cursor;
    /// Persist the resume cursor.
    async fn set_cursor(&self, cursor: Cursor);
}

// ------------------------------------------------------------------------------------------------
// MemStore
// ------------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    /// id -> event.
    events: HashMap<String, IndexedEvent>,
    cursor: Cursor,
}

#[derive(Clone, Default)]
pub struct MemStore {
    inner: Arc<Mutex<MemInner>>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Sort a slice newest-first: block desc, then log_index desc (stable, deterministic paging).
pub fn sort_newest_first(v: &mut [IndexedEvent]) {
    v.sort_by(|a, b| {
        b.block_number
            .cmp(&a.block_number)
            .then(b.log_index.cmp(&a.log_index))
    });
}

#[async_trait]
impl Store for MemStore {
    async fn upsert_events(&self, events: &[IndexedEvent]) {
        let mut g = self.inner.lock().unwrap();
        for ev in events {
            g.events.insert(ev.id.clone(), ev.clone());
        }
    }

    async fn query_events(&self, q: &EventQuery, scope: &Scope) -> (Vec<IndexedEvent>, usize) {
        // Snapshot out of the lock first, then filter.
        let snapshot: Vec<IndexedEvent> = {
            let g = self.inner.lock().unwrap();
            g.events.values().cloned().collect()
        };
        let mut hits: Vec<IndexedEvent> = Vec::new();
        for e in snapshot {
            if scope.admits(&e) && q.matches(&e) {
                hits.push(e);
            }
        }
        sort_newest_first(&mut hits);
        let total = hits.len();
        let page = hits.into_iter().skip(q.offset).take(q.limit).collect();
        (page, total)
    }

    async fn delete_from_block(&self, from_block: u64) -> usize {
        let mut g = self.inner.lock().unwrap();
        let before = g.events.len();
        g.events.retain(|_, e| e.block_number < from_block);
        before - g.events.len()
    }

    async fn delete_pending(&self) -> usize {
        let mut g = self.inner.lock().unwrap();
        let before = g.events.len();
        g.events.retain(|_, e| e.finality != Finality::Pending);
        before - g.events.len()
    }

    async fn get_cursor(&self) -> Cursor {
        self.inner.lock().unwrap().cursor.clone()
    }

    async fn set_cursor(&self, cursor: Cursor) {
        self.inner.lock().unwrap().cursor = cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventType;

    fn ev(id: &str, block: u64, log_index: u64, ty: EventType) -> IndexedEvent {
        ev_fin(id, block, log_index, ty, Finality::Finalized)
    }

    fn ev_fin(
        id: &str,
        block: u64,
        log_index: u64,
        ty: EventType,
        finality: Finality,
    ) -> IndexedEvent {
        IndexedEvent {
            id: id.into(),
            event_type: ty,
            contract: "0xc".into(),
            block_number: block,
            block_hash: "0xbh".into(),
            tx_hash: id.split(':').next().unwrap().into(),
            log_index,
            block_timestamp: 1000 + block,
            finality,
            actor: Some("0xaa".into()),
            clone: Some("0xcc".into()),
            record_type: None,
            name: None,
            root: None,
            dog_tag_id: None,
            purpose: None,
            nullifier: None,
            deadline: None,
            onchain_ts: None,
        }
    }

    #[tokio::test]
    async fn upsert_is_idempotent_and_query_is_newest_first() {
        let s = MemStore::new();
        s.upsert_events(&[
            ev("0x1:0", 10, 0, EventType::RootIssued),
            ev("0x2:0", 12, 0, EventType::RootRevoked),
            ev("0x1:0", 10, 0, EventType::RootIssued), // duplicate id
        ])
        .await;
        let (page, total) = s
            .query_events(&EventQuery { limit: 10, ..Default::default() }, &Scope::Unscoped)
            .await;
        assert_eq!(total, 2, "duplicate id must not double-count");
        assert_eq!(page[0].block_number, 12, "newest first");
    }

    #[tokio::test]
    async fn delete_from_block_rewinds_watermark() {
        let s = MemStore::new();
        s.upsert_events(&[
            ev("0x1:0", 10, 0, EventType::RootIssued),
            ev("0x2:0", 20, 0, EventType::RootIssued),
        ])
        .await;
        let removed = s.delete_from_block(15).await;
        assert_eq!(removed, 1);
        let (_, total) = s
            .query_events(&EventQuery { limit: 10, ..Default::default() }, &Scope::Unscoped)
            .await;
        assert_eq!(total, 1);
    }

    #[tokio::test]
    async fn delete_pending_keeps_finalized() {
        let s = MemStore::new();
        s.upsert_events(&[
            ev_fin("0x1:0", 10, 0, EventType::RootIssued, Finality::Finalized),
            ev_fin("0x2:0", 20, 0, EventType::RootIssued, Finality::Pending),
        ])
        .await;
        let removed = s.delete_pending().await;
        assert_eq!(removed, 1, "only the pending event is dropped");
        let (page, total) = s
            .query_events(&EventQuery { limit: 10, ..Default::default() }, &Scope::Unscoped)
            .await;
        assert_eq!(total, 1);
        assert_eq!(page[0].finality, Finality::Finalized);
    }

    #[tokio::test]
    async fn finality_filter_narrows() {
        let s = MemStore::new();
        s.upsert_events(&[
            ev_fin("0x1:0", 10, 0, EventType::RootIssued, Finality::Finalized),
            ev_fin("0x2:0", 20, 0, EventType::RootIssued, Finality::Pending),
        ])
        .await;
        let (_, fin) = s
            .query_events(
                &EventQuery { finality: Some(Finality::Finalized), limit: 10, ..Default::default() },
                &Scope::Unscoped,
            )
            .await;
        assert_eq!(fin, 1);
    }
}
