package io.liberalize.dogtag

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.json.JSONObject

// The UniFFI-generated foreign-callable surface over the Rust DogTag SDK.
import uniffi.dogtag_standard.buildMerkleRootHex
import uniffi.dogtag_standard.hashLeafHex

/**
 * Result of running the verify-core over the bundled testvectors.json through the Rust FFI.
 */
data class ParityResult(
    val pass: Boolean,
    val leafChecks: Int,
    val merkleChecks: Int,
    val detail: String,
)

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val vectors = assets.open("testvectors.json").bufferedReader().use { it.readText() }
        val result = runParity(vectors)
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    VerifyScreen(result)
                }
            }
        }
    }
}

/**
 * Drive the Rust FFI (hashLeafHex / buildMerkleRootHex) over the SAME shared vectors the server/TS
 * assert, and compare mobile output to the server-side expected hex. "mobile root == server root".
 */
fun runParity(vectorsJson: String): ParityResult {
    return try {
        val root = JSONObject(vectorsJson)

        // 1) Leaf parity: every leaf hashed on-device must equal the server expected_hex.
        val leaves = root.getJSONArray("leaves")
        var leafChecks = 0
        for (i in 0 until leaves.length()) {
            val v = leaves.getJSONObject(i)
            val keyPath = v.getString("keyPath")
            val saltHex = v.getString("saltHex")
            val tag = v.getInt("tag").toUByte()
            val value = if (v.isNull("value")) "" else v.getString("value")
            val expected = v.getString("expected_hex")
            val got = hashLeafHex(keyPath, saltHex, tag, value)
            if (got != expected) {
                return ParityResult(false, leafChecks, 0, "leaf mismatch '${v.getString("name")}': $got != $expected")
            }
            leafChecks++
        }

        // 2) Merkle-root parity: every tree built on-device must equal the server root_hex.
        val merkle = root.getJSONArray("merkle")
        var merkleChecks = 0
        for (i in 0 until merkle.length()) {
            val m = merkle.getJSONObject(i)
            val leafHexesArr = m.getJSONArray("leaf_hexes")
            val leafHexes = ArrayList<String>(leafHexesArr.length())
            for (j in 0 until leafHexesArr.length()) leafHexes.add(leafHexesArr.getString(j))
            val expected = m.getString("root_hex")
            val got = buildMerkleRootHex(leafHexes)
            if (got != expected) {
                return ParityResult(false, leafChecks, merkleChecks, "root mismatch '${m.getString("name")}': $got != $expected")
            }
            merkleChecks++
        }

        ParityResult(true, leafChecks, merkleChecks, "all leaves + roots match the server vectors")
    } catch (t: Throwable) {
        ParityResult(false, 0, 0, "exception: ${t.message}")
    }
}

@Composable
fun VerifyScreen(result: ParityResult) {
    var scanMsg by remember { mutableStateOf("") }
    val scroll = rememberScrollState()
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(scroll)
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("DogTag — Phase 6 verify core", fontSize = 22.sp, fontWeight = FontWeight.Bold)
        Text("Rust SDK via UniFFI (native .so + Kotlin binding)", fontSize = 13.sp)

        val verdict = if (result.pass) "PASS" else "FAIL"
        val color = if (result.pass) Color(0xFF1B7F3B) else Color(0xFFB00020)
        Text(
            "mobile root == server root: $verdict",
            fontSize = 20.sp,
            fontWeight = FontWeight.Bold,
            color = color,
        )
        Text(
            "leaves checked: ${result.leafChecks}    merkle trees checked: ${result.merkleChecks}",
            fontSize = 14.sp,
        )
        Text(result.detail, fontSize = 12.sp, fontFamily = FontFamily.Monospace)

        Button(onClick = { scanMsg = "Scan QR — not yet implemented (Phase 6 later pass)" }) {
            Text("Scan QR")
        }
        if (scanMsg.isNotEmpty()) Text(scanMsg, fontSize = 12.sp)
    }
}
