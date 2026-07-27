package io.liberalize.dogtag.ui.screens

import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color as AndroidColor
import androidx.compose.foundation.Image
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.VerdictDisplay
import io.liberalize.dogtag.data.WrappedDoc
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.ui.DogTagTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.dogtag_standard.obfuscateDocumentJson
import java.time.Instant

/**
 * The pet-owner's mobile TRAVEL_CLEARANCE receipt — the CDC-modeled "show this on your phone" surface
 * (govarch-r8 §3, mirrors the web receipt `stacks/government/web/src/pages/Receipt.tsx` and the iOS
 * `TravelReceiptView`).
 *
 * Renders entirely from the LOCALLY stored `wrappedDocJson` (offline except the one live on-chain
 * status read): letterhead, effectiveStatus banner, Receipt ID / issuance / validity block, the legal
 * preamble, Section A/B/C tables, and a Verification block whose QR points at the PII-free public
 * status page `/r/:receiptId`.
 *
 * PRESENT is holder-controlled selective disclosure: Section-A person-PII leaves default to WITHHELD,
 * Section B/C default to shown, `dogTagId` is locked visible. "Share redacted copy" runs the merkle
 * `obfuscate()` (exposed in the mobile FFI as `obfuscateDocumentJson`) LOCALLY so the shared WrappedDoc
 * hides the withheld leaves while the tree still rebuilds to the same on-chain root R.
 */
