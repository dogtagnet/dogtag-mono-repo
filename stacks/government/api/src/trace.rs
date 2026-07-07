//! Government oversight join — the UNSCOPED "see every issuer's activity, highlight my own" logic
//! (govarch PR-5). The government authority consumes the oversight indexer unscoped (every issuer's
//! events), then joins each on-chain event to its OWN issued credentials / verification records so the
//! oversight console reads "this root is credential #… we issued" alongside the cross-issuer feed.
//!
//! Unlike the vet/groomer traceability portal there is NO scope gate — the government is allowed to see
//! every issuer. The join is a best-effort highlight: an event that matches a government record carries
//! its non-PII summary; everything else is shown as-is (another issuer's activity, `local: null`).

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::store::{IssuedCredential, Store, VerificationRecord};

/// Lowercase + trim a hex key for canonical comparison (the indexer stores lowercase hex).
fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// A join index from on-chain identifiers to the government's own off-chain records: issued
/// credentials keyed by anchored root / tx, verification audit rows keyed by root.
#[derive(Default)]
pub struct LocalIndex {
    by_root: HashMap<String, Value>,
    by_tx: HashMap<String, Value>,
}

impl LocalIndex {
    fn insert_by_root(&mut self, root: &str, summary: Value) {
        if !root.trim().is_empty() {
            self.by_root.insert(norm(root), summary);
        }
    }
    fn insert_by_root_if_absent(&mut self, root: &str, summary: Value) {
        if !root.trim().is_empty() {
            self.by_root.entry(norm(root)).or_insert(summary);
        }
    }
    fn insert_by_tx(&mut self, tx: &str, summary: Value) {
        if !tx.trim().is_empty() {
            self.by_tx.entry(norm(tx)).or_insert(summary);
        }
    }
    pub fn len(&self) -> usize {
        self.by_root.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_root.is_empty() && self.by_tx.is_empty()
    }

    /// The government record joined to `ev`, if any: anchored root first, then tx hash.
    fn lookup(&self, ev: &Value) -> Option<Value> {
        if let Some(r) = ev.get("root").and_then(|v| v.as_str()) {
            if let Some(hit) = self.by_root.get(&norm(r)) {
                return Some(hit.clone());
            }
        }
        if let Some(tx) = ev.get("txHash").and_then(|v| v.as_str()) {
            if let Some(hit) = self.by_tx.get(&norm(tx)) {
                return Some(hit.clone());
            }
        }
        None
    }
}

/// The outcome of joining an indexer page of (unscoped) events to the government's own records.
pub struct Joined {
    /// Every indexer event, each with a `"local"` field (the government's own record summary or null).
    pub events: Vec<Value>,
    /// Count of events that joined to a government record (its own on-chain activity).
    pub matched: usize,
}

/// Join every indexer event to the government's own records (unscoped: no event is dropped). Events
/// that match a government-issued root/tx carry its non-PII summary; the rest carry `local: null`.
pub fn join_events(events: Vec<Value>, idx: &LocalIndex) -> Joined {
    let mut out = Vec::with_capacity(events.len());
    let mut matched = 0usize;
    for mut ev in events {
        let local = idx.lookup(&ev);
        if local.is_some() {
            matched += 1;
        }
        if let Value::Object(map) = &mut ev {
            map.insert("local".into(), local.unwrap_or(Value::Null));
        }
        out.push(ev);
    }
    Joined {
        events: out,
        matched,
    }
}

/// Build the join index from the government's own store: every issued credential keyed by its anchored
/// root + issuance/revocation tx, and every verification audit row keyed by its checked root.
pub async fn build_index(store: &dyn Store) -> LocalIndex {
    let mut idx = LocalIndex::default();

    for c in store.list_credentials().await {
        let summary = credential_summary(&c);
        idx.insert_by_root(&c.root, summary.clone());
        if let Some(tx) = &c.tx_hash {
            idx.insert_by_tx(tx, summary.clone());
        }
        if let Some(tx) = &c.revoked_tx_hash {
            idx.insert_by_tx(tx, summary);
        }
    }

    // Verification audit rows join a `verified` event's root (the government verified this credential).
    // Credentials win on a root collision (issuance is the more informative join); `or_insert` keeps it.
    for v in store.list_verifications().await {
        idx.insert_by_root_if_absent(&v.root, verification_summary(&v));
    }

    idx
}

/// The **non-PII** join projection of a government-issued credential. Deliberately excludes the
/// obfuscatable `subject` block (Section A importer PII) and the wrapped doc — the oversight feed is
/// the non-PII chain layer, and the government's own portal is no exception to that doctrine.
fn credential_summary(c: &IssuedCredential) -> Value {
    json!({
        "kind": "issuance",
        "root": c.root,
        "recordType": c.record_type,
        "dogTagId": c.dog_tag_id,
        "receiptId": c.receipt_id,
        "status": c.status,
        "label": c.label,
        "notes": c.notes,
        "anchored": c.anchored,
    })
}

/// The non-PII join projection of a government verification audit row.
fn verification_summary(v: &VerificationRecord) -> Value {
    json!({
        "kind": "verification",
        "id": v.id,
        "recordType": v.record_type,
        "verdict": v.verdict,
        "checkedAt": v.checked_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ty: &str, root: Option<&str>, clone: Option<&str>) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("type".into(), json!(ty));
        if let Some(r) = root {
            m.insert("root".into(), json!(r));
        }
        if let Some(c) = clone {
            m.insert("clone".into(), json!(c));
        }
        Value::Object(m)
    }

    #[test]
    fn unscoped_join_admits_all_and_highlights_own() {
        let mut idx = LocalIndex::default();
        idx.insert_by_root("0xGOV", json!({ "kind": "issuance", "root": "0xgov" }));
        let events = vec![
            // the government's own issuance (root matches, case-insensitively)
            ev("rootIssued", Some("0xgov"), Some("0xgovclone")),
            // ANOTHER issuer's issuance — still shown (unscoped), but unmatched
            ev("rootIssued", Some("0xother"), Some("0xotherclone")),
        ];
        let out = join_events(events, &idx);
        assert_eq!(
            out.events.len(),
            2,
            "unscoped: every issuer's event is kept"
        );
        assert_eq!(
            out.matched, 1,
            "only the government's own event joins a record"
        );
        assert_eq!(out.events[0]["local"]["kind"], "issuance");
        assert_eq!(out.events[1]["local"], Value::Null);
    }

    #[test]
    fn credential_wins_root_collision_over_verification() {
        let mut idx = LocalIndex::default();
        // Mirror build_index's order: the issued credential is inserted first...
        idx.insert_by_root("0xROOT", json!({ "kind": "issuance", "root": "0xroot" }));
        // ...then a verification audit row for the SAME root must NOT overwrite it.
        idx.insert_by_root_if_absent("0xROOT", json!({ "kind": "verification" }));
        assert_eq!(
            idx.by_root.get("0xroot").unwrap()["kind"],
            "issuance",
            "credentials win on a root collision"
        );
    }
}
