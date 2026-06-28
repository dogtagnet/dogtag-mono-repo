//! Pure-logic unit coverage for `store.rs`'s `MemStore` - the in-memory `Store` impl that every
//! admin-api E2E test runs against. Its security- and correctness-sensitive contracts (one-time jti
//! consume, atomic microchip reservation, monotonic dogTagId, the case-insensitive email index, the
//! sole-allocator `alloc_rev_and_apply` rev monotonicity, the per-owner list filters, the due-deletion
//! window, and the erasure row deletes / PII clears) were exercised only indirectly through HTTP
//! handlers; this file pins them directly.
//!
//! Behavior-preserving: every assertion captures the existing contract, no source is modified.

use admin_api::crypto::Sealed;
use admin_api::store::{
    Appointment, Consent, ConsentReceipt, Credential, Deletion, MemStore, Microchip, Owner, Pet,
    Store, VerificationRecord,
};

fn sealed(id: &str) -> Sealed {
    Sealed {
        dek_id: id.to_string(),
        nonce: "00".repeat(12),
        ct: "deadbeef".to_string(),
    }
}

fn owner(id: &str, email: Option<&str>) -> Owner {
    Owner {
        owner_id: id.to_string(),
        email: email.map(|e| e.to_string()),
        password_hash: None,
        wallet_address: "0xabc".to_string(),
        push_token: None,
        profile_pii: Some(sealed("pii")),
    }
}

fn pet(id: &str, owner_id: &str, microchip: &str) -> Pet {
    Pet {
        pet_id: id.to_string(),
        owner_id: owner_id.to_string(),
        name: "Rex".to_string(),
        microchip: Microchip {
            code: microchip.to_string(),
            standard: "ISO_11784_11785".to_string(),
            implant_date: "2020-01-01".to_string(),
            body_location: "neck".to_string(),
        },
        profile: Default::default(),
        dog_tag_id: None,
        root: None,
        mint_tx: None,
        sealed_doc: Some(sealed("doc")),
    }
}

fn credential(id: &str, owner_id: &str) -> Credential {
    Credential {
        credential_id: id.to_string(),
        owner_id: owner_id.to_string(),
        dog_tag_id: "1".to_string(),
        root: "0xroot".to_string(),
        sealed_doc: sealed("cred"),
    }
}

fn appt(id: &str, owner_id: &str, updated_at: u64) -> Appointment {
    Appointment {
        appointment_id: id.to_string(),
        business_id: "biz-1".to_string(),
        dog_tag_id: "1".to_string(),
        owner_id: owner_id.to_string(),
        slot: "2026-01-01T10:00:00Z".to_string(),
        rev: 1,
        state: "REQUESTED".to_string(),
        updated_at,
    }
}

fn consent(id: &str, owner_id: &str) -> Consent {
    Consent {
        consent_id: id.to_string(),
        owner_id: owner_id.to_string(),
        purpose: "boarding_intake".to_string(),
        lawful_basis: "consent".to_string(),
        granted_at: 0,
        withdrawn: false,
    }
}

fn receipt(id: &str, owner_id: &str) -> ConsentReceipt {
    ConsentReceipt {
        receipt_id: id.to_string(),
        owner_id: owner_id.to_string(),
        hash: "0xh".to_string(),
        issued_at: 0,
        sealed: sealed("rcpt"),
    }
}

fn vrecord(id: &str, owner_id: &str) -> VerificationRecord {
    VerificationRecord {
        record_id: id.to_string(),
        owner_id: owner_id.to_string(),
        dog_tag_id: "1".to_string(),
        purpose: "boarding_intake".to_string(),
        relayer: "0xrelayer".to_string(),
        mode: "ThirdParty".to_string(),
        status: "verified".to_string(),
        sealed: sealed("vrec"),
    }
}

