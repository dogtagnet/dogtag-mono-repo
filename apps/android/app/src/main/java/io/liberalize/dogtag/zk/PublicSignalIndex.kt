package io.liberalize.dogtag.zk

/**
 * Public-signal index constants for the two Groth16 circuits.
 *
 * Both circuits emit a SEVEN-element `pubSignals` vector and the two orders DISAGREE from index 3
 * onward. Nothing catches a mix-up: both are same-length string lists, so reading a Level-B proof
 * with Level-A indices compiles, runs, and yields a plausible-looking field element. The failure
 * lands far away and silently - reading `pubSignals[4]` as the nullifier under Level-B actually
 * yields `R`, so the app polls `VerificationRegistry.consumed(R)`, which is never set, and a
 * verification that SUCCEEDED on-chain spins until timeout.
 *
 * ```text
 * index   Level-A (verification.circom)    Level-B (consent.circom)
 *   0     dogTagId                         dogTagId
 *   1     purpose                          purpose
 *   2     relayer                          relayer
 *   3     subject                          nullifier     <-- diverges here
 *   4     nullifier                        R
 *   5     keyHash                          recordType
 *   6     R                                deadline
 * ```
 *
 * **Use [LevelA] for everything the app does today.** This binary bundles `verification_final.zkey`
 * (pinned `dogtag-levela/1` in `ZkeyAsset`, whose test asserts `dogtag-levelb/1` FAILS to resolve)
 * and calls `proveVerification`, so every proof it handles is Level-A. [LevelB] is here so the other
 * order has a named home rather than being re-derived from a comment; at the M-4 cutover, migrating a
 * call site is a one-line change at a place that already says which order it means.
 *
 * Do NOT add a third, level-neutral set: a bare `NULLIFIER` would have to pick one value, and
 * whichever it picked would silently contradict either this app or `VerificationRegistryConsent.sol`
 * (whose on-chain `P_NULLIFIER = 3` / `P_ROOT = 4` are Level-B-valued). Naming the level is the
 * safety property.
 *
 * Mirrored, with the same reasoning, in `crates/dogtag-standard-rs/src/public_signals.rs` and
 * `apps/ios/DogTag/PublicSignalIndex.swift`.
 */
object PublicSignalIndex {
    /**
     * Width of the public-signal vector. Shared by BOTH circuits - which is exactly why the differing
     * ORDER cannot be caught by a length check.
     */
    const val COUNT = 7

    /**
     * Level-A (`verification.circom`): `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]`.
     * The order every proof this app produces today uses.
     */
    object LevelA {
        const val DOG_TAG_ID = 0
        const val PURPOSE = 1
        const val RELAYER = 2

        /**
         * The subject as a field element. Level-B has no `subject` signal at all - not revealing the
         * owner is the point of that redesign.
         */
        const val SUBJECT = 3
        const val NULLIFIER = 4

        /**
         * Checked on-chain against `ConsentKeyRegistry.keyOf(subject)`. Level-B deletes the registry,
         * so this signal has no Level-B counterpart.
         */
        const val KEY_HASH = 5

        /** The credential root `R`. */
        const val ROOT = 6
    }

    /**
     * Level-B (`consent.circom`): `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
     * Frozen with the M3 verifying key and mirrored on-chain by `VerificationRegistryConsent.sol`.
     * Nothing in this app produces or consumes a Level-B proof yet.
     */
    object LevelB {
        const val DOG_TAG_ID = 0
        const val PURPOSE = 1
        const val RELAYER = 2
        const val NULLIFIER = 3

        /**
         * The credential root `R`. Note this is the slot Level-A uses for the NULLIFIER - the single
         * most consequential collision between the two orders.
         */
        const val ROOT = 4
        const val RECORD_TYPE = 5
        const val DEADLINE = 6
    }
}
