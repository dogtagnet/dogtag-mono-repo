//! The microchip cross-check: does the credential being attached describe THIS animal?
//!
//! # What this is for
//!
//! A DogTag is linked to a pet by an operator typing (or scanning) a tag id. Nothing about that act
//! is self-checking: a mistyped digit, a mixed-up pair of cage cards, or two similar dogs in the same
//! afternoon all produce a link that is structurally perfect and about the wrong animal. Until now
//! the only thing standing between a correct link and a wrong one was the operator remembering.
//!
//! A credential already carries the animal's microchip as a salted Merkle leaf, so it is
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
//! # Four states, and the middle two are the whole point
//!
//! [`MicrochipCheck`] is `Matched` / `Mismatch` / `NotComparable` / `UnrecognisedCredentialLeaf`,
//! and `NotComparable` must never render as either neighbour. This codebase's defining defect class
//! is a check that did not run being reported as one that passed; the inverse — reporting it as a
//! failure — is just as wrong and is the one this particular feature would get wrong, because
//! refusing every unchipped cat would make the field unusable. So `NotComparable` carries a
//! [`NotComparable`] reason that says which side was missing, or which read did not resolve, and the
//! caller renders that sentence.
//!
//! # Why inertness gets its OWN state
//!
//! This check shipped once reading a single key path — `credentialSubject.microchip.code` — that NO
//! real issuer emits, so it was inert on every real credential: it would have passed its tests and
//! protected nothing. What let that survive is subtler than the key-path list, and is the reason
//! [`MicrochipCheck::UnrecognisedCredentialLeaf`] exists.
//!
//! "The credential has no microchip" is an ORDINARY, quiet, benign state (see above) — and a reader
//! that cannot find a microchip that IS present produces exactly the same quiet answer. Our own
//! not-comparable state camouflages a broken check. So the two are kept structurally apart:
//!
//!   * neither side carries a microchip → `NotComparable`, neutral, silent, never blocks; and
//!   * the credential carries microchip-shaped data at a key path this build does not recognise →
//!     `UnrecognisedCredentialLeaf`, LOUD, naming the paths it found.
//!
//! The loud state is deliberately NOT a [`NotComparable`] reason, under any spelling: it is not a
//! pet without a chip, it is our reader failing to read one. It still does not REFUSE the write —
//! it is evidence that our reader is wrong, never evidence about the animal.
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

/// The credential leaves this check reads, as trailing key-path SEGMENT runs.
///
/// # What this accepts, and why it is a suffix rather than an exact path
///
/// A leaf's key path is issuer-shaped, not protocol-fixed, and the four shapes real issuers emit
/// today already disagree on the prefix. `flatten_data` produces, verbatim:
///
///   * `microchip.code` — the vet portal's VACCINATION form. Its field def is
///     `path: "microchip.code"` with NO prefix (`packages/ui/src/schema/recordTypes.ts`),
///     `buildFieldsObject` nests from the ROOT, and `app::build_vc` clones the operator's fields
///     verbatim while injecting only `credentialSubject.dogTagId` — so this leaf sits at the `data`
///     TOP LEVEL, a sibling of `credentialSubject` rather than a child of it.
///   * `credentialSubject.microchip.code` — the schema-conformant nested variant that
///     `dogtag_standard::schema::validate_schema` describes.
///   * `credentialSubject.microchipNumber` — government `EU_HEALTH_CERT`
///     (`stacks/government/api/src/app.rs`).
///   * `credentialSubject.animal.microchipNumber` — government `TRAVEL_CLEARANCE`, nested under the
///     CDC Section B `animal` block (same file).
///
/// Two suffixes cover all four. Reading ONE exact path is what made this check inert on every real
/// credential, so the shape of the rule is part of the fix, not an implementation detail.
///
/// # Why it cannot collide with an unrelated leaf
///
/// Matching is on whole trailing SEGMENTS, split on `.` — never `str::ends_with`, which would make
/// `credentialSubject.previousMicrochipNumber` match `microchipNumber` and compare against a
/// different animal's retired chip. `microchip.code` likewise requires a segment `microchip` whose
/// child segment is exactly `code`, so the vet schema's own `microchip.standard` and
/// `microchip.implantDate` siblings cannot match. A colliding leaf would therefore have to be named
/// with these exact final segments and mean something else, which no emitter in the fleet does.
pub const RECOGNISED_MICROCHIP_SUFFIXES: &[&[&str]] = &[&["microchip", "code"], &["microchipNumber"]];

