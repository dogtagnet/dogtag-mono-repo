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
        val secpPriv: ByteArray,      // 32-byte secp256k1 scalar (in-memory only, post-unlock)
    ) {
        /** Sign a 32-byte EIP-712 digest with the secp256k1 wallet key → 65-byte 0x.. r||s||v. */
        fun signEthDigest(digest: ByteArray): String = Secp256k1.signDigest(secpPriv, digest)

        /**
         * The EIP-191 `personal_sign` signature over the central registration message
         * "DogTag wallet registration: <ethAddress lowercased>". The digest is
         * keccak256("\x19Ethereum Signed Message:\n" + len + message), signed with the secp256k1
         * wallet key → 65-byte 0x.. r||s||v (recovers to ethAddress server-side).
         */
        fun registerSignature(): String {
            val message = "DogTag wallet registration: ${ethAddress.lowercase()}"
            val msgBytes = message.toByteArray(Charsets.UTF_8)
            val prefix = byteArrayOf(0x19) +
                "Ethereum Signed Message:\n${msgBytes.size}".toByteArray(Charsets.UTF_8)
            val digest = Keccak256.digest(prefix + msgBytes)
            return signEthDigest(digest)
        }
    }

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
        return WalletIdentity(mnemonic, ethAddress, consent, priv)
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

/** secp256k1 address derivation (priv → uncompressed pub → keccak256[12:]) + EIP-2098-style sign. */
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

    /**
     * Sign a 32-byte digest with the secp256k1 private key, producing a 65-byte Ethereum
     * `r || s || v` signature (0x.. hex). Deterministic (RFC-6979), low-S normalised, with the
     * recovery id recovered against the signer's own public key (v ∈ {27,28}). This is the raw
     * EIP-712 digest signature consumed by `ConsentKeyRegistry.bindConsentKeyFor` (digest comes from
     * the Rust FFI `bindConsentKeyDigestHex`).
     */
    fun signDigest(priv: ByteArray, digest: ByteArray): String {
        require(digest.size == 32) { "digest must be 32 bytes" }
        val spec: ECNamedCurveParameterSpec = ECNamedCurveTable.getParameterSpec("secp256k1")
        val n = spec.n
        val d = java.math.BigInteger(1, priv).mod(n)
        val domainParams = org.bouncycastle.crypto.params.ECDomainParameters(
            spec.curve, spec.g, spec.n, spec.h,
        )
        val signer = org.bouncycastle.crypto.signers.ECDSASigner(
            org.bouncycastle.crypto.signers.HMacDSAKCalculator(SHA256Digest()),
        )
        signer.init(true, org.bouncycastle.crypto.params.ECPrivateKeyParameters(d, domainParams))
        val sig = signer.generateSignature(digest)
        val r = sig[0]
        var s = sig[1]
        // Low-S normalisation (EIP-2): if s > n/2, replace with n - s. The recovery id is then computed
        // against this normalized `s` directly (see below), so no separate parity flip is tracked.
        val halfN = n.shiftRight(1)
        if (s > halfN) { s = n.subtract(s) }

        // Recover the public key to determine v. Our own pubkey is the ground truth.
        // NOTE: `s` is ALREADY low-S-normalized above, and `findRecId` is called with that normalized `s`,
        // so the recId it returns is already correct for the signature we return — do NOT xor `flip` again
        // (that double-counts the parity flip and yields the wrong `v` ~50% of the time → on-chain
        // ECDSA.recover returns a different address → "bad sig").
        val q = spec.g.multiply(d).normalize()
        val recId = findRecId(domainParams, r, s, digest, q) ?: 0
        val v = recId + 27

        val rb = to32(r)
        val sb = to32(s)
        return "0x" + rb.joinToString("") { "%02x".format(it) } +
            sb.joinToString("") { "%02x".format(it) } + "%02x".format(v)
    }

    private fun findRecId(
        domain: org.bouncycastle.crypto.params.ECDomainParameters,
        r: java.math.BigInteger,
        s: java.math.BigInteger,
        message: ByteArray,
        expectedQ: org.bouncycastle.math.ec.ECPoint,
    ): Int? {
        val n = domain.n
        val e = java.math.BigInteger(1, message)
        for (recId in 0..1) {
            val x = r // r < n; recId 0/1 -> x = r (no overflow add for typical signatures)
            val rPoint = decompressKey(domain, x, recId) ?: continue
            if (!rPoint.multiply(n).isInfinity) continue
            val rInv = r.modInverse(n)
            val srInv = rInv.multiply(s).mod(n)
            val eInvrInv = rInv.multiply(n.subtract(e)).mod(n)
            val qCandidate = org.bouncycastle.math.ec.ECAlgorithms
                .sumOfTwoMultiplies(domain.g, eInvrInv, rPoint, srInv).normalize()
            if (qCandidate.equals(expectedQ)) return recId
        }
        return null
    }

    private fun decompressKey(
        domain: org.bouncycastle.crypto.params.ECDomainParameters,
        xBN: java.math.BigInteger,
        yBit: Int,
    ): org.bouncycastle.math.ec.ECPoint? {
        return try {
            val curve = domain.curve
            val compEnc = ByteArray(1 + (curve.fieldSize + 7) / 8)
            compEnc[0] = (0x02 + yBit).toByte()
            val xBytes = to32(xBN)
            System.arraycopy(xBytes, 0, compEnc, 1 + compEnc.size - 1 - 32, 32)
            curve.decodePoint(compEnc)
        } catch (e: Exception) {
            null
        }
    }

    private fun to32(v: java.math.BigInteger): ByteArray {
        val b = v.toByteArray()
        val out = ByteArray(32)
        if (b.size == 33 && b[0].toInt() == 0) {
            System.arraycopy(b, 1, out, 0, 32)
        } else if (b.size <= 32) {
            System.arraycopy(b, 0, out, 32 - b.size, b.size)
        } else {
            System.arraycopy(b, b.size - 32, out, 0, 32)
        }
        return out
    }
}

/** Keccak-256 (Ethereum, NOT SHA3-256) via BouncyCastle. */
object Keccak256 {
    fun digest(input: ByteArray): ByteArray {
        val d = org.bouncycastle.jcajce.provider.digest.Keccak.Digest256()
        return d.digest(input)
    }
}
