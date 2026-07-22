package io.liberalize.dogtag.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class ZkeyAssetTest {
    @Test
    fun ownerHiddenConsentArtifactsAreTheOnlyCurrentSet() {
        val descriptor = ZkeyAsset.current()
        assertEquals(ZkeyAsset.OWNER_HIDDEN_V1, descriptor)
        assertEquals("dogtag-levelb/1", descriptor.version)
        assertEquals("consent_final.zkey", descriptor.zkeyAsset)
        assertEquals("consent.graph", descriptor.graphAsset)
        assertEquals(listOf(descriptor), ZkeyAsset.REGISTRY)
        assertEquals(descriptor, ZkeyAsset.resolve())
        assertEquals(descriptor, ZkeyAsset.resolve("dogtag-levelb/1"))
    }

    @Test
    fun unknownVersionFailsClosed() {
        assertThrows(IllegalArgumentException::class.java) {
            ZkeyAsset.resolve("dogtag-levela/1")
        }
    }
}
