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
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.Pet
import io.liberalize.dogtag.data.RecordImporter
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.ZkeyAsset
import io.liberalize.dogtag.BuildConfig
import io.liberalize.dogtag.net.AnchorResolver
import io.liberalize.dogtag.net.CentralApi
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.profile.ProfileTreeStore
import io.liberalize.dogtag.qr.QrPayload
import io.liberalize.dogtag.qr.QrScannerView
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.Keccak256
import io.liberalize.dogtag.wallet.Wallet
import io.liberalize.dogtag.zk.PublicSignalIndex
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import io.liberalize.dogtag.profile.BackedUpAttribute
import io.liberalize.dogtag.profile.ProfileTreeBuilder
import androidx.compose.material3.Switch
import uniffi.dogtag_standard.AttributeLeafFfi
import uniffi.dogtag_standard.ConvenienceClaims
import uniffi.dogtag_standard.ProfileTreeFfi
import uniffi.dogtag_standard.TrustedAnchor
import uniffi.dogtag_standard.buildProfileDisclosureJson
import uniffi.dogtag_standard.dogTagIdFieldHex
import uniffi.dogtag_standard.proveConsent
import uniffi.dogtag_standard.validateDiscovery
import uniffi.dogtag_standard.verifyWhitelistKeyHex
import java.security.SecureRandom

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
    // THIS APP's IssuerRegistry + DogTagIssuerFactory (bundled roax.json) for the import-time
    // issuer-whitelist pillar. Both must come from our own config, never from the scanned document:
    // a forged issuer block would otherwise nominate the factory that resolves it AND the registry
    // that vouches for it, and answer its own question twice.
    val roaxIssuerRegistry = remember { RoaxConfig.load(context).issuerRegistry }
    val roaxIssuerFactory = remember { RoaxConfig.load(context).dogTagIssuerFactory }

    val walletExists = remember { Wallet.exists(context) }

    var scanning by remember { mutableStateOf(true) }
    var payload by remember { mutableStateOf<QrPayload?>(null) }
    var status by remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }

    // Import, issuance and export all need the seed-backed owner-hidden wallet.
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
                        val r = RecordImporter.import(p, roaxIssuerRegistry, roaxIssuerFactory)
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
                        val r = RecordImporter.import(p, roaxIssuerRegistry, roaxIssuerFactory)
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
 * Owner-hidden issuance. Resolve the pet profile, build and persist R on-device, reveal only R to the
 * issuer, then poll the server-owned custodial transaction to completion.
 */
private data class IssuedDogTag(val dogTagId: String, val root: String, val txHash: String)