@Composable
fun TravelReceiptScreen(cred: Credential, onBack: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scroll = rememberScrollState()
    // The amber "expired" accent. The theme's own `warning` token rather than a second hardcoded
    // #b45309 — the list badges paint EXPIRED from that token, and a literal here would guarantee the
    // two drifted (and would ignore dark mode, as this one did: light `warning` IS #b45309, dark is
    // #f59e0b, so only the dark rendering changes). Mirrors iOS `TravelReceiptView.amber`.
    val amber = c.warning
    val slate = Color(0xFF334155)

    val doc = remember(cred.wrappedDocJson) { runCatching { WrappedDoc(cred.wrappedDocJson) }.getOrNull() }

    // credentialSubject leaves flattened to dotted paths WITHOUT the `credentialSubject.` prefix.
    val subject = remember(doc) {
        val m = HashMap<String, String>()
        doc?.decodedFields()?.forEach { f ->
            if (f.keyPath.startsWith("credentialSubject.")) {
                m[f.keyPath.removePrefix("credentialSubject.")] = f.value
            }
        }
        m
    }
    fun pick(path: String): String = subject[path] ?: ""
    fun isTrue(path: String): Boolean = pick(path).lowercase().let { it == "true" || it == "1" || it == "yes" }

    val isTravel = (cred.recordType.ifBlank { doc?.recordType ?: "" }).uppercase().contains("TRAVEL")
    val receiptId = pick("receiptId")
    val issuanceDate = pick("validity.issuedOn").ifBlank { cred.issuedOn }
    // The SHARED expiry accessor, not a private `pick("validity.validUntil")`. This sheet renders
    // EU_HEALTH_CERT too (CredentialGroup maps it to Travel), and that type states its window in the
    // flat Annex-IV `rabiesValidUntil` with no `validity` block at all — so picking the nested leaf
    // here left the pill claiming VALID on a document the list badge had already called EXPIRED.
    val validUntil = doc?.validUntil ?: ""

    // Public PII-free status URL: <protocol.statusBaseUrl>/r/<receiptId>.
    //
    // The base is stamped at issuance from the issuer's DEPLOYMENT_URL — the only host in the document
    // a phone can actually reach. It deliberately does NOT fall back to issuer.domain: that is a
    // did:web identity, and the shipped default `gov.example` is RFC-2606 reserved (NXDOMAIN), so
    // every QR built from it encoded a dead link that reads as a working live-status check.
    //
    // Blank when the issuer stamped no base — including every pre-stamping document. The card then
    // renders no QR and says so, rather than printing a URL that resolves to nothing.
    val publicUrl = run {
        val base = doc?.statusBaseUrl ?: ""
        if (base.isBlank() || receiptId.isBlank()) "" else "$base/r/$receiptId"
    }

    // Section-A person/importer leaves — the PII block that defaults to WITHHELD when presenting.
    val sectionAPaths = listOf(
        "First Name" to "importer.firstName",
        "Middle Name/Initial" to "importer.middleName",
        "Last Name" to "importer.lastName",
        "Role" to "importer.role",
        "Identification Type" to "importer.idType",
        "ID Jurisdiction" to "importer.idJurisdiction",
        "Identification No." to "importer.idNumber",
        "Date of Birth" to "importer.dateOfBirth",
        "Email" to "importer.email",
        "Phone Number" to "importer.phone",
        "Consignee/Additional Owner" to "consignee.fullName",
        "Consignee Email" to "consignee.email",
        "Consignee ID Type" to "consignee.idType",
    )
    val presentSectionA = sectionAPaths.filter { pick(it.second).isNotBlank() }

    var withheld by remember { mutableStateOf(presentSectionA.map { it.second }.toSet()) }
    var shareError by remember { mutableStateOf<String?>(null) }
    var live by remember { mutableStateOf<RoaxRpc.Result>(RoaxRpc.Result.Unknown("checking…")) }

    LaunchedEffect(cred.id) {
        val d = doc ?: return@LaunchedEffect
        val root = d.merkleRoot.ifBlank { cred.credentialRoot }
        live = RoaxRpc.isValid(AppConfig.ROAX_RPC, d.documentStore, root)
    }

    // effectiveStatus: a live revoke wins, then a lapsed validity window, else VALID; chain-unreachable
    // falls back to the stored verdict — but ONLY while that stored verdict is still fresh.
    //
    // The lapse test is `VerdictDisplay.lapsed`, the same one the list badges use. It lived here as a
    // private inline comparison, which is how this sheet came to be the only mobile surface that
    // enforced expiry at all: one rule with one implementation cannot be half-adopted.
    //
    // The freshness gate on the Unknown arm closes the other half. This pill previously read a green
    // VALID off `cred.verdict` whenever the chain was unreachable, no matter how old that stored
    // answer was, while the sub-line underneath said "On-chain status unconfirmed" — two contradictory
    // claims on one screen with the loud one wrong. An unreachable chain plus a stale stored answer is
    // exactly "I could not check", and UNCONFIRMED is what that looks like.
    val now = remember { Instant.now() }
    val lapsed = VerdictDisplay.lapsed(validUntil, now)
    val (effLabel, effColor) = when (live) {
        is RoaxRpc.Result.Invalid -> "REVOKED" to c.danger
        is RoaxRpc.Result.Valid -> if (lapsed) "EXPIRED" to amber else "VALID" to c.success
        is RoaxRpc.Result.Unknown ->
            if (lapsed) "EXPIRED" to amber
            else if (cred.verdict == "VALID" && VerdictDisplay.isFresh(cred.lastCheckedAt, now)) {
                "VALID" to c.success
            } else {
                "UNCONFIRMED" to c.muted
            }
    }

    fun shareRedacted() {
        shareError = null
        val paths = withheld.map { "credentialSubject.$it" }
        val redacted = try {
            if (paths.isEmpty()) cred.wrappedDocJson
            else obfuscateDocumentJson(cred.wrappedDocJson, paths)
        } catch (e: Exception) {
            shareError = "Could not redact: ${e.message}"
            return
        }
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "application/json"
            putExtra(Intent.EXTRA_TEXT, redacted)
            putExtra(Intent.EXTRA_SUBJECT, "DogTag travel receipt${if (receiptId.isNotBlank()) " $receiptId" else ""}")
        }
        context.startActivity(Intent.createChooser(send, "Share redacted receipt"))
    }

    Column(
        Modifier.fillMaxSize().background(c.background).verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Top bar: back + share.
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(c.surfaceVariant).clickable { onBack() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.ArrowBack, "Back", tint = c.onBackground) }
            Spacer(Modifier.weight(1f))
            Box(
                Modifier.clip(RoundedCornerShape(20.dp)).background(c.accent)
                    .clickable { shareRedacted() }.padding(horizontal = 14.dp, vertical = 8.dp),
            ) { Text("Share redacted", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = c.onAccent) }
        }

        // Selective-disclosure controls.
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface).padding(14.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Selective disclosure", fontSize = 13.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text(
                "Person (Section A) details are hidden by default. Reveal only what an official needs; hidden fields are stripped from a shared copy but still verify against the on-chain root.",
                fontSize = 11.sp, color = c.muted,
            )
            if (presentSectionA.isEmpty()) {
                Text("This receipt carries no Section-A personal fields.", fontSize = 11.sp, color = c.muted)
            } else {
                presentSectionA.forEach { (label, path) ->
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Text("Reveal $label", fontSize = 12.sp, color = c.onSurface, modifier = Modifier.weight(1f))
                        Switch(
                            checked = !withheld.contains(path),
                            onCheckedChange = { reveal ->
                                withheld = if (reveal) withheld - path else withheld + path
                            },
                            colors = SwitchDefaults.colors(checkedThumbColor = c.accent),
                        )
                    }
                }
            }
            shareError?.let { Text(it, fontSize = 11.sp, color = c.danger) }
        }

        // Receipt sheet.
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(c.surface)
                .border(1.dp, c.outline, RoundedCornerShape(18.dp)),
        ) {
            // Letterhead.
            Row(
                Modifier.fillMaxWidth().background(c.surfaceVariant).padding(16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("🏛", fontSize = 26.sp)
                Column {
                    Text(doc?.issuerName ?: "Competent Authority", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                    Text("Pet Travel Clearance · authority-endorsed credential", fontSize = 12.sp, color = c.muted)
                }
            }

            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
                // Title + status.
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        if (isTravel) "Pet Travel Clearance Receipt" else "Pet Health Certificate",
                        fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground,
                        modifier = Modifier.weight(1f),
                    )
                    Box(Modifier.clip(RoundedCornerShape(20.dp)).background(effColor).padding(horizontal = 10.dp, vertical = 5.dp)) {
                        Text("● $effLabel", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Color.White)
                    }
                }

                // Identity block.
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Receipt ID: ${receiptId.ifBlank { "—" }}", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                    Kv("Date of issuance", issuanceDate.ifBlank { "—" })
                    Kv(if (isTrue("validity.multipleEntries")) "Valid for multiple entries until" else "Valid until", validUntil.ifBlank { "—" })
                    Kv("Dog Tag ID (SBT)", "#${pick("dogTagId").ifBlank { cred.dogTagId }}")
                }

                // Legal preamble.
                val binding = pick("validity.countryOfDepartureBinding")
                Text(
                    buildAnnotatedString {
                        withStyle(SpanStyle(color = c.muted)) {
                            append("This receipt is valid for the animal listed for the validity window shown above")
                            if (binding.isNotBlank()) append(", for entry from the listed country of departure ($binding)")
                            append(". If the animal travels via a different or high-risk country, a new clearance may be required. ")
                        }
                        withStyle(SpanStyle(color = c.onBackground, fontWeight = FontWeight.Bold)) {
                            append("You must show this receipt (printed or on your phone) to airline staff and port-of-entry officials.")
                        }
                        withStyle(SpanStyle(color = c.muted)) {
                            append(" The authority reserves the right to request additional supporting documentation on arrival.")
                        }
                    },
                    fontSize = 12.sp,
                )

                (doc?.obfuscatedCount ?: 0).let { n ->
                    if (n > 0) Text(
                        "Note: $n field(s) were withheld by the holder in this copy; withheld leaves still verify against the on-chain root.",
                        fontSize = 11.sp, color = amber,
                    )
                }

                // Sectioned content.
                if (isTravel) {
                    val sex = pick("animal.sex")
                    val sexValue = if (sex.isBlank()) "" else
                        sex.replaceFirstChar { it.uppercase() } + (if (isTrue("animal.neutered")) " Neutered" else "")
                    SectionTable("Section A - Person Importing the Animal", slate, presentSectionA.map { (label, path) ->
                        val v = if (path == "importer.role" || path == "importer.idType" || path == "consignee.idType") humanize(pick(path)) else pick(path)
                        Row3(label, v, withheld.contains(path))
                    }.filter { it.value.isNotBlank() })
                    SectionTable("Section B - Animal Information", slate, listOf(
                        Row3("Animal Name", pick("animal.name"), false),
                        Row3("Age - Year(s)", pick("animal.ageYears"), false),
                        Row3("Age - Month(s)", pick("animal.ageMonths"), false),
                        Row3("Sex", sexValue, false),
                        Row3("Dog Breed", pick("animal.breed"), false),
                        Row3("Color/Markings", pick("animal.colorMarkings"), false),
                        Row3("Microchip Number", pick("animal.microchipNumber"), false),
                        Row3("Importation Purpose", humanize(pick("animal.importationPurpose")), false),
                    ).filter { it.value.isNotBlank() })
                    SectionTable("Section C - Travel Information", slate, listOf(
                        Row3("Travel Type", humanize(pick("travel.travelType")), false),
                        Row3("Country or Area of Departure", pick("travel.countryOfDeparture"), false),
                        Row3("Date of Arrival", pick("travel.dateOfArrival"), false),
                        Row3("Port of Entry", pick("travel.portOfEntry"), false),
                        Row3("Carrier / Flight", pick("travel.carrierOrFlight"), false),
                    ).filter { it.value.isNotBlank() })
                } else {
                    SectionTable("Health Certificate (Annex IV)", slate, listOf(
                        Row3("Species", pick("species"), false),
                        Row3("Microchip Number", pick("microchipNumber"), false),
                        Row3("Rabies Vaccination Date", pick("rabiesVaccinationDate"), false),
                        Row3("Rabies Valid Until", pick("rabiesValidUntil"), false),
                        Row3("Examining Veterinarian", pick("examiningVeterinarian"), false),
                        Row3("Clinical Health Status", humanize(pick("clinicalHealthStatus")), false),
                        Row3("Examination Date", pick("examinationDate"), false),
                    ).filter { it.value.isNotBlank() })
                }

                // Verification block.
                VerificationBlock(
                    slate = slate,
                    publicUrl = publicUrl,
                    root = doc?.merkleRoot?.ifBlank { cred.credentialRoot } ?: cred.credentialRoot,
                    documentStore = doc?.documentStore ?: "",
                    live = live,
                )
            }
        }
        Spacer(Modifier.size(24.dp))
    }
}

