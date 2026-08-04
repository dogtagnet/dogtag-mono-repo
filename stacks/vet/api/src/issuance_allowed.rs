//! The clone's own issuance list, read as a ROSTER rather than as a history.
//!
//! `DogTagIssuer.issue` needs BOTH layers (`contracts/src/DogTagIssuer.sol:230-233`):
//!
//! ```text
//! registry.canIssue(address(this), msg.sender)   // the authority's scope-free grant + lifecycle
//! && issuanceAllowed[msg.sender]                 // THIS clone's own list
//! ```
//!
//! Layer 1 has a screen - the admin registrar surface. Layer 2 had none, in either direction: no
//! portal, no route, no client method, which is why a correctly registered provider with a correctly
//! attached contract could still not issue, and why the crew that walked `docs/DEMO_CLICKS.md` had to
//! send `setIssuanceAllowed` from a terminal to get past its own step 3.
//!
//! # THE LOG IS THE INDEX; STORAGE IS THE VALUE
//!
//! The one rule this module exists to enforce. `issuanceAllowed` is a `mapping(address => bool)` with
//! a per-address getter and no enumeration, so the only way to learn WHICH addresses to ask about is
//! the `IssuanceAllowedSet` log - and that log is complete, because `initialize` deliberately emits
//! the creation seed through the same event rather than a second topic
//! (`DogTagIssuer.sol:262-269`).
//!
//! But the log's own `allowed` word is never rendered. Every value comes from a fresh
//! `issuanceAllowed(address)` storage read. Three things follow, and each is a defect this repo has
//! paid for elsewhere:
//!
//! * **A pending transaction can never read as a completed grant.** A log the node volunteers before
//!   it is mined contributes at most a candidate address; the value still comes from storage, which
//!   says `false` until it lands.
//! * **Ordering stops mattering.** Folding an ordered log is what forces a `(blockNumber, logIndex)`
//!   sequence and makes an unpositioned log unanswerable. Reading storage sidesteps it: the chain has
//!   already folded the history for us, and it cannot disagree with itself.
//! * **A stale decoder cannot invent a grant.** The only thing taken from the log is an indexed
//!   address topic.
//!
//! Positions are still REQUIRED, and that is deliberate rather than vestigial - see
//! [`RosterRead::Unavailable`] and the note on `ever_named`.
//!
//! # Reading is not writing, and this backend does neither half of the write
//!
//! `setIssuanceAllowed` admits only from the clone's `owner()`; the protocol admin is excluded from
//! that direction on purpose, because it also writes the authority bit and holding both layers is
//! exactly the cross-provider issuance layer 2 exists to prevent. This backend holds no key that is
//! any clone's owner - its custody signer is the address that needs ADMITTING, which is the whole
//! gap - and it has no way to authenticate one either: an operator session proves "staff of this
//! shop", never "owner of this contract", and no signature-recovery path over an arbitrary address
//! survives in this crate.
//!
//! So there is no backend admit route and there must not be one. The write is a wallet transaction
//! from the owner's own key; this module answers the read the page is built on.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

/// One address this clone's list has an opinion about, with its CURRENT storage value.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RosterEntry {
    /// Lowercase `0x`-hex. Normalized so a caller comparing it against a connected wallet or against
    /// `owner()` cannot miss a match on EIP-55 casing alone.
    pub address: String,
    /// `issuanceAllowed(address)` as read from STORAGE, never from the log's own word.
    pub allowed: bool,
    /// Whether a MINED `IssuanceAllowedSet` has ever named this address.
    ///
    /// This is what separates the two ways of being unable to issue, which a bare `allowed: false`
    /// spells identically: `ever_named` means the list held this address and it was WITHDRAWN,
    /// while `!ever_named` means it was never admitted at all. Different facts, different remedies,
    /// and the task this module answers names misreading exactly this distinction as the failure to
    /// avoid - so it is carried as data rather than left to styling.
    #[serde(rename = "everNamed")]
    pub ever_named: bool,
}

/// The result of reading one clone's issuance list.
///
/// Two members, and collapsing them is the defect this type exists to prevent: an empty `Resolved`
/// says this clone admits nobody, while `Unavailable` says we could not ask. Rendering the second as
/// the first tells a provider that nobody may issue in their name on the strength of a read that
/// never happened - and this page is where they decide who signs medical records.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum RosterRead {
    Resolved {
        /// The clone's `owner()`, lowercase. The only address that may ADMIT.
        owner: String,
        entries: Vec<RosterEntry>,
        /// Whether THIS deployment's own custody signer may currently anchor through this contract.
        ///
        /// `None` only when there is no active signer to ask about (custody locked, or no seal), so a
        /// consumer must branch rather than reading `Some(false)` into a locked backend. It is
        /// derived from the same storage reads as `entries`, so it can never disagree with the row
        /// rendered beside it.
        #[serde(rename = "activeSignerAllowed")]
        active_signer_allowed: Option<bool>,
    },
    Unavailable {
        reason: String,
    },
}

