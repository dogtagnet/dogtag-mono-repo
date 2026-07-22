package io.liberalize.dogtag.zk

import org.junit.Assert.assertEquals
import org.junit.Test

/** Pins the frozen consent circuit order used on-device and on-chain. */
class PublicSignalIndexTest {
    @Test
    fun consentOrderIsFrozen() {
        assertEquals(7, PublicSignalIndex.COUNT)
        assertEquals(0, PublicSignalIndex.DOG_TAG_ID)
        assertEquals(1, PublicSignalIndex.PURPOSE)
        assertEquals(2, PublicSignalIndex.RELAYER)
        assertEquals(3, PublicSignalIndex.NULLIFIER)
        assertEquals(4, PublicSignalIndex.ROOT)
        assertEquals(5, PublicSignalIndex.RECORD_TYPE)
        assertEquals(6, PublicSignalIndex.DEADLINE)
    }
}