private data class Row3(val label: String, val value: String, val withheld: Boolean)

@Composable
private fun Kv(label: String, value: String) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top) {
        Text(label, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = c.muted, modifier = Modifier.width(150.dp))
        Text(value, fontSize = 13.sp, color = c.onSurface, modifier = Modifier.weight(1f))
    }
}

@Composable
private fun SectionTable(title: String, slate: Color, rows: List<Row3>) {
    if (rows.isEmpty()) return
    val c = DogTagTheme.colors
    val amber = c.warning
    Column(Modifier.fillMaxWidth().border(1.dp, c.outline)) {
        Text(
            title, fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Color.White,
            modifier = Modifier.fillMaxWidth().background(slate).padding(horizontal = 12.dp, vertical = 8.dp),
        )
        rows.forEachIndexed { i, r ->
            Row(
                Modifier.fillMaxWidth()
                    .background(if (i % 2 == 0) c.surface else c.surfaceVariant.copy(alpha = 0.5f))
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.Top,
            ) {
                Text(r.label, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = c.muted, modifier = Modifier.width(130.dp))
                if (r.withheld) {
                    Text("— withheld by holder —", fontSize = 13.sp, fontWeight = FontWeight.Medium, color = amber, modifier = Modifier.weight(1f))
                } else {
                    Text(r.value, fontSize = 13.sp, color = c.onSurface, modifier = Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun VerificationBlock(slate: Color, publicUrl: String, root: String, documentStore: String, live: RoaxRpc.Result) {
    val c = DogTagTheme.colors
    var qr by remember(publicUrl) { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
    LaunchedEffect(publicUrl) {
        qr = if (publicUrl.isBlank()) null else withContext(Dispatchers.Default) { encodeQr(publicUrl, 232) }
    }
    Column(Modifier.fillMaxWidth().border(1.dp, c.outline)) {
        Text(
            "Verification", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = Color.White,
            modifier = Modifier.fillMaxWidth().background(slate).padding(horizontal = 12.dp, vertical = 8.dp),
        )
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.Top) {
                qr?.let {
                    Image(it, "Public status QR", modifier = Modifier.size(116.dp).clip(RoundedCornerShape(6.dp)).background(Color.White))
                }
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        if (publicUrl.isBlank()) "No public status page for this receipt"
                        else "Scan to confirm live status on-chain",
                        fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground,
                    )
                    Text(
                        publicUrl.ifBlank {
                            "This credential's issuer published no reachable status URL. " +
                                "Confirm it against the on-chain record below."
                        },
                        fontSize = 11.sp, color = c.muted, maxLines = 3, overflow = TextOverflow.Ellipsis,
                    )
                    Text(
                        "The public page shows NO personal data — only the live VALID / EXPIRED / REVOKED verdict and provenance.",
                        fontSize = 10.sp, color = c.muted,
                    )
                }
            }
            // Live status strip.
            val (text, color) = when (live) {
                is RoaxRpc.Result.Valid -> "On-chain: anchored & valid" to c.success
                is RoaxRpc.Result.Invalid -> "On-chain: REVOKED / not anchored" to c.danger
                is RoaxRpc.Result.Unknown -> "On-chain status unconfirmed (${live.reason})" to c.muted
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Box(Modifier.size(9.dp).clip(CircleShape).background(color))
                Text(text, fontSize = 12.sp, fontWeight = FontWeight.Medium, color = color)
            }
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text("Credential root", fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = c.muted)
                Text(root, fontSize = 11.sp, fontFamily = FontFamily.Monospace, color = c.onSurface, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (documentStore.isNotBlank()) {
                Text("Anchored: ROAX chainId 135 · issuer ${documentStore.take(12)}…", fontSize = 11.sp, color = c.muted)
            }
        }
    }
}

/** Humanize a snake_case enum value ("service_animal" → "Service Animal"); leave codes/blank as-is. */
private fun humanize(v: String): String = when {
    v.isBlank() -> ""
    v == "true" -> "Yes"
    v == "false" -> "No"
    !v.contains('_') -> v
    else -> v.split('_').joinToString(" ") { it.replaceFirstChar { ch -> ch.uppercase() } }
}

/** Encode a QR code (PII-free public status URL) to an ImageBitmap. zxing-core, offline. */
private fun encodeQr(value: String, size: Int): androidx.compose.ui.graphics.ImageBitmap? = runCatching {
    val matrix = QRCodeWriter().encode(value, BarcodeFormat.QR_CODE, size, size)
    val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.RGB_565)
    for (x in 0 until size) for (y in 0 until size) {
        bmp.setPixel(x, y, if (matrix.get(x, y)) AndroidColor.BLACK else AndroidColor.WHITE)
    }
    bmp.asImageBitmap()
}.getOrNull()