/// `true` when `key_path`'s trailing segments are exactly `suffix`.
fn has_suffix(key_path: &str, suffix: &[&str]) -> bool {
    let segments: Vec<&str> = key_path.split('.').collect();
    segments.len() >= suffix.len() && &segments[segments.len() - suffix.len()..] == suffix
}

/// `true` when `key_path` is one of [`RECOGNISED_MICROCHIP_SUFFIXES`].
fn is_recognised(key_path: &str) -> bool {
    RECOGNISED_MICROCHIP_SUFFIXES
        .iter()
        .any(|s| has_suffix(key_path, s))
}

/// `true` when a leaf LOOKS like it carries a microchip but is not one this build can read.
///
/// The detector is: the FINAL segment contains `microchip`, case-insensitively. Chosen over "any
/// segment contains microchip" because the vet schema's `microchip.standard` and
/// `microchip.implantDate` are real, common leaves that are NOT codes — flagging them would fire the
/// loud state on every ordinary credential that carries a chip container and no code, which is
/// precisely the benign absent case. Their final segments are `standard` and `implantDate`, so this
/// rule leaves them alone while catching every plausible spelling of a code leaf itself
/// (`microchipNumber`, `microchipId`, `microchipCode`, a bare `microchip`).
///
/// The trade, stated rather than hidden: a code leaf whose own final segment does not name the chip
/// (a hypothetical `microchip.number`) still reads as absent. That is a narrower miss than the false
/// alarms the wider rule would produce, and any such emitter belongs in
/// [`RECOGNISED_MICROCHIP_SUFFIXES`] anyway.
fn is_microchip_shaped(key_path: &str) -> bool {
    key_path
        .rsplit('.')
        .next()
        .is_some_and(|last| last.to_ascii_lowercase().contains("microchip"))
}

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
    /// The leaf is present but does not parse as a packed scalar, holds an empty value, or two
    /// recognised leaves disagree about the code.
    Unreadable(String),
    /// The document carries microchip-shaped leaves, but NONE at a key path this build recognises.
    ///
    /// The paths are carried so the message can NAME them: the remedy is a one-line addition to
    /// [`RECOGNISED_MICROCHIP_SUFFIXES`], and it should be obvious from the sentence alone.
    UnrecognisedKeyPath(Vec<String>),
}

