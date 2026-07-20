package io.liberalize.dogtag.profile

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import io.liberalize.dogtag.wallet.SeedBackup
import java.io.File
import java.io.FileOutputStream
import java.security.KeyStore
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.TimeZone
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.dogtag_standard.ProfileTreeFfi

/**
 * The recoverable owner-secret's device-local store - the Android counterpart of iOS
 * `ProfileTreeStore` (Level-B M5 / M-2b).
 *
 * `<noBackupFilesDir>/dogtag-owner-secrets.json.enc` is a DEVICE-LOCAL store, not a cross-device
 * backup. It holds the owner-secret for every Level-B tag on the device: anyone who reads it can
 * generate that tag's proofs. It is never uploaded, never logged, and never leaves the device.
 *
 * # Why the file looks different from iOS's, and why that is still parity
 *
 * iOS reaches "encrypted at rest + excluded from device backups" with `.completeFileProtection` and
 * `isExcludedFromBackup`. Neither exists on Android, so this mirrors the CONTRACT with the platform's
 * own mechanisms - copying the Foundation ceremony would be a translation, not a guarantee:
 *
 *  - **Excluded from backups:** written under [Context.getNoBackupFilesDir], which Android's
 *    auto-backup and D2D transfer both skip. The manifest sets `allowBackup="true"` for the rest of
 *    the app, so relying on that flag would be wrong here; `noBackupFilesDir` is excluded regardless
 *    of it, which is the property we actually need.
 *  - **Encrypted at rest:** AES-256-GCM under a hardware-backed Android Keystore key, matching the
 *    envelope `Wallet`'s `SeedVault` already uses for the seed. The key is generated through the
 *    tiered ladder in [ensureKey], whose top two tiers additionally require an UNLOCKED device -
 *    the direct analogue of `.completeFileProtection`. That gate is only surrendered on the last
 *    tier (API < 28, or a device that rejects it), which is logged rather than silently accepted.
 *
 * The key is deliberately NOT `setUserAuthenticationRequired`, unlike the wallet seed's: iOS's
 * `.completeFileProtection` gates on device lock, not on a fresh biometric per read, and this store
 * is read on paths (listing a tag, rebuilding a proof) where a biometric prompt per access would be
 * a UX difference from iOS rather than a security parity with it. The seed itself keeps its stronger,
 * auth-gated key.
 *
 * # Recovery: the seed, plus the credential - never this file
 *
 * The file never leaves the device, so it cannot carry a tag to a replacement one. Cross-device
 * recovery rests on two inputs, BOTH required:
 *
 *  1. **The wallet seed (the 24-word phrase)** regenerates the owner-control core - owner-secret,
 *     consent key, reserved-leaf salts - bound to `dogTagId`. It is the ONLY cross-device path to
 *     the owner-secret, which is why [buildAndPersist] refuses to run until the phrase is confirmed
 *     backed up.
 *  2. **The credential's attribute leaves.** Values AND salts are caller-supplied and NOT
 *     seed-derivable; they come back from the issued wrapped credential.
 *
 * When both are gone the owner-secret is gone for good and there is no on-chain repair
 * (`profileRoot` is write-once); per decision D3 the remedy is a re-issue under a NEW `dogTagId`.
 *
 * @see docs/MOBILE_OWNER_SECRET.md for the documented contract
 */
class ProfileTreeStore(private val context: Context) {