impl RosterRead {
    /// The entries when the read resolved. `None` when it did not - callers must branch rather than
    /// defaulting to empty.
    pub fn entries(&self) -> Option<&[RosterEntry]> {
        match self {
            RosterRead::Resolved { entries, .. } => Some(entries),
            RosterRead::Unavailable { .. } => None,
        }
    }

    /// Whether `address` may currently anchor, as a tri-state: `Some(..)` when the list resolved,
    /// `None` when it did not.
    ///
    /// An address the resolved roster does not mention is a definite `false` - the read succeeded and
    /// the chain holds no entry for it, which IS an answer.
    pub fn allowed(&self, address: &str) -> Option<bool> {
        let entries = self.entries()?;
        let want = normalize_addr(address);
        Some(
            entries
                .iter()
                .find(|e| e.address == want)
                .map(|e| e.allowed)
                .unwrap_or(false),
        )
    }
}

/// What the chain layer gathers for one clone before it is shaped into a [`RosterRead`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuanceRoster {
    /// `owner()`, lowercase.
    pub owner: String,
    pub entries: Vec<RosterEntry>,
}

/// Lowercase a `0x`-hex address so every comparison in this module is a plain string equality.
///
/// Deliberately does NOT validate: a malformed address that reached a log topic is still the thing
/// the chain recorded, and silently dropping it would hide an entry that really is on the list.
pub fn normalize_addr(a: &str) -> String {
    a.trim().to_ascii_lowercase()
}