fn deletion(id: &str, owner_id: &str, due_by: u64, status: &str) -> Deletion {
    Deletion {
        request_id: id.to_string(),
        owner_id: owner_id.to_string(),
        scope: "all".to_string(),
        due_by,
        status: status.to_string(),
    }
}

// --------------------------------------------------------------------------------------------
// owners + the case-insensitive email index.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn owner_email_index_is_case_insensitive_and_only_for_email_owners() {
    let s = MemStore::new();
    s.put_owner(owner("o-1", Some("Alice@Example.com"))).await;
    s.put_owner(owner("o-2", None)).await;

    // round-trips by id.
    assert_eq!(s.get_owner("o-1").await.unwrap().owner_id, "o-1");
    assert_eq!(s.get_owner("o-2").await.unwrap().owner_id, "o-2");

    // email lookup is case-insensitive (index stored lowercased, query lowercased).
    assert_eq!(
        s.get_owner_by_email("alice@example.com")
            .await
            .unwrap()
            .owner_id,
        "o-1"
    );
    assert_eq!(
        s.get_owner_by_email("ALICE@EXAMPLE.COM")
            .await
            .unwrap()
            .owner_id,
        "o-1"
    );
    // an owner with no email is not reachable via the email index.
    assert!(s.get_owner_by_email("o-2").await.is_none());
    assert!(s.get_owner_by_email("missing@x.com").await.is_none());
    assert!(s.get_owner("absent").await.is_none());
}

#[tokio::test]
async fn put_owner_overwrites_by_id() {
    let s = MemStore::new();
    s.put_owner(owner("o-1", Some("a@x.com"))).await;
    let mut updated = owner("o-1", Some("a@x.com"));
    updated.wallet_address = "0xdef".to_string();
    s.put_owner(updated).await;
    assert_eq!(s.get_owner("o-1").await.unwrap().wallet_address, "0xdef");
}

// --------------------------------------------------------------------------------------------
// sessions + admin sessions.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn sessions_and_admin_sessions_are_membership_lookups() {
    let s = MemStore::new();
    s.put_session("tok-1".to_string(), "o-1".to_string()).await;
    assert_eq!(s.session_owner("tok-1").await.as_deref(), Some("o-1"));
    assert!(s.session_owner("nope").await.is_none());

    assert!(!s.has_admin_session("adm").await);
    s.put_admin_session("adm".to_string()).await;
    assert!(s.has_admin_session("adm").await);
    assert!(!s.has_admin_session("other").await);
}

// --------------------------------------------------------------------------------------------
// pets - microchip reservation + monotonic dogTagId.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn reserve_microchip_is_atomic_one_time_and_visible_to_exists() {
    let s = MemStore::new();
    assert!(!s.microchip_exists("chip-a").await);
    // first reservation wins, the second sees it taken.
    assert!(s.reserve_microchip("chip-a").await);
    assert!(!s.reserve_microchip("chip-a").await);
    assert!(s.microchip_exists("chip-a").await);
    // a distinct code reserves independently.
    assert!(s.reserve_microchip("chip-b").await);
}

#[tokio::test]
async fn next_dog_tag_id_is_monotonic_from_one() {
    let s = MemStore::new();
    assert_eq!(s.next_dog_tag_id().await, 1);
    assert_eq!(s.next_dog_tag_id().await, 2);
    assert_eq!(s.next_dog_tag_id().await, 3);
}

#[tokio::test]
async fn pets_of_owner_filters_by_owner() {
    let s = MemStore::new();
    s.put_pet(pet("p-1", "o-1", "c1")).await;
    s.put_pet(pet("p-2", "o-1", "c2")).await;
    s.put_pet(pet("p-3", "o-2", "c3")).await;
    assert_eq!(s.get_pet("p-1").await.unwrap().pet_id, "p-1");
    assert!(s.get_pet("absent").await.is_none());

    let mut ids: Vec<String> = s
        .pets_of_owner("o-1")
        .await
        .into_iter()
        .map(|p| p.pet_id)
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["p-1", "p-2"]);
    assert_eq!(s.pets_of_owner("o-2").await.len(), 1);
    assert!(s.pets_of_owner("o-3").await.is_empty());
}

