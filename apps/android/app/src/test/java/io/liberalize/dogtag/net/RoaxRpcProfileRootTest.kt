package io.liberalize.dogtag.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class RoaxRpcProfileRootTest {
    private val zero = "0x" + "0".repeat(64)
    private val expected = "0x" + "a".repeat(64)

    @Test
    fun validatesBytes32RpcResults() {
        assertEquals(expected, RoaxRpc.normalizeBytes32("A".repeat(64)))
        assertNull(RoaxRpc.normalizeBytes32("a".repeat(63)))
        assertNull(RoaxRpc.normalizeBytes32("g".repeat(64)))
    }

    @Test
    fun classifiesPendingMatchedAndMismatchedRoots() {
        assertEquals(
            RoaxRpc.ProfileRootObservation.Pending,
            RoaxRpc.classifyProfileRoot(null, expected),
        )
        assertEquals(
            RoaxRpc.ProfileRootObservation.Pending,
            RoaxRpc.classifyProfileRoot(zero, expected),
        )
        assertEquals(
            RoaxRpc.ProfileRootObservation.Pending,
            RoaxRpc.classifyProfileRoot("malformed", expected),
        )
        assertEquals(
            RoaxRpc.ProfileRootObservation.Matched,
            RoaxRpc.classifyProfileRoot("0x" + "A".repeat(64), expected),
        )
        assertEquals(
            RoaxRpc.ProfileRootObservation.Mismatch,
            RoaxRpc.classifyProfileRoot("0x" + "b".repeat(64), expected),
        )
    }
}
