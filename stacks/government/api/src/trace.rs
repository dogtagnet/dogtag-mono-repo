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

/// The on-chain form of a dogTagId, as the indexer serves it on `verified` events: the DECIMAL string
/// of `field_of_value(Integer(handle))` (the Verified event's `uint256 dogTagId` topic). The
/// authority's credentials store the OPERATOR-entered handle (e.g. `"7"`), while the consent circuit
/// field-hashes that handle into `pub[0]` - mirroring the vet stack's `onchain_dog_tag_id`, plus the
/// hex→decimal step because the indexer renders the event topic with `U256::to_string()`.
///
/// `None` on a malformed handle: a credential that cannot be canonically hashed simply never joins.
pub fn onchain_dog_tag_id_decimal(handle: &str) -> Option<String> {
    let scalar =
        dogtag_standard::wrap::scalar_from_packed(dogtag_standard::types::TypeTag::Integer, handle)
            .ok()?;
    let f = dogtag_standard::leaf::field_of_value(&scalar).ok()?;
    let hex = dogtag_standard::field::to_hex32(&f);
    let v = alloy::primitives::U256::from_str_radix(hex.trim_start_matches("0x"), 16).ok()?;
    Some(v.to_string())
}

/// A join index from on-chain identifiers to the government's own off-chain records: issued
/// credentials keyed by anchored root / tx / on-chain dogTagId, verification audit rows keyed by root.
#[derive(Default)]
pub struct LocalIndex {
    by_root: HashMap<String, Value>,
    by_tx: HashMap<String, Value>,
    /// on-chain (field-hashed, decimal) dogTagId -> the NEWEST credential the authority issued for
    /// that tag. Joins the owner-blind `verified` events, which carry no root - only the tag id.
    by_dog_tag_id: HashMap<String, (u64, Value)>,
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
    /// Keep the NEWEST credential per tag (`created_at` wins; ties → later insert wins) so a tag with
    /// several clearances joins its current one regardless of store iteration order.
    fn insert_by_dog_tag_id(&mut self, onchain_id: &str, created_at: u64, summary: Value) {
        if onchain_id.trim().is_empty() {
            return;
        }
        match self.by_dog_tag_id.entry(onchain_id.to_string()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if created_at >= e.get().0 {
                    e.insert((created_at, summary));
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert((created_at, summary));
            }
        }
    }
    pub fn len(&self) -> usize {
        self.by_root.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_root.is_empty() && self.by_tx.is_empty() && self.by_dog_tag_id.is_empty()
    }

    /// The government record joined to `ev`, if any: anchored root first, then tx hash, then - for the
    /// owner-blind `verified` events, which carry neither - the on-chain dogTagId. A dogTagId hit
    /// means "this event concerns a tag we credentialed", NOT "our credential was verified": the
    /// Verified event binds no root/recordType, so the credential itself cannot be identified -
    /// `purpose` is the only disambiguator, and the join is deliberately labeled at tag granularity.
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
        if let Some(id) = ev.get("dogTagId").and_then(|v| v.as_str()) {
            if let Some((_, hit)) = self.by_dog_tag_id.get(id.trim()) {
                // Stamp HOW this joined so the portal can label it honestly ("tag we credentialed")
                // instead of implying the verification was of this credential.
                let mut hit = hit.clone();
                if let Value::Object(map) = &mut hit {
                    map.insert("joinedBy".into(), json!("dogTagId"));
                }
                return Some(hit);
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
            idx.insert_by_tx(tx, summary.clone());
        }
        // The owner-blind `verified` events carry only the on-chain (field-hashed) dogTagId - no
        // root. Index each credential under that key too, so "a tag we credentialed was verified"
        // surfaces in the oversight feed (travel-clearance oversight). Skipped when the stored
        // handle cannot be canonically field-hashed.
        if let Some(onchain_id) = onchain_dog_tag_id_decimal(&c.dog_tag_id) {
            idx.insert_by_dog_tag_id(&onchain_id, c.created_at, summary);
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
    fn onchain_dog_tag_id_decimal_is_a_real_field_hash() {
        let a = onchain_dog_tag_id_decimal("7").expect("valid handle");
        let b = onchain_dog_tag_id_decimal("7").expect("valid handle");
        assert_eq!(a, b, "deterministic");
        // The transform is a REAL Poseidon field-hash, not an identity mapping: production events
        // carry the hashed id, so an untransformed key would silently never join.
        assert_ne!(a, "7", "the on-chain id is NOT the raw handle");
        assert!(
            a.chars().all(|c| c.is_ascii_digit()),
            "indexer renders U256::to_string() - decimal digits only: {a}"
        );
        // And it matches the vet stack's canonical hex form, decimal-rendered.
        let hex = {
            let scalar = dogtag_standard::wrap::scalar_from_packed(
                dogtag_standard::types::TypeTag::Integer,
                "7",
            )
            .unwrap();
            let f = dogtag_standard::leaf::field_of_value(&scalar).unwrap();
            dogtag_standard::field::to_hex32(&f)
        };
        let expected = alloy::primitives::U256::from_str_radix(hex.trim_start_matches("0x"), 16)
            .unwrap()
            .to_string();
        assert_eq!(a, expected);
        // Garbage handles fail closed (no join, no panic).
        assert!(onchain_dog_tag_id_decimal("not-a-number").is_none());
    }

    #[test]
    fn verified_event_joins_credential_by_onchain_dog_tag_id_only() {
        let onchain = onchain_dog_tag_id_decimal("7").unwrap();
        let mut idx = LocalIndex::default();
        idx.insert_by_dog_tag_id(
            &onchain,
            1000,
            json!({ "kind": "issuance", "receiptId": "RCPT7", "dogTagId": "7" }),
        );

        // The owner-blind verified event carries the FIELD-HASHED id (as the indexer serves it).
        let verified = json!({ "type": "verified", "dogTagId": onchain, "purpose": "0xp" });
        let out = join_events(vec![verified], &idx);
        assert_eq!(out.matched, 1, "field-hashed id joins");
        assert_eq!(out.events[0]["local"]["receiptId"], "RCPT7");
        assert_eq!(
            out.events[0]["local"]["joinedBy"], "dogTagId",
            "the join is stamped tag-granular so the portal labels it honestly"
        );

        // An event carrying the RAW handle (never produced on-chain) must NOT join - proving the
        // index is keyed by the transform, not the stored handle.
        let raw = json!({ "type": "verified", "dogTagId": "7" });
        let out = join_events(vec![raw], &idx);
        assert_eq!(out.matched, 0, "raw handle never joins");
    }

    #[test]
    fn newest_credential_wins_the_dog_tag_id_key() {
        let mut idx = LocalIndex::default();
        idx.insert_by_dog_tag_id("123", 1000, json!({ "receiptId": "OLD" }));
        idx.insert_by_dog_tag_id("123", 2000, json!({ "receiptId": "NEW" }));
        idx.insert_by_dog_tag_id("123", 1500, json!({ "receiptId": "MID" }));
        let ev = json!({ "type": "verified", "dogTagId": "123" });
        let out = join_events(vec![ev], &idx);
        assert_eq!(
            out.events[0]["local"]["receiptId"], "NEW",
            "a tag with several clearances joins its newest"
        );
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
