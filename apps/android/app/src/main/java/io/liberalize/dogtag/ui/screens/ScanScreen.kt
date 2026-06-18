package io.liberalize.dogtag.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
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
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.consent.ConsentMode
import io.liberalize.dogtag.consent.ConsentSigner
import io.liberalize.dogtag.consent.VerificationRequest
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.RecordImporter
import io.liberalize.dogtag.net.CentralApi
import io.liberalize.dogtag.qr.QrPayload
import io.liberalize.dogtag.qr.QrScannerView
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.Wallet
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch

/**
 * The single scan entry point for the user app. The owner ONLY scans — there is no QR display here.
 * A scanned QR routes to one of two outcomes (architecture §7, impl §3.9 / §6.5):
 *   - Import a record (issuer -> user): fetch the wrapped doc, verify, store under the pet.
 *   - Verify (verifier -> user): pick which stored record to present, sign consent, relay to central.
 */
@Composable
fun ScanScreen(activity: FragmentActivity, onDone: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember { LocalStore.get(context) }
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()

    var scanning by remember { mutableStateOf(true) }
    var payload by remember { mutableStateOf<QrPayload?>(null) }
    var status by remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }

    if (scanning) {
        Box(Modifier.fillMaxSize()) {
            QrScannerView(onResult = { raw ->
                scanning = false
                payload = QrPayload.parse(raw)
            })
            Column(Modifier.align(Alignment.BottomCenter).padding(20.dp)) {
                Text(
                    "Point the camera at the vet/groomer's QR",
                    color = Color.White, fontSize = 13.sp,
                    modifier = Modifier.padding(bottom = 8.dp),
                )
                Button(onClick = { scanning = false; onDone() }) { Text("Cancel") }
            }
        }
        return
    }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Text("Scan", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)

        when (val p = payload) {
            is QrPayload.ImportRecord -> ImportPanel(
                host = p.host, idLabel = p.recordId, working = working, status = status,
                onImport = {
                    working = true; status = "Fetching + verifying record…"
                    scope.launch {
                        val r = RecordImporter.import(p)
                        working = false
                        if (r.credential != null) {
                            store.addCredential(r.credential)
                            status = "Imported (${r.verdict}) — ${r.detail}"
                        } else {
                            status = "Import failed: ${r.detail}"
                        }
                    }
                },
            )

            is QrPayload.ImportRecordToken -> ImportPanel(
                host = p.host, idLabel = p.token, working = working, status = status,
                onImport = {
                    working = true; status = "Fetching + verifying record…"
                    scope.launch {
                        val r = RecordImporter.import(p)
                        working = false
                        if (r.credential != null) {
                            store.addCredential(r.credential)
                            status = "Imported (${r.verdict}) — ${r.detail}"
                        } else {
                            status = "Import failed: ${r.detail}"
                        }
                    }
                },
            )

            is QrPayload.VerifySession -> VerifyPanel(
                session = p, activity = activity, store = store, status = status,
                onStatus = { status = it },
            )

            is QrPayload.Unknown -> {
                Card {
                    Text("Unrecognised QR", fontWeight = FontWeight.Bold, color = c.danger, fontSize = 15.sp)
                    Text(
                        "This isn't a DogTag record link (/r/<token> or /r?t=) or verify session (/v).",
                        fontSize = 12.sp, color = c.muted,
                    )
                    Text(p.raw.take(120), fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
                }
            }

            null -> {}
        }

        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(
                onClick = { status = ""; payload = null; scanning = true },
                colors = ButtonDefaults.buttonColors(containerColor = c.surfaceVariant, contentColor = c.onBackground),
            ) { Text("Scan again") }
            Button(
                onClick = onDone,
                colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
            ) { Text("Done") }
        }
        Spacer(Modifier.size(24.dp))
    }
}

