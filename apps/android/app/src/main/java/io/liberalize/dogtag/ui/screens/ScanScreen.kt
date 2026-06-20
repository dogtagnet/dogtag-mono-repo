package io.liberalize.dogtag.ui.screens

import android.os.Build
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
import androidx.compose.foundation.layout.height
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.ui.graphics.graphicsLayer
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.consent.ConsentKeyBind
import io.liberalize.dogtag.consent.ConsentMode
import io.liberalize.dogtag.consent.ConsentSigner
import io.liberalize.dogtag.consent.VerificationRequest
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.RecordImporter
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.ZkeyAsset
import io.liberalize.dogtag.net.CentralApi
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.qr.QrPayload
import io.liberalize.dogtag.qr.QrScannerView
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.Wallet
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.dogtag_standard.EddsaSigInput
import uniffi.dogtag_standard.bindConsentKeyDigestHex
import uniffi.dogtag_standard.proveVerification
import uniffi.dogtag_standard.signConsentEddsa
import uniffi.dogtag_standard.verifyWhitelistKeyHex

/**
 * The single scan entry point for the user app. The owner ONLY scans — there is no QR display here.
 * A scanned QR routes to one of two outcomes (architecture §7, impl §3.9 / §6.5):
 *   - Import a record (issuer -> user): fetch the wrapped doc, verify, store under the pet.
 *   - Export (user -> groomer): pick which stored record to present, DNS-verify the groomer, prove
 *     on-device, POST the proof to the groomer host.
 */
