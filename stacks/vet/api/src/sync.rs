//! Calendar sync engine + appointment-replica reconciliation (impl §3.6 / §3.7, architecture §8).
//!
//! These are plain `async fn`s over [`AppState`] so they are callable from BOTH the HTTP handlers
//! (`routes.rs`) AND tests directly (the cron renewal + sync pass need no live scheduler).

use crate::app::AppState;
use crate::calendar::{ListOutcome, UpsertEvent};
use crate::store::{ApptReplica, GcalEventMap, GcalSyncState};

/// Outcome of one `/calendar/sync` pass (for the HTTP response + test assertions).
#[derive(Debug, Default)]
pub struct SyncReport {
    /// our own writes recognized via the etag-primary discriminator and SKIPPED (echo loop avoided).
    pub echoes_skipped: usize,
    /// untagged external events upserted as read-only busy blocks.
    pub busy_blocks: usize,
    /// owned events whose etag CHANGED (a human edited them in Google) — detected, NOT dropped.
    pub human_edits: usize,
    /// owned events reconciled into the mapping (rev advanced etc).
    pub reconciled: usize,
    /// true if a 410 triggered a full mirror wipe + resync.
    pub full_resync: bool,
}

/// Run an incremental sync pass (impl §3.6). On HTTP 410 it discards the token, wipes the mirror,
/// and performs a full resync. Echo discriminator is **etag-PRIMARY** (§13.3): an owned event whose
/// stored etag MATCHES is our own echo (skip); a CHANGED etag is a human edit (detect + reconcile).
pub async fn run_sync(st: &AppState) -> SyncReport {
    let mut report = SyncReport::default();
    let mut state = st.store.get_sync_state().await;

    // first attempt: incremental with the stored token (None == full list).
    let outcome = st.store_list(state.sync_token.as_deref()).await;
    let (events, next_token) = match outcome {
        ListOutcome::Gone => {
            // HTTP 410: discard token + WIPE mirror + full resync.
            report.full_resync = true;
            st.store.wipe_gcal_mirror().await;
            state.sync_token = None;
            st.store.put_sync_state(state.clone()).await;
            match st.store_list(None).await {
                ListOutcome::Page {
                    events,
                    next_sync_token,
                } => (events, next_sync_token),
                // a 410 on a full list too — give up this pass without crashing.
                ListOutcome::Gone => (vec![], String::new()),
            }
        }
        ListOutcome::Page {
            events,
            next_sync_token,
        } => (events, next_sync_token),
    };

    for ev in events {
        if ev.owned {
            // OUR event. Look up the stored mapping by google event id.
            let stored = st.store.get_gcal_map_by_event(&ev.id).await;
            match stored {
                Some(m) if m.etag == ev.etag => {
                    // etag MATCHES -> our own echo. SKIP (no duplicate appointment created).
                    report.echoes_skipped += 1;
                }
                Some(_) => {
                    // etag CHANGED on an owned event -> a HUMAN edited it in Google. NOT silently
                    // dropped (§8.1): platform-wins, but the edit is DETECTED and reconciled. We
                    // re-mirror the platform's authoritative state back over the human edit.
                    report.human_edits += 1;
                    if let Some(appt_id) = ev.appt_id.clone() {
                        if let Some(appt) = st.store.get_appt(&appt_id).await {
                            mirror_to_google(st, &appt).await;
                            report.reconciled += 1;
                        }
                    }
                }
                None => {
                    // owned tag but no mapping (e.g. post-wipe full resync): rebuild the mapping.
                    if let Some(appt_id) = ev.appt_id.clone() {
                        st.store
                            .put_gcal_map(GcalEventMap {
                                appointment_id: appt_id,
                                google_event_id: ev.id.clone(),
                                etag: ev.etag.clone(),
                                rev: ev.rev.unwrap_or(0),
                                direction: "out".to_string(),
                            })
                            .await;
                        report.reconciled += 1;
                    }
                }
            }
        } else if ev.cancelled {
            // an external event was deleted in Google -> drop its busy-block mapping.
            st.store.delete_gcal_map_by_event(&ev.id).await;
        } else {
            // untagged EXTERNAL event -> upsert a read-only BUSY BLOCK (direction "in").
            st.store
                .put_gcal_map(GcalEventMap {
                    appointment_id: format!("busy:{}", ev.id),
                    google_event_id: ev.id.clone(),
                    etag: ev.etag.clone(),
                    rev: 0,
                    direction: "in".to_string(),
                })
                .await;
            report.busy_blocks += 1;
        }
    }

    // persist nextSyncToken (only if we got one — an empty string means no progress).
    if !next_token.is_empty() {
        state.sync_token = Some(next_token);
    }
    st.store.put_sync_state(state).await;
    report
}

