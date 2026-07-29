//! The ingest loop — scans the ROAX logs forward from the resume cursor, decodes into the non-PII
//! index, and heals shallow reorgs.
//!
//! Design (arch §4.3, "handle reorg/resume sensibly"):
//!   - **Finality buffer.** Only blocks below `head - confirmations` are indexed; the tip is left to
//!     settle, so the common case never indexes a block that later disappears.
//!   - **Resume.** The cursor (highest indexed block + its hash) is persisted after every chunk, so a
//!     restart continues from where it stopped instead of re-scanning from the deploy block.
//!   - **Reorg detection + rollback.** Each tick re-checks that the cursor block's stored hash still
//!     matches the live chain. On a mismatch (history was rewritten under us) it deletes every event
//!     at/above a rewind point and rewinds the cursor, so the next scan re-indexes the canonical
//!     chain. Upserts are keyed by `txHash:logIndex`, so re-scanning is idempotent — never a dup.
//!   - **Clone discovery.** `IssuerCreated` clones seen in a scan extend the known-clone set used to
//!     gate `RootIssued`/`RootRevoked` (anti-spoof), rebuilt from the store on startup so restarts
//!     keep attributing pre-restart clones.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::app::AppState;
use crate::chain::ChainError;
use crate::events::{EventType, Finality, IndexedEvent};
use crate::scope::Scope;
use crate::store::{Cursor, EventQuery, StoreError};

pub struct Indexer {
    state: AppState,
    /// Clones discovered from `IssuerCreated`, mapped to the factory generation that vouched for
    /// them (in addition to the per-generation deployment seeds in `Config`).
    discovered: Arc<Mutex<HashMap<String, String>>>,
}

