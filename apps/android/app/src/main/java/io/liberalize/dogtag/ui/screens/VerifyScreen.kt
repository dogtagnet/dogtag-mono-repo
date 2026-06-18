package io.liberalize.dogtag.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.ui.DogTagTheme
import org.json.JSONObject
import uniffi.dogtag_standard.buildMerkleRootHex
import uniffi.dogtag_standard.hashLeafHex

private data class ParityResult(val pass: Boolean, val leaf: Int, val merkle: Int, val detail: String)

private fun runParity(json: String): ParityResult = try {
    val root = JSONObject(json)
    val leaves = root.getJSONArray("leaves")
    var lc = 0
    for (i in 0 until leaves.length()) {
        val v = leaves.getJSONObject(i)
        val got = hashLeafHex(
            v.getString("keyPath"), v.getString("saltHex"), v.getInt("tag").toUByte(),
            if (v.isNull("value")) "" else v.getString("value"),
        )
        if (got != v.getString("expected_hex")) {
            return ParityResult(false, lc, 0, "leaf mismatch '${v.getString("name")}'")
        }
        lc++
    }
    val merkle = root.getJSONArray("merkle")
    var mc = 0
    for (i in 0 until merkle.length()) {
        val m = merkle.getJSONObject(i)
        val arr = m.getJSONArray("leaf_hexes")
        val hexes = ArrayList<String>(arr.length())
        for (j in 0 until arr.length()) hexes.add(arr.getString(j))
        if (buildMerkleRootHex(hexes) != m.getString("root_hex")) {
            return ParityResult(false, lc, mc, "root mismatch '${m.getString("name")}'")
        }
        mc++
    }
    ParityResult(true, lc, mc, "all leaves + roots match the server vectors")
} catch (t: Throwable) {
    ParityResult(false, 0, 0, "exception: ${t.message}")
}

/**
 * The Verify tab. The user app SCANS — it never displays a QR. The primary action ("Scan a QR") opens
 * the unified [ScanScreen], which routes to import-a-record or present-a-record (consent) by the QR
 * shape. The trust-core parity panel below is a real diagnostic (mobile Merkle root == server root).
 */
@Composable
fun VerifyScreen(activity: FragmentActivity, onScan: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scroll = rememberScrollState()

    val parity = remember {
        runParity(context.assets.open("testvectors.json").bufferedReader().use { it.readText() })
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("Verify", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)

        Text(
            "Scan a vet or groomer's QR to import a verified record, or to present one of your stored " +
                "records for an on-chain proof-of-verification. Your app only scans — it never shows a QR.",
            fontSize = 13.sp, color = c.muted,
        )
        Button(
            onClick = onScan,
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
        ) { Text("Scan a QR") }

        // --- Verify-core parity panel (real diagnostic) ---
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text("Trust-core parity", fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text("Rust SDK via UniFFI (native .so + Kotlin binding)", fontSize = 12.sp, color = c.muted)
            Text(
                "mobile root == server root: ${if (parity.pass) "PASS" else "FAIL"}",
                fontSize = 16.sp, fontWeight = FontWeight.Bold,
                color = if (parity.pass) c.success else c.danger,
            )
            Text("leaves: ${parity.leaf}   merkle trees: ${parity.merkle}", fontSize = 12.sp, color = c.muted)
            Text(parity.detail, fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
        }

        Spacer(Modifier.aspectRatio(8f))
    }
}
