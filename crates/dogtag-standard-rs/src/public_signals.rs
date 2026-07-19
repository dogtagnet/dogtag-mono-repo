//! Public-signal index constants for the two Groth16 circuits.
//!
//! Both circuits emit a SEVEN-element public-signal vector, and the two orders DISAGREE from index 3
//! onward. Nothing in the type system catches a mix-up: both are `[String; 7]`, so reading a Level-B
//! proof with Level-A indices compiles, runs, and returns a plausible-looking field element. The
//! failure is silent and downstream - e.g. polling `VerificationRegistry.consumed(R)` for a nullifier
//! that is never set, so a verification that actually SUCCEEDED on-chain hangs the phone until timeout.
//!
//! ```text
//! index   Level-A (`verification.circom`)   Level-B (`consent.circom`)
//!   0     dogTagId                          dogTagId
//!   1     purpose                           purpose
//!   2     relayer                           relayer
//!   3     subject                           nullifier     <-- diverges here
//!   4     nullifier                         R
//!   5     keyHash                           recordType
//!   6     R                                 deadline
//! ```
//!
//! # Which set should I use?
//!
//! **Use [`level_a`] for anything on the live serving path today.** Level-A is the only circuit with a
//! network around it: both mobile apps bundle `verification_final.zkey` (pinned `dogtag-levela/1`) and
//! call `proveVerification`, and the relayer submits the 4-arg `recordVerificationZK` alongside a
//! `ConsentKeyRegistry` bind. Level-B is a complete, tested cryptographic core that nothing yet
//! produces or consumes end-to-end.
//!
//! [`level_b`] exists so the Level-B order has a single named home rather than being re-derived from a
//! comment at each call site. At the M-4 cutover, migrating a call site is a one-line import swap
//! (`level_a` -> `level_b`) at a place that is already explicit about which order it means - which is
//! the entire point of routing reads through these constants ahead of the flip.
//!
//! Do NOT introduce a third, level-neutral set. A bare `P_NULLIFIER` would have to pick one value, and
//! whichever it picked would silently contradict either the live off-chain code or
//! `VerificationRegistryConsent.sol`, whose on-chain `P_NULLIFIER = 3` / `P_ROOT = 4` are Level-B-valued.
//! Naming the level is the safety property.
//!
//! Mirrored, with the same reasoning, in `apps/ios/DogTag/PublicSignalIndex.swift` and
//! `apps/android/app/src/main/java/io/liberalize/dogtag/zk/PublicSignalIndex.kt`.

/// Width of the public-signal vector. Shared by BOTH circuits, which is exactly why the differing
/// ORDER cannot be caught by a length check.
pub const NUM_PUBLIC: usize = 7;

/// Level-A (`verification.circom`) order: `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]`.
///
/// This is the order every shipped producer emits today. Source of truth:
/// `crates/dogtag-prover-rs/src/lib.rs` (`Groth16Output`) and `prover_ffi.rs`.
pub mod level_a {
    pub const DOG_TAG_ID: usize = 0;
    pub const PURPOSE: usize = 1;
    pub const RELAYER: usize = 2;
    /// The subject as a field element. Level-B has no `subject` signal at all - the whole point of
    /// the redesign is that the owner is not revealed.
    pub const SUBJECT: usize = 3;
    pub const NULLIFIER: usize = 4;
    /// `keyHash`, checked on-chain against `ConsentKeyRegistry.keyOf(subject)`. Level-B deletes the
    /// registry, so this signal has no Level-B counterpart.
    pub const KEY_HASH: usize = 5;
    /// The credential root `R`.
    pub const ROOT: usize = 6;
}

/// Level-B (`consent.circom`) order: `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
///
/// Frozen with the M3 verifying key and mirrored on-chain by `VerificationRegistryConsent.sol`'s
/// `P_*` constants. Source of truth: `circuits/consent.circom` output-declaration order.
pub mod level_b {
    pub const DOG_TAG_ID: usize = 0;
    pub const PURPOSE: usize = 1;
    pub const RELAYER: usize = 2;
    pub const NULLIFIER: usize = 3;
    /// The credential root `R`. Note this is the slot Level-A uses for the NULLIFIER - the single
    /// most consequential collision between the two orders.
    pub const ROOT: usize = 4;
    pub const RECORD_TYPE: usize = 5;
    pub const DEADLINE: usize = 6;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the Level-B constants against `VerificationRegistryConsent.sol:80-87`. If the circuit's
    /// output order is ever changed, this and the contract must move together or on-chain and
    /// off-chain silently disagree about what they are comparing.
    #[test]
    fn level_b_matches_the_on_chain_constants() {
        assert_eq!(level_b::DOG_TAG_ID, 0, "P_DOGTAGID");
        assert_eq!(level_b::PURPOSE, 1, "P_PURPOSE");
        assert_eq!(level_b::RELAYER, 2, "P_RELAYER");
        assert_eq!(level_b::NULLIFIER, 3, "P_NULLIFIER");
        assert_eq!(level_b::ROOT, 4, "P_ROOT");
        assert_eq!(level_b::RECORD_TYPE, 5, "P_RECORDTYPE");
        assert_eq!(level_b::DEADLINE, 6, "P_DEADLINE");
    }

    /// The drift that motivated this module: the two orders agree on the first three signals and
    /// diverge from index 3 on. A change making them agree (or disagree earlier) is a redesign, not a
    /// refactor, and should not pass silently.
    #[test]
    fn the_two_orders_diverge_exactly_from_index_three() {
        assert_eq!(level_a::DOG_TAG_ID, level_b::DOG_TAG_ID);
        assert_eq!(level_a::PURPOSE, level_b::PURPOSE);
        assert_eq!(level_a::RELAYER, level_b::RELAYER);
        // Level-A's nullifier slot is Level-B's root slot - reading one as the other is the E-1 bug.
        assert_ne!(level_a::NULLIFIER, level_b::NULLIFIER);
        assert_eq!(level_a::NULLIFIER, level_b::ROOT);
        assert_ne!(level_a::ROOT, level_b::ROOT);
    }
}
