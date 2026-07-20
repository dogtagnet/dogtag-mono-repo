package io.liberalize.dogtag.profile

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * Cross-platform parity: the Android device-side profile-tree builder must produce the SAME `R` as
 * the Rust/iOS reference for the same witness (Level-B M-2b).
 *
 * The anchor is `contracts/test/device-profile-root.json`, the shared fixture whose `R` is built by
 * `profile_tree::build_profile_tree` from the fixed demo witness in
 * `profile_tree::device_root_fixture_witness()` - and which `CustodialIssuance.t.sol` mints and
 * asserts `profileRoot(dogTagId) == R` against. So this is not a self-asserted constant: it is a root
 * the contract already stores. iOS reaches it through the same `buildProfileTreeHex` FFI, so
 * matching the fixture IS matching iOS.
 *
 * What this actually guards is the Android WRAPPER, since the crypto is one compiled Rust
 * implementation shared by both platforms. The wiring is where a platform can silently diverge:
 * the seed's hex encoding, the decimal→field `dogTagId` conversion, the attribute mapping and the
 * type tag. Any of those going wrong yields a well-formed but WRONG `R`, and because `profileRoot`
 * is write-once on-chain, a tag issued against a wrong `R` can never be proven or repaired.
 *
 * Pure JVM, no Android runtime. Needs the host-built Rust core, which `build.gradle.kts` compiles
 * and puts on `jna.library.path` before the test task runs: `cd apps/android && gradle test`.
 */
class ProfileTreeParityTest {

    private companion object {
        /**
         * `profile_tree::device_root_fixture_witness()` - the fixed demo wallet seed, as RAW ASCII
         * bytes. Rust's `build_profile_tree` takes `&[u8]`, but the FFI takes `seed_hex` and decodes
         * it, so Android must hand over the hex encoding of exactly these bytes. Passing the string
         * itself, or hashing it first, gives a different seed and a different (silently wrong) `R`.
         */
        const val FIXTURE_SEED = "DogTag demo wallet seed - TEST MATERIAL ONLY, never hold value"

        const val OWNER_ADDRESS = "0x00000000000000000000000000000000deadbeef"
        const val DOG_TAG_ID_DEC = "424242"

        /**
         * The fixture's `dogTagId` as the RAW integer field - `Fr::from(424242u64)` in
         * `device_root_fixture_witness()`, i.e. 424242 big-endian in 32 bytes.
         *
         * Deliberately NOT `dogTagIdFieldHex("424242")`. Those two are different values (see
         * `the fixture id is the raw integer field not the canonical dogTagId`), and the committed
         * `R` was built from THIS one. Reaching for the canonical helper here because it "looks more
         * correct" silently changes the witness and the root, which is exactly the confusion the
         * pinning test below exists to stop.
         */
        const val DOG_TAG_ID_FIELD =
            "0x0000000000000000000000000000000000000000000000000000000000067932"

        /** `TypeTag::String`, the tag the fixture's three attribute leaves carry. */
        val TAG_STRING: UByte = 2u

        /** `profile_tree::SALT_LEN` is 16 bytes; the fixture uses repeated-byte salts. */
        fun salt(byte: Int): String = "0x" + "%02x".format(byte).repeat(16)

        fun hex(s: String): String =
            "0x" + s.toByteArray(Charsets.UTF_8).joinToString("") { "%02x".format(it) }

        /**
         * Resolve a repo-relative path. `build.gradle.kts` passes the workspace root in, because the
         * unit-test working directory is the Gradle MODULE dir (`apps/android/app`) - an AGP detail,
         * not something a test should hard-code as a `../../..` climb.
         */
        fun repoFile(relative: String): File {
            val root = System.getProperty("dogtag.repoRoot")
            assertTrue(
                "dogtag.repoRoot is unset - the test task must set it (see app/build.gradle.kts)",
                !root.isNullOrBlank(),
            )
            return File(root, relative)
        }
    }

    /** The fixture's three attribute leaves, in `device_root_fixture_witness()` order. */
    private fun fixtureAttributes() = listOf(
        ProfileTreeBuilder.Attribute("credentialSubject.name", salt(7), TAG_STRING, "Rex"),
        ProfileTreeBuilder.Attribute("credentialSubject.breedLabel", salt(9), TAG_STRING, "Shiba Inu"),
        ProfileTreeBuilder.Attribute("credentialSubject.dateOfBirth", salt(11), TAG_STRING, "2021-04-02"),
    )

    private fun buildFixtureTree() = ProfileTreeBuilder.buildForIdField(
        seedHex = hex(FIXTURE_SEED),
        dogTagIdFieldHex = DOG_TAG_ID_FIELD,
        ownerAddress = OWNER_ADDRESS,
        attributes = fixtureAttributes(),
    )

