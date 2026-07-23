//! Public-signal index constants for the consent Groth16 circuit.
//!
//! The circuit emits a SEVEN-element public-signal vector. Nothing in the type system catches a
//! wrong index: every slot is a `String`, so a misread compiles, runs, and returns a
//! plausible-looking field element. The failure is silent and downstream - e.g. polling
//! `consumed(R)` for a nullifier that is never set, so a verification that actually SUCCEEDED
//! on-chain hangs the phone until timeout. Read every `pub[n]` through these constants, never
//! through a bare literal.
//!
//! ```text
//! index   `consent.circom`
//!   0     dogTagId
//!   1     purpose
//!   2     relayer
//!   3     nullifier
//!   4     R
//!   5     recordType
//!   6     deadline
//! ```
//!
//! The module keeps the `level_b` name because it mirrors the internal on-chain version key
//! `dogtag-levelb/1` (an internal identifier, not a user-facing label) and the frozen
//! `VerificationRegistryConsent.sol` `P_*` constants; renaming it is an all-sites-atomic workstep
//! deliberately not taken.
//!
//! (The retired `verification.circom` emitted the same width in a DIFFERENT order -
//! `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]` - which is why reads were routed
//! through named constants in the first place: the two orders diverged from index 3 on and only the
//! name could tell them apart.)
//!
//! Mirrored, with the same reasoning, in `apps/ios/DogTag/PublicSignalIndex.swift` and
//! `apps/android/app/src/main/java/io/liberalize/dogtag/zk/PublicSignalIndex.kt`.

/// Width of the public-signal vector.
pub const NUM_PUBLIC: usize = 7;

/// The consent (`consent.circom`) order: `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
///
/// Frozen with the M3 verifying key and mirrored on-chain by `VerificationRegistryConsent.sol`'s
/// `P_*` constants. Source of truth: `circuits/consent.circom` output-declaration order.
pub mod level_b {
    pub const DOG_TAG_ID: usize = 0;
    pub const PURPOSE: usize = 1;
    pub const RELAYER: usize = 2;
    pub const NULLIFIER: usize = 3;
    /// The credential root `R`.
    pub const ROOT: usize = 4;
    pub const RECORD_TYPE: usize = 5;
    pub const DEADLINE: usize = 6;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the constants against accidental drift, the repo's usual mirrored-constant
    /// pattern. The values were transcribed from `VerificationRegistryConsent.sol:81-87`'s `P_*`
    /// constants, which remain the authority - but this test asserts literals and never reads the
    /// Solidity, so a CONTRACT-side change would not fail it. If the circuit's output order is ever
    /// changed, this and the contract must be moved together by hand, or on-chain and off-chain
    /// silently disagree about what they are comparing.
    #[test]
    fn level_b_matches_the_on_chain_constants() {
        assert_eq!(level_b::DOG_TAG_ID, 0, "P_DOGTAGID");
        assert_eq!(level_b::PURPOSE, 1, "P_PURPOSE");
        assert_eq!(level_b::RELAYER, 2, "P_RELAYER");
        assert_eq!(level_b::NULLIFIER, 3, "P_NULLIFIER");
        assert_eq!(level_b::ROOT, 4, "P_ROOT");
        assert_eq!(level_b::RECORD_TYPE, 5, "P_RECORDTYPE");
        assert_eq!(level_b::DEADLINE, 6, "P_DEADLINE");
        assert_eq!(NUM_PUBLIC, 7);
    }
}
