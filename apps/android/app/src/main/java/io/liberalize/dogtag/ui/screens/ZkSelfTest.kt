package io.liberalize.dogtag.ui.screens

import android.content.Context
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.ZkeyAsset
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import uniffi.dogtag_standard.EddsaSigInput
import uniffi.dogtag_standard.bindConsentKeyDigestHex
import uniffi.dogtag_standard.keyHashHex
import uniffi.dogtag_standard.proveVerification
import uniffi.dogtag_standard.signConsentEddsa

/**
 * Debug-only ON-DEVICE ZK self-test — the mobile end-to-end check the audit could previously only
 * read (no device in the lab). It drives the SAME native code path the privacy-preserving groomer
 * export uses, end to end, with no camera / biometric / network: a fixed *imported record* (a
 * deterministic [io.liberalize.dogtag.data.Models]-shaped WrappedDoc) is signed, proved, and the
 * proof is checked against the server-recomputed public signals — all on the device's own arm64
 * native libraries (UniFFI → Rust SDK + circom-prover graph witness calculator + bundled zkey).
 *
 * The fixed vector (`assets/zk_selftest.json`) is produced by, and byte-for-byte mirrors,
 * `dogtag-standard-rs/tests/prove_parity.rs::fixed_prove_inputs` (regenerate via its
 * `dump_selftest_fixture` test), so the device proof MUST reproduce the same 7 public signals the
 * server SDK computes — and the on-chain `Groth16Verifier` was generated from the same vkey.
 *
 * Steps (each a real native call):
 *   1. `signConsentEddsa`     — EdDSA-BabyJubjub consent signature (consent signing). The circuit
 *                               re-verifies this signature as a constraint inside step 2's proof.
 *   2. `proveVerification`    — generate the Groth16 proof ON-DEVICE (graph witnesscalc + zkey).
 *   3. public-signal check    — proof's `pubSignals` == the server-recomputed expected vector, and
 *                               the 32-bit-regression guard (nullifier & keyHash non-zero). Matching
 *                               signals are themselves proof the consent signature verified.
 *   4. `keyHashHex` +
 *      `bindConsentKeyDigestHex` — derive the consent keyHash and the EIP-712 consent-key bind
 *                               digest (consent-key bind).
 *
 * The result line renders the stable text `ZK-SELFTEST: PASS` / `ZK-SELFTEST: FAIL` that the Maestro
 * flow (`apps/android/maestro/zk_e2e.yaml`) asserts on. Gated behind `BuildConfig.DEBUG` by its
 * caller so it never ships in a release build.
 */
@Composable
fun ZkSelfTestCard() {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var running by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<ZkSelfTestResult?>(null) }
    var status by remember { mutableStateOf("") }

    SectionTitle("Developer · ZK self-test")
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            "Runs the REAL on-device Groth16 prover (UniFFI → Rust circom-prover, graph witness " +
                "calculator + bundled proving key) over a fixed imported-record vector, then checks " +
                "the proof's public signals match the server-recomputed values. Debug builds only.",
            fontSize = 12.sp, color = c.muted,
        )
        Button(
            onClick = {
                running = true
                result = null
                status = "Starting…"
                scope.launch {
                    val r = withContext(Dispatchers.Default) {
                        runZkSelfTest(context) { s -> status = s }
                    }
                    result = r
                    running = false
                }
            },
            enabled = !running,
            modifier = Modifier.fillMaxWidth().testTag("zk_selftest_run"),
            colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
        ) { Text(if (running) "Running…" else "Run ZK self-test") }

        val r = result
        val headline = when {
            running -> "ZK-SELFTEST: RUNNING"
            r == null -> "ZK-SELFTEST: IDLE"
            r.pass -> "ZK-SELFTEST: PASS"
            else -> "ZK-SELFTEST: FAIL"
        }
        Text(
            headline,
            fontSize = 16.sp,
            fontWeight = FontWeight.Bold,
            color = when {
                r == null || running -> c.muted
                r.pass -> c.success
                else -> c.danger
            },
            modifier = Modifier.testTag("zk_selftest_result"),
        )
        val detail = r?.detail ?: status
        if (detail.isNotBlank()) {
            Text(
                detail,
                fontSize = 11.sp,
                fontFamily = FontFamily.Monospace,
                color = c.muted,
                modifier = Modifier.testTag("zk_selftest_detail"),
            )
        }
    }
}