@Composable
private fun IssuePanel(
    qr: QrPayload.DogTagIssueSession,
    activity: FragmentActivity,
    store: LocalStore,
) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    var working by remember { mutableStateOf(false) }
    var status by remember { mutableStateOf("") }
    var issued by remember { mutableStateOf<IssuedDogTag?>(null) }
    var err by remember { mutableStateOf("") }
    var session by remember { mutableStateOf<CentralApi.ProfileIssueSession?>(null) }
    var resolveErr by remember { mutableStateOf<String?>(null) }

    androidx.compose.runtime.LaunchedEffect(qr.token) {
        val resolved = withContext(Dispatchers.IO) {
            CentralApi.resolveProfileIssueSession(qr.host, qr.token)
        }
        if (resolved == null) resolveErr = "Could not resolve issuance session (expired or offline)."
        else session = resolved
    }

    Card {
        Text("Issue dog tag", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text("From ${qr.host}", fontSize = 12.sp, color = c.muted)
        Text("Token ${qr.token.take(18)}…", fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.muted)
        val resolved = session
        if (resolved == null) {
            Text(resolveErr ?: "Resolving pet profile…", fontSize = 12.sp,
                color = if (resolveErr == null) c.muted else c.danger)
        } else {
            Field("Pet", resolved.pet.name.ifBlank { "Unnamed" })
            Field("dogTagId", resolved.dogTagId)
            Text(
                "Your phone will build the owner-hidden profile tree and keep its secret locally. " +
                    "The vet receives only the Merkle root R.",
                fontSize = 12.sp, color = c.muted,
            )
        }

        if (issued == null && !working && resolved != null) {
            Button(
                onClick = {
                    err = ""
                    Biometric.prompt(
                        activity, "Issue dog tag", "Authenticate to create this tag's owner secret",
                        onSuccess = {
                            val wallet = runCatching { Wallet.load(context) }.getOrNull()
                            if (wallet == null) { err = "Create your wallet first (Profile)."; return@prompt }
                            val seedHex = runCatching { Wallet.seedHex(context) }.getOrNull()
                            if (seedHex == null) { err = "Wallet seed unavailable."; return@prompt }
                            working = true
                            status = "Building owner-hidden profile tree…"
                            scope.launch {
                                try {
                                    val treeStore = ProfileTreeStore(context)
                                    val existing = withContext(Dispatchers.IO) {
                                        treeStore.load().firstOrNull { it.dogTagIdDec == resolved.dogTagId }
                                    }
                                    // D1: identity leaves keep the VET's salts (the bind-time
                                    // full-leaf-list gate requires the posted identity openings to
                                    // EXACTLY match the vet's own {keyPath,salt,value} set); pet
                                    // attrs keep device-random salts. One combined list -
                                    // pet-then-identity - feeds the build, the retry compare, the
                                    // persisted openings, and the bind's leaf openings alike.
                                    val identityAttrs = resolved.identityLeaves.map { it.asAttribute() }
                                    val attributes: List<BackedUpAttribute>
                                    val tree: ProfileTreeFfi
                                    if (existing != null) {
                                        check(existing.derivationVersion == ProfileTreeStore.DERIVATION_VERSION) {
                                            "existing owner secret uses an unsupported derivation version"
                                        }
                                        check(existing.ownerAddress.equals(wallet.ethAddress, ignoreCase = true)) {
                                            "this dog tag belongs to a different wallet on this device"
                                        }
                                        check(resolved.matchesStored(existing.attributes)) {
                                            "issuance retry metadata differs from the persisted profile"
                                        }
                                        attributes = existing.attributes
                                        tree = withContext(Dispatchers.Default) {
                                            treeStore.verifyRecoverable(seedHex, existing)
                                            // Deterministic rebuild from the persisted record for
                                            // the reserved leaf hashes; nothing new is persisted.
                                            ProfileTreeBuilder.buildForIdField(
                                                seedHex = seedHex,
                                                dogTagIdFieldHex = existing.dogTagIdHex,
                                                ownerAddress = existing.ownerAddress,
                                                attributes = existing.attributes.map {
                                                    ProfileTreeBuilder.Attribute(it.keyPath, it.saltHex, it.tag, it.value)
                                                },
                                            )
                                        }
                                    } else {
                                        attributes = resolved.pet.attributes(::randomSalt16) + identityAttrs
                                        tree = withContext(Dispatchers.Default) {
                                            treeStore.buildAndPersist(
                                                seedHex = seedHex,
                                                dogTagIdDec = resolved.dogTagId,
                                                ownerAddress = wallet.ethAddress,
                                                attributes = attributes,
                                            )
                                        }
                                    }
                                    // The bind's full-leaf-list commitment: every attribute opening
                                    // plus the three OPAQUE reserved leaf hashes (never their
                                    // preimages) - the vet rebuilds R from exactly this list.
                                    val root = tree.rootHex
                                    val reservedLeafHashes = listOf(
                                        tree.ownerAddressLeafHex,
                                        tree.consentKeyLeafHex,
                                        tree.ownerSecretLeafHex,
                                    )

                                    status = "Sending the profile commitment to the vet…"
                                    val bind = withContext(Dispatchers.IO) {
                                        CentralApi.bindCustodialIssue(
                                            qr.host, qr.token, root, attributes, reservedLeafHashes,
                                        )
                                    }
                                    if (bind != null) {
                                        check(bind.dogTagId.isBlank() || bind.dogTagId == resolved.dogTagId) {
                                            "issuer returned a different dogTagId"
                                        }
                                        check(bind.root.isBlank() || bind.root.equals(root, ignoreCase = true)) {
                                            "issuer returned a different profile root"
                                        }
                                    }

                                    // A null bind means the response may have been lost after the
                                    // server accepted it, and the issuance-session status route is
                                    // operator-gated (the device holds no operator session). Confirm
                                    // completion straight from the public chain instead: `mintCustodial`
                                    // writes `DogTagSBTConsent.profileRoot(id) = R` once both txs mine,
                                    // keyed by the CANONICAL field-hashed id (never the raw handle).
                                    // Poll the already-persisted root; never generate a second set of salts.
                                    status = "Issuing owner-hidden root on-chain…"
                                    val roax = RoaxConfig.load(context)
                                    val onchainId = dogTagIdFieldHex(resolved.dogTagId)
                                    var anchored = false
                                    var delayMs = 2_000L
                                    for (attempt in 0 until 40) {
                                        val chainRoot = withContext(Dispatchers.IO) {
                                            RoaxRpc.profileRoot(AppConfig.ROAX_RPC, roax.dogTagSbt, onchainId)
                                        }
                                        when (RoaxRpc.classifyProfileRoot(chainRoot, root)) {
                                            RoaxRpc.ProfileRootObservation.Pending -> Unit
                                            RoaxRpc.ProfileRootObservation.Matched -> {
                                                anchored = true
                                                break
                                            }
                                            RoaxRpc.ProfileRootObservation.Mismatch ->
                                                error("dog tag is already anchored to a different profile root")
                                        }
                                        kotlinx.coroutines.delay(delayMs)
                                        delayMs = minOf(delayMs + 500L, 5_000L)
                                    }
                                    if (!anchored) {
                                        working = false
                                        status = "Submitted; anchoring is still pending. Check the vet portal for completion."
                                        return@launch
                                    }
                                    store.upsertPet(
                                        Pet(
                                            dogTagId = resolved.dogTagId,
                                            name = resolved.pet.name.ifBlank { "DogTag #${resolved.dogTagId}" },
                                            breed = resolved.pet.breedLabel.ifBlank { resolved.pet.breedVbo },
                                            ageLabel = resolved.pet.dateOfBirth,
                                            microchip = resolved.pet.microchip.code.takeIf { it.isNotBlank() },
                                        ),
                                    )
                                    issued = IssuedDogTag(resolved.dogTagId, root, bind?.txHash.orEmpty())
                                    working = false
                                    status = "Issued — owner hidden."
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
            ) { Text("Build & issue") }
        }
        if (working) {
            ForgingAnimation(status)
        }
        if (!working && status.isNotBlank()) {
            Text(status, fontSize = 12.sp, color = if (issued != null) c.success else c.muted)
        }
        if (err.isNotBlank()) Text(err, fontSize = 12.sp, color = c.danger)
    }

    issued?.let { res ->
        Card {
            Text("Dog tag issued", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.success)
            Field("dogTagId", res.dogTagId.ifBlank { "—" })
            Field("Root", res.root.take(18).ifBlank { "—" } + "…")
            if (res.txHash.isNotBlank()) Field("Tx", res.txHash.take(18) + "…")
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
        Field("Privacy", "Owner hidden")
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

    // D1 disclosure picker: the `owner.identity.*` leaves of the SELECTED tag, each an explicit
    // per-leaf opt-in for THIS verifier only. Defaults to nothing revealed; resets on re-selection.
    var identityLeaves by remember { mutableStateOf<List<BackedUpAttribute>>(emptyList()) }
    var revealKeyPaths by remember { mutableStateOf<Set<String>>(emptySet()) }
    androidx.compose.runtime.LaunchedEffect(sel?.id) {
        revealKeyPaths = emptySet()
        identityLeaves = if (sel == null) emptyList() else withContext(Dispatchers.IO) {
            runCatching {
                identityAttributesFor(
                    ProfileTreeStore(context).load().firstOrNull { it.dogTagIdDec == sel.dogTagId },
                )
            }.getOrDefault(emptyList())
        }
    }
    if (sel != null && identityLeaves.isNotEmpty()) {
        Card {
            Text("Share your identity (optional)", fontSize = 15.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text(
                "Each detail you switch on is revealed to this verifier, proven against your dog tag's " +
                    "sealed profile. Everything stays hidden by default.",
                fontSize = 12.sp, color = c.muted,
            )
            identityLeaves.forEach { leaf ->
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(identityLabelFor(leaf.keyPath), fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                        Text(leaf.value, fontSize = 12.sp, fontFamily = FontFamily.Monospace, color = c.muted)
                    }
                    Switch(
                        checked = leaf.keyPath in revealKeyPaths,
                        onCheckedChange = { on ->
                            revealKeyPaths = if (on) revealKeyPaths + leaf.keyPath else revealKeyPaths - leaf.keyPath
                        },
                    )
                }
            }
        }
    }

    var busy by remember { mutableStateOf(false) }
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
                    if (runCatching { Wallet.load(context) }.getOrNull() == null) {
                        err = "Create your wallet first (Profile)."
                        return@prompt
                    }
                    busy = true
                    scope.launch {
                        runLevelBFlow(
                            context = context,
                            sess = sess,
                            credential = sel,
                            host = qr.host,
                            token = qr.token,
                            groomerAddr = qr.groomerAddr,
                            revealKeyPaths = revealKeyPaths.toList(),
                            onStatus = onStatus,
                            onDone = { errMsg ->
                                busy = false
                                if (errMsg != null) {
                                    err = errMsg
                                    onStatus(errMsg)
                                }
                            },
                        )
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
 * The one owner-hidden consent flow: validate discovery, rebuild the private witness, prove on-device,
 * submit to the canonical consent route, and read back the detached on-chain broadcast.
 */
private suspend fun runLevelBFlow(
    context: android.content.Context,
    sess: CentralApi.ExportSession,
    credential: Credential,
    host: String,
    token: String,
    groomerAddr: String,
    revealKeyPaths: List<String>,
    onStatus: (String) -> Unit,
    onDone: (String?) -> Unit,
) {
    onStatus("Validating owner-hidden discovery anchor…")
    val claims = sess.claims
    if (claims == null) {
        onDone("Owner-hidden session is missing its discovery claims — refusing.")
        return
    }
    val roax = RoaxConfig.load(context)
    val version = AnchorResolver.PROTOCOL_VERSION
    // Resolve BOTH on-chain axes. Either null — registry unconfigured/undeployed, version unpublished,
    // or no artifact binding — fails closed.
    val cs = withContext(Dispatchers.IO) {
        RoaxRpc.getContractSet(AppConfig.ROAX_RPC, roax.protocolRegistry, version)
    }
    val arti = withContext(Dispatchers.IO) {
        RoaxRpc.getActiveArtifactSet(AppConfig.ROAX_RPC, roax.protocolRegistry, version)
    }
    if (cs == null || arti == null) {
        onDone("Owner-hidden verification is not available yet (discovery anchor unpublished).")
        return
    }
    // Build the FFI `TrustedAnchor`. `contractSetActive`/`artifactSetActive` come from the two records
    // SEPARATELY (never AND-ed) — `validateDiscovery` requires both true independently.
    val anchor = TrustedAnchor(
        version = version,
        versionId = cs.contractSetId,
        artifactSet = AnchorResolver.ARTIFACT_SET,
        artifactSetId = arti.artifactSetId,
        chainId = roax.chainId.toULong(),
        verificationRegistry = cs.verificationRegistry,
        circuitId = AnchorResolver.CIRCUIT_ID,
        minAppVersion = arti.minAppVersion,
        contractSetActive = cs.active,
        artifactSetActive = arti.active,
    )
    val ffiClaims = ConvenienceClaims(
        protocolVersion = claims.protocolVersion,
        chainId = claims.chainId.toULong(),
        verificationRegistry = claims.verificationRegistry,
        issuerClone = claims.issuerClone,
        purpose = claims.purpose,
    )
    // `appVersion`: this build's versionName (dotted semver core). `expectedPurpose`: the session's
    // purpose. The app has no purpose independent of the scanned QR today, so
    // `validateDiscovery`'s purpose check (§5.3 step 4) is INTENTIONALLY WEAK here — claim vs the same
    // session it came from. The load-bearing anti-redirect weight sits in the registry/chainId/
    // version/versionId/both-active/
    // minAppVersion checks, which all still fire. An independent app-side purpose is queued follow-up.
    try {
        withContext(Dispatchers.IO) {
            validateDiscovery(ffiClaims, anchor, BuildConfig.VERSION_NAME, sess.purpose)
        }

        // Hard-stop before proving if the scanned groomer is not authorized.
        onStatus("Checking groomer authorization…")
        val verifyKey = verifyWhitelistKeyHex(sess.purpose)
        val wl = withContext(Dispatchers.IO) {
            RoaxRpc.isWhitelistedFor(
                AppConfig.ROAX_RPC, roax.issuerRegistry, verifyKey, sess.relayer,
            )
        }
        if (wl !is RoaxRpc.Result.Valid) {
            onDone("This groomer is not authorized (not whitelisted).")
            return
        }
        if (!io.liberalize.dogtag.net.DnsVerify.isLocalHost(host)) {
            onStatus("Verifying groomer DNS…")
            val dnsOk = withContext(Dispatchers.IO) {
                io.liberalize.dogtag.net.DnsVerify.verifyGroomer(host, groomerAddr)
            }
            if (!dnsOk) {
                onDone("Groomer DNS not verified — refusing to present.")
                return
            }
        }

        // Wallet.seedHex(context) supplies the seed, while the
        // per-tag ProfileTreeStore record supplies the decimal handle, owner address and salted
        // attributes. ownerSecretHex never crosses this seam; proveConsent derives it internally.
        val seedHex = runCatching { Wallet.seedHex(context) }.getOrNull()
        if (seedHex == null) {
            onDone("Wallet seed unavailable — authenticate and try again.")
            return
        }
        // Use the throwing accessor: an encrypted store that exists but cannot be read is not the
        // same as "no secret", and owner-hidden proving must fail closed in that case.
        val owner = ProfileTreeStore(context).load()
            .firstOrNull { it.dogTagIdDec == credential.dogTagId }
        if (owner == null) {
            onDone("No owner-hidden secret exists for this dog tag.")
            return
        }
        if (owner.derivationVersion != ProfileTreeStore.DERIVATION_VERSION) {
            onDone("This owner-hidden secret uses an unsupported derivation version.")
            return
        }
        val attributesJson = org.json.JSONArray().apply {
            owner.attributes.forEach { attribute ->
                put(org.json.JSONObject().apply {
                    put("keyPath", attribute.keyPath)
                    put("salt", attribute.saltHex)
                    put("tag", attribute.tag.toInt())
                    put("value", attribute.value)
                })
            }
        }.toString()
        val nonceBytes = ByteArray(32).also { SecureRandom().nextBytes(it) }
        val consentNonce = "0x" + nonceBytes.joinToString("") { "%02x".format(it) }
        // The server requires >120s at preflight because broadcast is detached and retried. Ten
        // minutes leaves ample room for proving plus deferred settlement.
        val deadlineDec = ((System.currentTimeMillis() / 1000) + 600).toString()
        val descriptor = ZkeyAsset.resolve(AnchorResolver.PROTOCOL_VERSION)
        val zkeyPath = withContext(Dispatchers.IO) { ZkeyAsset.ensure(context, descriptor) }
        val graphPath = withContext(Dispatchers.IO) { ZkeyAsset.ensureGraph(context, descriptor) }

        onStatus("Generating owner-hidden proof…")
        val proof = withContext(Dispatchers.Default) {
            proveConsent(
                seedHex = seedHex,
                dogTagIdHandle = owner.dogTagIdDec,
                ownerAddressHex = owner.ownerAddress,
                attributesJson = attributesJson,
                purposeHex = labelFieldHex(sess.purpose),
                relayerHex = sess.relayer,
                recordTypeHex = labelFieldHex(sess.recordType),
                consentNonceHex = consentNonce,
                deadlineDec = deadlineDec,
                zkeyPath = zkeyPath,
                graphPath = graphPath,
            )
        }

        // The frozen consent nullifier is public signal 3; signal 4 is R.
        val nullifier = proof.pubSignals
            .getOrNull(PublicSignalIndex.NULLIFIER).orEmpty()
        if (nullifier.isBlank()) {
            onDone("Owner-hidden proof omitted its nullifier.")
            return
        }
        val verificationRegistry = cs.verificationRegistry
        val alreadyRecorded = withContext(Dispatchers.IO) {
            RoaxRpc.consumed(AppConfig.ROAX_RPC, verificationRegistry, nullifier)
        }
        if (alreadyRecorded) {
            onDone("This verification was already recorded.")
            return
        }

        val payloadJson = org.json.JSONObject().apply {
            put("exportToken", token)
            put("proof", org.json.JSONObject().apply {
                put("a", org.json.JSONArray(proof.a))
                put("b", org.json.JSONArray(proof.b.map { org.json.JSONArray(it) }))
                put("c", org.json.JSONArray(proof.c))
                put("pubSignals", org.json.JSONArray(proof.pubSignals))
            })
            // D1: the owner-picked identity disclosure rides ALONGSIDE the consent proof - same
            // session, same R - so it inherits the proof's relayer/deadline anti-replay binding.
            // Built by the SAME Rust core the verifier checks with; the proof stays leaf-blind.
            if (revealKeyPaths.isNotEmpty()) {
                val disclosure = buildProfileDisclosureJson(
                    seedHex,
                    dogTagIdFieldHex(owner.dogTagIdDec),
                    owner.ownerAddress,
                    owner.attributes.map { AttributeLeafFfi(it.keyPath, it.saltHex, it.tag, it.value) },
                    revealKeyPaths,
                )
                put("profileDisclosure", org.json.JSONObject(disclosure))
            }
        }.toString()
        onStatus("Submitting owner-hidden proof to groomer…")
        val response = withContext(Dispatchers.IO) {
            runCatching { CentralApi.postVerifyConsentToHost(host, payloadJson) }.getOrNull()
        }
        if (response != null && response.code in 400..499) {
            val reject = runCatching {
                org.json.JSONObject(response.body).optString("error", "")
            }.getOrNull().orEmpty()
            if (reject.isNotBlank()) {
                onDone("Submit rejected ($reject).")
                return
            }
        }

        // Detached-broadcast read-back: session status surfaces terminal errors while consumed(nf)
        // is canonical success; never an inline HTTP wait.
        onStatus("Recording your owner-hidden verification on-chain…")
        var done = false
        var failedMsg: String? = null
        for (attempt in 0 until 40) {
            val recorded = withContext(Dispatchers.IO) {
                RoaxRpc.consumed(AppConfig.ROAX_RPC, verificationRegistry, nullifier)
            }
            if (recorded) {
                done = true
                break
            }
            val session = withContext(Dispatchers.IO) {
                runCatching { CentralApi.verifySessionStatus(host, sess.sessionId, token) }.getOrNull()
            }
            if (session?.status == "error") {
                failedMsg = session.txHash?.ifBlank { null } ?: "recording failed"
                break
            }
            kotlinx.coroutines.delay(3000)
        }
        if (failedMsg != null) {
            onDone("Verification failed: $failedMsg")
            return
        }
        if (done) {
            val tx = withContext(Dispatchers.IO) {
                runCatching { CentralApi.verifySessionStatus(host, sess.sessionId, token) }.getOrNull()
            }?.txHash
            onStatus(
                if (!tx.isNullOrBlank()) "Verified on-chain — owner hidden. tx ${tx.take(14)}…"
                else "Verified on-chain — owner hidden.",
            )
        } else {
            onStatus("Submitted; awaiting confirmation.")
        }
        onDone(null)
    } catch (e: Exception) {
        onDone("Owner-hidden verification refused: ${e.message}")
    }
}

private fun labelFieldHex(label: String): String {
    if (label.startsWith("0x") && label.length == 66) return label
    if (label.isBlank()) return "0x" + "00".repeat(32)
    return "0x" + Keccak256.digest(label.toByteArray(Charsets.UTF_8))
        .joinToString("") { "%02x".format(it) }
}

/** The persisted `owner.identity.*` openings for a tag - the leaves the owner CAN disclose. */
private fun identityAttributesFor(record: io.liberalize.dogtag.profile.OwnerSecretRecord?): List<BackedUpAttribute> =
    record?.attributes?.filter { it.keyPath.startsWith("owner.identity.") } ?: emptyList()

/** Owner-readable label for an identity keyPath (falls back to the raw suffix). */
private fun identityLabelFor(keyPath: String): String = when (keyPath) {
    "owner.identity.fullName" -> "Full name"
    "owner.identity.country" -> "Country"
    "owner.identity.docNumber" -> "ID number"
    else -> keyPath.removePrefix("owner.identity.")
}

private fun randomSalt16(): String {
    val bytes = ByteArray(16).also { SecureRandom().nextBytes(it) }
    return "0x" + bytes.joinToString("") { "%02x".format(it) }
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
