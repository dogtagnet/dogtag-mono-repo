//! The microchip cross-check: does the credential being attached describe THIS animal?
//!
//! # What this is for
//!
//! A DogTag is linked to a pet by an operator typing (or scanning) a tag id. Nothing about that act
//! is self-checking: a mistyped digit, a mixed-up pair of cage cards, or two similar dogs in the same
//! afternoon all produce a link that is structurally perfect and about the wrong animal. Until now
//! the only thing standing between a correct link and a wrong one was the operator remembering.
//!
//! A credential already carries `credentialSubject.microchip.code` as a salted Merkle leaf, so it is
//! covered by `signature.merkleRoot` and cannot be edited without breaking integrity. Recording the
//! same code on the shop's own [`ClientPet`](crate::store::ClientPet) gives the two sides something
//! to compare, which turns operator memory into evidence.
//!
//! This module is the DECISION and nothing else: no store, no HTTP, no chain. It is pure so every
//! branch below — including the ones that are awkward to reach through a route — is unit-testable.
//!
//! # Absent is NORMAL
//!
//! **Many animals have no microchip at all.** Cats routinely are not chipped in Singapore. So an
//! absent code is a first-class ordinary state, never a failure and never a reason to refuse a link.
//! The check fires ONLY when both sides carry a code.
//!
//! # Three states, and the middle one is the whole point
//!
//! [`MicrochipCheck`] is `Matched` / `Mismatch` / `NotComparable`, and `NotComparable` must never
//! render as either neighbour. This codebase's defining defect class is a check that did not run
//! being reported as one that passed; the inverse — reporting it as a failure — is just as wrong and
//! is the one this particular feature would get wrong, because refusing every unchipped cat would
//! make the field unusable. So `NotComparable` carries a [`NotComparable`] reason that says which
//! side was missing, or which read did not resolve, and the caller renders that sentence.
//!
//! # Facts and failures are rendered differently
//!
//! The reasons split into two kinds and [`NotComparable::is_failure`] is what tells them apart:
//!
//!   * **Facts** — nothing to establish. "This pet has no microchip on file", "this credential
//!     carries no microchip", "this shop holds no credential for that tag". These are neutral,
//!     ordinary, and get plain neutral wording. Deliberately NOT the warning treatment the
//!     issuer-whitelist pillar's `unresolved` state gets: that rule is for failing to establish
//!     something we needed, and an unchipped cat is not a failure to establish anything. Painting it
//!     amber would over-claim in the other direction and nag the commonest case in the product.
//!   * **Failures** — we needed to look and could not. An unreadable store, a credential whose leaf
//!     will not parse, or a tag stored in a form this shop cannot look a credential up under. These
//!     DO get the warning treatment, because something that should have resolved did not.
//!
//! # What it deliberately does not do
//!
//! * **A mismatch is not an invalid credential.** The credential is genuine; the LINK is wrong. Same
//!   rule that keeps `issuer_mismatch` apart from `issuer_not_whitelisted` in
//!   [`crate::verify`] — different accusations, different remedies. So a mismatch refuses the
//!   *write* and never touches a verdict.
//! * **It never copies the credential's code onto the pet.** That would make the check permanently
//!   vacuous (it would forever match what it copied) and would assert an identity no human confirmed.
//! * **It does not validate the shop's code format.** See [`ClientPet::microchip_code`] — a legacy
//!   pre-ISO code that differs from the credential's is a genuine mismatch, and the refusal names
//!   both values so the operator can see it is a format difference and correct their own record.
//!
//! [`ClientPet::microchip_code`]: crate::store::ClientPet::microchip_code

use dogtag_standard::wrap::{flatten_data, parse_packed, WrappedDoc};
use serde_json::{json, Value};

/// The credential leaf this check reads. A salted Merkle leaf, so its value is covered by
/// `signature.merkleRoot` — this module CONSUMES that coverage and never redefines it.
pub const MICROCHIP_KEY_PATH: &str = "credentialSubject.microchip.code";

