package io.liberalize.dogtag.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.background
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Domain
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.RemoveCircleOutline
import androidx.compose.material3.Icon
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.liberalize.dogtag.data.AppConfig
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.CredentialGroup
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.WrappedDoc
import io.liberalize.dogtag.net.BindingTone
import io.liberalize.dogtag.net.IssuerBinding
import io.liberalize.dogtag.net.IssuerBindingResolver
import io.liberalize.dogtag.net.IssuerBindingState
import io.liberalize.dogtag.net.IssuerDidAssertion
import io.liberalize.dogtag.net.IssuerIdentity
import io.liberalize.dogtag.ui.DogTagTheme

/**
 * Full-screen credential detail. Shows the verdict + dogTagId header, the on-chain bits (Merkle root,
 * issuer domain, recordType), and every decoded Merkle leaf (the underlying record fields).
 */
@Composable
fun CredentialDetailScreen(opened: Credential, onBack: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scroll = rememberScrollState()
    val store = remember(context) { LocalStore.get(context) }
    val stored by store.credentials.collectAsStateWithLifecycle()
    // Always render the STORED record, not the copy this screen was opened with, so a refresh
    // landing while it is up is reflected here too. Falls back to the opened copy if it was deleted.
    val cred = stored.firstOrNull { it.id == opened.id } ?: opened
    var pendingDelete by remember { mutableStateOf(false) }
    var showReceipt by remember { mutableStateOf(false) }
    if (showReceipt) {
        TravelReceiptScreen(cred, onBack = { showReceipt = false })
        return
    }

    val doc = remember(cred.wrappedDocJson) {
        runCatching { WrappedDoc(cred.wrappedDocJson) }.getOrNull()
    }
    val fields = remember(doc) { doc?.decodedFields().orEmpty() }

    // The issuer↔domain binding. Starts Pending and is filled by a REAL resolution — never pre-filled
    // with a guess, because flashing "no domain claimed" and then flipping would show a state that was
    // never observed. Served from a short TTL cache, so this is not a DNS lookup per render.
    var binding by remember { mutableStateOf(IssuerBinding()) }
    LaunchedEffect(cred.id) {
        val clone = (doc?.documentStore ?: "").trim()
        if (clone.isBlank()) return@LaunchedEffect
        val roax = RoaxConfig.load(context)
        binding = IssuerBindingResolver.resolve(
            rpcUrl = AppConfig.ROAX_RPC,
            factory = roax.issuerFactory,
            domainRegistry = roax.issuerDomainRegistry,
            clone = clone,
        )
    }

    Column(
        Modifier.fillMaxSize().background(c.background).verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        // Back row.
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(c.surfaceVariant).clickable { onBack() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.ArrowBack, "Back", tint = c.onBackground) }
            Spacer(Modifier.size(12.dp))
            Text("Credential", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        }

        // Header card: record type + verdict + dogTagId.
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    cred.title.ifBlank { doc?.displayTitle() ?: "Record" },
                    fontSize = 20.sp, fontWeight = FontWeight.Bold, color = c.onBackground,
                    modifier = Modifier.weight(1f),
                )
                DetailVerdictBadge(cred.verdict)
            }
            val rt = cred.recordType.ifBlank { doc?.recordType ?: "" }
            if (rt.isNotBlank()) Text(rt, fontSize = 13.sp, color = c.muted)
            val tag = cred.dogTagId.ifBlank { doc?.dogTagId ?: "" }
            if (tag.isNotBlank()) Text("DogTag #$tag", fontSize = 13.sp, color = c.muted)
        }

        // Travel credentials get the CDC-modeled receipt as their "present to an official" surface
        // (holder-controlled selective disclosure + PII-free public-status QR).
        if (cred.group == CredentialGroup.Travel) {
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.accent)
                    .clickable { showReceipt = true }.padding(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Filled.Description, null, tint = c.onAccent)
                Spacer(Modifier.size(10.dp))
                Text("Show travel receipt", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onAccent, modifier = Modifier.weight(1f))
                Icon(Icons.Filled.KeyboardArrowRight, null, tint = c.onAccent)
            }
        }

        // Status + provenance: what the chain last said, how old that answer is, and when this phone
        // took the record in. The refresh button re-reads the chain right here.
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Status", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = c.muted,
                    modifier = Modifier.weight(1f),
                )
                RefreshCredentialButton(cred)
            }
            CredentialStatusLine(cred, fontSize = 13)
            KeyValueRow("Imported", cred.importedAtValue)
            Text(
                "Refreshing re-reads this record's anchor on ROAX. If the chain cannot be reached the " +
                    "status becomes UNVERIFIED with the reason: not being able to check is not the same " +
                    "as checking and finding it good.",
                fontSize = 11.sp, color = c.muted,
            )
        }

        // On-chain card.
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text("On-chain", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = c.muted)
            val root = doc?.merkleRoot?.ifBlank { cred.credentialRoot } ?: cred.credentialRoot
            MonoCopyRow(context, "Merkle root", root)

            // The issuer, as it may honestly be shown: the ON-CHAIN name and the ON-CHAIN domain. The
            // credential's own `issuer` block is outside the Merkle root, so it is never presented as the
            // issuer — only as a stated disagreement. That is what stops a relabelled document from
            // displaying a fabricated authority.
            val docName = (doc?.issuerName ?: "").trim()
            val docDomain = (doc?.issuerDomain ?: "").trim()
            val chainName = binding.onchainName.trim()
            if (chainName.isNotBlank()) {
                KeyValueRow("Issuer", chainName)
                if (docName.isNotBlank() && normaliseName(docName) != normaliseName(chainName)) {
                    // Deliberately MUTED, not red: free-form labels drift between what an issuing stack
                    // stamps and what the clone was created with, so a red line here would fire on
                    // legitimate credentials and train people to ignore it. The authoritative name is
                    // already displayed above; this is a footnote. The DOMAIN discrepancy below stays red —
                    // that one is a structured comparison against a root-covered value.
                    Text(
                        "The document names a different issuer: \u201C$docName\u201D",
                        fontSize = 11.sp, color = c.muted,
                    )
                }
            } else if (docName.isNotBlank()) {
                // A fallback is never presented as authoritative.
                KeyValueRow("Issuer (from document)", docName)
            }
            if (binding.domain.isNotBlank()) {
                KeyValueRow("Issuer domain", binding.domain)
                if (docDomain.isNotBlank() && !docDomain.equals(binding.domain, ignoreCase = true)) {
                    Text(
                        "The document claims the domain \u201C$docDomain\u201D",
                        fontSize = 11.sp, color = c.danger,
                    )
                }
            } else if (docDomain.isNotBlank()) {
                KeyValueRow("Issuer domain (from document)", docDomain)
            }

            // The DID assertion — the OTHER half of issuer identity, required alongside the DNS binding.
            // DNS proves the domain owner vouches for the address; this proves the document has not been
            // relabelled since issuance. It works even when the binding is unavailable, because it needs
            // nothing but the document itself.
            when (val a = IssuerIdentity.assertDomain(doc)) {
                is IssuerDidAssertion.Mismatch -> Text(
                    "This credential was issued under \u201C${a.rootCovered}\u201D, but the document " +
                        "claims \u201C${a.displayed}\u201D",
                    fontSize = 11.sp, color = c.danger,
                )
                // Deliberately NOT a pass, and said so rather than left silent.
                IssuerDidAssertion.NotAssertable -> Text(
                    "This document carries no issuer identity inside its Merkle root, so its domain " +
                        "could not be cross-checked.",
                    fontSize = 11.sp, color = c.muted,
                )
                is IssuerDidAssertion.Match -> Unit
            }

            DomainBindingLine(binding)
            val rt = doc?.recordType ?: cred.recordType
            if (rt.isNotBlank()) KeyValueRow("Record type", rt)
            Text(
                "Anchored on the verification registry. Look the Merkle root up on the chain explorer to confirm validity.",
                fontSize = 11.sp, color = c.muted,
            )
        }

        // Decoded fields card.
        Text("Credential fields", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        if (fields.isEmpty()) {
            Text(
                "No readable fields could be decoded from this credential.",
                fontSize = 13.sp, color = c.muted,
            )
        } else {
            Column(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(4.dp),
            ) {
                fields.forEach { f ->
                    KeyValueRow(f.label, f.value, padding = 12)
                }
            }
        }

        val redacted = doc?.obfuscatedCount ?: 0
        if (redacted > 0) {
            Text(
                "$redacted field(s) redacted (selective disclosure)",
                fontSize = 12.sp, color = c.muted,
            )
        }

        Row(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp))
                .background(c.danger.copy(alpha = 0.12f))
                .clickable { pendingDelete = true }
                .padding(vertical = 13.dp),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Filled.Delete, null, tint = c.danger, modifier = Modifier.size(17.dp))
            Spacer(Modifier.size(8.dp))
            Text("Delete from this phone", fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.danger)
        }
        Spacer(Modifier.size(24.dp))
    }

    if (pendingDelete) {
        DeleteCredentialDialog(
            cred = cred,
            onDismiss = { pendingDelete = false },
            onConfirm = { pendingDelete = false; store.deleteCredential(cred.id); onBack() },
        )
    }
}