/// Build the roster from the addresses a MINED log named and the storage values read for each.
///
/// `named` is every address an `IssuanceAllowedSet` topic carried, in any order and with repeats -
/// order is irrelevant here precisely because no value is taken from the log.
///
/// `also` is an extra address to include even when the log never named it: this deployment's own
/// custody signer, so the one address the page exists to get admitted always has a row to point at
/// instead of being invisible until someone has already admitted it.
///
/// `allowed` maps a normalized address to its storage value. An address with no entry is treated as
/// `false`, which is only reachable when the caller failed to read it - and the chain layer returns
/// `Err` in that case rather than calling here, so this is a defensive floor, not a silent default.
pub fn build_roster(
    named: &[String],
    also: Option<&str>,
    allowed: &BTreeMap<String, bool>,
) -> Vec<RosterEntry> {
    let named_set: HashSet<String> = named.iter().map(|a| normalize_addr(a)).collect();
    // BTreeMap keeps the output deterministic (fixed-width hex sorts stably), so two reads of the
    // same chain state render in the same order rather than in whatever order the node returned.
    let mut rows: BTreeMap<String, RosterEntry> = BTreeMap::new();
    for address in named_set.iter().cloned().chain(also.map(normalize_addr)) {
        let ever_named = named_set.contains(&address);
        let allowed = allowed.get(&address).copied().unwrap_or(false);
        rows.insert(
            address.clone(),
            RosterEntry {
                address,
                allowed,
                ever_named,
            },
        );
    }
    rows.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(n: u8) -> String {
        format!("0x{:040x}", n)
    }

    fn values(pairs: &[(&str, bool)]) -> BTreeMap<String, bool> {
        pairs
            .iter()
            .map(|(a, v)| (normalize_addr(a), *v))
            .collect()
    }

    /// THE headline rule. The log says WHO to ask about; storage says what the answer is. A log that
    /// recorded an admit whose storage now reads `false` is a WITHDRAWN entry, not a current one.
    #[test]
    fn the_value_comes_from_storage_and_never_from_the_log() {
        let rows = build_roster(
            &[addr(1), addr(2)],
            None,
            &values(&[(&addr(1), true), (&addr(2), false)]),
        );
        assert_eq!(rows.len(), 2);
        assert!(rows[0].allowed, "the admitted address lost its grant");
        assert!(
            !rows[1].allowed,
            "an address whose storage reads false was rendered as still admitted"
        );
    }

    /// Withdrawn and never-admitted are DIFFERENT facts, and `allowed: false` spells them the same.
    /// This is the distinction the guide-walk crew misread on the admin page; carrying it as data is
    /// what lets a test - and a screen reader, and a flattened text dump - tell them apart.
    #[test]
    fn a_withdrawn_entry_is_distinguishable_from_one_that_was_never_admitted() {
        let ours = addr(9);
        let rows = build_roster(&[addr(1)], Some(&ours), &values(&[(&addr(1), false)]));

        let withdrawn = rows.iter().find(|r| r.address == addr(1)).unwrap();
        assert!(!withdrawn.allowed);
        assert!(
            withdrawn.ever_named,
            "a withdrawn entry must record that the list once held it"
        );

        let never = rows.iter().find(|r| r.address == ours).unwrap();
        assert!(!never.allowed);
        assert!(
            !never.ever_named,
            "an address the log never named was reported as withdrawn"
        );
    }

    /// The one address the page exists to get admitted must have a row BEFORE anyone admits it -
    /// otherwise the backend's own signer is invisible in exactly the state the provider is trying
    /// to diagnose.
    #[test]
    fn our_own_signer_always_has_a_row_even_before_it_is_admitted() {
        let ours = addr(7);
        let rows = build_roster(&[addr(1)], Some(&ours), &values(&[(&addr(1), true)]));
        assert!(
            rows.iter().any(|r| r.address == ours && !r.allowed),
            "the deployment's own signer had no row to point at"
        );
    }

    /// It must not be DUPLICATED once it is admitted, either.
    #[test]
    fn our_own_signer_is_one_row_not_two_once_the_log_names_it() {
        let ours = addr(7);
        let rows = build_roster(
            &[addr(1), ours.clone()],
            Some(&ours),
            &values(&[(&addr(1), true), (&ours, true)]),
        );
        assert_eq!(rows.iter().filter(|r| r.address == ours).count(), 1);
        let row = rows.iter().find(|r| r.address == ours).unwrap();
        assert!(row.allowed && row.ever_named);
    }

    /// Repeats and EIP-55 casing are the same address. A log carrying several writes for one signer
    /// is the normal case (admit, withdraw, re-admit), and rendering it three times would read as
    /// three staff keys.
    #[test]
    fn repeated_and_differently_cased_writes_collapse_to_one_row() {
        let mixed = "0xAbCdEf0000000000000000000000000000000001";
        let rows = build_roster(
            &[mixed.to_string(), mixed.to_lowercase(), mixed.to_string()],
            None,
            &values(&[(mixed, true)]),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].address, mixed.to_lowercase());
    }

    /// Deterministic order, so two reads of one chain state render identically.
    #[test]
    fn the_order_is_deterministic_regardless_of_the_order_the_node_returned() {
        let a = build_roster(&[addr(3), addr(1), addr(2)], None, &values(&[]));
        let b = build_roster(&[addr(2), addr(3), addr(1)], None, &values(&[]));
        assert_eq!(a, b);
        assert_eq!(
            a.iter().map(|r| r.address.clone()).collect::<Vec<_>>(),
            vec![addr(1), addr(2), addr(3)]
        );
    }

    /// `Unavailable` carries NO entries field at all, so it cannot be spread into a list as `[]`.
    /// The accessor forces a caller to branch.
    #[test]
    fn an_unavailable_read_yields_no_entries_and_no_verdict() {
        let unavailable = RosterRead::Unavailable {
            reason: "rpc error".into(),
        };
        assert!(unavailable.entries().is_none());
        assert!(
            unavailable.allowed(&addr(1)).is_none(),
            "a read that never happened answered a question about an address"
        );
    }

    /// An EMPTY resolved read is an ANSWER, and a different one: the chain was asked and holds no
    /// entry. That is a definite `false`, never a `None`.
    #[test]
    fn an_empty_resolved_read_is_a_definite_answer_not_an_absent_one() {
        let empty = RosterRead::Resolved {
            owner: addr(1),
            entries: vec![],
            active_signer_allowed: Some(false),
        };
        assert_eq!(empty.entries().map(|e| e.len()), Some(0));
        assert_eq!(empty.allowed(&addr(2)), Some(false));
    }

    /// The accessor matches regardless of casing, so a caller holding a checksummed address from a
    /// wallet is not told the chain has no opinion about it.
    #[test]
    fn the_allowed_accessor_is_case_insensitive() {
        let mixed = "0xAbCdEf0000000000000000000000000000000001";
        let read = RosterRead::Resolved {
            owner: addr(1),
            entries: build_roster(&[mixed.to_string()], None, &values(&[(mixed, true)])),
            active_signer_allowed: None,
        };
        assert_eq!(read.allowed(mixed), Some(true));
        assert_eq!(read.allowed(&mixed.to_lowercase()), Some(true));
    }
}