    companion object {
        const val FILE_NAME = "dogtag-owner-secrets.json.enc"

        /** The outgoing store, parked by [write] and recovered by [load]. */
        const val BACKUP_FILE_NAME = "$FILE_NAME.bak"

        private const val TAG = "ProfileTreeStore"

        /**
         * Stamped into every record so a future KDF change is detectable rather than silently
         * producing a different `R`. Must track `profile_tree::OWNER_SECRET_DOMAIN`.
         */
        const val DERIVATION_VERSION = "DogTag/owner-secret/v1"

        /**
         * Serialises read-modify-write across ALL instances, mirroring iOS's static `storeLock`.
         * `ProfileTreeStore(context)` is constructed per call site, so locking on `this` would let
         * two instances interleave a load/upsert/write and lose one tag's record.
         */
        private val storeLock = Any()

        private const val KEY_ALIAS = "dogtag_owner_secrets_key"
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val GCM_TAG_BITS = 128
        private const val IV_LEN = 12

        private fun nowIso8601(): String =
            SimpleDateFormat("yyyy-MM-dd'T'HH:mm:ss'Z'", Locale.US)
                .apply { timeZone = TimeZone.getTimeZone("UTC") }
                .format(Date())
    }

    /** The wallet phrase has not been confirmed backed up; creating a secret now could lose a tag. */
    class SeedBackupNotConfirmedException : IllegalStateException(
        "the wallet recovery phrase has not been confirmed as backed up; refusing to create an " +
            "owner-secret that a lost phone would destroy permanently",
    )

    /** The store exists but could not be read. Never reported as "no records" - see [load]. */
    class UnreadableStoreException(cause: Throwable) : IllegalStateException(
        "$FILE_NAME exists but could not be read; refusing to overwrite it (it holds recovery " +
            "secrets): ${cause.message}",
        cause,
    )

    /** Rebuilding the tree no longer reproduces the recorded `R`. See [verifyRecoverable]. */
    class RootMismatchException(expected: String, got: String) : IllegalStateException(
        "rebuilt R $got != recorded R $expected",
    )

    private val file: File get() = File(context.noBackupFilesDir, FILE_NAME)

    /** Where [write] parks the outgoing store so no window holds zero copies. */
    private val backupFile: File get() = File(context.noBackupFilesDir, BACKUP_FILE_NAME)

    // ---- build ---------------------------------------------------------------------------------

    /**
     * Build the tag's tree on-device from the wallet seed and persist the recovery record.
     *
     * Returns the full owner-private witness; hand the issuer ONLY [ProfileTreeFfi.rootHex].
     *
     * Throws [SeedBackupNotConfirmedException] unless the user has confirmed they backed up their
     * recovery phrase ([SeedBackup]). That gate is the point: this store is excluded from device
     * backups, so the phrase is the ONLY thing that can regenerate this owner-secret on a
     * replacement phone. Creating one without it would let a lost phone permanently destroy the tag,
     * silently and with no on-chain remedy. Call `SeedBackup.confirm` from the phrase-backup UX
     * first. Mirrors the same gate on iOS `buildAndPersist`.
     */
    fun buildAndPersist(
        seedHex: String,
        dogTagIdDec: String,
        ownerAddress: String,
        attributes: List<BackedUpAttribute>,
    ): ProfileTreeFfi {
        if (!SeedBackup.isConfirmed(context, seedHex)) throw SeedBackupNotConfirmedException()

        val dogTagIdHex = ProfileTreeBuilder.dogTagIdField(dogTagIdDec)
        val tree = ProfileTreeBuilder.buildForIdField(
            seedHex = seedHex,
            dogTagIdFieldHex = dogTagIdHex,
            ownerAddress = ownerAddress,
            attributes = attributes.map { it.toAttribute() },
        )
        upsert(
            OwnerSecretRecord(
                dogTagIdHex = dogTagIdHex,
                dogTagIdDec = dogTagIdDec,
                ownerSecretHex = tree.ownerSecretHex,
                rootHex = tree.rootHex,
                ownerAddress = ownerAddress,
                attributes = attributes,
                derivationVersion = DERIVATION_VERSION,
                savedAt = nowIso8601(),
            ),
        )
        return tree
    }

    // ---- recover -------------------------------------------------------------------------------

