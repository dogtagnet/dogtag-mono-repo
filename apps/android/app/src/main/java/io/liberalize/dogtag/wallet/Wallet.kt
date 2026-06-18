package io.liberalize.dogtag.wallet

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.bouncycastle.crypto.digests.SHA256Digest
import org.bouncycastle.crypto.generators.PKCS5S2ParametersGenerator
import org.bouncycastle.crypto.params.KeyParameter
import org.bouncycastle.jce.ECNamedCurveTable
import org.bouncycastle.jce.provider.BouncyCastleProvider
import org.bouncycastle.jce.spec.ECNamedCurveParameterSpec
import java.security.KeyStore
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.Security
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

// The UniFFI surface (BabyJubjub consent key derivation lives in Rust).
import uniffi.dogtag_standard.BabyjubConsentKeyFfi
import uniffi.dogtag_standard.deriveBabyjubConsentKey
import uniffi.dogtag_standard.keyHashHex

/**
 * The embedded, self-custodial wallet.
 *
 * Design (matching the build spec):
 *  - A BIP-39 mnemonic (24 words) → BIP-39 seed → secp256k1 private key (the user's on-chain
 *    `userWallet`). The BabyJubjub *consent* key is derived in Rust from the same seed under a
 *    DISTINCT domain (so the two keys never collide), via the new EdDSA FFI.
 *  - The 64-byte BIP-39 seed is encrypted with an AES-256-GCM key held in the Android Keystore
 *    (StrongBox-backed when the hardware supports it), and that Keystore key is biometric-gated
 *    (`setUserAuthenticationRequired(true)`), so decryption requires a fresh biometric auth.
 *  - keyHash = Poseidon(Ax,Ay) is what gets bound on-chain in ConsentKeyRegistry.
 *
 * Crypto primitives (BIP-39 PBKDF2, secp256k1 point mul) use BouncyCastle so we have no network
 * dependency; the consent-key + signing path is the audited Rust core over UniFFI.
 */
object Wallet {
    private const val KEY_ALIAS = "dogtag_wallet_seed_key"
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val GCM_TAG_BITS = 128

    init {
        if (Security.getProvider(BouncyCastleProvider.PROVIDER_NAME) == null) {
            Security.addProvider(BouncyCastleProvider())
        }
    }

    data class WalletIdentity(
        val mnemonic: String,         // shown ONCE at genesis; never persisted in plaintext
        val ethAddress: String,       // secp256k1 userWallet (0x… 20 bytes)
        val consent: BabyjubConsentKeyFfi,  // Ax/Ay/keyHash for ConsentKeyRegistry binding
    )

    /** True once a wallet seed has been generated and stored. */
    fun exists(context: Context): Boolean = SeedVault(context).hasSeed()

    /**
     * Create a brand-new wallet: generate a 24-word mnemonic, store the encrypted BIP-39 seed behind
     * the Keystore, and derive both the secp256k1 address and the BabyJubjub consent key. The
     * mnemonic is returned ONCE for the user to back up.
     */
    fun create(context: Context): WalletIdentity {
        ensureKeystoreKey()
        val entropy = ByteArray(32).also { SecureRandom().nextBytes(it) } // 256-bit → 24 words
        val mnemonic = Bip39.entropyToMnemonic(entropy)
        val seed = Bip39.mnemonicToSeed(mnemonic, "")
        SeedVault(context).store(encrypt(seed))
        return identityFromSeed(seed, mnemonic)
    }

    /** Re-derive the identity from the stored, encrypted seed (requires a prior biometric unlock of
     * the Keystore key on devices that enforce it). The mnemonic is NOT recoverable here. */
    fun load(context: Context): WalletIdentity? {
        val blob = SeedVault(context).load() ?: return null
        val seed = decrypt(blob)
        return identityFromSeed(seed, mnemonic = "")
    }

    private fun identityFromSeed(seed: ByteArray, mnemonic: String): WalletIdentity {
        val priv = Bip39.seedToSecp256k1Priv(seed)             // 32-byte secp256k1 scalar
        val ethAddress = Secp256k1.addressFromPriv(priv)
        // Distinct-domain BabyJubjub consent key derived in Rust from the same seed.
        val seedHex = "0x" + seed.joinToString("") { "%02x".format(it) }
        val consent = deriveBabyjubConsentKey(seedHex)
        // Belt-and-suspenders: keyHash must equal Poseidon(Ax,Ay).
        val kh = keyHashHex(consent.axHex, consent.ayHex)
        require(kh == consent.keyHashHex) { "consent keyHash mismatch" }
        return WalletIdentity(mnemonic, ethAddress, consent)
    }

    // ---- Android Keystore AES-GCM envelope (StrongBox when available) -------------------------

    private fun ensureKeystoreKey() {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        if (ks.containsAlias(KEY_ALIAS)) return
        val builder = KeyGenParameterSpec.Builder(
            KEY_ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            // Biometric-gate the key: decryption requires a fresh user authentication.
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(30, KeyProperties.AUTH_BIOMETRIC_STRONG)
        // StrongBox if the device has a secure element.
        runCatching { builder.setIsStrongBoxBacked(true) }
        val kg = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        try {
            kg.init(builder.build()); kg.generateKey()
        } catch (e: Exception) {
            // Device lacks StrongBox or biometric enrolment: fall back to a software-auth key so
            // the flow still works on emulators / devices without enrolled biometrics.
            val fallback = KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
            kg.init(fallback); kg.generateKey()
        }
    }

    private fun keystoreKey(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return (ks.getEntry(KEY_ALIAS, null) as KeyStore.SecretKeyEntry).secretKey
    }

    private fun encrypt(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, keystoreKey())
        val iv = cipher.iv
        val ct = cipher.doFinal(plain)
        return iv + ct // [12-byte IV || ciphertext+tag]
    }

