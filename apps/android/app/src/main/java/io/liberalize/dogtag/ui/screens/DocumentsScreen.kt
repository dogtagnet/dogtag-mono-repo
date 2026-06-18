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
import androidx.compose.material.icons.filled.Description
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
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
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle

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
    val shown = if (filterPetId == null) creds else creds.filter { it.dogTagId == filterPetId }

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

        SectionTitle("Records", "${shown.size}")
        if (shown.isEmpty()) {
            Text("No records for this dog yet.", fontSize = 13.sp, color = c.muted)
        }
        shown.forEach { cred ->
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface)
                    .clickable { onOpen(cred) }.padding(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier.size(38.dp).clip(CircleShape).background(c.surfaceVariant),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.Filled.Description, cred.title, tint = c.accent, modifier = Modifier.size(18.dp)) }
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    Text("${cred.group.title} · ${cred.recordType}", fontSize = 12.sp, color = c.muted)
                    val petName = pets.firstOrNull { it.dogTagId == cred.dogTagId }?.name ?: "DogTag #${cred.dogTagId}"
                    Text(petName, fontSize = 11.sp, color = c.muted)
                }
                VerdictBadge(cred.verdict)
            }
        }
        Spacer(Modifier.size(24.dp))
    }
}

/** A chip row with an "All pets" option plus one chip per dog. Shared by Travel + Documents. */
@Composable
fun PetFilterRow(pets: List<Pet>, selectedId: String?, onSelect: (String?) -> Unit) {
    val c = DogTagTheme.colors
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

@Composable
private fun VerdictBadge(verdict: String) {
    val c = DogTagTheme.colors
    val (bg, fg) = when (verdict) {
        "VALID" -> c.success.copy(alpha = 0.18f) to c.success
        "INVALID" -> c.danger.copy(alpha = 0.18f) to c.danger
        else -> c.surfaceVariant to c.muted
    }
    Box(Modifier.clip(CircleShape).background(bg).padding(horizontal = 10.dp, vertical = 4.dp)) {
        Text(verdict, fontSize = 10.sp, fontWeight = FontWeight.Bold, color = fg)
    }
}
