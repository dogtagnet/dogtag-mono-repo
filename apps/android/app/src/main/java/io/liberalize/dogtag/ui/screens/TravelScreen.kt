package io.liberalize.dogtag.ui.screens

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
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Flight
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.CredentialGroup
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle

/**
 * Travel tab — the dog's travel documents (CDC import form, DOT service form, USDA health cert, etc).
 * These are REAL imported records filtered by a per-pet selector. No mock data; legitimately empty
 * until a travel record is scanned in.
 */
@Composable
fun TravelScreen(onScan: () -> Unit, onOpen: (Credential) -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember { LocalStore.get(context) }
    val pets by store.pets.collectAsStateWithLifecycle()
    val creds by store.credentials.collectAsStateWithLifecycle()
    val scroll = rememberScrollState()

    var filterPetId by remember { mutableStateOf<String?>(null) }
    val travel = creds.filter { it.group == CredentialGroup.Travel }
        .filter { filterPetId == null || it.dogTagId == filterPetId }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(36.dp).clip(CircleShape).background(c.travelTint), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.Flight, "Travel", tint = c.accent, modifier = Modifier.size(20.dp))
            }
            Spacer(Modifier.size(10.dp))
            Text("Travel", fontSize = 22.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        }

        val anyTravel = creds.any { it.group == CredentialGroup.Travel }
        if (!anyTravel) {
            EmptyState(
                "No travel documents yet",
                "Travel records (CDC import form, DOT service form, USDA health certificate, rabies " +
                    "certificate) appear here once a vet or USDA endorser shares them. Scan their QR to import.",
                onScan,
            )
            return@Column
        }

        PetFilterRow(pets, filterPetId) { filterPetId = it }

        SectionTitle("Travel records", "${travel.size}")
        if (travel.isEmpty()) {
            Text("No travel records for this dog yet.", fontSize = 13.sp, color = c.muted)
        }
        travel.forEach { cred ->
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface)
                    .clickable { onOpen(cred) }.padding(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.size(38.dp).clip(CircleShape).background(c.surfaceVariant), contentAlignment = Alignment.Center) {
                    Icon(Icons.Filled.Description, cred.title, tint = c.accent, modifier = Modifier.size(18.dp))
                }
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    val petName = pets.firstOrNull { it.dogTagId == cred.dogTagId }?.name ?: "DogTag #${cred.dogTagId}"
                    Text("$petName · ${cred.recordType}", fontSize = 12.sp, color = c.muted)
                }
                Text(cred.verdict, fontSize = 11.sp, fontWeight = FontWeight.Bold,
                    color = if (cred.verdict == "VALID") c.success else if (cred.verdict == "INVALID") c.danger else c.muted)
            }
        }
        Spacer(Modifier.size(24.dp))
    }
}