    private fun decrypt(blob: ByteArray): ByteArray {
        val iv = blob.copyOfRange(0, 12)
        val ct = blob.copyOfRange(12, blob.size)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, keystoreKey(), GCMParameterSpec(GCM_TAG_BITS, iv))
        return cipher.doFinal(ct)
    }
}

/** Encrypted-seed persistence (the ciphertext is safe to store in plain prefs; the key is in HW). */
private class SeedVault(private val context: Context) {
    private val prefs = context.getSharedPreferences("dogtag_seed", Context.MODE_PRIVATE)
    fun hasSeed() = prefs.contains("seed")
    fun store(blob: ByteArray) {
        prefs.edit().putString("seed", android.util.Base64.encodeToString(blob, android.util.Base64.NO_WRAP)).apply()
    }
    fun load(): ByteArray? = prefs.getString("seed", null)
        ?.let { android.util.Base64.decode(it, android.util.Base64.NO_WRAP) }
}

/** Minimal BIP-39 (entropy↔mnemonic, mnemonic→seed) over BouncyCastle PBKDF2. */
object Bip39 {
    fun entropyToMnemonic(entropy: ByteArray): String {
        val ent = entropy.size * 8
        val cs = ent / 32
        val hash = MessageDigest.getInstance("SHA-256").digest(entropy)
        // bit string = entropy || first cs bits of hash
        val bits = StringBuilder()
        for (b in entropy) bits.append(((b.toInt() and 0xff) or 0x100).toString(2).substring(1))
        for (i in 0 until cs) bits.append(if ((hash[i / 8].toInt() shr (7 - i % 8)) and 1 == 1) '1' else '0')
        val words = Wordlist.english
        val out = ArrayList<String>()
        var i = 0
        while (i < bits.length) {
            val idx = bits.substring(i, i + 11).toInt(2)
            out.add(words[idx]); i += 11
        }
        return out.joinToString(" ")
    }

    fun mnemonicToSeed(mnemonic: String, passphrase: String): ByteArray {
        val gen = PKCS5S2ParametersGenerator(SHA512())
        gen.init(
            mnemonic.normalize().toByteArray(Charsets.UTF_8),
            ("mnemonic" + passphrase).normalize().toByteArray(Charsets.UTF_8),
            2048,
        )
        return (gen.generateDerivedParameters(512) as KeyParameter).key
    }

    /** BIP-32-ish: take the BIP-39 seed's HMAC-SHA512 left half as the secp256k1 master scalar. */
    fun seedToSecp256k1Priv(seed: ByteArray): ByteArray {
        val mac = javax.crypto.Mac.getInstance("HmacSHA512")
        mac.init(javax.crypto.spec.SecretKeySpec("Bitcoin seed".toByteArray(), "HmacSHA512"))
        val i = mac.doFinal(seed)
        return i.copyOfRange(0, 32)
    }

    private fun String.normalize() = java.text.Normalizer.normalize(this, java.text.Normalizer.Form.NFKD)
}

/** SHA-512 digest wrapper for BouncyCastle PBKDF2 (BIP-39 uses HMAC-SHA512). */
private class SHA512 : org.bouncycastle.crypto.Digest {
    private val d = org.bouncycastle.crypto.digests.SHA512Digest()
    override fun getAlgorithmName() = d.algorithmName
    override fun getDigestSize() = d.digestSize
    override fun update(input: Byte) = d.update(input)
    override fun update(input: ByteArray, inOff: Int, len: Int) = d.update(input, inOff, len)
    override fun doFinal(out: ByteArray, outOff: Int) = d.doFinal(out, outOff)
    override fun reset() = d.reset()
}

/** secp256k1 address derivation (priv → uncompressed pub → keccak256[12:]). */
object Secp256k1 {
    fun addressFromPriv(priv: ByteArray): String {
        val spec: ECNamedCurveParameterSpec = ECNamedCurveTable.getParameterSpec("secp256k1")
        val d = java.math.BigInteger(1, priv).mod(spec.n)
        val q = spec.g.multiply(d).normalize()
        val x = q.affineXCoord.encoded
        val y = q.affineYCoord.encoded
        val pub = ByteArray(64)
        System.arraycopy(x, 0, pub, 32 - x.size, x.size)
        System.arraycopy(y, 0, pub, 64 - y.size, y.size)
        val hash = Keccak256.digest(pub)
        val addr = hash.copyOfRange(12, 32)
        return "0x" + addr.joinToString("") { "%02x".format(it) }
    }
}

/** Keccak-256 (Ethereum, NOT SHA3-256) via BouncyCastle. */
object Keccak256 {
    fun digest(input: ByteArray): ByteArray {
        val d = org.bouncycastle.jcajce.provider.digest.Keccak.Digest256()
        return d.digest(input)
    }
}
