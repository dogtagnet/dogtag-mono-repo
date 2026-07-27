package io.liberalize.dogtag.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
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
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.Pet
import io.liberalize.dogtag.data.RefreshCenter
import io.liberalize.dogtag.data.VerdictDisplay
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import kotlinx.coroutines.launch

@Composable
fun DocumentsScreen(onScan: () -> Unit, onOpen: (Credential) -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember { LocalStore.get(context) }
    val pets by store.pets.collectAsStateWithLifecycle()
    val creds by store.credentials.collectAsStateWithLifecycle()
    val scroll = rememberScrollState()

    // null == "All pets"
    var filterPetId by remember { mutableStateOf<String?>(null) }
    var pendingDelete by remember { mutableStateOf<Credential?>(null) }
    val shown = if (filterPetId == null) creds else creds.filter { it.dogTagId == filterPetId }

    Box(Modifier.fillMaxSize()) {
        Column(
            Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Documents", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)

            if (creds.isEmpty()) {
                EmptyState(
                    "No documents yet",
                    "Scan a vet or groomer's QR to import a verified record. Imported records appear here, " +
                        "grouped by dog.",
                    onScan,
                )
                return@Column
            }

            PetFilterRow(pets, filterPetId) { filterPetId = it }

            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.weight(1f)) { SectionTitle("Records", "${shown.size}") }
                Spacer(Modifier.size(8.dp))
                RefreshAllButton(shown)
            }
            if (shown.isEmpty()) {
                Text("No records for this dog yet.", fontSize = 13.sp, color = c.muted)
            }
            shown.forEach { cred ->
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface)
                        .clickable { onOpen(cred) }.padding(14.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    Box(
                        Modifier.size(38.dp).clip(CircleShape).background(c.surfaceVariant),
                        contentAlignment = Alignment.Center,
                    ) { Icon(Icons.Filled.Description, cred.title, tint = c.accent, modifier = Modifier.size(18.dp)) }
                    Spacer(Modifier.size(12.dp))
                    Column(Modifier.weight(1f)) {
                        Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                        Text("${cred.group.title} · ${cred.recordType}", fontSize = 12.sp, color = c.muted)
                        // The dog plus who issued it: both help tell two same-type records apart.
                        val who = pets.petLabel(cred)
                        val owner = if (cred.issuer.isBlank()) who else "$who · ${cred.issuer}"
                        Text(owner, fontSize = 11.sp, color = c.muted)
                        Text(cred.importedAtLabel, fontSize = 11.sp, color = c.muted)
                        CredentialStatusLine(cred)
                    }
                    Spacer(Modifier.size(8.dp))
                    Column(horizontalAlignment = Alignment.End) {
                        VerdictBadge(cred)
                        Spacer(Modifier.size(4.dp))
                        Row {
                            RefreshCredentialButton(cred)
                            IconButton(onClick = { pendingDelete = cred }, modifier = Modifier.size(32.dp)) {
                                Icon(
                                    Icons.Filled.Delete, "Delete ${cred.title}",
                                    tint = c.muted, modifier = Modifier.size(17.dp),
                                )
                            }
                        }
                    }
                }
            }
            Spacer(Modifier.size(24.dp))
        }

        pendingDelete?.let { cred ->
            DeleteCredentialDialog(
                cred = cred,
                onDismiss = { pendingDelete = null },
                onConfirm = { store.deleteCredential(cred.id); pendingDelete = null },
            )
        }
    }
}

/** A chip row with an "All pets" option plus one chip per dog. Shared by Travel + Documents. */
@Composable
fun PetFilterRow(pets: List<Pet>, selectedId: String?, onSelect: (String?) -> Unit) {
    val row = rememberScrollState()
    Row(
        Modifier.fillMaxWidth().horizontalScroll(row),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Chip("All pets", selectedId == null) { onSelect(null) }
        pets.forEach { p -> Chip(p.name, selectedId == p.dogTagId) { onSelect(p.dogTagId) } }
    }
}

@Composable
private fun Chip(label: String, selected: Boolean, onClick: () -> Unit) {
    val c = DogTagTheme.colors
    Box(
        Modifier.clip(CircleShape)
            .background(if (selected) c.accent else c.surfaceVariant)
            .clickable { onClick() }
            .padding(horizontal = 14.dp, vertical = 8.dp),
    ) {
        Text(label, fontSize = 13.sp, fontWeight = FontWeight.SemiBold,
            color = if (selected) c.onAccent else c.onBackground)
    }
}

/**
 * The one place a badge tone becomes colour. Both badge sizes and the status line read it, so the
 * palette cannot drift between the surfaces the way the verdict RULE once drifted between import and
 * refresh.
 *
 * NEUTRAL is deliberately the same muted chip UNVERIFIED already wears: "I could not check" and "I
 * have not checked recently" are the same claim about evidence, and neither may borrow the colour of
 * VALID or of INVALID.
 */
@Composable
internal fun badgeForeground(tone: VerdictDisplay.Tone): Color {
    val c = DogTagTheme.colors
    return when (tone) {
        VerdictDisplay.Tone.POSITIVE -> c.success
        VerdictDisplay.Tone.WARNING -> c.warning
        VerdictDisplay.Tone.NEGATIVE -> c.danger
        VerdictDisplay.Tone.NEUTRAL -> c.muted
    }
}

@Composable
private fun badgeBackground(tone: VerdictDisplay.Tone): Color =
    if (tone == VerdictDisplay.Tone.NEUTRAL) DogTagTheme.colors.surfaceVariant
    else badgeForeground(tone).copy(alpha = 0.18f)

