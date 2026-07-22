package io.liberalize.dogtag.profile

import java.text.Normalizer
import uniffi.dogtag_standard.AttributeLeafFfi
import uniffi.dogtag_standard.ProfileTreeFfi
import uniffi.dogtag_standard.buildProfileTreeHex
import uniffi.dogtag_standard.dogTagIdFieldHex
import uniffi.dogtag_standard.deriveOwnerSecretHex

/**
 * Device-side owner-hidden per-tag Merkle tree building, and the exact parity
 * counterpart of iOS `ProfileTreeStore`'s build path.
 *
 * The owner's app builds the tree LOCALLY and hands the issuer only the root `R`, which the issuer
 * seals into `DogTagSBTConsent.profileRoot(dogTagId)` (write-once). The owner's wallet never reaches
 * the chain. Everything else this returns - above all `ownerSecretHex`, the nullifier's secret leaf -
 * is owner-private and must never be transmitted: a server that learned it could link nullifiers
 * back to the owner, which the owner-hidden model prevents.
 *
 * **No crypto lives here.** The tree math, the Poseidon parameter set, the reserved-leaf encoding
 * and every KDF are in Rust (`crates/dogtag-standard-rs/src/profile_tree.rs`) and are reached
 * through the SAME `buildProfileTreeHex` FFI that iOS calls. That is what makes `R` byte-for-byte
 * identical across Android / iOS / Rust for the same inputs: both platforms are thin wrappers over
 * one compiled implementation, not two ports of one spec. This file's job is wiring - hex encoding,
 * the dec→field id conversion, attribute mapping - plus the fail-fast guard below.
 *
 * Kept free of `android.content.Context` on purpose so the cross-platform parity test can drive it
 * as a plain JVM unit test (`ProfileTreeParityTest`). Persistence lives next door in
 * [ProfileTreeStore], which is the piece that needs a `Context`.
 *
 * @see io.liberalize.dogtag.profile.ProfileTreeStore for the device-local owner-secret store
 * @see docs/MOBILE_OWNER_SECRET.md for the recovery contract this implements
 */
object ProfileTreeBuilder {

    /**
     * The three RESERVED owner-control keyPaths, pinned by `circuits/consent.circom:50-52` and
     * mirrored from `profile_tree::RESERVED_KEY_PATH_FIELDS`.
     *
     * The Rust core remains the authority - `build_profile_tree` rejects these itself, so this
     * constant cannot weaken the invariant even if it drifts. It exists to fail FAST, on the Android
     * side, with a message that names the offending keyPath.
     */
    val RESERVED_KEY_PATHS = listOf("owner.address", "owner.consentKey", "owner.secret")

    /** One attribute leaf, mirroring `AttributeLeafFfi` / iOS `BackedUpAttribute`. */
    data class Attribute(
        val keyPath: String,
        val saltHex: String,
        val tag: UByte,
        val value: String,
    )

    /**
     * Raised before the FFI is ever reached when a caller hands in an attribute that would build a
     * SECOND owner-control leaf. See [assertSingleOwnerTriple].
     */
    class ReservedKeyPathException(val keyPath: String) : IllegalArgumentException(
        "attribute keyPath \"$keyPath\" is reserved for the owner-control leaves " +
            "(owner.address / owner.consentKey / owner.secret); the consent circuit's soundness " +
            "assumes exactly ONE such triple in R",
    )