private data class ZkSelfTestResult(val pass: Boolean, val detail: String)

/**
 * Execute the on-device ZK self-test off the main thread. Returns PASS only if every native step
 * succeeds AND the proof's public signals equal the server-recomputed expected vector.
 */
private fun runZkSelfTest(context: Context, onStatus: (String) -> Unit): ZkSelfTestResult = try {
    val fixture = JSONObject(
        context.assets.open("zk_selftest.json").bufferedReader().use { it.readText() },
    )
    val wrappedDocJson = fixture.getString("wrappedDocJson")
    val consentJson = fixture.getString("consentJson")
    val prvHex = fixture.getString("consentPrvHex")
    val axHex = fixture.getString("consentAxHex")
    val ayHex = fixture.getString("consentAyHex")
    val expected = fixture.getJSONArray("expectedPubSignals").let { arr ->
        ArrayList<String>(arr.length()).apply { for (i in 0 until arr.length()) add(arr.getString(i)) }
    }
    val cj = JSONObject(consentJson)
    fun field(k: String) = cj.getString(k)

    // 1. EdDSA-BabyJubjub consent signature (real native signing). The circuit verifies this
    //    signature as a constraint inside the proof below, so a proof whose public signals match the
    //    expected vector is itself proof that the signature was valid — no separate verify needed.
    onStatus("Signing EdDSA consent…")
    val sig = signConsentEddsa(
        prvHex,
        field("dogTagId"), field("recordType"), field("purpose"), field("credentialRoot"),
        field("challenge"), field("relayer"), field("subject"), field("nonce"), field("deadline"),
    )

    // 2. Generate the Groth16 proof ON-DEVICE (graph witness calculator + bundled zkey).
    onStatus("Materialising zkey + graph…")
    val zkeyPath = ZkeyAsset.ensure(context)
    val graphPath = ZkeyAsset.ensureGraph(context)
    onStatus("Generating Groth16 proof on-device…")
    val eddsaInput = EddsaSigInput(sig.r8xDec, sig.r8yDec, sig.sDec, axHex, ayHex)
    val proof = proveVerification(wrappedDocJson, consentJson, eddsaInput, zkeyPath, graphPath)

    // 3. The proof's public signals MUST equal the server-recomputed expected vector.
    if (proof.pubSignals.size != 7) {
        return ZkSelfTestResult(false, "expected 7 public signals, got ${proof.pubSignals.size}")
    }
    if (proof.pubSignals != expected) {
        val firstBad = proof.pubSignals.indices.firstOrNull { proof.pubSignals[it] != expected[it] } ?: -1
        return ZkSelfTestResult(false, "public-signal mismatch at index $firstBad")
    }
    // 32-bit witness regression guard (wasm2c zeroed the last-computed output wires).
    if (proof.pubSignals[4] == "0") return ZkSelfTestResult(false, "nullifier (pub[4]) is zero")
    if (proof.pubSignals[5] == "0") return ZkSelfTestResult(false, "keyHash (pub[5]) is zero")

    // 4. Consent-key bind: derive the keyHash and the EIP-712 bind digest (real native calls).
    onStatus("Deriving consent-key bind digest…")
    val keyHash = keyHashHex(axHex, ayHex)
    val roax = RoaxConfig.load(context)
    val bindDigest = bindConsentKeyDigestHex(
        roax.consentKeyRegistry, keyHash, field("subject"), 0u, roax.chainId.toULong(),
    )
    if (!bindDigest.startsWith("0x") || bindDigest.length != 66) {
        return ZkSelfTestResult(false, "bad consent-key bind digest: $bindDigest")
    }

    ZkSelfTestResult(
        true,
        "7/7 public signals match · nullifier+keyHash non-zero · " +
            "bind digest ${bindDigest.take(12)}… · prover=on-device(arm64)",
    )
} catch (t: Throwable) {
    ZkSelfTestResult(false, "exception: ${t::class.simpleName}: ${t.message}")
}
