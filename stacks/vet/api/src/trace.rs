//! Role-traceability join — the vet/groomer "trace my credentials" domain logic (govarch PR-5).
//!
//! The traceability portal answers: *"show me the on-chain activity for credentials THIS business
//! issued/handled, joined to my own off-chain records."* The on-chain half comes from the oversight
//! indexer via `crate::oversight` (already scoped server-side to this business's signer/clone by its
//! bearer token). This module supplies the two pieces the role backend layers on top:
//!
//!   1. **A local scope gate** ([`LocalScope`]) — a second, server-side re-check that every event's
//!      `actor`/`clone` belongs to THIS operator, derived from its own config + custody + records.
//!      This mirrors the indexer's `scope.rs` doctrine (`actor ∈ signers OR clone ∈ clones`) and is
//!      pure defense-in-depth: even if this backend's indexer token were mis-scoped, a foreign
//!      operator's event can never reach the response. A vet can never see another vet's activity.
//!   2. **A DB-record join** ([`LocalIndex`]) — matches each indexed event to this operator's own
//!      `Record` (by anchored root / tx) or `VerifySession` (by verification nullifier / tx), so the
//!      feed reads as "my record #… ↔ its on-chain anchor" instead of opaque hex.
//!
//! The government stack consumes the same shapes UNSCOPED (`scope = None`): every issuer's event is
//! admitted, and the join simply highlights which events are the government's own.

use std::collections::{BTreeSet, HashMap};

use serde_json::{json, Value};

use crate::app::Config;
use crate::store::Store;