    /**
     * Reads the fixture rather than hard-coding `R`, so regenerating it
     * (`cargo run -p dogtag-standard-rs --bin gen-device-profile-root`) moves this test with it -
     * a transcribed constant would silently keep asserting the stale root.
     */
    private fun fixture(): Map<String, String> {
        val f = repoFile("contracts/test/device-profile-root.json")
        assertTrue(
            "missing ${f.absolutePath} - regenerate with " +
                "`cargo run -p dogtag-standard-rs --bin gen-device-profile-root`",
            f.exists(),
        )
        // Read the fixture's flat string fields directly rather than through `org.json`: on the
        // unit-test classpath `org.json` is Android's STUB, whose every method throws "not mocked"
        // unless Robolectric supplies the real runtime. Pulling Robolectric in for four string
        // lookups would also make this test need the network on a cold Gradle cache (see the
        // QrPayloadTest note in build.gradle.kts), and the whole point of this one is that it is
        // pure JVM over the real Rust core.
        val fields = Regex(""""([^"]+)"\s*:\s*"([^"]*)"""")
            .findAll(f.readText())
            .associate { it.groupValues[1] to it.groupValues[2] }
        assertTrue("fixture parsed no fields from ${f.absolutePath}", fields.isNotEmpty())
        return fields
    }

    private fun Map<String, String>.field(name: String): String =
        this[name] ?: fail("fixture has no \"$name\" field").let { error("unreachable") }

    @Test
    fun `android builds the same R as the rust and ios reference`() {
        val f = fixture()
        val tree = buildFixtureTree()

        assertEquals(
            "Android-built R must equal the shared device-root fixture byte-for-byte",
            f.field("R").lowercase(),
            tree.rootHex.lowercase(),
        )
        // Guards the rest of the witness the fixture pins, so a wrong address or id cannot pass by
        // coincidence of the root alone.
        assertEquals(OWNER_ADDRESS.lowercase(), f.field("_ownerAddress").lowercase())
        assertEquals(DOG_TAG_ID_DEC, f.field("dogTagId"))
    }

    /**
     * The recovery round-trip, on the Android side: the owner-secret is a pure function of the seed
     * (bound to `dogTagId`), so re-deriving it from a restored seed rebuilds the identical `R`.
     *
     * This is what makes a lost device recoverable at all - see `docs/MOBILE_OWNER_SECRET.md`. If it
     * ever stops holding, an owner who restored their 24-word phrase would rebuild a DIFFERENT root
     * and their tag would be permanently unprovable, because `profileRoot` is write-once.
     */
    @Test
    fun `owner secret and R regenerate from the seed`() {
        val first = buildFixtureTree()
        val second = buildFixtureTree()

        assertEquals("same seed must rebuild the same R", first.rootHex, second.rootHex)
        assertEquals(first.ownerSecretHex, second.ownerSecretHex)

        // The standalone derivation agrees with the one the builder embedded, so a store that only
        // kept the secret can be checked against a rebuilt tree.
        assertEquals(
            first.ownerSecretHex,
            ProfileTreeBuilder.ownerSecret(hex(FIXTURE_SEED), DOG_TAG_ID_FIELD),
        )
    }

    /**
     * A DIFFERENT seed must not reproduce the tag - the negative leg of recoverability. Without this,
     * "regenerates from the seed" could pass trivially if the seed were being ignored.
     */
    @Test
    fun `a different seed rebuilds neither the secret nor R`() {
        val other = ProfileTreeBuilder.buildForIdField(
            seedHex = hex("a different wallet seed entirely - also TEST MATERIAL ONLY"),
            dogTagIdFieldHex = DOG_TAG_ID_FIELD,
            ownerAddress = OWNER_ADDRESS,
            attributes = fixtureAttributes(),
        )
        val fixtureTree = buildFixtureTree()
        assertNotEquals(fixtureTree.rootHex, other.rootHex)
        assertNotEquals(fixtureTree.ownerSecretHex, other.ownerSecretHex)
    }

    /**
     * The owner-secret and the consent key are bound to `dogTagId`, so one wallet's two tags get
     * INDEPENDENT owner-control material. That is what keeps a wallet's tags mutually unlinkable;
     * sharing either across tags would let one tag's nullifiers be correlated with another's.
     */
    @Test
    fun `two tags on one seed get independent owner control material`() {
        val a = buildFixtureTree()
        val b = ProfileTreeBuilder.build(
            seedHex = hex(FIXTURE_SEED),
            dogTagIdDec = "515151",
            ownerAddress = OWNER_ADDRESS,
            attributes = fixtureAttributes(),
        )
        assertNotEquals(a.ownerSecretHex, b.ownerSecretHex)
        assertNotEquals("owner.consentKey must differ per tag", a.keyHashHex, b.keyHashHex)
        assertNotEquals(a.rootHex, b.rootHex)
    }

    // ---- the single-owner-triple invariant (foundations P-e) -----------------------------------

    /**
     * The device must never build an `R` carrying a SECOND `(address, consentKey, secret)` reserved
     * triple: `consent.circom` proves inclusion of the three reserved leaves by pinned keyPath and
     * its soundness assumes each identifies a unique leaf.
     *
     * Asserted per keyPath rather than once, so dropping any single entry from
     * `RESERVED_KEY_PATHS` fails here instead of leaving one keyPath quietly unguarded.
     */
    @Test
    fun `a reserved keypath attribute is rejected before the tree is built`() {
        ProfileTreeBuilder.RESERVED_KEY_PATHS.forEach { reserved ->
            try {
                ProfileTreeBuilder.buildForIdField(
                    seedHex = hex(FIXTURE_SEED),
                    dogTagIdFieldHex = DOG_TAG_ID_FIELD,
                    ownerAddress = OWNER_ADDRESS,
                    attributes = fixtureAttributes() +
                        ProfileTreeBuilder.Attribute(reserved, salt(3), TAG_STRING, "smuggled"),
                )
                fail("reserved keyPath \"$reserved\" must be rejected")
            } catch (e: ProfileTreeBuilder.ReservedKeyPathException) {
                assertEquals(reserved, e.keyPath)
            }
        }
    }

    /**
     * Drift guard: the Kotlin reserved-keyPath list must be exactly the set Rust pins.
     *
     * The Kotlin guard is a fail-fast MIRROR of `build_profile_tree`'s own rejection, so a keyPath
     * that Rust reserves but Kotlin has forgotten is not a hole - the FFI still refuses it. What it
     * becomes is a confusing cross-FFI error instead of a named Kotlin one, and if Rust ever ADDS a
     * fourth reserved keyPath, a silently stale Kotlin list is how that would go unnoticed. Reading
     * the Rust source keeps the two in step rather than trusting a transcription.
     *
     * There is deliberately NO test for the guard's NFC normalization: all three reserved keyPaths
     * are pure ASCII, so no other string normalizes onto them and such a test could only assert a
     * tautology. The normalization is still correct and matches `leaf.rs:9` - it is what keeps the
     * guard sound if a reserved keyPath ever gains a composable character.
     */
    @Test
    fun `the kotlin reserved keypath list matches the rust constants`() {
        val src = repoFile("crates/dogtag-standard-rs/src/profile_tree.rs")
        assertTrue("missing ${src.absolutePath}", src.exists())
        val pinned = Regex("""pub const KP_[A-Z_]+: &str = "([^"]+)";""")
            .findAll(src.readText())
            .map { it.groupValues[1] }
            .toList()

        assertEquals("profile_tree.rs must still pin exactly three reserved keyPaths", 3, pinned.size)
        assertEquals(pinned.toSet(), ProfileTreeBuilder.RESERVED_KEY_PATHS.toSet())
    }

    /**
     * Pins a sharp edge in the SHARED fixture, so the next reader does not "fix" it into a bug.
     *
     * `device_root_fixture_witness()` binds the tree to `Fr::from(424242u64)` - the raw integer
     * field - whereas the production path (iOS `buildAndPersist`, Android [ProfileTreeBuilder.build])
     * binds to `dogTagIdFieldHex(dec)`, which folds the ENCODED decimal and is a different value.
     * The committed `R` therefore corresponds to the raw-field id, and a parity test that reached
     * for the canonical helper would compare against a root nothing ever built.
     *
     * This asserts the two encodings really are distinct, so if `dogTagIdFieldHex` ever became the
     * identity on integers this test - not a confusing root mismatch - is what reports it.
     */
    @Test
    fun `the fixture id is the raw integer field not the canonical dogTagId`() {
        val canonical = ProfileTreeBuilder.dogTagIdField(DOG_TAG_ID_DEC)
        assertNotEquals(
            "dogTagIdFieldHex folds the encoded decimal; the fixture uses the bare integer field",
            canonical.lowercase(),
            DOG_TAG_ID_FIELD.lowercase(),
        )
        // And the production path really does use the canonical encoding, so the two entry points
        // are not accidentally the same function.
        assertNotEquals(
            buildFixtureTree().rootHex,
            ProfileTreeBuilder.build(
                seedHex = hex(FIXTURE_SEED),
                dogTagIdDec = DOG_TAG_ID_DEC,
                ownerAddress = OWNER_ADDRESS,
                attributes = fixtureAttributes(),
            ).rootHex,
        )
    }

    /** The ordinary attribute leaves the fixture itself uses must NOT trip the guard. */
    @Test
    fun `ordinary attributes pass the guard`() {
        ProfileTreeBuilder.assertSingleOwnerTriple(fixtureAttributes())
    }
}
