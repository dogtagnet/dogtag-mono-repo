package io.liberalize.dogtag.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * Pins the version-keyed artifact resolver's contract (M7 §3.2).
 *
 * `ZkeyAsset` used to hard-code one zkey + one graph filename; it now resolves them from a
 * version-keyed [ArtifactDescriptor] table. Two properties matter and are easy to break silently:
 *
 *  1. **Back-compat** — every caller on this app names no version (`ScanScreen`, `ZkSelfTest` call
 *     `ensure(context)` / `ensureGraph(context)`), so the default MUST stay the Level-A set, pointing
 *     at the same bundled assets as before.
 *  2. **Fail-closed** — an unknown version must never quietly resolve to the current artifacts. A
 *     proof built with the wrong proving key is rejected by that version's verifier, so a fallback
 *     would turn a clear error into a confusing one.
 *
 * These are pure resolution tests: they need no `Context`, since materialising an asset (the copy
 * into `filesDir`) is Android-framework work covered by the on-device flow.
 */
class ZkeyAssetTest {
    @Test
    fun resolveWithNoVersionIsTheLevelASet() {
        assertEquals(ZkeyAsset.LEVEL_A_V1, ZkeyAsset.resolve())
        assertEquals(ZkeyAsset.LEVEL_A_V1, ZkeyAsset.current())
    }

    /**
     * The default descriptor names the SAME assets the pre-M7 hard-coded constants did. If this
     * drifts, the app materialises a file the APK does not ship and proving dies at runtime.
     */
    @Test
    fun levelADescriptorNamesTheBundledAssets() {
        val d = ZkeyAsset.current()
        assertEquals("dogtag-levela/1", d.version)
        assertEquals("verification_final.zkey", d.zkeyAsset)
        assertEquals("verification.graph", d.graphAsset)
    }

    @Test
    fun resolveKnownVersionReturnsItsDescriptor() {
        assertEquals(ZkeyAsset.LEVEL_A_V1, ZkeyAsset.resolve("dogtag-levela/1"))
    }

    /**
     * An unknown version fails closed — it must NOT fall back to the current artifact set.
     *
     * The probe version is `dogtag-levelc/9`, a version this build genuinely does not know. It used
     * to be `dogtag-levelb/1`, but M-4 REGISTERS Level-B (see [levelBDescriptorNamesTheBundledAssets]),
     * so that key now resolves and would no longer exercise the fail-closed path. This mirrors the
     * Rust `resolve_unknown_version_fails_closed`, which probes the same `dogtag-levelc/9` for the
     * same reason.
     */
    @Test
    fun resolveUnknownVersionFailsClosed() {
        val e = assertThrows(IllegalArgumentException::class.java) {
            ZkeyAsset.resolve("dogtag-levelc/9")
        }
        assertNotNull(e.message)
        assertEquals(true, e.message!!.contains("dogtag-levelc/9"))
    }

    /**
     * M-4: Level-B is REGISTERED and names the consent artifacts the mobile workflow vendors. If this
     * drifts, the app materialises a file the APK does not ship and the owner-hidden proof dies at
     * runtime. The version key and filenames must match the Rust `LEVEL_B_V1_DESCRIPTOR` and iOS
     * `ZkeyAsset.levelBV1`.
     */
    @Test
    fun levelBDescriptorNamesTheBundledAssets() {
        val d = ZkeyAsset.resolve("dogtag-levelb/1")
        assertEquals(ZkeyAsset.LEVEL_B_V1, d)
        assertEquals("dogtag-levelb/1", d.version)
        assertEquals("consent_final.zkey", d.zkeyAsset)
        assertEquals("consent.graph", d.graphAsset)
    }

    /**
     * Registering Level-B must NOT change the default: every current caller names no version and must
     * still get Level-A. This is the load-bearing "available, not default" guard for the artifact
     * resolver — the mirror of the same guard on the server's convenience tier.
     */
    @Test
    fun registeringLevelBDoesNotChangeTheDefault() {
        assertEquals(ZkeyAsset.LEVEL_A_V1, ZkeyAsset.current())
        assertEquals(ZkeyAsset.LEVEL_A_V1, ZkeyAsset.resolve())
    }

    /** Every registered version is resolvable by its own key, and keys are unique. */
    @Test
    fun registryEntriesAreUniqueAndResolvable() {
        val keys = ZkeyAsset.REGISTRY.map { it.version }
        assertEquals(keys.size, keys.distinct().size)
        ZkeyAsset.REGISTRY.forEach { assertEquals(it, ZkeyAsset.resolve(it.version)) }
    }

    /**
     * The Kotlin descriptor's version key must equal the Rust
     * `dogtag_prover::artifact::LEVEL_A_V1` and the iOS `ZkeyAsset.levelAV1.version`. The three are
     * separate declarations of one protocol constant — the server refuses a version string it does
     * not know, so a typo here is a runtime rejection, not a compile error.
     */
    @Test
    fun levelAVersionKeyMatchesTheProtocolConstant() {
        assertEquals("dogtag-levela/1", ZkeyAsset.LEVEL_A_V1.version)
    }
}