@Composable
private fun ImportPanel(
    host: String,
    idLabel: String,
    working: Boolean,
    status: String,
    onImport: () -> Unit,
) {
    val c = DogTagTheme.colors
    Card {
        Text("Import record", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text("From $host", fontSize = 12.sp, color = c.muted)
        Text("Record ${idLabel.take(18)}…", fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
        Text(
            "We'll fetch the wrapped document, recompute its Merkle root (offline) and re-check " +
                "DogTagIssuer.isValid on ROAX before storing it under your pet.",
            fontSize = 12.sp, color = c.muted,
        )
        Button(
            onClick = onImport,
            enabled = !working,
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
        ) { Text(if (working) "Working…" else "Verify & import") }
        if (status.isNotBlank()) {
            val good = status.startsWith("Imported (VALID")
            Text(status, fontSize = 12.sp, color = if (good) c.success else c.muted)
        }
    }
}

@Composable
private fun VerifyPanel(
    session: QrPayload.VerifySession,
    activity: FragmentActivity,
    store: LocalStore,
    status: String,
    onStatus: (String) -> Unit,
) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var selected by remember { mutableStateOf<Credential?>(null) }
    var err by remember { mutableStateOf("") }

    // candidate records: all the user's stored credentials (optionally filtered by requested recordType).
    val all by store.credentials.collectAsStateWithLifecycle()
    val wantGroup = io.liberalize.dogtag.data.CredentialGroup.fromRecordType(session.recordType)
    val candidates = all.filter { it.group == wantGroup }.ifEmpty { all }

    Card {
        Text("Verification request", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Field("Verifier", session.relayer.ifBlank { "Unknown" })
        Field("Purpose", session.purpose.ifBlank { "—" })
        Field("Record type", session.recordType.ifBlank { "any" })
        Field("Mode", if (session.mode.lowercase() == "normal" || session.mode.lowercase() == "ecdsa") "ECDSA (EIP-712)" else "Zero-knowledge")
    }

    Card {
        Text("Select the record to present", fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        if (candidates.isEmpty()) {
            Text("No matching records yet — scan a vet's QR to import one first.", fontSize = 12.sp, color = c.muted)
        }
        candidates.forEach { cred ->
            val isSel = selected?.id == cred.id
            Row(
                Modifier.fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(if (isSel) c.accent.copy(alpha = 0.14f) else c.surfaceVariant)
                    .border(
                        if (isSel) 1.5.dp else 0.dp,
                        if (isSel) c.accent else Color.Transparent,
                        RoundedCornerShape(12.dp),
                    )
                    .clickable { selected = cred }
                    .padding(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    Text("${cred.group.title} · ${cred.verdict}", fontSize = 11.sp, color = c.muted)
                }
            }
        }
    }

    val sel = selected
    Button(
        onClick = {
            err = ""
            if (sel == null) { onStatus("Select a record first."); return@Button }
            Biometric.prompt(
                activity, "Authorize consent",
                "Present '${sel.title}' to ${session.relayer.ifBlank { "the verifier" }}",
                onSuccess = {
                    try {
                        val wallet = runCatching { Wallet.load(context) }.getOrNull()
                        val subject = wallet?.ethAddress
                        val consentPriv = if (session.mode.lowercase() != "normal" && session.mode.lowercase() != "ecdsa")
                            wallet?.consent?.prvHex else null
                        val req = VerificationRequest.from(
                            session = session,
                            dogTagIdDec = sel.dogTagId,
                            credentialRoot = sel.credentialRoot,
                            subjectWallet = subject,
                            callbackUrl = "${AppConfig.CENTRAL_API}/v1/verify/consent",
                        )
                        val signed = ConsentSigner.sign(req, consentPriv)
                        scope.launch {
                            onStatus("Signed (${signed.mode}); submitting…")
                            val token = AppConfig.sessionToken(context)
                            val r = runCatching { CentralApi.postConsent(token, signed.payloadJson) }.getOrNull()
                            onStatus(
                                if (r == null) "Signed locally; submit failed (no network / session)."
                                else "POST /v1/verify/consent → ${r.code}",
                            )
                        }
                    } catch (e: Exception) { err = "sign failed: ${e.message}" }
                },
                onError = { err = it },
            )
        },
        enabled = sel != null,
        modifier = Modifier.fillMaxWidth(),
        colors = ButtonDefaults.buttonColors(containerColor = c.success, contentColor = Color.White),
    ) { Text("Approve & present") }

    if (err.isNotBlank()) Text(err, fontSize = 12.sp, color = c.danger)
    if (status.isNotBlank()) Text(status, fontSize = 12.sp, color = c.muted)
}

@Composable
private fun Card(content: @Composable androidx.compose.foundation.layout.ColumnScope.() -> Unit) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
        content = content,
    )
}

@Composable
private fun Field(label: String, value: String) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth()) {
        Text(label, fontSize = 12.sp, color = c.muted, modifier = Modifier.fillMaxWidth(0.34f))
        Text(value, fontSize = 12.sp, color = c.onBackground, fontFamily = FontFamily.Monospace)
    }
}