// ------------------------------------------------------------------------------------------------
// the credential side
// ------------------------------------------------------------------------------------------------

/// What the credential could tell us about the animal's microchip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialMicrochip {
    /// The leaf is present and readable.
    Present(String),
    /// The credential carries no readable microchip leaf.
    ///
    /// `withheld_leaves` is `privacy.obfuscated.len()`. It is reported because the two ways to reach
    /// this state are indistinguishable from here and pretending otherwise would be a lie: a holder
    /// may have withheld the leaf under selective disclosure, in which case only its Poseidon image
    /// survives in `privacy.obfuscated`, and recovering the key path from that image would need both
    /// the salt and the value — neither of which we have. So the count is offered as context ("some
    /// fields were withheld") rather than a claim about WHICH field.
    Absent { withheld_leaves: usize },
    /// The leaf is present but does not parse as a packed scalar, or holds an empty value.
    Unreadable(String),
}

/// Read the microchip leaf out of a wrapped credential.
///
/// Reads `data`, which is what `check_integrity` folds — never `issuer` or any other block outside
/// the Merkle root, where an attacker-supplied value would be uncovered.
pub fn credential_microchip(doc: &WrappedDoc) -> CredentialMicrochip {
    let withheld_leaves = doc.privacy.obfuscated.len();
    let Some((_, packed)) = flatten_data(&doc.data)
        .into_iter()
        .find(|(kp, _)| kp == MICROCHIP_KEY_PATH)
    else {
        return CredentialMicrochip::Absent { withheld_leaves };
    };
    match parse_packed(&packed) {
        Ok((_, _, value)) => {
            let v = value.trim();
            if v.is_empty() {
                // A leaf that is present and blank is not the same as no leaf: the issuer wrote
                // something we cannot compare, which is a failure to read rather than an absence.
                CredentialMicrochip::Unreadable(
                    "the credential's microchip leaf is empty".to_string(),
                )
            } else {
                CredentialMicrochip::Present(v.to_string())
            }
        }
        Err(e) => CredentialMicrochip::Unreadable(format!(
            "the credential's microchip leaf could not be decoded: {e}"
        )),
    }
}

// ------------------------------------------------------------------------------------------------
// the decision
// ------------------------------------------------------------------------------------------------

/// Why the two sides could not be compared. Never a pass, never a refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotComparable {
    /// This shop holds no credential filed under that tag. A FACT about the shop's own records.
    NoCredentialHeld,
    /// The credential carries no readable microchip. A FACT about the credential.
    CredentialHasNoMicrochip { withheld_leaves: usize },
    /// The shop's own record has no microchip for this pet. A FACT, and the ordinary state of an
    /// unchipped animal.
    PetHasNoMicrochip,
    /// No pet on file is linked to this tag, so there is no local record to compare against. A FACT.
    ///
    /// Reachable only from the IMPORT direction, where the credential arrives before any link
    /// exists. Kept apart from [`Self::PetHasNoMicrochip`] because that one says "this pet has no
    /// microchip on file", and saying that about a tag no pet is linked to would name an animal that
    /// is not there.
    NoLinkedPet,
    /// The pet is linked by the full on-chain FIELD ELEMENT, and the held-credential cache is keyed
    /// by the short handle. Handle -> field element is a Poseidon hash and cannot be inverted, so
    /// there is no handle to look under.
    ///
    /// A FAILURE, not an absence — it is a lookup this shop cannot perform, and reporting it as
    /// "holds no credential" would be exactly the understatement `PetTagCredentials` already refuses
    /// to make about the same cache.
    CannotLookUpByFieldElement,
    /// The held-credential read did not resolve. A FAILURE.
    CouldNotRead(String),
    /// The credential's leaf is present but unreadable. A FAILURE.
    CredentialUnreadable(String),
}