/// Read the microchip leaf out of a wrapped credential.
///
/// Reads `data`, which is what `check_integrity` folds — never `issuer` or any other block outside
/// the Merkle root, where an attacker-supplied value would be uncovered.
///
/// A document may now match MORE than one recognised leaf (a government `TRAVEL_CLEARANCE` carrying
/// both a Section B chip and a top-level one, say). Taking whichever `flatten_data` happened to emit
/// first would decide a refusal on arbitrary evidence, so agreement is required: identical values
/// are `Present`, and a disagreement is [`CredentialMicrochip::Unreadable`] naming both paths — a
/// failure to read, never a silent pick and never a mismatch attributed to the pet.
pub fn credential_microchip(doc: &WrappedDoc) -> CredentialMicrochip {
    let withheld_leaves = doc.privacy.obfuscated.len();
    let flat = flatten_data(&doc.data);

    let recognised: Vec<(String, String)> = flat
        .iter()
        .filter(|(kp, _)| is_recognised(kp))
        .cloned()
        .collect();

    if recognised.is_empty() {
        // Only once NO recognised leaf exists does the shape detector get a say — a credential we
        // CAN read is never reported as one we cannot.
        let mut unrecognised: Vec<String> = flat
            .into_iter()
            .map(|(kp, _)| kp)
            .filter(|kp| is_microchip_shaped(kp))
            .collect();
        if !unrecognised.is_empty() {
            unrecognised.sort();
            unrecognised.dedup();
            return CredentialMicrochip::UnrecognisedKeyPath(unrecognised);
        }
        return CredentialMicrochip::Absent { withheld_leaves };
    }

    let mut seen: Vec<(String, String)> = Vec::with_capacity(recognised.len());
    for (kp, packed) in recognised {
        let value = match parse_packed(&packed) {
            Ok((_, _, v)) => v,
            Err(e) => {
                return CredentialMicrochip::Unreadable(format!(
                    "the credential's microchip leaf at {kp} could not be decoded: {e}"
                ))
            }
        };
        let v = value.trim();
        if v.is_empty() {
            // A leaf that is present and blank is not the same as no leaf: the issuer wrote
            // something we cannot compare, which is a failure to read rather than an absence.
            return CredentialMicrochip::Unreadable(format!(
                "the credential's microchip leaf at {kp} is empty"
            ));
        }
        seen.push((kp, v.to_string()));
    }
    let (first_path, first_value) = &seen[0];
    if let Some((other_path, other_value)) = seen.iter().find(|(_, v)| v != first_value) {
        return CredentialMicrochip::Unreadable(format!(
            "the credential carries two different microchips ({first_path} says {first_value}, \
             {other_path} says {other_value}), so which one describes the animal is unknown"
        ));
    }
    CredentialMicrochip::Present(first_value.clone())
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
    /// The check COULD NOT RUN: the credential carries microchip-shaped data at key path(s) this
    /// build does not recognise, so the field it exists to compare was never read.
    ///
    /// Its OWN state, never a [`NotComparable`] reason — see the module header. That separation is
    /// the whole remedy: folded in, this reads as the ordinary unchipped animal, which is
    /// indistinguishable from success and is exactly how an inert check ships unnoticed.
    ///
    /// It does NOT refuse. It is evidence that our reader is wrong, not evidence about the animal.
    UnrecognisedCredentialLeaf { key_paths: Vec<String> },
}

impl MicrochipCheck {
    /// Whether a write carrying this verdict must be refused. ONLY a mismatch refuses; every other
    /// state passes through, because a check that could not run is not a failure of the thing being
    /// checked — least of all when what could not run is our own reader.
    pub fn refuses(&self) -> bool {
        matches!(self, Self::Mismatch { .. })
    }

    pub fn wire_state(&self) -> &'static str {
        match self {
            Self::Matched(_) => "matched",
            Self::Mismatch { .. } => "mismatch",
            Self::NotComparable(_) => "notComparable",
            Self::UnrecognisedCredentialLeaf { .. } => "unrecognisedCredentialLeaf",
        }
    }

    /// The sentence for [`Self::UnrecognisedCredentialLeaf`], naming every path it found.
    fn unrecognised_detail(key_paths: &[String]) -> String {
        format!(
            "The microchip could NOT be checked: this credential carries microchip data at {}, \
             which this system does not know how to read, so nothing was compared. This is a defect \
             in this system's reader, not a statement about the animal or the credential — report it \
             so the key path can be added.",
            key_paths.join(", ")
        )
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
            // Deliberately carries NO `reason`/`isFailure` pair: those belong to `notComparable`,
            // and giving this state their shape is how a client's fallback branch would quietly
            // absorb it back into "nothing to compare".
            Self::UnrecognisedCredentialLeaf { key_paths } => json!({
                "state": "unrecognisedCredentialLeaf",
                "keyPaths": key_paths,
                "detail": Self::unrecognised_detail(key_paths),
            }),
        }
    }
}