    /**
     * Rebuild a tag's `R` from the wallet seed + the record's attributes and assert it still equals
     * the recorded `R` - the recovery round-trip, on-device.
     *
     * Throws [RootMismatchException] if the tree no longer reproduces, which would mean the tag's
     * proofs can never verify again (`profileRoot` is write-once, so `R` cannot be moved).
     */
    fun verifyRecoverable(seedHex: String, record: OwnerSecretRecord): String {
        val tree = ProfileTreeBuilder.buildForIdField(
            seedHex = seedHex,
            // The record stores the CANONICAL field; converting again would bind to a different id.
            dogTagIdFieldHex = record.dogTagIdHex,
            ownerAddress = record.ownerAddress,
            attributes = record.attributes.map { it.toAttribute() },
        )
        if (!tree.rootHex.equals(record.rootHex, ignoreCase = true)) {
            throw RootMismatchException(record.rootHex, tree.rootHex)
        }
        return tree.rootHex
    }

    // ---- persistence ---------------------------------------------------------------------------

    /**
     * The only legitimately empty state is "no file yet".
     *
     * An existing-but-undecryptable file (corrupt, written by a newer schema, or encrypted under a
     * Keystore key that was invalidated) throws [UnreadableStoreException] rather than reporting zero
     * records, so [upsert] cannot rebuild an empty list over it and wipe every other tag's attribute
     * salts - which are not seed-derivable and therefore exist nowhere else.
     */
    fun load(): List<OwnerSecretRecord> = synchronized(storeLock) {
        val f = file
        if (!f.exists()) {
            // A write that died between parking the old store aside and committing the new one
            // leaves the only copy in the backup. Reporting that as "no records" would let [upsert]
            // rebuild an empty list over it, so recover it before deciding the store is absent.
            val bak = backupFile
            if (!bak.exists()) return emptyList()
            if (!bak.renameTo(f)) {
                throw UnreadableStoreException(
                    IllegalStateException("could not restore ${bak.name} after an interrupted write"),
                )
            }
        }
        try {
            OwnerSecretRecords.decode(String(decrypt(f.readBytes()), Charsets.UTF_8))
        } catch (e: Exception) {
            throw UnreadableStoreException(e)
        }
    }

    /** Records, or an empty list if the store cannot be read. Use [load] where failure must surface. */
    fun all(): List<OwnerSecretRecord> = runCatching { load() }.getOrDefault(emptyList())

    fun record(dogTagIdDec: String): OwnerSecretRecord? = all().firstOrNull { it.dogTagIdDec == dogTagIdDec }

    fun upsert(record: OwnerSecretRecord) = synchronized(storeLock) {
        write(OwnerSecretRecords.upsert(load(), record))
    }

    /**
     * Replace the store without ever passing through a state that holds zero readable copies of it -
     * the attribute salts in here exist nowhere else on the device, and `profileRoot` is write-once
     * on-chain, so losing them strands every tag they belong to.
     *
     * The sequence is: write the new contents to a staging sibling and `fsync` it (a bare
     * `writeBytes` only reaches the page cache, so a rename could otherwise be durable while the
     * bytes it points at are not); park the current store as [backupFile] rather than deleting it;
     * rename staging into place; and only then drop the backup. [backupFile] is the recoverable copy:
     * any failure restores it, and a crash inside the window where only it exists is repaired by
     * [load]. No failure path may delete it - it is a copy of a store this call did not create, so
     * dropping it could cost the only surviving one. The staging file is the opposite case and is
     * dropped on every path: this call created it, it replaced nothing, and nothing ever reads it
     * back, so leaving it behind would only accumulate whole encrypted copies of the store in a
     * directory nothing sweeps.
     */
    private fun write(records: List<OwnerSecretRecord>) {
        val dir = context.noBackupFilesDir
        val dest = file
        val bak = backupFile
        val staging = File.createTempFile(".$FILE_NAME", ".tmp", dir)
        var parked = false
        var committed = false
        try {
            val bytes = encrypt(OwnerSecretRecords.encode(records).toByteArray(Charsets.UTF_8))
            FileOutputStream(staging).use { out ->
                out.write(bytes)
                out.flush()
                out.fd.sync()
            }
            if (dest.exists()) {
                bak.delete()
                // Refuse rather than delete: an unparked destination is the only copy there is.
                check(dest.renameTo(bak)) { "could not stage a backup of $FILE_NAME before replacing it" }
                parked = true
            }
            check(staging.renameTo(dest)) { "could not commit $FILE_NAME" }
            committed = true
        } finally {
            if (committed) {
                bak.delete()
            } else if (parked && !dest.exists()) {
                bak.renameTo(dest)
            }
            if (staging.exists()) staging.delete()
        }
    }