@Composable
fun ScanScreen(activity: FragmentActivity, onDone: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember { LocalStore.get(context) }
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()

    val walletExists = remember { Wallet.exists(context) }

    var scanning by remember { mutableStateOf(true) }
    var payload by remember { mutableStateOf<QrPayload?>(null) }
    var status by remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }

    // SCAN GATE (B1): import + export both need a wallet (the device address is what the record is
    // minted to / the consent is signed with). No wallet → don't scan; point the user to Profile.
    if (!walletExists) {
        Column(
            Modifier.fillMaxSize().padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text("Scan", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Card {
                Text("Create your wallet first", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                Text(
                    "You need an embedded wallet before you can import or export records. " +
                        "Go to Profile → Create embedded wallet.",
                    fontSize = 12.sp, color = c.muted,
                )
            }
            Button(
                onClick = onDone,
                colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
            ) { Text("Back") }
        }
        return
    }

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

            is QrPayload.DogTagIssueSession -> IssuePanel(
                qr = p, activity = activity, store = store,
            )

            is QrPayload.ExportSession -> ExportPanel(
                qr = p, activity = activity, store = store, status = status,
                onStatus = { status = it },
            )

            is QrPayload.Unknown -> {
                Card {
                    Text("Unrecognised QR", fontWeight = FontWeight.Bold, color = c.danger, fontSize = 15.sp)
                    Text(
                        "This isn't a DogTag record link (/r/<token> or /r?t=), dog-tag issuance (/p/<token>) " +
                            "or export session (/x/<token>).",
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

/**
 * The dog-tag issuance panel (vet-issues-the-dog-tag). POST <host>/profiles/issue/bind with the wallet
 * address + its registration signature; on `{ wrappedDoc, dogTagId, root, txHash }` verify the issued
 * DOG_PROFILE against the DogTagSBT (profileRoot + ownerOf) AND offline integrity, store it as a
 * Credential, and show a success card with the dogTagId + txHash.
 */
@Composable
private fun IssuePanel(
    qr: QrPayload.DogTagIssueSession,
    activity: FragmentActivity,
    store: LocalStore,
) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val roax = remember { RoaxConfig.load(context) }

    var working by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    var issued by remember { mutableStateOf<CentralApi.DogTagIssue?>(null) }
    var verdict by remember { mutableStateOf("") }
    var err by remember { mutableStateOf("") }

    Card {
        Text("Issue dog tag", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text("From ${qr.host}", fontSize = 12.sp, color = c.muted)
        Text("Token ${qr.token.take(18)}…", fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
        Text(
            "Your vet will bind a new dog tag to this wallet. We'll sign the binding, then verify the " +
                "issued profile against the DogTagSBT (profileRoot + ownerOf) before storing it.",
            fontSize = 12.sp, color = c.muted,
        )

        if (issued == null && !working) {
            Button(
                onClick = {
                    err = ""
                    Biometric.prompt(
                        activity, "Issue dog tag", "Authenticate to bind this dog tag to your wallet",
                        onSuccess = {
                            val wallet = runCatching { Wallet.load(context) }.getOrNull()
                            if (wallet == null) { err = "Create your wallet first (Profile)."; return@prompt }
                            working = true
                            status = "Binding dog tag…"
                            scope.launch {
                                try {
                                    val sig = wallet.registerSignature()
                                    val res = withContext(Dispatchers.IO) {
                                        CentralApi.bindDogTagIssue(qr.host, qr.token, wallet.ethAddress, sig)
                                    }
                                    if (res == null) {
                                        working = false; err = "Bind failed (expired token / network)."; return@launch
                                    }
                                    // The bind responds immediately (status "minting") and the vet mints the
                                    // SBT in the background. Poll the chain (profileRoot + ownerOf) until the
                                    // mint lands — retrying a miss rather than failing on the first read.
                                    status = "Minting your dog tag on-chain…"
                                    val poll = withContext(Dispatchers.IO) {
                                        RecordImporter.pollSbtMint(
                                            dogTagId = res.dogTagId,
                                            expectedRoot = res.root,
                                            walletAddress = wallet.ethAddress,
                                            dogTagSbt = roax.dogTagSbt,
                                            rpcUrl = AppConfig.ROAX_RPC,
                                        )
                                    }
                                    if (poll is RecordImporter.MintPoll.Timeout) {
                                        working = false
                                        err = "Mint not confirmed — check the vet portal."
                                        return@launch
                                    }
                                    // Mint confirmed on-chain: run the offline integrity + (now-landed) SBT check.
                                    status = "Verifying against DogTagSBT…"
                                    val r = withContext(Dispatchers.IO) {
                                        RecordImporter.verifyIssuedDogTag(
                                            wrappedDocJson = res.wrappedDocJson,
                                            dogTagId = res.dogTagId,
                                            expectedRoot = res.root,
                                            walletAddress = wallet.ethAddress,
                                            dogTagSbt = roax.dogTagSbt,
                                            rpcUrl = AppConfig.ROAX_RPC,
                                        )
                                    }
                                    working = false
                                    if (r.credential != null) {
                                        store.addCredential(r.credential)
                                        issued = res
                                        verdict = r.verdict
                                        status = "Issued (${r.verdict}) — ${r.detail}"
                                    } else {
                                        err = "Verify failed: ${r.detail}"
                                    }
                                } catch (e: Exception) {
                                    working = false; err = "Issue failed: ${e.message}"
                                }
                            }
                        },
                        onError = { err = it },
                    )
                },
                enabled = !working,
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
            ) { Text("Issue & verify") }
        }
        if (working) {
            ForgingAnimation(status)
        }
        if (!working && status.isNotBlank()) {
            Text(status, fontSize = 12.sp, color = if (verdict == "VALID") c.success else c.muted)
        }
        if (err.isNotBlank()) Text(err, fontSize = 12.sp, color = c.danger)
    }

    issued?.let { res ->
        Card {
            Text("Dog tag issued", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.success)
            Field("dogTagId", res.dogTagId.ifBlank { "—" })
            Field("Verdict", verdict.ifBlank { "—" })
            Field("Root", res.root.take(18).ifBlank { "—" } + "…")
            Field("Tx", res.txHash.take(18).ifBlank { "—" } + "…")
            Text("Stored under your dog tags.", fontSize = 12.sp, color = c.muted)
        }
    }
}

@Composable
private fun ExportPanel(
    qr: QrPayload.ExportSession,
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

    // Resolve the export-session metadata from the one-time token (non-consuming GET /x/<token>).
    var session by remember { mutableStateOf<CentralApi.ExportSession?>(null) }
    var resolveErr by remember { mutableStateOf<String?>(null) }
    androidx.compose.runtime.LaunchedEffect(qr.token) {
        val s = withContext(Dispatchers.IO) { CentralApi.resolveExportSession(qr.host, qr.token) }
        if (s == null) {
            resolveErr = "Could not resolve export session (expired or offline)."
        } else if (!s.relayer.equals(qr.groomerAddr, ignoreCase = true)) {
            // (b) The QR-claimed groomer address must match the session relayer — hard-stop on mismatch.
            resolveErr = "Groomer address mismatch — refusing to present."
        } else {
            session = s
        }
    }

    val sess = session
    if (sess == null) {
        Card {
            Text("Export request", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text(resolveErr ?: "Resolving export session…", fontSize = 12.sp,
                color = if (resolveErr != null) c.danger else c.muted)
        }
        return
    }

    // candidate records: all the user's stored credentials (optionally filtered by requested recordType).
    val all by store.credentials.collectAsStateWithLifecycle()
    val wantGroup = io.liberalize.dogtag.data.CredentialGroup.fromRecordType(sess.recordType)
    val candidates = all.filter { it.group == wantGroup }.ifEmpty { all }

    Card {
        Text("Export request", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Field("Groomer", sess.relayer.ifBlank { "Unknown" })
        Field("Purpose", sess.purpose.ifBlank { "—" })
        Field("Record type", sess.recordType.ifBlank { "any" })
        Field("Mode", if (sess.mode.lowercase() == "normal" || sess.mode.lowercase() == "ecdsa") "ECDSA (EIP-712)" else "Zero-knowledge")
    }

    Card {
        Text("Select the record to export", fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
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
    var busy by remember { mutableStateOf(false) }
    val isZk = sess.mode.lowercase() != "normal" && sess.mode.lowercase() != "ecdsa"
    if (busy) {
        ForgingAnimation(
            status.ifBlank { "Recording your verification on-chain…" },
            title = "Recording your verification on-chain",
        )
    }
    Button(
        onClick = {
            err = ""
            if (sel == null) { onStatus("Select a record first."); return@Button }
            Biometric.prompt(
                activity, "Authorize consent",
                "Present '${sel.title}' to ${sess.relayer.ifBlank { "the groomer" }}",
                onSuccess = {
                    val wallet = runCatching { Wallet.load(context) }.getOrNull()
                    val subject = wallet?.ethAddress
                    val consentPriv = if (isZk) wallet?.consent?.prvHex else null
                    val req = VerificationRequest.from(
                        exportToken = qr.token,
                        relayer = sess.relayer,
                        purpose = sess.purpose,
                        recordType = sess.recordType,
                        challenge = sess.challenge,
                        mode = sess.mode,
                        dogTagIdDec = sel.dogTagId,
                        credentialRoot = sel.credentialRoot,
                        subjectWallet = subject,
                        callbackUrl = "${AppConfig.centralApi(context)}/v1/verify/consent",
                    )
                    if (!isZk) {
                        // ECDSA (legacy) path — relay through central as before.
                        scope.launch {
                            try {
                                val signed = ConsentSigner.sign(req, null)
                                onStatus("Signed (${signed.mode}); submitting…")
                                val token = AppConfig.sessionToken(context)
                                val r = runCatching { CentralApi.postConsent(AppConfig.centralApi(context), token, signed.payloadJson) }.getOrNull()
                                onStatus(
                                    if (r == null) "Signed locally; submit failed (no network / session)."
                                    else "POST /v1/verify/consent → ${r.code}",
                                )
                            } catch (e: Exception) { err = "sign failed: ${e.message}" }
                        }
                        return@prompt
                    }

                    // -------- ZERO-KNOWLEDGE on-device path --------
                    busy = true
                    scope.launch {
                        try {
                            val roax = RoaxConfig.load(context)

                            // (a) PRE-PROOF GROOMER CHECK — hard-stop if the relayer is not a
                            // whitelisted groomer for this purpose. Never sign/prove/disclose otherwise.
                            onStatus("Checking groomer authorization…")
                            val verifyKey = verifyWhitelistKeyHex(sess.purpose)
                            val wl = withContext(Dispatchers.IO) {
                                RoaxRpc.isWhitelistedFor(
                                    AppConfig.ROAX_RPC, roax.issuerRegistry, verifyKey, sess.relayer,
                                )
                            }
                            if (wl !is RoaxRpc.Result.Valid) {
                                busy = false
                                err = "This groomer is not authorized (not whitelisted)."
                                onStatus("Blocked — not an authorized groomer (${wl}).")
                                return@launch
                            }

                            // (a2) DNS VERIFY (prod/remote only) — the groomer's domain must publish a
                            // TXT `dogtag-verify=<groomerAddr>`. Local hosts skip this entirely.
                            if (!io.liberalize.dogtag.net.DnsVerify.isLocalHost(qr.host)) {
                                onStatus("Verifying groomer DNS…")
                                val dnsOk = withContext(Dispatchers.IO) {
                                    io.liberalize.dogtag.net.DnsVerify.verifyGroomer(qr.host, qr.groomerAddr)
                                }
                                if (!dnsOk) {
                                    busy = false
                                    err = "Groomer DNS not verified — refusing to present."
                                    onStatus("Blocked — groomer DNS not verified.")
                                    return@launch
                                }
                            }

                            // (b) Sign the EdDSA consent + generate the Groth16 proof on-device.
                            if (consentPriv == null || wallet == null) {
                                busy = false; err = "Create your wallet first (Profile)."; return@launch
                            }
                            val eddsa = signConsentEddsa(
                                consentPriv,
                                req.dogTagId, req.recordType, req.purpose, req.credentialRoot, req.challenge,
                                req.relayer, req.subject, req.nonce, req.deadline,
                            )
                            // 32-BIT FALLBACK DECISION. A 32-bit-only device (no arm64 ABI) cannot run
                            // the on-device circom-prover, so it queries the trusted PROVER SERVICE for
                            // the proof instead. 64-bit devices (arm64 Android, iOS) keep true
                            // on-device proving — that path is unchanged. `SUPPORTED_64_BIT_ABIS` is
                            // empty iff the device is 32-bit only.
                            val is32BitOnly = Build.SUPPORTED_64_BIT_ABIS.isEmpty()
                            val consentJson = eddsaConsentJson(req)
                            val proof = if (is32BitOnly) {
                                // ---- 32-bit: prove on the prover service (groomer never sees witness) ----
                                onStatus("Proving on the prover service (32-bit device)…")
                                val proverUrl = AppConfig.proverApiUrl(context)
                                if (proverUrl.isBlank()) {
                                    busy = false
                                    err = "No prover service configured for this 32-bit device."
                                    onStatus("Blocked — 32-bit device has no prover service configured.")
                                    return@launch
                                }
                                val served = withContext(Dispatchers.IO) {
                                    CentralApi.proveOnServer(
                                        proverUrl,
                                        sel.wrappedDocJson,
                                        consentJson,
                                        CentralApi.ProverEddsaSig(
                                            r8xDec = eddsa.r8xDec, r8yDec = eddsa.r8yDec, sDec = eddsa.sDec,
                                            axHex = wallet.consent.axHex, ayHex = wallet.consent.ayHex,
                                        ),
                                    )
                                }
                                if (served == null) {
                                    busy = false
                                    err = "Prover service failed to return a proof."
                                    onStatus("Blocked — prover service unavailable.")
                                    return@launch
                                }
                                served
                            } else {
                                // ---- 64-bit: TRUE on-device proving (UNCHANGED) ----
                                onStatus("Generating proof…")
                                val zkeyPath = withContext(Dispatchers.IO) { ZkeyAsset.ensure(context) }
                                val graphPath = withContext(Dispatchers.IO) { ZkeyAsset.ensureGraph(context) }
                                val eddsaInput = EddsaSigInput(
                                    r8xDec = eddsa.r8xDec, r8yDec = eddsa.r8yDec, sDec = eddsa.sDec,
                                    axHex = wallet.consent.axHex, ayHex = wallet.consent.ayHex,
                                )
                                withContext(Dispatchers.Default) {
                                    proveVerification(sel.wrappedDocJson, consentJson, eddsaInput, zkeyPath, graphPath)
                                }
                            }

                            // (c) CONSENT-KEY BIND (gasless) — owner signs the EIP-712 bind digest;
                            // the RELAYER submits bindConsentKeyFor (owner pays no gas).
                            val bind: ConsentKeyBind? = runCatching {
                                val nonce = withContext(Dispatchers.IO) {
                                    RoaxRpc.bindNonce(AppConfig.ROAX_RPC, roax.consentKeyRegistry, wallet.ethAddress)
                                } ?: 0L
                                val digestHex = bindConsentKeyDigestHex(
                                    roax.consentKeyRegistry, wallet.consent.keyHashHex,
                                    wallet.ethAddress, nonce.toULong(), roax.chainId.toULong(),
                                )
                                val digest = hexToBytes(digestHex)
                                val ownerSig = wallet.signEthDigest(digest)
                                ConsentKeyBind(wallet.ethAddress, wallet.consent.keyHashHex, ownerSig)
                            }.getOrNull()

                            // The proof's nullifier = pubSignals[4] (a decimal field element). This is
                            // the on-chain `VerificationRegistry.consumed(bytes32)` key — the canonical
                            // completion signal we poll the CHAIN for below.
                            val nullifier = proof.pubSignals.getOrNull(4).orEmpty()

                            // PRE-SUBMIT REPLAY GUARD: if this nullifier is already consumed on-chain,
                            // the verification was recorded before — submitting again is a doomed replay
                            // the relayer will reject. Stop early with a clear message.
                            val alreadyRecorded = withContext(Dispatchers.IO) {
                                RoaxRpc.consumed(AppConfig.ROAX_RPC, roax.verificationRegistry, nullifier)
                            }
                            if (alreadyRecorded) {
                                busy = false
                                err = "This verification was already recorded."
                                onStatus("Already recorded on-chain.")
                                return@launch
                            }

                            // (d) SUBMIT to the QR host (groomer), NOT central. The one-time exportToken
                            // is consumed server-side on record-success. The submit now returns 200
                            // {status:"recording"} FAST and records on-chain in the background, so we
                            // ALWAYS proceed to the chain poll — even on a null/non-2xx submit response
                            // (the relayer may still be recording). Only a clearly-rejected 4xx carrying
                            // an {error} body is a hard failure.
                            onStatus("Submitting proof to groomer…")
                            val signed = ConsentSigner.signWithProof(req, consentPriv, proof, bind)
                            val r = withContext(Dispatchers.IO) {
                                runCatching { CentralApi.postVerifyConsentToHost(qr.host, signed.payloadJson) }.getOrNull()
                            }
                            if (r != null && r.code in 400..499) {
                                val rejectMsg = runCatching {
                                    org.json.JSONObject(r.body).optString("error", "")
                                }.getOrNull().orEmpty()
                                if (rejectMsg.isNotBlank()) {
                                    busy = false
                                    err = "Rejected: $rejectMsg"
                                    onStatus("Submit rejected ($rejectMsg).")
                                    return@launch
                                }
                            }
                            // Otherwise (2xx, null/network, or a 4xx without an {error} body) proceed
                            // to the chain poll — the canonical success signal is consumed(nullifier).

                            // (e) POLL THE CHAIN until VerificationRegistry.consumed(nullifier) == true.
                            // Do NOT rely on the token-gated session poll: the export token is consumed
                            // at record-success, so that GET starts returning 401 exactly when it
                            // succeeds. Poll every ~3s up to ~120s.
                            // The export token is consumed server-side only on record-SUCCESS, so the
                            // token-gated session poll keeps returning until success/error. We poll it
                            // ALONGSIDE the chain: a bind/record failure flips the session to
                            // status="error" (the reason is carried in txHash/message) and would
                            // otherwise never set consumed(nullifier)=true — leaving the phone stuck
                            // until the ~120s timeout. On error we STOP and surface it immediately.
                            onStatus("Recording your verification on-chain…")
                            var done = false
                            var failedMsg: String? = null
                            for (i in 0 until 40) {
                                val ok = withContext(Dispatchers.IO) {
                                    RoaxRpc.consumed(AppConfig.ROAX_RPC, roax.verificationRegistry, nullifier)
                                }
                                if (ok) { done = true; break }
                                val st = withContext(Dispatchers.IO) {
                                    runCatching { CentralApi.verifySessionStatus(qr.host, sess.sessionId, qr.token) }.getOrNull()
                                }
                                if (st?.status == "error") {
                                    failedMsg = st.txHash?.ifBlank { null } ?: "recording failed"
                                    break
                                }
                                kotlinx.coroutines.delay(3000)
                            }
                            if (failedMsg != null) {
                                busy = false
                                err = "Verification failed: $failedMsg"
                                onStatus("Verification failed: $failedMsg")
                                return@launch
                            }
                            if (done) {
                                // Optional: one best-effort session read to surface a txHash for display.
                                val tx = withContext(Dispatchers.IO) {
                                    runCatching { CentralApi.verifySessionStatus(qr.host, sess.sessionId, qr.token) }.getOrNull()
                                }?.txHash
                                onStatus(
                                    if (!tx.isNullOrBlank()) "Verified on-chain — no data disclosed. tx ${tx.take(14)}…"
                                    else "Verified on-chain — no data disclosed.",
                                )
                            } else {
                                onStatus("Submitted; awaiting confirmation.")
                            }
                            busy = false
                        } catch (e: Exception) {
                            busy = false
                            err = "ZK verify failed: ${e.message}"
                        }
                    }
                },
                onError = { err = it },
            )
        },
        enabled = sel != null && !busy,
        modifier = Modifier.fillMaxWidth(),
        colors = ButtonDefaults.buttonColors(containerColor = c.success, contentColor = Color.White),
    ) { Text(if (busy) "Working…" else "Approve & export") }

    if (err.isNotBlank()) Text(err, fontSize = 12.sp, color = c.danger)
    // While busy the ForgingAnimation already surfaces the live status; show the plain status text
    // only when idle (the final success/timeout line).
    if (!busy && status.isNotBlank()) {
        val good = status.startsWith("Verified on-chain")
        Text(status, fontSize = 12.sp, color = if (good) c.success else c.muted)
    }
}

/**
 * The consent JSON the Rust prover consumes for `proveVerification` — the canonical §1.10 consent
 * fields (all 0x.. hex). The prover internally re-derives the circuit signals from these + the
 * wrapped doc + the EdDSA signature.
 */
private fun eddsaConsentJson(req: VerificationRequest): String =
    org.json.JSONObject().apply {
        put("dogTagId", req.dogTagId)
        put("recordType", req.recordType)
        put("purpose", req.purpose)
        put("credentialRoot", req.credentialRoot)
        put("challenge", req.challenge)
        put("relayer", req.relayer)
        put("subject", req.subject)
        put("nonce", req.nonce)
        put("deadline", req.deadline)
    }.toString()

private fun hexToBytes(hex: String): ByteArray {
    val h = hex.removePrefix("0x")
    return ByteArray(h.length / 2) { i -> ((Character.digit(h[i * 2], 16) shl 4) + Character.digit(h[i * 2 + 1], 16)).toByte() }
}

/** Animated waiting screen shown while the dog tag is minted on-chain (the bind returns instantly, the
 * SBT mint lands ~12-24s later). A pulsing/glowing dog-tag forging while the phone polls the chain. */
@Composable
private fun ForgingAnimation(status: String, title: String = "Forging your dog tag") {
    val c = DogTagTheme.colors
    val infinite = rememberInfiniteTransition(label = "forge")
    val scale by infinite.animateFloat(
        initialValue = 0.82f, targetValue = 1.16f,
        animationSpec = infiniteRepeatable(tween(750, easing = FastOutSlowInEasing), RepeatMode.Reverse),
        label = "scale",
    )
    val glow by infinite.animateFloat(
        initialValue = 0.4f, targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(750, easing = LinearEasing), RepeatMode.Reverse),
        label = "glow",
    )
    Column(
        modifier = Modifier.fillMaxWidth().padding(vertical = 20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            "🏷️",
            fontSize = 54.sp,
            modifier = Modifier.graphicsLayer {
                scaleX = scale; scaleY = scale; alpha = glow
            },
        )
        Spacer(Modifier.height(16.dp))
        Text(title, fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Spacer(Modifier.height(4.dp))
        Text(status.ifBlank { "Minting on-chain…" }, fontSize = 12.sp, color = c.muted)
        Spacer(Modifier.height(16.dp))
        LinearProgressIndicator(modifier = Modifier.fillMaxWidth(0.78f), color = c.accent)
    }
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
