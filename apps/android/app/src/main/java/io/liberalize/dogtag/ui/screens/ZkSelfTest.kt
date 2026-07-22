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
import io.liberalize.dogtag.data.ZkeyAsset
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import io.liberalize.dogtag.zk.PublicSignalIndex
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import uniffi.dogtag_standard.proveConsent

/** Debug-only device proof over the exact Rust `consent_prove_parity` fixture. */
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
            "Runs the real owner-hidden consent prover and checks all seven public signals against " +
                "the Rust parity vector. Debug builds only.",
            fontSize = 12.sp, color = c.muted,
        )
        Button(
            onClick = {
                running = true
                result = null
                status = "Starting…"
                scope.launch {
                    result = withContext(Dispatchers.Default) { runZkSelfTest(context) { status = it } }
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
        (r?.detail ?: status).takeIf { it.isNotBlank() }?.let {
            Text(
                it, fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted,
                modifier = Modifier.testTag("zk_selftest_detail"),
            )
        }
    }
}

private data class ZkSelfTestResult(val pass: Boolean, val detail: String)

private fun runZkSelfTest(context: Context, onStatus: (String) -> Unit): ZkSelfTestResult = try {
    val seedHex = "0x636f6e73656e74207061726974792077616c6c65742073656564202d2054455354204d4154455249414c204f4e4c592c206e6576657220686f6c642076616c7565"
    val attributes = JSONArray().apply {
        put(JSONObject().put("keyPath", "credentialSubject.name")
            .put("salt", "0x07070707070707070707070707070707").put("tag", 2).put("value", "Rex"))
        put(JSONObject().put("keyPath", "credentialSubject.breedLabel")
            .put("salt", "0x09090909090909090909090909090909").put("tag", 2).put("value", "Shiba Inu"))
    }.toString()
    val expected = listOf(
        "19282080935305080861096842252900215298393603684181619512414474199363734335896",
        "7",
        "97433442488726861213578988847752201310395502865",
        "10082827006016799336744122064490401655461844512429394449644692943092461376845",
        "17331201248350577047385658568212808264533738615016772120609589632882809234778",
        "19",
        "1893456000",
    )
    val word = { value: String -> "0x" + value.padStart(64, '0') }
    onStatus("Materialising consent artifacts…")
    val descriptor = ZkeyAsset.current()
    val zkeyPath = ZkeyAsset.ensure(context, descriptor)
    val graphPath = ZkeyAsset.ensureGraph(context, descriptor)
    onStatus("Generating owner-hidden proof on-device…")
    val proof = proveConsent(
        seedHex = seedHex,
        dogTagIdHandle = "424242",
        ownerAddressHex = "0x00000000000000000000000000000000deadbeef",
        attributesJson = attributes,
        purposeHex = word("7"),
        relayerHex = "0x" + "11".repeat(20),
        recordTypeHex = word("13"),
        consentNonceHex = word("63"),
        deadlineDec = "1893456000",
        zkeyPath = zkeyPath,
        graphPath = graphPath,
    )
    if (proof.pubSignals.size != PublicSignalIndex.COUNT) {
        ZkSelfTestResult(false, "expected 7 public signals, got ${proof.pubSignals.size}")
    } else if (proof.pubSignals != expected) {
        val bad = proof.pubSignals.indices.firstOrNull { proof.pubSignals[it] != expected[it] } ?: -1
        ZkSelfTestResult(false, "public-signal mismatch at index $bad")
    } else if (proof.pubSignals[PublicSignalIndex.NULLIFIER] == "0") {
        ZkSelfTestResult(false, "nullifier is zero")
    } else {
        ZkSelfTestResult(true, "7/7 consent public signals match · prover=on-device")
    }
} catch (t: Throwable) {
    ZkSelfTestResult(false, "exception: ${t::class.simpleName}: ${t.message}")
}