/// `Some(_)` when the CREDENTIAL alone makes the check impossible to run, whatever the pet side says.
///
/// Today that is exactly the unrecognised-key-path case. It is a separate function, and every caller
/// that decides a verdict routes through it, because the loud state has to OUTRANK every pet-side
/// fact: the commonest of those is "this pet has no microchip on file", so a reader bug reached
/// through an unchipped pet — or through a tag no pet is linked to — would land back on the quiet
/// benign answer this state exists to escape.
///
/// This function's `Some` set and [`compare`]'s `unreachable!` arm MUST move together: `compare`
/// relies on every variant answered here being intercepted before its own match, and the compiler
/// cannot check that. A new [`CredentialMicrochip`] variant routed through here without the matching
/// arm turns a state into a panic.
pub fn unrunnable(credential: &CredentialMicrochip) -> Option<MicrochipCheck> {
    match credential {
        CredentialMicrochip::UnrecognisedKeyPath(paths) => {
            Some(MicrochipCheck::UnrecognisedCredentialLeaf {
                key_paths: paths.clone(),
            })
        }
        _ => None,
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
    // BEFORE anything pet-side, so an unchipped pet cannot camouflage a reader that failed to find a
    // microchip the credential is carrying. See [`unrunnable`].
    if let Some(loud) = unrunnable(credential) {
        return loud;
    }
    let pet_code = pet.map(str::trim).filter(|s| !s.is_empty());
    let credential_code = match credential {
        CredentialMicrochip::Unreadable(e) => {
            return MicrochipCheck::NotComparable(NotComparable::CredentialUnreadable(e.clone()))
        }
        CredentialMicrochip::UnrecognisedKeyPath(_) => unreachable!("handled by `unrunnable` above"),
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

    // ---- the four REAL emitter shapes ----
    //
    // Reading one exact key path made this check inert on every real credential. These are the four
    // shapes the fleet actually emits, each named for its emitter — the end-to-end counterparts live
    // in `tests/microchip_binding.rs`.

    #[test]
    fn the_vet_portals_top_level_leaf_is_read() {
        // `recordTypes.ts` declares `path: "microchip.code"` with NO prefix, `buildFieldsObject`
        // nests from the ROOT, and `build_vc` relocates nothing — so this leaf is a SIBLING of
        // `credentialSubject`, not a child. Narrowing the matcher back to the exact nested path
        // makes this read as a credential with no microchip at all.
        let d = doc(
            json!({
                "credentialSubject": { "dogTagId": "aaaa:2:4" },
                "microchip": {
                    "code": "bbbb:5:985141006580319",
                    "standard": "cccc:2:ISO_11784_11785",
                    "implantDate": "dddd:2:2023-10-01",
                },
                "vaccinationDate": "eeee:2:2026-01-11",
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    #[test]
    fn the_schema_conformant_nested_leaf_is_read() {
        assert_eq!(
            credential_microchip(&chipped("985141006580319")),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    #[test]
    fn the_government_eu_health_cert_leaf_is_read() {
        // `credentialSubject.microchipNumber` — a different leaf NAME under a different parent.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "species": "bbbb:2:dog",
                    "microchipNumber": "cccc:5:985141006580319",
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    #[test]
    fn the_government_travel_clearance_leaf_is_read_from_section_b() {
        // `credentialSubject.animal.microchipNumber`, one level deeper again.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "animal": { "name": "bbbb:2:Blaze", "microchipNumber": "cccc:5:985141006580319" },
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    #[test]
    fn matching_is_on_whole_segments_so_a_different_leaf_cannot_masquerade() {
        // `str::ends_with` would make this match `microchipNumber` and compare the animal against a
        // RETIRED chip — a refusal on evidence about a different implant.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "previousMicrochipNumber": "bbbb:5:900000000000001",
                }
            }),
            vec![],
        );
        // Not read as the chip; reported as microchip-SHAPED but unreadable rather than silently
        // absent, which is the point of the loud state.
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::UnrecognisedKeyPath(vec![
                "credentialSubject.previousMicrochipNumber".into()
            ])
        );
    }

    #[test]
    fn two_recognised_leaves_that_disagree_are_a_read_failure_not_an_arbitrary_pick() {
        // Widening to a suffix set makes it possible for one document to match twice. Taking
        // whichever `flatten_data` emitted first would decide a REFUSAL on arbitrary evidence.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "microchipNumber": "bbbb:5:985141006580319",
                    "animal": { "microchipNumber": "cccc:5:900000000000001" },
                }
            }),
            vec![],
        );
        let v = credential_microchip(&d);
        assert!(matches!(v, CredentialMicrochip::Unreadable(_)), "{v:?}");
        // ...and it does not become a mismatch attributed to the pet.
        assert!(!compare(Some("985141006580319"), &v).refuses());
    }

    #[test]
    fn two_recognised_leaves_that_agree_are_simply_the_code() {
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "microchipNumber": "bbbb:5:985141006580319",
                    "animal": { "microchipNumber": "cccc:5:985141006580319" },
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    // ---- the LOUD state: a microchip we cannot READ is not a microchip that is not there ----

    #[test]
    fn a_microchip_shaped_leaf_we_do_not_recognise_is_its_own_state_and_names_the_path() {
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "chipDetails": { "microchipIdentifier": "bbbb:5:985141006580319" },
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::UnrecognisedKeyPath(vec![
                "credentialSubject.chipDetails.microchipIdentifier".into()
            ])
        );
    }

    #[test]
    fn the_loud_state_outranks_every_pet_side_fact_and_never_refuses() {
        // Absent-microchip is the commonest state in the product, so if it could stand in front of
        // the loud state the camouflage that let this check ship inert would come straight back.
        let c = CredentialMicrochip::UnrecognisedKeyPath(vec!["credentialSubject.microchipRef".into()]);
        for pet in [None, Some("985141006580319"), Some("900000000000001"), Some("  ")] {
            let v = compare(pet, &c);
            assert_eq!(
                v,
                MicrochipCheck::UnrecognisedCredentialLeaf {
                    key_paths: vec!["credentialSubject.microchipRef".into()]
                },
                "pet={pet:?}"
            );
            assert!(!v.refuses(), "a reader defect is not evidence about the animal");
        }
    }

    #[test]
    fn the_loud_state_never_borrows_the_not_comparable_shape_or_its_wording() {
        // Folded into `notComparable` under any reason, this reads as an ordinary unchipped animal —
        // which is indistinguishable from success and is exactly how an inert check ships unnoticed.
        let v = MicrochipCheck::UnrecognisedCredentialLeaf {
            key_paths: vec!["credentialSubject.chipDetails.microchipIdentifier".into()],
        };
        let j = v.to_json();
        assert_eq!(v.wire_state(), "unrecognisedCredentialLeaf");
        assert_eq!(j["state"], json!("unrecognisedCredentialLeaf"));
        assert!(j["reason"].is_null(), "{j}");
        assert!(j["isFailure"].is_null(), "{j}");
        assert_eq!(
            j["keyPaths"],
            json!(["credentialSubject.chipDetails.microchipIdentifier"])
        );
        let detail = j["detail"].as_str().unwrap();
        assert!(
            detail.contains("credentialSubject.chipDetails.microchipIdentifier"),
            "the path is the remedy and must be named: {detail}"
        );
        let lower = detail.to_lowercase();
        assert!(
            !lower.contains("nothing to compare"),
            "that is the unchipped animal's sentence, and it reads as success: {detail}"
        );
        assert!(
            lower.contains("not be checked") || lower.contains("could not"),
            "it must say the check did not RUN: {detail}"
        );
    }

    #[test]
    fn a_chip_container_with_no_code_is_absent_not_loud() {
        // The detector's false-positive guard, and the reason it reads the FINAL segment only. The
        // vet schema's `standard` and `implantDate` are real, common leaves that are not codes, so
        // flagging them would fire the loud state on ordinary unchipped animals.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "microchip": { "standard": "bbbb:2:ISO_11784_11785", "implantDate": "cccc:2:2023-10-01" },
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Absent { withheld_leaves: 0 }
        );
    }

    #[test]
    fn a_recognised_leaf_wins_over_an_unrecognised_one() {
        // A credential we CAN read is never reported as one we cannot: the loud state is for the
        // case where nothing recognised exists at all.
        let d = doc(
            json!({
                "credentialSubject": {
                    "dogTagId": "aaaa:2:4",
                    "microchipNumber": "bbbb:5:985141006580319",
                    "legacyMicrochipRef": "cccc:2:old-scanner-dump",
                }
            }),
            vec![],
        );
        assert_eq!(
            credential_microchip(&d),
            CredentialMicrochip::Present("985141006580319".into())
        );
    }

    // ---- the credential side ----

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