impl NotComparable {
    /// Whether this is a failure to look (`true`) or an ordinary fact about what exists (`false`).
    ///
    /// The renderer keys its tone off this, so that an unchipped cat reads neutral while an
    /// unreadable store reads as the warning it is. Both still refuse to be shown as a pass.
    pub fn is_failure(&self) -> bool {
        match self {
            Self::NoCredentialHeld
            | Self::CredentialHasNoMicrochip { .. }
            | Self::PetHasNoMicrochip
            | Self::NoLinkedPet => false,
            Self::CannotLookUpByFieldElement
            | Self::CouldNotRead(_)
            | Self::CredentialUnreadable(_) => true,
        }
    }

    /// The stable wire token. Kept apart from the human sentence so a client can branch on the
    /// reason without parsing prose.
    pub fn wire(&self) -> &'static str {
        match self {
            Self::NoCredentialHeld => "noCredentialHeld",
            Self::CredentialHasNoMicrochip { .. } => "credentialHasNoMicrochip",
            Self::PetHasNoMicrochip => "petHasNoMicrochip",
            Self::NoLinkedPet => "noLinkedPet",
            Self::CannotLookUpByFieldElement => "cannotLookUpByFieldElement",
            Self::CouldNotRead(_) => "couldNotRead",
            Self::CredentialUnreadable(_) => "credentialUnreadable",
        }
    }

    /// The sentence shown to the operator. Every one of them says plainly that the two sides could
    /// not be compared and why — never "ok", never "failed".
    pub fn detail(&self) -> String {
        match self {
            Self::NoCredentialHeld => {
                "This shop holds no credential for that DogTag, so there is nothing to compare the \
                 microchip against."
                    .to_string()
            }
            Self::CredentialHasNoMicrochip { withheld_leaves } if *withheld_leaves > 0 => format!(
                "The credential carries no readable microchip, so the two could not be compared. \
                 The holder withheld {withheld_leaves} field(s) from this copy; which ones cannot \
                 be determined from the document."
            ),
            Self::CredentialHasNoMicrochip { .. } => {
                "The credential carries no microchip, so the two could not be compared.".to_string()
            }
            Self::PetHasNoMicrochip => {
                "This pet has no microchip on file, so the two could not be compared. Many animals \
                 are not chipped; record one here if this pet has it."
                    .to_string()
            }
            Self::NoLinkedPet => {
                "No pet on file is linked to that DogTag, so there is nothing to compare the \
                 microchip against yet. It will be checked when the tag is linked to a pet."
                    .to_string()
            }
            Self::CannotLookUpByFieldElement => {
                "This pet is linked by the full on-chain id, and held credentials are filed under \
                 the short tag handle, so this shop cannot look one up to compare. That is a lookup \
                 it cannot perform, not evidence it holds nothing."
                    .to_string()
            }
            Self::CouldNotRead(e) => format!(
                "The held credential could not be read, so the microchip was not compared: {e}"
            ),
            Self::CredentialUnreadable(e) => {
                format!("The microchip could not be compared: {e}")
            }
        }
    }
}

/// The verdict of the cross-check between a credential and the shop's own record of a pet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MicrochipCheck {
    /// Both sides carry a code and they are the same. The only state that is positive evidence the
    /// tag and the animal belong together.
    Matched(String),
    /// Both sides carry a code and they DIFFER. Refused.
    Mismatch { pet: String, credential: String },
    /// The check did not run. See [`NotComparable`].
    NotComparable(NotComparable),
}

impl MicrochipCheck {
    /// Whether a write carrying this verdict must be refused. ONLY a mismatch refuses; every
    /// `NotComparable` state passes through, because a check that could not run is not a failure of
    /// the thing being checked.
    pub fn refuses(&self) -> bool {
        matches!(self, Self::Mismatch { .. })
    }