/// Lowercase + trim an address/hex key for canonical comparison (the indexer stores lowercase hex).
fn norm(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

/// A usable, non-zero `0x…` address. Rejects blanks and the all-zero address so a `0x0` issuer-clone
/// config (a groomer that is a pure verifier) never widens scope to "the zero signer".
fn is_addr(s: &str) -> bool {
    let s = s.trim();
    s.len() >= 3 && s.starts_with("0x") && s.as_bytes()[2..].iter().any(|&b| b != b'0')
}

/// This operator's own on-chain identity: the signer + `DogTagIssuer` clone addresses that belong to
/// THIS business. An event is in scope iff its acting `actor` ∈ `signers` OR its `clone` ∈ `clones` —
/// the exact admission rule the indexer's `Scope::Signers` enforces, re-applied locally.
#[derive(Clone, Debug, Default)]
pub struct LocalScope {
    pub signers: BTreeSet<String>,
    pub clones: BTreeSet<String>,
}

impl LocalScope {
    pub fn add_signer(&mut self, s: &str) {
        if is_addr(s) {
            self.signers.insert(norm(s));
        }
    }
    pub fn add_clone(&mut self, c: &str) {
        if is_addr(c) {
            self.clones.insert(norm(c));
        }
    }
    pub fn is_empty(&self) -> bool {
        self.signers.is_empty() && self.clones.is_empty()
    }
    /// Admit an event JSON iff its `actor` is one of ours OR its `clone` is one of ours.
    pub fn admits(&self, ev: &Value) -> bool {
        if let Some(a) = ev.get("actor").and_then(|v| v.as_str()) {
            if self.signers.contains(&norm(a)) {
                return true;
            }
        }
        if let Some(c) = ev.get("clone").and_then(|v| v.as_str()) {
            if self.clones.contains(&norm(c)) {
                return true;
            }
        }
        false
    }
}

/// A join index from on-chain identifiers to this operator's own off-chain records. Each indexed event
/// is matched to its local record by anchored **root** (issuances/revocations), verification
/// **nullifier**, or **tx hash** (a belt on either path).
#[derive(Default)]
pub struct LocalIndex {
    by_root: HashMap<String, Value>,
    by_nullifier: HashMap<String, Value>,
    by_tx: HashMap<String, Value>,
}

impl LocalIndex {
    pub fn insert_by_root(&mut self, root: &str, summary: Value) {
        if !root.trim().is_empty() {
            self.by_root.insert(norm(root), summary);
        }
    }
    pub fn insert_by_nullifier(&mut self, nullifier: &str, summary: Value) {
        if !nullifier.trim().is_empty() {
            self.by_nullifier.insert(norm(nullifier), summary);
        }
    }
    pub fn insert_by_tx(&mut self, tx: &str, summary: Value) {
        if !tx.trim().is_empty() {
            self.by_tx.entry(norm(tx)).or_insert(summary);
        }
    }
    pub fn len(&self) -> usize {
        self.by_root.len() + self.by_nullifier.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_root.is_empty() && self.by_nullifier.is_empty() && self.by_tx.is_empty()
    }

    /// The local record joined to `ev`, if any: root first (issuance/revocation), then nullifier
    /// (verification), then tx hash.
    fn lookup(&self, ev: &Value) -> Option<Value> {
        if let Some(r) = ev.get("root").and_then(|v| v.as_str()) {
            if let Some(hit) = self.by_root.get(&norm(r)) {
                return Some(hit.clone());
            }
        }
        if let Some(n) = ev.get("nullifier").and_then(|v| v.as_str()) {
            if let Some(hit) = self.by_nullifier.get(&norm(n)) {
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

/// The outcome of gating + joining an indexer page of events.
pub struct Joined {
    /// The events that survived the local scope gate, each with a `"local"` join field (record JSON or
    /// `null`).
    pub events: Vec<Value>,
    /// Count of events admitted by the local scope gate (== `events.len()`).
    pub in_scope: usize,
    /// Count of admitted events that joined to a local record.
    pub matched: usize,
    /// Count of events the indexer returned that the local scope gate REJECTED (should be 0 when the
    /// indexer token is correctly scoped; > 0 signals a scope mismatch worth surfacing).
    pub dropped_out_of_scope: usize,
}

/// Gate (when `scope` is `Some` — vet/groomer) and join each indexer event to its local record.
///
/// `scope = None` ⇒ UNSCOPED (government): every event passes; the join is a best-effort highlight of
/// the government's own records. `scope = Some(..)` ⇒ SCOPED: an event is dropped unless its
/// actor/clone is one of this operator's own — a foreign operator's event can never be emitted.
pub fn join_events(events: Vec<Value>, scope: Option<&LocalScope>, idx: &LocalIndex) -> Joined {
    let mut out = Vec::with_capacity(events.len());
    let mut matched = 0usize;
    let mut dropped = 0usize;
    for mut ev in events {
        if let Some(sc) = scope {
            if !sc.admits(&ev) {
                dropped += 1;
                continue;
            }
        }
        let local = idx.lookup(&ev);
        if local.is_some() {
            matched += 1;
        }
        if let Value::Object(map) = &mut ev {
            map.insert("local".into(), local.unwrap_or(Value::Null));
        }
        out.push(ev);
    }
    let in_scope = out.len();
    Joined {
        events: out,
        in_scope,
        matched,
        dropped_out_of_scope: dropped,
    }
}

/// Build this operator's local scope from everything that authoritatively identifies it on-chain:
/// its configured issuer-clone addresses, its custody signer accounts, and the signer/clone addresses
/// stamped onto its own records + verification sessions. All non-zero, lowercased.
pub async fn build_scope(
    store: &dyn Store,
    cfg: &Config,
    records: &[crate::store::Record],
    sessions: &[crate::store::VerifySession],
) -> LocalScope {
    let mut scope = LocalScope::default();

    // Configured DogTagIssuer clones (recordType -> clone address).
    for clone in cfg.issuer_addrs.values() {
        scope.add_clone(clone);
    }

    // Custody signer accounts (addresses only — never the seed). Present even when custody is locked.
    if let Some(blob) = store.get_custody().await {
        for acct in &blob.meta.accounts {
            scope.add_signer(&acct.address);
        }
    }

    // Signer/clone addresses observed on this operator's own issued records.
    for r in records {
        if let Some(s) = &r.signer_address {
            scope.add_signer(s);
        }
        scope.add_clone(&r.issuer_addr);
    }

    // Relayer addresses on this operator's own verification sessions (the verifier == this signer).
    for s in sessions {
        scope.add_signer(&s.relayer);
    }

    scope
}

/// Build the DB-record join index from this operator's own store: every issued `Record` keyed by its
/// anchored root + issuance/revocation tx, and every `VerifySession` keyed by its nullifier + tx.
pub fn build_index(
    records: &[crate::store::Record],
    sessions: &[crate::store::VerifySession],
) -> LocalIndex {
    let mut idx = LocalIndex::default();

    for r in records {
        let summary = record_summary(r);
        idx.insert_by_root(&r.root, summary.clone());
        if let Some(tx) = &r.confirmed_tx_hash {
            idx.insert_by_tx(tx, summary.clone());
        }
        if let Some(tx) = &r.tx_hash {
            idx.insert_by_tx(tx, summary.clone());
        }
        if let Some(tx) = &r.revoked_tx_hash {
            idx.insert_by_tx(tx, summary.clone());
        }
    }

    for s in sessions {
        let summary = session_summary(s);
        if let Some(n) = &s.nullifier {
            idx.insert_by_nullifier(n, summary.clone());
        }
        if let Some(tx) = &s.tx_hash {
            idx.insert_by_tx(tx, summary);
        }
    }

    idx
}

/// The non-PII join projection of an issued record shown next to its on-chain event. Includes the
/// operator's own off-chain label/notes (this is the operator's own portal, viewing its own records).
fn record_summary(r: &crate::store::Record) -> Value {
    json!({
        "kind": "issuance",
        "recordId": r.record_id,
        "recordType": r.record_type,
        "dogTagId": r.dog_tag_id,
        "status": r.status,
        "label": r.label,
        "notes": r.notes,
    })
}

/// The non-PII join projection of a verification session shown next to its on-chain `Verified` event.
fn session_summary(s: &crate::store::VerifySession) -> Value {
    json!({
        "kind": "verification",
        "sessionId": s.session_id,
        "recordType": s.record_type,
        "purpose": s.purpose,
        "mode": s.mode,
        "status": s.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(actor: Option<&str>, clone: Option<&str>, root: Option<&str>) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("type".into(), json!("rootIssued"));
        if let Some(a) = actor {
            m.insert("actor".into(), json!(a));
        }
        if let Some(c) = clone {
            m.insert("clone".into(), json!(c));
        }
        if let Some(r) = root {
            m.insert("root".into(), json!(r));
        }
        Value::Object(m)
    }

    #[test]
    fn is_addr_rejects_zero_and_blanks() {
        assert!(is_addr("0xAbC0000000000000000000000000000000000001"));
        assert!(!is_addr("0x0000000000000000000000000000000000000000"));
        assert!(!is_addr(""));
        assert!(!is_addr("0x"));
        assert!(!is_addr("not-hex")); // no 0x prefix
    }

    #[test]
    fn scope_admits_own_signer_or_clone_case_insensitively() {
        let mut s = LocalScope::default();
        s.add_signer("0xAA00000000000000000000000000000000000001");
        s.add_clone("0xCC00000000000000000000000000000000000001");
        // own signer (mixed case in the event)
        assert!(s.admits(&ev(
            Some("0xaa00000000000000000000000000000000000001"),
            None,
            None
        )));
        // own clone via a foreign signer
        assert!(s.admits(&ev(
            Some("0xother0000000000000000000000000000000000"),
            Some("0xCC00000000000000000000000000000000000001"),
            None
        )));
        // neither ours -> rejected
        assert!(!s.admits(&ev(
            Some("0xbb00000000000000000000000000000000000002"),
            Some("0xdd00000000000000000000000000000000000002"),
            None
        )));
        // factory event with no actor/clone -> rejected
        assert!(!s.admits(&ev(None, None, None)));
    }

    #[test]
    fn zero_address_never_widens_scope() {
        let mut s = LocalScope::default();
        s.add_clone("0x0000000000000000000000000000000000000000"); // pure-verifier groomer clone
        assert!(s.is_empty(), "the zero clone is not a real scope key");
        assert!(!s.admits(&ev(
            None,
            Some("0x0000000000000000000000000000000000000000"),
            None
        )));
    }

    #[test]
    fn join_scoped_drops_foreign_and_matches_own_record() {
        let mut scope = LocalScope::default();
        scope.add_signer("0xaa00000000000000000000000000000000000001");
        let mut idx = LocalIndex::default();
        idx.insert_by_root("0xR00T", json!({ "recordId": "rec-1", "kind": "issuance" }));

        let events = vec![
            // ours + has a matching local record
            ev(
                Some("0xaa00000000000000000000000000000000000001"),
                None,
                Some("0xR00T"),
            ),
            // ours but no local record (on-chain-only — still shown, unmatched)
            ev(
                Some("0xaa00000000000000000000000000000000000001"),
                None,
                Some("0xUNKNOWN"),
            ),
            // a DIFFERENT operator's event — the indexer should never have returned it, but the local
            // gate drops it regardless (defense-in-depth).
            ev(
                Some("0xbb00000000000000000000000000000000000002"),
                None,
                Some("0xR00T"),
            ),
        ];
        let out = join_events(events, Some(&scope), &idx);
        assert_eq!(out.in_scope, 2, "only our two events survive the gate");
        assert_eq!(out.dropped_out_of_scope, 1, "the foreign event is dropped");
        assert_eq!(out.matched, 1, "one of ours joins a local record");
        // the matched event carries the record; the unmatched one carries null.
        assert_eq!(out.events[0]["local"]["recordId"], "rec-1");
        assert_eq!(out.events[1]["local"], Value::Null);
        // the foreign operator's event is absent entirely.
        assert!(out
            .events
            .iter()
            .all(|e| e["actor"] != json!("0xbb00000000000000000000000000000000000002")));
    }

    #[test]
    fn join_unscoped_admits_all_and_highlights_own() {
        let mut idx = LocalIndex::default();
        idx.insert_by_root("0xmine", json!({ "recordId": "gov-1" }));
        let events = vec![
            ev(
                Some("0xany1000000000000000000000000000000000000"),
                None,
                Some("0xmine"),
            ),
            ev(
                Some("0xany2000000000000000000000000000000000000"),
                None,
                Some("0xtheirs"),
            ),
        ];
        // scope = None -> government/unscoped: every event passes.
        let out = join_events(events, None, &idx);
        assert_eq!(out.in_scope, 2, "unscoped admits every issuer's event");
        assert_eq!(out.dropped_out_of_scope, 0);
        assert_eq!(out.matched, 1, "only the government's own record joins");
        assert_eq!(out.events[0]["local"]["recordId"], "gov-1");
        assert_eq!(out.events[1]["local"], Value::Null);
    }

    #[test]
    fn join_matches_verification_by_nullifier() {
        let mut idx = LocalIndex::default();
        idx.insert_by_nullifier(
            "0xnull",
            json!({ "kind": "verification", "sessionId": "s-1" }),
        );
        let verified = json!({ "type": "verified", "actor": "0xrelayer", "nullifier": "0xNULL" });
        let out = join_events(vec![verified], None, &idx);
        assert_eq!(out.matched, 1);
        assert_eq!(out.events[0]["local"]["sessionId"], "s-1");
    }
}