    /**
     * Enforce the **single-owner-triple invariant**: an `R` must contain exactly one
     * `(owner.address, owner.consentKey, owner.secret)` reserved triple.
     *
     * `consent.circom` proves inclusion of the three reserved leaves by their PINNED keyPaths, and
     * its soundness argument assumes those keyPaths identify a unique leaf each. An attribute
     * carrying a reserved keyPath with `typeTag = 5` (`RESERVED_TYPE_TAG`, i.e. `Bytes`) would build
     * a leaf the circuit is willing to accept as a reserved leaf, giving the tree a second triple the
     * owner controls independently - which defeats the pinning that makes the real one unique (D5).
     *
     * Compared on the NFC-normalized keyPath, not the raw string, because the leaf commits to
     * `field_of_keypath(kp)` = `bytes_to_field(nfc(kp))` (`crates/dogtag-standard-rs/src/leaf.rs:9`).
     * Two distinct Kotlin strings that NFC-normalize to the same text therefore build the SAME leaf,
     * and a naive `==` compare would wave one of them through.
     *
     * This mirrors the SDK guard rather than replacing it: `build_profile_tree` performs the same
     * rejection on the derived field and is the authority. Duplicating it here turns a
     * cross-FFI error into an immediate, well-named Kotlin exception at the call site.
     */
    fun assertSingleOwnerTriple(attributes: List<Attribute>) {
        val reserved = RESERVED_KEY_PATHS.map { nfc(it) }.toSet()
        attributes.forEach { a ->
            if (nfc(a.keyPath) in reserved) throw ReservedKeyPathException(a.keyPath)
        }
    }

    private fun nfc(s: String): String = Normalizer.normalize(s, Normalizer.Form.NFC)

    /**
     * Build the tag's profile tree on-device and return `R` plus the owner-private witness.
     *
     * Hand the issuer ONLY [ProfileTreeFfi.rootHex]; every other field is owner-private.
     *
     * `dogTagIdDec` is the human-facing decimal id - it is converted to the canonical dogTagId FIELD
     * here (`dogTagIdFieldHex`), which is what the tree is actually bound to. Passing an
     * already-hashed field value would bind the tree to the wrong id, so callers pass the decimal.
     *
     * @param seedHex the 64-byte BIP-39 seed as `0x…` hex (`Wallet.seedHex`)
     * @param ownerAddress the owner's 20-byte wallet address, `0x…` hex
     */
    fun build(
        seedHex: String,
        dogTagIdDec: String,
        ownerAddress: String,
        attributes: List<Attribute>,
    ): ProfileTreeFfi = buildForIdField(
        seedHex = seedHex,
        dogTagIdFieldHex = dogTagIdFieldHex(dogTagIdDec),
        ownerAddress = ownerAddress,
        attributes = attributes,
    )

    /**
     * Build for an ALREADY-canonical `dogTagId` field, the form the tree is actually bound to.
     *
     * This is the recovery entry point: a stored record keeps the canonical field
     * ([OwnerSecretRecord.dogTagIdHex]), so rebuilding from it must not re-run the decimal→field
     * conversion - doing that twice would bind the rebuilt tree to a different id and produce a
     * different `R`. iOS splits the same two ways (`buildAndPersist` converts, `verifyRecoverable`
     * passes the stored field through).
     *
     * Prefer [build] anywhere a human-facing decimal id is what you hold.
     */
    fun buildForIdField(
        seedHex: String,
        dogTagIdFieldHex: String,
        ownerAddress: String,
        attributes: List<Attribute>,
    ): ProfileTreeFfi {
        assertSingleOwnerTriple(attributes)
        return buildProfileTreeHex(
            seedHex,
            dogTagIdFieldHex,
            ownerAddress,
            attributes.map { AttributeLeafFfi(it.keyPath, it.saltHex, it.tag, it.value) },
        )
    }

    /** The canonical dogTagId field (`0x…` 32 bytes) a tree is bound to, for the decimal id. */
    fun dogTagIdField(dogTagIdDec: String): String = dogTagIdFieldHex(dogTagIdDec)

    /**
     * The seed-derived, `dogTagId`-bound owner-secret on its own - the recoverable nullifier secret.
     *
     * Deterministic: restoring the 24-word phrase regenerates the identical secret. Rebuilding the
     * same `R` additionally needs the owner address and the exact attribute values AND salts, which
     * are NOT seed-derivable (see `docs/MOBILE_OWNER_SECRET.md`).
     *
     * Owner-private. Never transmit it.
     */
    fun ownerSecret(seedHex: String, dogTagIdFieldHex: String): String =
        deriveOwnerSecretHex(seedHex, dogTagIdFieldHex)
}