impl Indexer {
    pub fn new(state: AppState) -> Self {
        Indexer {
            state,
            discovered: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Rebuild the discovered-clone set from previously-indexed `IssuerCreated` events, so a restart
    /// re-attributes clones deployed before this process began.
    ///
    /// The new set is built in full and only then swapped in, so a failed read leaves the previous
    /// trust set intact instead of an empty one. That matters because this is not always re-reachable:
    /// the post-rewind caller has already cleared the watermark hash, so the divergence branch cannot
    /// fire again, and a cleared-then-failed rebuild would silently drop every subsequent
    /// `RootIssued`/`RootRevoked` from pre-restart clones for the life of the process.
    pub async fn rebuild_known_clones(&self) -> Result<(), StoreError> {
        let q = EventQuery {
            event_type: Some(EventType::IssuerCreated),
            limit: usize::MAX,
            ..Default::default()
        };
        let (events, _) = self.state.store.query_events(&q, &Scope::Unscoped).await?;
        let mut rebuilt: HashMap<String, String> = HashMap::new();
        for e in events {
            let Some(c) = e.clone else {
                continue;
            };
            // Removing a generation from config must remove its clones from the anti-spoof trust
            // set after restart. Do not blindly trust a historical IssuerCreated row: require both
            // its stamped generation and its emitting factory to agree with the current watch set.
            // An unstamped legacy row carries an empty generation, which matches no configured
            // `0x…` id, so it is inadmissible here rather than trusted on its contract alone.
            if self.state.cfg.generation_for_factory(&e.contract) == Some(e.generation.as_str()) {
                rebuilt.insert(c.to_ascii_lowercase(), e.generation);
            }
        }
        *self.discovered.lock().unwrap() = rebuilt;
        Ok(())
    }

    fn discovered_snapshot(&self) -> HashMap<String, String> {
        self.discovered.lock().unwrap().clone()
    }

    /// Run the ingest loop forever, ticking every `poll_interval_secs`. Errors are logged and the loop
    /// continues (a transient RPC blip must not kill the indexer).
    pub async fn run(self: Arc<Self>) {
        let interval = self.state.cfg.poll_interval_secs.max(1);
        loop {
            if let Err(e) = self.tick().await {
                tracing::warn!("indexer tick failed: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    }

    /// One ingest iteration. Returns after advancing the cursor to the current safe head (or after a
    /// reorg rollback). Public so tests can drive it deterministically.
    pub async fn tick(&self) -> Result<(), ChainError> {
        let cfg = &self.state.cfg;
        let source = &self.state.source;
        let store = &self.state.store;

        let head = source.head_block().await?;

        // --- resolve the finalized watermark ----------------------------------------------------
        // Prefer the chain's `finalized` tag (true PoS finality — those blocks can never reorg). If
        // the node does not expose it, fall back to a confirmations-depth watermark (weaker: those
        // blocks are only "N-deep", not provably final) and say so.
        let (finalized, finality_source) = match source.finalized_block().await? {
            Some(n) => (n.min(head), FinalitySource::Tag),
            None => (head.saturating_sub(cfg.confirmations), FinalitySource::Confirmations),
        };
        let mut cursor = store.get_cursor().await;

        // --- defensive watermark guard ----------------------------------------------------------
        // Under true finality the finalized block's hash can never change, so this never fires. Under
        // the confirmations fallback a deeper-than-N reorg could rewrite a block we already promoted;
        // detect it by hash and rewind the watermark + delete the affected (finalized) rows.
        if let (Some(lf), Some(stored_hash)) =
            (cursor.last_finalized, cursor.last_finalized_hash.clone())
        {
            let live = source.block_hash(lf).await?;
            let diverged = match &live {
                Some(h) => !h.eq_ignore_ascii_case(&stored_hash),
                None => true,
            };
            if diverged {
                let rewind_to = cfg.start_block.max(lf.saturating_sub(cfg.confirmations.max(1)));
                let removed = store.delete_from_block(rewind_to).await;
                tracing::warn!(
                    "finalized-watermark divergence at block {lf} ({finality_source:?} mode; stored \
                     {stored_hash}, live {live:?}); rewound {removed} events from block {rewind_to}"
                );
                cursor = Cursor {
                    last_finalized: (rewind_to > cfg.start_block).then_some(rewind_to - 1),
                    last_finalized_hash: None,
                };
                store.set_cursor(cursor.clone()).await;
                self.rebuild_known_clones().await.map_err(|e| {
                    ChainError::Other(format!(
                        "clone-trust rebuild after a watermark rewind failed: {e}"
                    ))
                })?;
            }
        }

        // --- forward scan [last_finalized+1 .. head] into a buffer FIRST ------------------------
        // Derive the full new finalized+pending set BEFORE touching the store, so a transient RPC
        // error on any chunk bails with the store untouched (the prior pending rows survive). The
        // pending range is only swapped after the whole fallible scan succeeds (atomic swap).
        let mut scan_from = cursor.last_finalized.map(|b| b + 1).unwrap_or(cfg.start_block);
        if scan_from > head {
            return Ok(()); // nothing new (everything is already finalized ≤ watermark)
        }
        let chunk = cfg.chunk_size.max(1);
        let mut scanned: Vec<IndexedEvent> = Vec::new();
        let mut finalized_count = 0u64;
        let mut pending_count = 0u64;
        while scan_from <= head {
            let scan_to = (scan_from + chunk - 1).min(head);
            let ctx = cfg.watch_context(&self.discovered_snapshot());
            // A fetch error here returns before any delete/upsert — the store keeps its prior state.
            let mut events = source.fetch_events(scan_from, scan_to, &ctx).await?;

            // Stamp each event's finality from the watermark, and fold newly-discovered clones so
            // subsequent chunks gate `RootIssued`/`RootRevoked` correctly.
            {
                let mut g = self.discovered.lock().unwrap();
                for e in &mut events {
                    e.finality = if e.block_number <= finalized {
                        finalized_count += 1;
                        Finality::Finalized
                    } else {
                        pending_count += 1;
                        Finality::Pending
                    };
                    if e.event_type == EventType::IssuerCreated {
                        if let Some(c) = &e.clone {
                            g.insert(c.to_ascii_lowercase(), e.generation.clone());
                        }
                    }
                }
            }
            scanned.append(&mut events);
            scan_from = scan_to + 1;
        }

        // --- atomic swap of the pending range ---------------------------------------------------
        // The fallible scan fully succeeded, so now (and only now) replace the pending range: drop all
        // prior pending rows and upsert the freshly-derived set. Any pending event orphaned by a reorg
        // is gone (absent from `scanned`); finalized rows re-derived from the canonical chain upsert
        // idempotently by id. Nothing here is fallible on the network, so there is no lossy window.
        let dropped = store.delete_pending().await;
        if dropped > 0 {
            tracing::debug!("dropped {dropped} prior pending events, swapping in re-derived set");
        }
        if !scanned.is_empty() {
            store.upsert_events(&scanned).await;
        }

        // --- advance + persist the finalized watermark ------------------------------------------
        // Everything up to `finalized` is now indexed + immutable; the pending range above it will be
        // re-derived next tick. Only record a watermark once it is within our scanned range.
        if finalized >= cfg.start_block {
            let hash = source.block_hash(finalized).await?;
            cursor = Cursor {
                last_finalized: Some(finalized),
                last_finalized_hash: hash,
            };
            store.set_cursor(cursor).await;
        }

        if finalized_count + pending_count > 0 {
            tracing::info!(
                "indexed {finalized_count} finalized + {pending_count} pending events \
                 (head {head}, finalized {finalized}, {finality_source:?})"
            );
        }
        Ok(())
    }
}

/// Where the finalized watermark came from, for logging + the `/v1/status` surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FinalitySource {
    /// The chain's `finalized` block tag (true PoS finality).
    Tag,
    /// A confirmations-depth watermark (`head - CONFIRMATIONS`) because the node exposes no tag.
    Confirmations,
}

impl FinalitySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            FinalitySource::Tag => "finalized-tag",
            FinalitySource::Confirmations => "confirmations-fallback",
        }
    }
}
