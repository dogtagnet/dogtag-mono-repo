//! Pure-logic unit coverage for `sync.rs` parsing/factory helpers (no AppState, no live Google).
//!
//! Targets the two `pub` pure functions that the calendar/replica integration tests exercise only
//! indirectly: `replica_from_json` (central appointment JSON -> `ApptReplica`) and the
//! `empty_sync_state` default factory. Behavior-preserving: asserts the existing contract.

use serde_json::json;
use vet_api::store::ApptReplica;
use vet_api::sync::{empty_sync_state, replica_from_json};

// --------------------------------------------------------------------------------------------
// replica_from_json - required fields, optional defaults, type coercion.
// --------------------------------------------------------------------------------------------

#[test]
fn replica_from_json_full_shape_maps_every_field() {
    let v = json!({
        "id": "appt-1",
        "businessId": "biz-7",
        "dogTagId": "dog-42",
        "slot": "2026-07-01T09:00:00Z",
        "rev": 5u64,
        "state": "CONFIRMED",
        "updatedAt": 1_700_000_000u64,
    });
    let r: ApptReplica = replica_from_json(&v, 0).expect("full shape parses");
    assert_eq!(r.appointment_id, "appt-1");
    assert_eq!(r.business_id, "biz-7");
    assert_eq!(r.dog_tag_id, "dog-42");
    assert_eq!(r.slot, "2026-07-01T09:00:00Z");
    assert_eq!(r.rev, 5);
    assert_eq!(r.state, "CONFIRMED");
    assert_eq!(r.updated_at, 1_700_000_000);
}

#[test]
fn replica_from_json_requires_id_business_and_rev() {
    let base = json!({"id": "a", "businessId": "b", "rev": 1u64});
    assert!(
        replica_from_json(&base, 0).is_some(),
        "minimal required set parses"
    );

    // each required field individually missing -> None
    let mut no_id = base.clone();
    no_id.as_object_mut().unwrap().remove("id");
    assert!(
        replica_from_json(&no_id, 0).is_none(),
        "missing id rejected"
    );

    let mut no_biz = base.clone();
    no_biz.as_object_mut().unwrap().remove("businessId");
    assert!(
        replica_from_json(&no_biz, 0).is_none(),
        "missing businessId rejected"
    );

    let mut no_rev = base.clone();
    no_rev.as_object_mut().unwrap().remove("rev");
    assert!(
        replica_from_json(&no_rev, 0).is_none(),
        "missing rev rejected"
    );
}

#[test]
fn replica_from_json_rev_must_be_unsigned_integer() {
    // a string "rev" cannot coerce via as_u64 -> the whole parse fails.
    let v = json!({"id": "a", "businessId": "b", "rev": "1"});
    assert!(replica_from_json(&v, 0).is_none());
}

#[test]
fn replica_from_json_applies_optional_field_defaults() {
    let v = json!({"id": "a", "businessId": "b", "rev": 9u64});
    let r = replica_from_json(&v, 4242).expect("parses with only required fields");
    assert_eq!(r.dog_tag_id, "", "absent dogTagId defaults to empty");
    assert_eq!(r.slot, "", "absent slot defaults to empty");
    assert_eq!(r.state, "REQUESTED", "absent state defaults to REQUESTED");
    assert_eq!(r.updated_at, 4242, "absent updatedAt falls back to `now`");
}

#[test]
fn replica_from_json_present_updated_at_overrides_now() {
    let v = json!({"id": "a", "businessId": "b", "rev": 1u64, "updatedAt": 100u64});
    let r = replica_from_json(&v, 999).unwrap();
    assert_eq!(r.updated_at, 100, "explicit updatedAt wins over now");
}

// --------------------------------------------------------------------------------------------
// empty_sync_state - the documented default factory.
// --------------------------------------------------------------------------------------------

#[test]
fn empty_sync_state_is_all_defaults() {
    let s = empty_sync_state();
    assert!(s.sync_token.is_none());
    assert!(s.channel_id.is_none());
    assert!(s.resource_id.is_none());
    assert!(s.refresh_token.is_none());
    assert_eq!(s.channel_created_at, 0);
}