    pub fn wire_state(&self) -> &'static str {
        match self {
            Self::Matched(_) => "matched",
            Self::Mismatch { .. } => "mismatch",
            Self::NotComparable(_) => "notComparable",
        }
    }

    /// The operator-facing sentence for the refusal, naming BOTH values.
    ///
    /// Naming both is what tells a typo from a genuinely wrong animal, and it is also what lets an
    /// operator recognise a legacy non-ISO value in their own record — the same reason
    /// `dog_tag_conflict` names the pet already holding a tag.
    pub fn refusal_message(&self) -> Option<String> {
        match self {
            Self::Mismatch { pet, credential } => Some(format!(
                "Microchip mismatch: this pet's record says {pet}, the credential says {credential}. \
                 The credential is not refused and stays valid — it describes a different animal to \
                 the one on this record. Check the DogTag id and this pet's microchip before linking."
            )),
            _ => None,
        }
    }

    /// The wire projection. Emitted on the binding routes' responses and inside every refusal body,
    /// so a client has ONE vocabulary rather than two.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Matched(code) => json!({
                "state": "matched",
                "microchip": code,
                "detail": "The credential's microchip matches this pet's record.",
            }),
            Self::Mismatch { pet, credential } => json!({
                "state": "mismatch",
                "petMicrochip": pet,
                "credentialMicrochip": credential,
                "detail": self.refusal_message(),
            }),
            Self::NotComparable(r) => json!({
                "state": "notComparable",
                "reason": r.wire(),
                // The renderer's tone comes from HERE rather than from the client re-deriving it
                // from `reason`, so a reason added later cannot silently default to the wrong one.
                "isFailure": r.is_failure(),
                "detail": r.detail(),
            }),
        }
    }
}

