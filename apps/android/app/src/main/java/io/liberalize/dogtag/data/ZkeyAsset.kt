package io.liberalize.dogtag.data

import android.content.Context
import java.io.File

/**
 * Materialises the bundled on-device prover assets — the Groth16 proving key
 * (`verification_final.zkey`, ~65 MB) and the witness graph (`verification.graph`, ~3 MB) — onto the
 * filesystem so the prover (`proveVerification`) can read them as absolute file paths.
 *
 * Both ship as uncompressed assets (see `androidResources { noCompress += "zkey"/"graph" }` in
 * `app/build.gradle.kts`). On first use each is copied once into `filesDir`; subsequent calls return
 * the cached path without re-copying (size-matched). The copy runs off the main thread — call it
 * from a background dispatcher.
 */
object ZkeyAsset {
    private const val ZKEY_ASSET = "verification_final.zkey"
    private const val GRAPH_ASSET = "verification.graph"

    /**
     * Copy the bundled zkey asset into `filesDir` (once) and return its absolute path. Idempotent:
     * if a same-size file already exists it is reused. Throws if the asset is missing.
     */
    fun ensure(context: Context): String = materialise(context, ZKEY_ASSET)

    /**
     * Copy the bundled witness-graph asset into `filesDir` (once) and return its absolute path.
     * Idempotent (size-matched), same contract as [ensure].
     */
    fun ensureGraph(context: Context): String = materialise(context, GRAPH_ASSET)

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