// --------------------------------------------------------------------------------------------
// credentials - owner filtering.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn credentials_of_owner_filters_by_owner() {
    let s = MemStore::new();
    s.put_credential(credential("cr-1", "o-1")).await;
    s.put_credential(credential("cr-2", "o-2")).await;
    assert_eq!(
        s.get_credential("cr-1").await.unwrap().credential_id,
        "cr-1"
    );
    assert!(s.get_credential("absent").await.is_none());
    assert_eq!(s.credentials_of_owner("o-1").await.len(), 1);
    assert!(s.credentials_of_owner("o-3").await.is_empty());
}

// --------------------------------------------------------------------------------------------
// jti - one-time consume (replay protection).
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn consume_jti_is_single_use_and_per_jti() {
    let s = MemStore::new();
    assert!(s.consume_jti("jti-a").await);
    assert!(!s.consume_jti("jti-a").await);
    assert!(s.consume_jti("jti-b").await);
    assert!(!s.consume_jti("jti-b").await);
}

// --------------------------------------------------------------------------------------------
// appointments - central is the sole monotonic rev allocator.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn alloc_rev_and_apply_creates_then_increments_rev() {
    let s = MemStore::new();
    // absent appointment with a creating closure -> rev starts at 1.
    let created = s
        .alloc_rev_and_apply(
            "a-1",
            Box::new(|cur, rev| {
                assert!(cur.is_none());
                let mut a = appt("a-1", "o-1", 100);
                a.rev = rev;
                Some(a)
            }),
        )
        .await
        .unwrap();
    assert_eq!(created.rev, 1);

    // a subsequent apply sees current rev=1 and allocates 2.
    let updated = s
        .alloc_rev_and_apply(
            "a-1",
            Box::new(|cur, rev| {
                let mut a = cur.expect("present");
                a.rev = rev;
                a.state = "CONFIRMED".to_string();
                Some(a)
            }),
        )
        .await
        .unwrap();
    assert_eq!(updated.rev, 2);
    assert_eq!(updated.state, "CONFIRMED");
    assert_eq!(s.get_appointment("a-1").await.unwrap().rev, 2);
}

#[tokio::test]
async fn alloc_rev_and_apply_aborts_without_persisting_when_closure_returns_none() {
    let s = MemStore::new();
    let out = s
        .alloc_rev_and_apply("a-x", Box::new(|_cur, _rev| None))
        .await;
    assert!(out.is_none());
    // nothing was persisted.
    assert!(s.get_appointment("a-x").await.is_none());
}

#[tokio::test]
async fn appointments_updated_since_is_owner_scoped_and_inclusive() {
    let s = MemStore::new();
    s.put_appointment(appt("a-1", "o-1", 100)).await;
    s.put_appointment(appt("a-2", "o-1", 200)).await;
    s.put_appointment(appt("a-3", "o-2", 300)).await;

    // inclusive lower bound at 200, owner-scoped to o-1.
    let mut got: Vec<String> = s
        .appointments_updated_since("o-1", 200)
        .await
        .into_iter()
        .map(|a| a.appointment_id)
        .collect();
    got.sort();
    assert_eq!(got, vec!["a-2"]);

    // since=0 returns all of the owner's appointments.
    assert_eq!(s.appointments_updated_since("o-1", 0).await.len(), 2);
    assert!(s.appointments_updated_since("o-2", 301).await.is_empty());
}