    // ---- Android Keystore AES-GCM envelope -----------------------------------------------------

    /** One rung of [ensureKey]'s ladder: what it asks the Keystore for, and what to call it. */
    private data class KeyTier(val label: String, val strongBox: Boolean, val unlockedRequired: Boolean)

    private fun keySpec(tier: KeyTier): KeyGenParameterSpec {
        val builder = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            if (tier.unlockedRequired) builder.setUnlockedDeviceRequired(true)
            if (tier.strongBox) builder.setIsStrongBoxBacked(true)
        }
        return builder.build()
    }

    /**
     * Generate the store key, surrendering ONE capability per rung so a device only loses what it
     * actually cannot provide.
     *
     * StrongBox and the unlocked-device gate are independent: `setIsStrongBoxBacked(true)` throws
     * `StrongBoxUnavailableException` on every device without a secure element and on all emulators,
     * which is common, while `setUnlockedDeviceRequired(true)` is the `.completeFileProtection`
     * analogue this store's whole at-rest story rests on. Collapsing both into a single catch-all
     * fallback - as the first cut did - dropped the unlocked gate on every non-StrongBox device,
     * encrypting recovery secrets under a key usable while the phone is locked. So the ladder is
     * (StrongBox + unlocked) -> (unlocked) -> (plain), and reaching the plain rung is logged rather
     * than accepted in silence.
     */
    private fun ensureKey() {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (ks.containsAlias(KEY_ALIAS)) return
        val tiers = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            listOf(
                KeyTier("StrongBox + unlocked-device-required", strongBox = true, unlockedRequired = true),
                KeyTier("unlocked-device-required", strongBox = false, unlockedRequired = true),
                KeyTier("software-backed, no unlocked-device gate", strongBox = false, unlockedRequired = false),
            )
        } else {
            // Both capabilities are API 28+; on 26-27 the plain rung is the only one that exists.
            listOf(KeyTier("software-backed, no unlocked-device gate (API < 28)", false, false))
        }
        val kg = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        var last: Exception? = null
        for (tier in tiers) {
            try {
                kg.init(keySpec(tier)); kg.generateKey()
            } catch (e: Exception) {
                last = e
                continue
            }
            if (!tier.unlockedRequired) {
                Log.w(
                    TAG,
                    "$FILE_NAME key created as ${tier.label}: this device cannot enforce the " +
                        "unlocked-device gate, so the store is decryptable while the screen is locked",
                )
            }
            return
        }
        throw IllegalStateException("could not create the $FILE_NAME Keystore key", last)
    }

    private fun key(): SecretKey {
        ensureKey()
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (ks.getEntry(KEY_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
    }

    private fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        return cipher.iv + cipher.doFinal(plain) // [12-byte IV || ciphertext+tag]
    }

    private fun decrypt(blob: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(),
            GCMParameterSpec(GCM_TAG_BITS, blob.copyOfRange(0, IV_LEN)),
        )
        return cipher.doFinal(blob.copyOfRange(IV_LEN, blob.size))
    }
}

private fun BackedUpAttribute.toAttribute() =
    ProfileTreeBuilder.Attribute(keyPath, saltHex, tag, value)
