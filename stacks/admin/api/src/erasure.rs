//! Right-to-erasure (impl §4.5 / §11.6 — crypto-shredding). `erase(ownerId, scope)` destroys the
//! per-record DEK behind every off-chain record in scope (so all ciphertext copies — DB, oplog, WAL,
//! backups, importer caches — become permanently undecryptable) AND deletes the row, INCLUDING
//! `verification_records` and consent receipts. The on-chain Verified tuple persists but is unlinkable.
//!
//! `fulfill_due_deletions(now)` runs `erase` for every pending deletion past its `dueBy` (cron/manual).

use crate::app::AppState;

/// Scopes a delete-request can target.
fn in_scope(scope: &str, kind: &str) -> bool {
    scope == "all" || scope == kind
}

/// Crypto-shred every off-chain record for `owner_id` in `scope`: destroy DEKs + delete rows.
/// Returns the count of (credentials, verification_records, receipts) shredded.
pub async fn erase(st: &AppState, owner_id: &str, scope: &str) -> (usize, usize, usize) {
    let mut creds = 0usize;
    let mut vers = 0usize;
    let mut receipts = 0usize;

    // credentials (salts/data) — shred DEK, delete row.
    if in_scope(scope, "credentials") {
        for c in st.store.credentials_of_owner(owner_id).await {
            st.vault.shred(&c.sealed_doc.dek_id).await;
            st.store.delete_credential(&c.credential_id).await;
            creds += 1;
        }
        // pet profile docs (the minted DOG_PROFILE wrapped doc).
        for p in st.store.pets_of_owner(owner_id).await {
            if let Some(s) = &p.sealed_doc {
                st.vault.shred(&s.dek_id).await;
                st.store.clear_pet_doc(&p.pet_id).await;
            }
        }
    }

    // verification_records (relayed consent copies) — DELETABLE; the on-chain tuple persists unlinkable.
    if in_scope(scope, "verifications") || in_scope(scope, "credentials") {
        for v in st.store.verification_records_of_owner(owner_id).await {
            st.vault.shred(&v.sealed.dek_id).await;
            st.store.delete_verification_record(&v.record_id).await;
            vers += 1;
        }
    }

    // consent receipts — off-chain, deletable.
    if in_scope(scope, "verifications") || in_scope(scope, "credentials") {
        for r in st.store.receipts_of_owner(owner_id).await {
            st.vault.shred(&r.sealed.dek_id).await;
            st.store.delete_consent_receipt(&r.receipt_id).await;
            receipts += 1;
        }
    }

    // owner PII — clear the sealed profile blob (DEK shredded above if present).
    if in_scope(scope, "all") {
        if let Some(o) = st.store.get_owner(owner_id).await {
            if let Some(pii) = &o.profile_pii {
                st.vault.shred(&pii.dek_id).await;
            }
        }
        st.store.clear_owner_pii(owner_id).await;
    }

    (creds, vers, receipts)
}

/// Run `erase` for every pending deletion request past its `dueBy`; mark completed. Returns the
/// count fulfilled.
pub async fn fulfill_due_deletions(st: &AppState, now: u64) -> usize {
    let due = st.store.due_deletions(now).await;
    let n = due.len();
    for mut d in due {
        erase(st, &d.owner_id, &d.scope).await;
        d.status = "completed".to_string();
        st.store.update_deletion(d).await;
    }
    n
}
