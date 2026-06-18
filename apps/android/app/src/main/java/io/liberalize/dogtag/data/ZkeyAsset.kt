package io.liberalize.dogtag.data

import android.content.Context
import java.io.File

/**
 * Materialises the bundled Groth16 proving key (`verification_final.zkey`, ~65 MB) onto the
 * filesystem so the on-device prover (`proveVerification`) can read it as an absolute file path.
 *
 * The zkey ships as an uncompressed asset (see `androidResources { noCompress += "zkey" }` in
 * `app/build.gradle.kts`). On first use it is copied once into `filesDir`; subsequent calls return
 * the cached path without re-copying (size-matched). The copy runs off the main thread — call it
 * from a background dispatcher.
 */
object ZkeyAsset {
    private const val ASSET_NAME = "verification_final.zkey"

    /**
     * Copy the bundled zkey asset into `filesDir` (once) and return its absolute path. Idempotent:
     * if a same-size file already exists it is reused. Throws if the asset is missing.
     */
    fun ensure(context: Context): String {
        val dest = File(context.filesDir, ASSET_NAME)
        val expected = runCatching {
            context.assets.openFd(ASSET_NAME).use { it.length }
        }.getOrNull()
        if (dest.exists() && expected != null && dest.length() == expected) {
            return dest.absolutePath
        }
        context.assets.open(ASSET_NAME).use { input ->
            dest.outputStream().use { output -> input.copyTo(output, 1 shl 16) }
        }
        return dest.absolutePath
    }
}