/**
 * A record's on-chain standing, discounted by how long ago it was actually established. Takes the
 * whole [Credential] rather than a verdict string on purpose: the badge cannot be rendered honestly
 * from the verdict alone, and passing only the verdict is what let five surfaces claim VALID off a
 * check run days earlier.
 */
@Composable
internal fun VerdictBadge(cred: Credential) {
    val badge = cred.badge()
    Box(
        Modifier.clip(CircleShape).background(badgeBackground(badge.tone))
            .padding(horizontal = 10.dp, vertical = 4.dp),
    ) {
        Text(badge.label, fontSize = 10.sp, fontWeight = FontWeight.Bold, color = badgeForeground(badge.tone))
    }
}

// ---- shared status / refresh / delete affordances (Documents, Travel, Home, detail) --------------

/**
 * The freshness of a record's verdict, and why it reads the way it does. Three distinguishable
 * states: a check in flight, a completed check with its age, and a check that could not reach the
 * chain (which shows UNVERIFIED plus the reason, never VALID).
 */
@Composable
internal fun CredentialStatusLine(cred: Credential, fontSize: Int = 11) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val center = remember(context) { RefreshCenter.get(context) }
    val inFlight by center.inFlight.collectAsStateWithLifecycle()

    if (inFlight.contains(cred.id)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(modifier = Modifier.size(10.dp), strokeWidth = 1.5.dp, color = c.muted)
            Spacer(Modifier.size(5.dp))
            Text("Checking on-chain…", fontSize = fontSize.sp, color = c.muted)
        }
    } else {
        // Driven by the SAME tone as the badge beside it, so the loud claim and the quiet one cannot
        // disagree. Only the two negatives take a colour - a green sub-line under every healthy record
        // would just be noise, and a stale record must stay visually quiet rather than reassuring.
        //
        // ONE clock read for both, so the tone and the text can never straddle a UTC midnight and
        // disagree about whether the record is expired.
        val now = java.time.Instant.now()
        val tone = cred.badge(now).tone
        val colour = when (tone) {
            VerdictDisplay.Tone.NEGATIVE, VerdictDisplay.Tone.WARNING -> badgeForeground(tone)
            else -> c.muted
        }
        Text(cred.statusLine(now), fontSize = fontSize.sp, color = colour)
    }
}

/** Re-read ONE record's status from the chain. */
@Composable
internal fun RefreshCredentialButton(cred: Credential) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val center = remember(context) { RefreshCenter.get(context) }
    val inFlight by center.inFlight.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val checking = inFlight.contains(cred.id)

    IconButton(
        onClick = { scope.launch { center.refresh(cred) } },
        enabled = !checking,
        modifier = Modifier.size(32.dp),
    ) {
        if (checking) {
            CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp, color = c.accent)
        } else {
            Icon(
                Icons.Filled.Refresh, "Refresh ${cred.title} from the chain",
                tint = c.accent, modifier = Modifier.size(17.dp),
            )
        }
    }
}

/** Re-read every record currently listed. */
@Composable
internal fun RefreshAllButton(credentials: List<Credential>) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val center = remember(context) { RefreshCenter.get(context) }
    val remaining by center.batchRemaining.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val running = remaining > 0
    val enabled = !running && credentials.isNotEmpty()

    Box(
        Modifier.clip(CircleShape).background(c.surfaceVariant)
            .clickable(enabled = enabled) { scope.launch { center.refreshAll(credentials) } }
            .padding(horizontal = 12.dp, vertical = 7.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (running) {
                CircularProgressIndicator(modifier = Modifier.size(10.dp), strokeWidth = 1.5.dp, color = c.accent)
                Spacer(Modifier.size(5.dp))
                Text("Checking $remaining left", fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = c.accent)
            } else {
                Icon(Icons.Filled.Refresh, null, tint = c.accent, modifier = Modifier.size(13.dp))
                Spacer(Modifier.size(5.dp))
                Text(
                    "Refresh all", fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
                    color = if (enabled) c.accent else c.muted,
                )
            }
        }
    }
}

/** Which dog a record belongs to: the synced pet's name, else the bare dog-tag handle. */
internal fun List<Pet>.petLabel(cred: Credential): String =
    firstOrNull { it.dogTagId == cred.dogTagId }?.name ?: "DogTag #${cred.dogTagId}"

/**
 * The single home of the delete confirmation, so the wording cannot drift between the surfaces that
 * offer it. The copy is deliberately local-only: deleting removes this phone's copy and nothing else.
 *
 * The pet label is resolved HERE, off the shared store, rather than passed in: a caller-supplied
 * name is what let the two surfaces name the same record differently.
 */
@Composable
internal fun DeleteCredentialDialog(
    cred: Credential,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember(context) { LocalStore.get(context) }
    val pets by store.pets.collectAsStateWithLifecycle()
    AlertDialog(
        onDismissRequest = onDismiss,
        containerColor = c.surface,
        title = { Text("Delete ${cred.title}?", color = c.onBackground) },
        text = { Text(cred.deleteConfirmationMessage(pets.petLabel(cred)), fontSize = 13.sp, color = c.muted) },
        confirmButton = {
            TextButton(onClick = onConfirm) {
                Text("Delete from this phone", color = c.danger, fontWeight = FontWeight.SemiBold)
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel", color = c.onBackground) } },
    )
}
