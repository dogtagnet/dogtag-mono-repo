package io.liberalize.dogtag.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.consent.ConsentMode
import io.liberalize.dogtag.consent.ConsentSigner
import io.liberalize.dogtag.consent.SignedConsent
import io.liberalize.dogtag.consent.VerificationRequest
import io.liberalize.dogtag.net.Http
import io.liberalize.dogtag.qr.QrScannerView
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.Wallet
import kotlinx.coroutines.launch
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

@Composable
fun VerifyScreen(activity: FragmentActivity) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()

    val parity = remember {
        runParity(context.assets.open("testvectors.json").bufferedReader().use { it.readText() })
    }

    var scanning by remember { mutableStateOf(false) }
    var request by remember { mutableStateOf<VerificationRequest?>(null) }
    var signed by remember { mutableStateOf<SignedConsent?>(null) }
    var status by remember { mutableStateOf("") }

    if (scanning) {
        Box(Modifier.fillMaxSize()) {
            QrScannerView(
                onResult = { raw ->
                    scanning = false
                    request = try { VerificationRequest.parse(raw) } catch (e: Exception) {
                        status = "Not a verifier QR: ${e.message}"; null
                    }
                },
            )
            Button(
                onClick = { scanning = false },
                modifier = Modifier.padding(20.dp),
            ) { Text("Cancel") }
        }
        return
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("Verify", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)

        // --- Verify-core parity panel (kept) ---
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

        // --- Consent signing flow ---
        Button(
            onClick = { status = ""; signed = null; request = null; scanning = true },
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
        ) { Text("Scan verifier QR") }

        val req = request
        if (req != null) {
            ConsentReview(req, activity, onSign = { sc -> signed = sc; status = "Signed locally — ready to submit." })
        }

        val sc = signed
        if (sc != null) {
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surfaceVariant).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Text("Signed consent (${sc.mode})", fontSize = 14.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                Text("nullifier: ${sc.nullifier.take(18)}…", fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
                if (sc.eddsa != null) {
                    Text("EdDSA S: ${sc.eddsa.sDec.take(20)}…", fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
                }
                Button(
                    onClick = {
                        val url = request?.callbackUrl
                        if (url == null) { status = "No callback URL in request; consent built but not submitted."; return@Button }
                        scope.launch {
                            status = try {
                                val r = Http.postJson(url, sc.payloadJson)
                                "POST $url → ${r.code}"
                            } catch (e: Exception) { "POST failed: ${e.message}" }
                        }
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
                ) { Text("Submit to /v1/verify/consent") }
            }
        }

        if (status.isNotBlank()) Text(status, fontSize = 12.sp, color = c.muted)
        Spacer(Modifier.aspectRatio(8f))
    }
}

@Composable
private fun ConsentReview(
    req: VerificationRequest,
    activity: FragmentActivity,
    onSign: (SignedConsent) -> Unit,
) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    var err by remember { mutableStateOf("") }

    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Text("Verification request", fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Field("Verifier", req.verifierName)
        Field("Mode", if (req.mode == ConsentMode.ZK) "Zero-knowledge (EdDSA-BabyJubjub)" else "ECDSA (EIP-712)")
        Field("DogTag", req.dogTagId)
        Field("Purpose", req.purpose.take(18) + "…")
        Field("Relayer", req.relayer)
        Spacer(Modifier.aspectRatio(40f))
        Button(
            onClick = {
                err = ""
                Biometric.prompt(
                    activity,
                    title = "Authorize consent",
                    subtitle = "Sign a ${if (req.mode == ConsentMode.ZK) "ZK" else "standard"} verification consent",
                    onSuccess = {
                        try {
                            val consentPriv = if (req.mode == ConsentMode.ZK) {
                                Wallet.load(context)?.consent?.prvHex
                            } else null
                            onSign(ConsentSigner.sign(req, consentPriv))
                        } catch (e: Exception) { err = "sign failed: ${e.message}" }
                    },
                    onError = { err = it },
                )
            },
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(containerColor = c.success, contentColor = androidx.compose.ui.graphics.Color.White),
        ) { Text("Approve & sign") }
        if (err.isNotBlank()) Text(err, fontSize = 12.sp, color = c.danger)
    }
}

@Composable
private fun Field(label: String, value: String) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth()) {
        Text(label, fontSize = 12.sp, color = c.muted, modifier = Modifier.fillMaxWidth(0.32f))
        Text(value, fontSize = 12.sp, color = c.onBackground, fontFamily = FontFamily.Monospace)
    }
}