/// Compare the shop's own record against the credential.
///
/// `pet` is the raw stored value; blank and whitespace-only are treated as absent, because a field
/// the operator tabbed through is not a microchip.
///
/// ORDER MATTERS when more than one side is missing, and it is chosen by which remedy the operator
/// can act on. The credential is the immovable side — an issuer-signed document this shop cannot
/// edit — so when the credential carries no microchip that is reported first: no amount of data
/// entry on the pet record will ever make this comparison possible, and telling the operator to go
/// find the chip number would be sending them on an errand that cannot succeed. Only once the
/// credential is known to carry one does a missing pet-side code become the actionable answer.
pub fn compare(pet: Option<&str>, credential: &CredentialMicrochip) -> MicrochipCheck {
    let pet_code = pet.map(str::trim).filter(|s| !s.is_empty());
    let credential_code = match credential {
        CredentialMicrochip::Unreadable(e) => {
            return MicrochipCheck::NotComparable(NotComparable::CredentialUnreadable(e.clone()))
        }
        CredentialMicrochip::Absent { withheld_leaves } => {
            return MicrochipCheck::NotComparable(NotComparable::CredentialHasNoMicrochip {
                withheld_leaves: *withheld_leaves,
            })
        }
        CredentialMicrochip::Present(c) => c,
    };
    let Some(pet_code) = pet_code else {
        return MicrochipCheck::NotComparable(NotComparable::PetHasNoMicrochip);
    };
    if pet_code == credential_code {
        MicrochipCheck::Matched(pet_code.to_string())
    } else {
        MicrochipCheck::Mismatch {
            pet: pet_code.to_string(),
            credential: credential_code.clone(),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// the held-credential lookup
// ------------------------------------------------------------------------------------------------

/// Whether a stored `dogTagId` can be used to look a held credential up.
///
/// The held-document cache is keyed by the tag HANDLE (`POST /import/pull` files each document under
/// its own `credentialSubject.dogTagId` leaf), while the link route deliberately also accepts the
/// full on-chain field element so a tag copied from an explorer still resolves. This mirrors
/// `resolveDogTagId` in `packages/ui/src/chain/tagDiscovery.ts`, whose `form` field draws the same
/// line for the same cache — keep the two in step.
///
/// A decimal string longer than this many significant digits is a field element rather than a
/// handle: handles come from `Store::next_dog_tag_id` and are small, while a BN254 field element is
/// up to 77 digits.
const MAX_HANDLE_DIGITS: usize = 20;

/// `true` when `tag` is the short handle the held-credential cache is keyed by.
pub fn is_handle_form(tag: &str) -> bool {
    let s = tag.trim();
    if s.is_empty() || s.starts_with("0x") || s.starts_with("0X") {
        return false;
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        // Not a usable id at all. Treated as a non-handle so the check reports that it could not
        // look one up rather than reporting an absence.
        return false;
    }
    s.trim_start_matches('0').len() <= MAX_HANDLE_DIGITS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built by DESERIALIZING the wire shape rather than by struct literal, so these fixtures stay
    /// valid when the SDK's envelope gains a field — and so they exercise the same `serde` path a
    /// document actually arrives through.
    fn doc(data: Value, obfuscated: Vec<String>) -> WrappedDoc {
        serde_json::from_value(json!({
            "version": "dogtag/1.0",
            "data": data,
            "signature": {
                "type": "MerkleRoot",
                "targetHash": "0x00",
                "proof": [],
                "merkleRoot": "0x00",
            },
            "privacy": { "obfuscated": obfuscated },
            "issuer": {
                "name": "Seaport Vet",
                "domain": "vet.example",
                "documentStore": "0x0000000000000000000000000000000000000000",
                "recordType": "VACCINATION",
            },
        }))
        .expect("fixture is a valid WrappedDoc")
    }

    fn chipped(code: &str) -> WrappedDoc {
        doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "microchip": { "code": format!("bbbb:5:{code}") }
                }
            }),
            vec![],
        )
    }

    // ---- the credential side ----

    #[test]
    fn reads_the_microchip_leaf_out_of_the_packed_scalar() {
        assert_eq!(
            credential_microchip(&chipped("985141006580319")),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    #[test]
    fn a_value_containing_colons_survives_the_packed_split() {
        // `parse_packed` splits on the FIRST TWO colons only; a value must never be truncated at a
        // third, or a comparison would be made against a prefix of the real code.
        let d = doc(
            json!({ "credentialSubject": { "microchip": { "code": "bbbb:5:98:51:41" } } }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("98:51:41".into())
        );
    }

    #[test]
    fn an_absent_leaf_reports_the_withheld_count_it_cannot_attribute() {
        let d = doc(
            json!({ "credentialSubject": { "dogTagId": "aaaa:2:4" } }),
            vec!["0xdead".into(), "0xbeef".into()],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Absent { withheld_leaves: 2 }
        );
    }

    #[test]
    fn an_unparseable_leaf_is_unreadable_not_absent() {
        let d = doc(
            json!({ "credentialSubject": { "microchip": { "code": "no-colons-here" } } }),
            vec![],
        );
        assert!(matches!(
            credential_microchip(&d),
            CredentialMicrochip::Unreadable(_)
        ));
    }

    #[test]
    fn a_present_but_blank_leaf_is_unreadable_not_absent() {
        // Absent means the issuer wrote nothing; blank means it wrote something we cannot compare.
        // Collapsing the two would render a broken document as an ordinary unchipped animal.
        let d = doc(
            json!({ "credentialSubject": { "microchip": { "code": "bbbb:5:   " } } }),
            vec![],
        );
        assert!(matches!(
            credential_microchip(&d),
            CredentialMicrochip::Unreadable(_)
        ));
    }

    // ---- the decision ----

    #[test]
    fn equal_codes_match() {
        let c = CredentialMicrochip::Present("985141006580319".into());
        assert_eq!(
            compare(Some("985141006580319"), &c),
            MicrochipCheck::Matched("985141006580319".into())
        );
    }

    #[test]
    fn surrounding_whitespace_is_not_a_mismatch() {
        let c = CredentialMicrochip::Present("985141006580319".into());
        assert_eq!(
            compare(Some("  985141006580319 "), &c),
            MicrochipCheck::Matched("985141006580319".into())
        );
    }

    #[test]
    fn differing_codes_mismatch_and_the_message_names_both() {
        let c = CredentialMicrochip::Present("985141006580319".into());
        let v = compare(Some("900000000000001"), &c);
        assert!(v.refuses());
        let msg = v.refusal_message().expect("a mismatch has a message");
        assert!(msg.contains("900000000000001"), "{msg}");
        assert!(msg.contains("985141006580319"), "{msg}");
    }

    #[test]
    fn a_legacy_short_code_is_a_mismatch_and_both_values_are_named() {
        // A pre-ISO shop value is NOT given a state of its own: it differs, so it is a mismatch. The
        // message names both so the operator can see it is a format difference in their own record.
        let c = CredentialMicrochip::Present("985141006580319".into());
        let v = compare(Some("1234567"), &c);
        assert!(v.refuses());
        assert!(v.refusal_message().unwrap().contains("1234567"));
    }

    #[test]
    fn an_unchipped_pet_is_not_comparable_and_never_refused() {
        let c = CredentialMicrochip::Present("985141006580319".into());
        let v = compare(None, &c);
        assert_eq!(
            v,
            MicrochipCheck::NotComparable(NotComparable::PetHasNoMicrochip)
        );
        assert!(!v.refuses(), "an unchipped pet must never block a link");
    }

    #[test]
    fn a_blank_pet_code_is_absent_not_a_mismatch() {
        let c = CredentialMicrochip::Present("985141006580319".into());
        assert_eq!(
            compare(Some("   "), &c),
            MicrochipCheck::NotComparable(NotComparable::PetHasNoMicrochip)
        );
    }

    #[test]
    fn a_credential_without_a_microchip_is_not_comparable_and_never_refused() {
        let c = CredentialMicrochip::Absent {
            withheld_leaves: 0,
        };
        let v = compare(Some("985141006580319"), &c);
        assert_eq!(
            v,
            MicrochipCheck::NotComparable(NotComparable::CredentialHasNoMicrochip {
                withheld_leaves: 0
            })
        );
        assert!(!v.refuses());
    }

    #[test]
    fn when_neither_side_has_one_the_immovable_side_is_reported() {
        // Both are missing and both statements are true, so the choice is which remedy to offer.
        // Naming the credential tells the operator not to go looking for a chip number, because no
        // entry on the pet record can make this comparison possible.
        let v = compare(None, &CredentialMicrochip::Absent { withheld_leaves: 0 });
        assert_eq!(
            v,
            MicrochipCheck::NotComparable(NotComparable::CredentialHasNoMicrochip {
                withheld_leaves: 0
            })
        );
        assert!(!v.refuses());
    }

    #[test]
    fn an_unreadable_leaf_outranks_a_missing_pet_code() {
        // Failure-to-read must not be reported as the ordinary "this pet has no chip" fact: the
        // tones differ, and a broken document deserves the warning.
        let v = compare(None, &CredentialMicrochip::Unreadable("boom".into()));
        assert!(matches!(
            v,
            MicrochipCheck::NotComparable(NotComparable::CredentialUnreadable(_))
        ));
        assert!(!v.refuses());
    }

    // ---- fact vs failure ----

    #[test]
    fn ordinary_absences_are_facts_and_unresolved_reads_are_failures() {
        // The tone split. Getting this backwards paints every unchipped cat amber (nagging the
        // commonest state in the product) or paints an unreadable store neutral (hiding a real one).
        assert!(!NotComparable::PetHasNoMicrochip.is_failure());
        assert!(!NotComparable::CredentialHasNoMicrochip { withheld_leaves: 0 }.is_failure());
        assert!(!NotComparable::NoCredentialHeld.is_failure());
        assert!(NotComparable::CannotLookUpByFieldElement.is_failure());
        assert!(NotComparable::CouldNotRead("x".into()).is_failure());
        assert!(NotComparable::CredentialUnreadable("x".into()).is_failure());
    }

    /// Compile-time exhaustiveness pin for the list in the test below.
    ///
    /// A test that walks a HAND-WRITTEN list of variants silently stops covering the enum the moment
    /// someone adds one — the list still passes, so nothing says the new reason was never checked.
    /// This match makes that a build error instead: adding a variant breaks HERE, and this comment
    /// is what says to add it to `ALL_REASONS` too.
    #[allow(dead_code)]
    fn exhaustiveness_guard(r: &NotComparable) {
        match r {
            NotComparable::NoCredentialHeld
            | NotComparable::CredentialHasNoMicrochip { .. }
            | NotComparable::PetHasNoMicrochip
            | NotComparable::NoLinkedPet
            | NotComparable::CannotLookUpByFieldElement
            | NotComparable::CouldNotRead(_)
            | NotComparable::CredentialUnreadable(_) => {}
        }
    }

    fn all_reasons() -> Vec<NotComparable> {
        vec![
            NotComparable::NoCredentialHeld,
            NotComparable::CredentialHasNoMicrochip { withheld_leaves: 0 },
            NotComparable::CredentialHasNoMicrochip { withheld_leaves: 3 },
            NotComparable::PetHasNoMicrochip,
            NotComparable::NoLinkedPet,
            NotComparable::CannotLookUpByFieldElement,
            NotComparable::CouldNotRead("connection reset".into()),
            NotComparable::CredentialUnreadable("bad leaf".into()),
        ]
    }

    #[test]
    fn every_not_comparable_reason_says_it_could_not_compare() {
        // The task's rule, asserted over the whole enum rather than a sample, so a reason added
        // later cannot ship with wording that reads as a pass or as a failure of the credential.
        for r in all_reasons() {
            let d = r.detail().to_lowercase();
            assert!(
                d.contains("not compared")
                    || d.contains("not be compared")
                    || d.contains("nothing to compare")
                    || d.contains("cannot look one up to compare"),
                "reason {} does not say the two could not be compared: {d}",
                r.wire()
            );
            // ...and never claims the credential is bad. A `notComparable` that reads as an
            // accusation is the same defect as one that reads as a pass, pointed the other way.
            for forbidden in ["invalid", "forged", "failed", "does not match"] {
                assert!(
                    !d.contains(forbidden),
                    "reason {} reads as a verdict against the credential ({forbidden:?}): {d}",
                    r.wire()
                );
            }
            let v = MicrochipCheck::NotComparable(r.clone());
            assert!(!v.refuses(), "{} must not refuse", r.wire());
            assert_eq!(v.wire_state(), "notComparable");
            assert_eq!(v.to_json()["isFailure"], json!(r.is_failure()));
        }
    }

    #[test]
    fn the_withheld_count_is_offered_as_context_not_as_an_attribution() {
        let d = NotComparable::CredentialHasNoMicrochip { withheld_leaves: 3 }.detail();
        assert!(d.contains('3'), "{d}");
        assert!(
            d.contains("cannot be determined"),
            "the count must not be presented as identifying WHICH field: {d}"
        );
    }

    // ---- lookup form ----

    #[test]
    fn a_short_decimal_is_the_handle_the_cache_is_keyed_by() {
        assert!(is_handle_form("4"));
        assert!(is_handle_form(" 12345 "));
        assert!(is_handle_form("00000000000000000004"));
    }

    #[test]
    fn a_field_element_is_not_a_handle_so_the_cache_cannot_be_asked() {
        assert!(!is_handle_form(
            "0x1736d1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e"
        ));
        assert!(!is_handle_form(
            "1195241908933892557940129631300775214454584041594363078565480038450625444405"
        ));
        assert!(!is_handle_form("not-an-id"));
        assert!(!is_handle_form(""));
    }
}
