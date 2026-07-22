package io.liberalize.dogtag.net

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class CentralApiProfileIssueTest {
    private val response = """
        {
          "sessionId":"s-1",
          "dogTagId":"424242",
          "ownerIdentity":{"firstName":"Ada","lastName":"Owner"},
          "pet":{"profile":{
            "name":"Rex","species":"dog","breedVbo":"VBO:123","breedLabel":"Shiba Inu",
            "sex":"male","neuterStatus":"neutered","dateOfBirth":"2020-03-04",
            "weightHistory":[{"unit":"kg","value":"11.4","measuredOn":"2026-01-02"}],
            "microchip":{"code":"9851","standard":"ISO11784","implantDate":"2020-04-01","bodyLocation":"shoulder"}
          }}
        }
    """.trimIndent()

    @Test
    fun parsesAllPetLeavesAndExcludesHumanIdentity() {
        val parsed = CentralApi.parseProfileIssueSession(response)
        assertNotNull(parsed)
        val session = parsed!!
        assertEquals("Ada", session.ownerIdentity["firstName"])
        var n = 0
        val attributes = session.pet.attributes { "0x" + (++n).toString(16).padStart(32, '0') }
        val byPath = attributes.associateBy { it.keyPath }
        assertEquals("Rex", byPath["credentialSubject.name"]?.value)
        assertEquals("11.4", byPath["credentialSubject.weightHistory[0].value"]?.value)
        assertEquals(4, byPath["credentialSubject.weightHistory[0].value"]?.tag?.toInt())
        assertEquals("shoulder", byPath["credentialSubject.microchip.bodyLocation"]?.value)
        assertFalse(byPath.keys.any { it.contains("ownerIdentity") || it.startsWith("owner.") })
        assertFalse(byPath.keys.any { it.contains("dogTagId") })
        assertTrue(session.pet.matches(attributes))
        assertFalse(session.pet.copy(name = "Different").matches(attributes))
    }

    @Test
    fun oldMetadataFreeResponseFailsClosed() {
        assertNull(CentralApi.parseProfileIssueSession(
            """{"sessionId":"s-1","dogTagId":"424242","claims":{}}""",
        ))
    }
}
