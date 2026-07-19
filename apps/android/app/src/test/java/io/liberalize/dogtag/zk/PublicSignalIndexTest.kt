package io.liberalize.dogtag.zk

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

/**
 * Pins the public-signal index constants. These are plain Ints, so nothing else in the build catches
 * a wrong one: a Level-A/Level-B mix-up reads a real field element from the wrong slot and fails
 * silently, far from the mistake. Mirrors `PublicSignalIndexTests.swift` and the Rust
 * `public_signals::tests`; pure JVM, no Android runtime needed.
 */
class PublicSignalIndexTest {

    /**
     * Guards the Level-B constants against accidental drift. The values were transcribed from
     * `VerificationRegistryConsent.sol:81-87`'s `P_*` constants, which remain the authority - but
     * this asserts literals and never reads the Solidity, so a CONTRACT-side change would not fail
     * it. If the circuit's output order ever changes, contract and app have to be moved together by
     * hand, or on-chain and off-chain silently disagree about what they are comparing.
     */
    @Test
    fun `level B matches the on-chain constants`() {
        assertEquals(0, PublicSignalIndex.LevelB.DOG_TAG_ID)
        assertEquals(1, PublicSignalIndex.LevelB.PURPOSE)
        assertEquals(2, PublicSignalIndex.LevelB.RELAYER)
        assertEquals(3, PublicSignalIndex.LevelB.NULLIFIER)
        assertEquals(4, PublicSignalIndex.LevelB.ROOT)
        assertEquals(5, PublicSignalIndex.LevelB.RECORD_TYPE)
        assertEquals(6, PublicSignalIndex.LevelB.DEADLINE)
    }

    /** Level-A is what this app actually produces and consumes today. */
    @Test
    fun `level A matches the shipped prover output`() {
        assertEquals(0, PublicSignalIndex.LevelA.DOG_TAG_ID)
        assertEquals(1, PublicSignalIndex.LevelA.PURPOSE)
        assertEquals(2, PublicSignalIndex.LevelA.RELAYER)
        assertEquals(3, PublicSignalIndex.LevelA.SUBJECT)
        assertEquals(4, PublicSignalIndex.LevelA.NULLIFIER)
        assertEquals(5, PublicSignalIndex.LevelA.KEY_HASH)
        assertEquals(6, PublicSignalIndex.LevelA.ROOT)
    }

    /**
     * The drift that motivated these constants: the orders agree on the first three signals and
     * diverge from index 3 on. Level-A's NULLIFIER slot is Level-B's ROOT slot - reading one as the
     * other is exactly the bug that makes a successful verification hang the phone.
     */
    @Test
    fun `the two orders diverge exactly from index three`() {
        assertEquals(PublicSignalIndex.LevelB.DOG_TAG_ID, PublicSignalIndex.LevelA.DOG_TAG_ID)
        assertEquals(PublicSignalIndex.LevelB.PURPOSE, PublicSignalIndex.LevelA.PURPOSE)
        assertEquals(PublicSignalIndex.LevelB.RELAYER, PublicSignalIndex.LevelA.RELAYER)
        assertNotEquals(PublicSignalIndex.LevelB.NULLIFIER, PublicSignalIndex.LevelA.NULLIFIER)
        assertEquals(PublicSignalIndex.LevelB.ROOT, PublicSignalIndex.LevelA.NULLIFIER)
        assertNotEquals(PublicSignalIndex.LevelB.ROOT, PublicSignalIndex.LevelA.ROOT)
    }

    /** Both circuits emit the same WIDTH, which is why a length check can never catch an order mix-up. */
    @Test
    fun `both orders are seven wide`() {
        assertEquals(7, PublicSignalIndex.COUNT)
        assertEquals(PublicSignalIndex.COUNT - 1, PublicSignalIndex.LevelA.ROOT)
        assertEquals(PublicSignalIndex.COUNT - 1, PublicSignalIndex.LevelB.DEADLINE)
    }
}