// --------------------------------------------------------------------------------------------
// consent / receipts / verification records - owner filtering.
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn consent_receipt_and_verification_lists_filter_by_owner() {
    let s = MemStore::new();
    s.put_consent(consent("c-1", "o-1")).await;
    s.put_consent(consent("c-2", "o-2")).await;
    s.put_consent_receipt(receipt("r-1", "o-1")).await;
    s.put_verification_record(vrecord("v-1", "o-1")).await;

    assert_eq!(s.get_consent("c-1").await.unwrap().consent_id, "c-1");
    assert_eq!(s.consents_of_owner("o-1").await.len(), 1);
    assert_eq!(s.receipts_of_owner("o-1").await.len(), 1);
    assert!(s.receipts_of_owner("o-2").await.is_empty());
    assert_eq!(s.verification_records_of_owner("o-1").await.len(), 1);
    assert!(s.verification_records_of_owner("o-2").await.is_empty());
}

// --------------------------------------------------------------------------------------------
// deletions - the due-window query (status pending AND due_by <= now).
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn due_deletions_returns_only_pending_at_or_before_now() {
    let s = MemStore::new();
    s.put_deletion(deletion("d-past", "o-1", 100, "pending"))
        .await;
    s.put_deletion(deletion("d-now", "o-2", 200, "pending"))
        .await;
    s.put_deletion(deletion("d-future", "o-3", 300, "pending"))
        .await;
    s.put_deletion(deletion("d-done", "o-4", 50, "completed"))
        .await;

    let mut due: Vec<String> = s
        .due_deletions(200)
        .await
        .into_iter()
        .map(|d| d.request_id)
        .collect();
    due.sort();
    // inclusive at now=200; the future one and the completed one are excluded.
    assert_eq!(due, vec!["d-now", "d-past"]);
}

#[tokio::test]
async fn update_deletion_overwrites_status() {
    let s = MemStore::new();
    s.put_deletion(deletion("d-1", "o-1", 100, "pending")).await;
    assert_eq!(s.due_deletions(100).await.len(), 1);
    s.update_deletion(deletion("d-1", "o-1", 100, "completed"))
        .await;
    // once completed it drops out of the due window.
    assert!(s.due_deletions(100).await.is_empty());
}

// --------------------------------------------------------------------------------------------
// erasure - row deletes and PII clears (DEK shredding is the caller's job).
// --------------------------------------------------------------------------------------------

#[tokio::test]
async fn erasure_deletes_rows_and_clears_sealed_pii() {
    let s = MemStore::new();
    s.put_credential(credential("cr-1", "o-1")).await;
    s.put_verification_record(vrecord("v-1", "o-1")).await;
    s.put_consent_receipt(receipt("r-1", "o-1")).await;
    s.put_owner(owner("o-1", Some("a@x.com"))).await;
    s.put_pet(pet("p-1", "o-1", "c1")).await;

    s.delete_credential("cr-1").await;
    s.delete_verification_record("v-1").await;
    s.delete_consent_receipt("r-1").await;
    assert!(s.get_credential("cr-1").await.is_none());
    assert!(s.verification_records_of_owner("o-1").await.is_empty());
    assert!(s.receipts_of_owner("o-1").await.is_empty());

    // clear_owner_pii nulls the sealed blob but keeps the row (the DEK is shredded separately).
    assert!(s.get_owner("o-1").await.unwrap().profile_pii.is_some());
    s.clear_owner_pii("o-1").await;
    let o = s.get_owner("o-1").await.unwrap();
    assert!(o.profile_pii.is_none());
    assert_eq!(o.owner_id, "o-1");

    // clear_pet_doc nulls the sealed doc but keeps the pet row.
    assert!(s.get_pet("p-1").await.unwrap().sealed_doc.is_some());
    s.clear_pet_doc("p-1").await;
    let p = s.get_pet("p-1").await.unwrap();
    assert!(p.sealed_doc.is_none());
    assert_eq!(p.pet_id, "p-1");

    // clearing an absent owner/pet is a silent no-op.
    s.clear_owner_pii("absent").await;
    s.clear_pet_doc("absent").await;
}
