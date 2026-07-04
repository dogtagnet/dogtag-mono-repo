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

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::app::AppState;
use crate::chain::ChainError;
use crate::events::EventType;
use crate::scope::Scope;
use crate::store::{Cursor, EventQuery};

pub struct Indexer {
    state: AppState,
    /// Clones discovered from `IssuerCreated` (in addition to the deployment seed in `Config`).
    discovered: Arc<Mutex<HashSet<String>>>,
}

impl Indexer {
    pub fn new(state: AppState) -> Self {
        Indexer {
            state,
            discovered: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Rebuild the discovered-clone set from previously-indexed `IssuerCreated` events, so a restart
    /// re-attributes clones deployed before this process began.
    pub async fn rebuild_known_clones(&self) {
        let q = EventQuery {
            event_type: Some(EventType::IssuerCreated),
            limit: usize::MAX,
            ..Default::default()
        };
        let (events, _) = self.state.store.query_events(&q, &Scope::Unscoped).await;
        let mut g = self.discovered.lock().unwrap();
        for e in events {
            if let Some(c) = e.clone {
                g.insert(c.to_ascii_lowercase());
            }
        }
    }

    fn discovered_snapshot(&self) -> HashSet<String> {
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
        let safe_head = head.saturating_sub(cfg.confirmations);
        let mut cursor = store.get_cursor().await;

        // --- reorg detection on the last-indexed block ------------------------------------------
        if let (Some(lb), Some(stored_hash)) = (cursor.last_block, cursor.last_block_hash.clone()) {
            let live = source.block_hash(lb).await?;
            let diverged = match &live {
                Some(h) => !h.eq_ignore_ascii_case(&stored_hash),
                None => true, // the block we thought was final is gone
            };
            if diverged {
                let rewind_to = cfg
                    .start_block
                    .max(lb.saturating_sub(cfg.confirmations.max(1)));
                let removed = store.delete_from_block(rewind_to).await;
                tracing::warn!(
                    "reorg detected at block {lb} (stored {stored_hash}, live {live:?}); \
                     rolled back {removed} events from block {rewind_to}"
                );
                cursor = Cursor {
                    last_block: if rewind_to > cfg.start_block {
                        Some(rewind_to - 1)
                    } else {
                        None
                    },
                    last_block_hash: None,
                };
                store.set_cursor(cursor.clone()).await;
                // Rebuild the known-clone set from what survived the rollback.
                self.discovered.lock().unwrap().clear();
                self.rebuild_known_clones().await;
            }
        }

        // --- forward scan in chunks -------------------------------------------------------------
        let mut scan_from = cursor.last_block.map(|b| b + 1).unwrap_or(cfg.start_block);
        if scan_from > safe_head {
            return Ok(()); // nothing new below the finality buffer
        }
        let chunk = cfg.chunk_size.max(1);
        while scan_from <= safe_head {
            let scan_to = (scan_from + chunk - 1).min(safe_head);
            let ctx = cfg.watch_context(&self.discovered_snapshot());
            let events = source.fetch_events(scan_from, scan_to, &ctx).await?;

            // Fold newly-discovered clones so subsequent chunks gate correctly.
            {
                let mut g = self.discovered.lock().unwrap();
                for e in &events {
                    if e.event_type == EventType::IssuerCreated {
                        if let Some(c) = &e.clone {
                            g.insert(c.to_ascii_lowercase());
                        }
                    }
                }
            }

            if !events.is_empty() {
                store.upsert_events(&events).await;
            }

            // Advance + persist the cursor for this chunk (chunk-granular resume).
            let hash = source.block_hash(scan_to).await?;
            cursor = Cursor {
                last_block: Some(scan_to),
                last_block_hash: hash,
            };
            store.set_cursor(cursor.clone()).await;

            if !events.is_empty() {
                tracing::info!(
                    "indexed {} events in blocks {scan_from}..={scan_to} (head {head})",
                    events.len()
                );
            }
            scan_from = scan_to + 1;
        }
        Ok(())
    }
}
