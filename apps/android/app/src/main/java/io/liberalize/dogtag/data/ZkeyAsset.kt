package io.liberalize.dogtag.data

import android.content.Context
import java.io.File

/**
 * The proving artifacts ONE protocol version needs, named by their asset filenames.
 *
 * Mirrors the Rust `dogtag_prover::artifact::ArtifactDescriptor` (and iOS `ArtifactDescriptor`) —
 * the same version-keyed model, expressed per platform. Only the fields this resolver needs to
 * materialise a file are here: the server-side descriptor additionally carries the r1cs/wasm the ark
 * backend uses (this app's witness backend is the graph interpreter) and the integrity pins.
 *
 * `zkeySha256`/`graphSha256` are deliberately absent: a bundled asset ships inside the signed APK,
 * so its integrity is the package signature's job, not a hash check here. A version whose artifacts
 * are FETCHED must pin and verify them before load — see the resolver docs.
 */
data class ArtifactDescriptor(
    /** The version key — what the protocol calls this artifact set. */
    val version: String,
    /** The Groth16 proving key asset (~65 MB). */
    val zkeyAsset: String,
    /** The precompiled witness graph asset (~3 MB) the on-device prover interprets. */
    val graphAsset: String,
)

/**
 * Resolves the on-device prover assets for a protocol version — the Groth16 proving key and the
 * witness graph — to absolute filesystem paths so the consent prover can read them by
 * path.
 *
 * ## Version-keyed, not fetched
 *
 * Which files a version needs comes from its [ArtifactDescriptor] ([REGISTRY]) rather than from
 * hard-coded filenames, so this app can hold more than one artifact set. What it does NOT do is
 * fetch: every registered version resolves to assets bundled in the APK, exactly as before. Making a
 * version resolvable from the network — download, verify its published SHA-256 BEFORE load, cache it
 * content-addressed — is the later discovery work, and [ensure] is the seam it plugs into.
 *
 * ## Materialising
 *
 * Bundled assets ship uncompressed (see `androidResources { noCompress += "zkey"/"graph" }` in
 * `app/build.gradle.kts`) but live inside the APK, so they have no filesystem path until copied. On
 * first use each is copied once into `filesDir`; subsequent calls return the cached path without
 * re-copying (size-matched). The copy runs off the main thread — call it from a background
 * dispatcher.
 *
 * Mirrors iOS `ZkeyAsset.swift`, which resolves the same descriptors from `Bundle.main` (bundled iOS
 * resources already have a path, so it needs no copy).
 */
object ZkeyAsset {
    /**
     * The one owner-hidden consent artifact set. The compatibility version key is intentionally
     * retained even though the old level names are no longer product or app vocabulary.
     */
    val OWNER_HIDDEN_V1 = ArtifactDescriptor(
        version = "dogtag-levelb/1",
        zkeyAsset = "consent_final.zkey",
        graphAsset = "consent.graph",
    )

    /** Every version this build can prove. There is exactly one owner-hidden protocol. */
    val REGISTRY: List<ArtifactDescriptor> = listOf(OWNER_HIDDEN_V1)

    /** The version a caller gets when it names none. */
    fun current(): ArtifactDescriptor = OWNER_HIDDEN_V1

    /**
     * Resolve a version key to its artifact set.
     *
     * `null` resolves to [current]. An unknown version throws rather than falling back — proving
     * with another version's key yields a
     * proof its verifier rejects, so failing closed beats answering wrongly.
     */
    fun resolve(version: String? = null): ArtifactDescriptor {
        if (version == null) return current()
        return REGISTRY.firstOrNull { it.version == version }
            ?: throw IllegalArgumentException(
                "unknown protocol version '$version' (known: ${REGISTRY.joinToString { it.version }})"
            )
    }

    /**
     * Copy [descriptor]'s zkey asset into `filesDir` (once) and return its absolute path. Idempotent:
     * if a same-size file already exists it is reused. Throws if the asset is missing.
     */
    fun ensure(context: Context, descriptor: ArtifactDescriptor = current()): String =
        materialise(context, descriptor.zkeyAsset)

    /**
     * Copy [descriptor]'s witness-graph asset into `filesDir` (once) and return its absolute path.
     * Idempotent (size-matched), same contract as [ensure].
     */
    fun ensureGraph(context: Context, descriptor: ArtifactDescriptor = current()): String =
        materialise(context, descriptor.graphAsset)

    private fun materialise(context: Context, assetName: String): String {
        val dest = File(context.filesDir, assetName)
        val expected = runCatching {
            context.assets.openFd(assetName).use { it.length }
        }.getOrNull()
        if (dest.exists() && expected != null && dest.length() == expected) {
            return dest.absolutePath
        }
        context.assets.open(assetName).use { input ->
            dest.outputStream().use { output -> input.copyTo(output, 1 shl 16) }
        }
        return dest.absolutePath
    }
}