@Composable
private fun KeyValueRow(label: String, value: String, padding: Int = 0) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().padding(horizontal = padding.dp, vertical = if (padding > 0) 8.dp else 0.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Text(label, fontSize = 12.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
        Text(value, fontSize = 14.sp, color = c.onBackground)
    }
}

@Composable
private fun MonoCopyRow(context: Context, label: String, value: String) {
    val c = DogTagTheme.colors
    val shown = if (value.length > 18) "${value.take(10)}…${value.takeLast(6)}" else value
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Text(label, fontSize = 12.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
        Row(
            Modifier.fillMaxWidth().clickable {
                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                cm.setPrimaryClip(ClipData.newPlainText(label, value))
                Toast.makeText(context, "$label copied", Toast.LENGTH_SHORT).show()
            },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                shown.ifBlank { "—" }, fontSize = 13.sp, color = c.onBackground,
                fontFamily = FontFamily.Monospace, modifier = Modifier.weight(1f),
            )
            if (value.isNotBlank()) {
                Icon(Icons.Filled.ContentCopy, "Copy", tint = c.muted, modifier = Modifier.size(16.dp))
            }
        }
    }
}

@Composable
private fun DetailVerdictBadge(verdict: String) {
    val c = DogTagTheme.colors
    val (bg, fg) = when (verdict) {
        "VALID" -> c.success.copy(alpha = 0.18f) to c.success
        "INVALID" -> c.danger.copy(alpha = 0.18f) to c.danger
        else -> c.surfaceVariant to c.muted
    }
    Box(Modifier.clip(CircleShape).background(bg).padding(horizontal = 12.dp, vertical = 5.dp)) {
        Text(verdict, fontSize = 11.sp, fontWeight = FontWeight.Bold, color = fg)
    }
}