/// Mirror a platform appointment to Google tagged `extendedProperties.private { dogtag.owned:1,
/// dogtag.apptId, dogtag.rev }` and store the returned etag so the NEXT sync recognizes the echo.
pub async fn mirror_to_google(st: &AppState, appt: &ApptReplica) {
    let existing = st.store.get_gcal_map_by_appt(&appt.appointment_id).await;
    let cancelled = matches!(appt.state.as_str(), "CANCELLED" | "DECLINED");
    let ev = UpsertEvent {
        google_event_id: existing.as_ref().map(|m| m.google_event_id.clone()),
        appt_id: appt.appointment_id.clone(),
        rev: appt.rev,
        summary: format!("DogTag appt {} ({})", appt.dog_tag_id, appt.state),
        start: appt.slot.clone(),
        end: appt.slot.clone(),
        cancelled,
    };
    match st.calendar.upsert_event(&ev).await {
        Ok(res) => {
            st.store
                .put_gcal_map(GcalEventMap {
                    appointment_id: appt.appointment_id.clone(),
                    google_event_id: res.google_event_id,
                    etag: res.etag,
                    rev: appt.rev,
                    direction: "out".to_string(),
                })
                .await;
        }
        Err(e) => {
            tracing::warn!(appt = %appt.appointment_id, err = %e, "mirror_to_google failed");
        }
    }
}

/// The watch-channel renewal "cron" (impl §3.6: every ~6 days re-create `events.watch`). A documented
/// function + test-callable method; no live scheduler. Re-creates the channel if older than 6 days
/// (or never created) and persists the new channel id + creation timestamp.
pub const WATCH_RENEW_SECS: u64 = 6 * 24 * 60 * 60;

pub async fn renew_watch_if_due(st: &AppState, now: u64) -> bool {
    let mut state = st.store.get_sync_state().await;
    let due = state.channel_id.is_none()
        || now.saturating_sub(state.channel_created_at) >= WATCH_RENEW_SECS;
    if !due {
        return false;
    }
    match st.calendar.watch().await {
        Ok((channel_id, resource_id)) => {
            state.channel_id = Some(channel_id);
            state.resource_id = Some(resource_id);
            state.channel_created_at = now;
            st.store.put_sync_state(state).await;
            true
        }
        Err(e) => {
            tracing::warn!(err = %e, "events.watch renewal failed");
            false
        }
    }
}

// --------------------------------------------------------------------------------------------
// AppState helper: a thin wrapper so `run_sync` reads cleanly. Kept here (not on AppState) to
// avoid widening the public surface beyond Phase 7.
// --------------------------------------------------------------------------------------------

impl AppState {
    async fn store_list(&self, sync_token: Option<&str>) -> ListOutcome {
        match self.calendar.list_events(sync_token).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(err = %e, "events.list failed; treating as empty page");
                ListOutcome::Page {
                    events: vec![],
                    next_sync_token: String::new(),
                }
            }
        }
    }

    /// Persist the Google refresh token into the sync state (operator connect flow).
    pub async fn store_refresh_token(&self, token: String) {
        let mut s = self.store.get_sync_state().await;
        s.refresh_token = Some(token);
        self.store.put_sync_state(s).await;
    }
}

/// Convenience constructor used by both the PUT handler and tests: build a replica from the central
/// appointment JSON shape `{id, businessId, dogTagId, slot, rev, state, updatedAt}`.
pub fn replica_from_json(v: &serde_json::Value, now: u64) -> Option<ApptReplica> {
    Some(ApptReplica {
        appointment_id: v.get("id").and_then(|x| x.as_str())?.to_string(),
        business_id: v.get("businessId").and_then(|x| x.as_str())?.to_string(),
        dog_tag_id: v
            .get("dogTagId")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        slot: v
            .get("slot")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string(),
        rev: v.get("rev").and_then(|x| x.as_u64())?,
        state: v
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("REQUESTED")
            .to_string(),
        updated_at: v.get("updatedAt").and_then(|x| x.as_u64()).unwrap_or(now),
    })
}

/// Unused-but-documented default sync state factory (keeps the type referenced for non-mongo builds).
pub fn empty_sync_state() -> GcalSyncState {
    GcalSyncState::default()
}