/** Casefold + collapse whitespace, for comparing free-form issuer labels. A padding difference is not
 *  evidence of anything, and flagging it would cry wolf on legitimate credentials. */
private fun normaliseName(s: String): String =
    s.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ").lowercase()

/**
 * The badge: ONE compact line beside the issuer, not a panel.
 *
 * `Verified` is a small green check plus the factual line, with the description the domain owner
 * published. `NotListed` is the symmetric red line — same size, same placement, same register. A
 * provenance failure gets its own mark and its own words. Everything we simply do not know stays neutral,
 * because a resolver timeout is evidence of nothing.
 *
 * Nothing here reads as a judgement on the credential or the organisation: a missing DNS record means the
 * domain owner has not published the binding, and the credential's validity is proven separately
 * on-chain.
 */
@Composable
fun DomainBindingLine(binding: IssuerBinding) {
    val c = DogTagTheme.colors
    val color = when (binding.tone) {
        BindingTone.Positive -> c.success
        BindingTone.Negative -> c.danger
        BindingTone.Neutral, BindingTone.Pending -> c.muted
    }
    val icon = when (binding.state) {
        is IssuerBindingState.Verified -> Icons.Filled.Check
        IssuerBindingState.NotADogTagIssuer -> Icons.Filled.Domain
        IssuerBindingState.NotListed -> Icons.Filled.RemoveCircleOutline
        IssuerBindingState.Pending -> Icons.Filled.MoreHoriz
        else -> Icons.Filled.Info
    }

    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        Row(verticalAlignment = Alignment.Top) {
            Icon(icon, null, tint = color, modifier = Modifier.size(13.dp))
            Spacer(Modifier.size(5.dp))
            Text(binding.line, fontSize = 11.sp, color = color, modifier = Modifier.weight(1f))
        }
        binding.publishedDescription?.let { published ->
            // The whole reason the TXT value is free-form: show what the domain chose to say.
            Text(
                "Domain says: \u201C$published\u201D",
                fontSize = 11.sp, color = c.muted,
                modifier = Modifier.padding(start = 18.dp),
            )
        }
        binding.provenanceLine()?.let { provenance ->
            // A "verified" with no "when" is not auditable against a world where DNS changes. The DNS
            // clause appears only when a DNS query really ran \u2014 see `provenanceLine`.
            Text(
                provenance,
                fontSize = 10.sp, color = c.muted,
                modifier = Modifier.padding(start = 18.dp),
            )
        }
    }
}
