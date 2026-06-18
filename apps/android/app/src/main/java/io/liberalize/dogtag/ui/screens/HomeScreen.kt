package io.liberalize.dogtag.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Flight
import androidx.compose.material.icons.filled.Pets
import androidx.compose.material.icons.filled.QrCodeScanner
import androidx.compose.material.icons.filled.Shield
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.CredentialGroup
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.Pet
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle

@Composable
fun HomeScreen(onScan: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val store = remember { LocalStore.get(context) }
    val pets by store.pets.collectAsStateWithLifecycle()
    val creds by store.credentials.collectAsStateWithLifecycle()
    val scroll = rememberScrollState()

    var selectedPetId by remember { mutableStateOf<String?>(null) }
    val currentPet: Pet? = pets.firstOrNull { it.dogTagId == selectedPetId } ?: pets.firstOrNull()
    var expanded by remember { mutableStateOf<CredentialGroup?>(null) }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // Header row.
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text("Dog Tags", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Spacer(Modifier.weight(1f))
            Box(
                Modifier.size(40.dp).clip(CircleShape).background(c.accent).clickable { onScan() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.QrCodeScanner, "Scan", tint = c.onAccent) }
        }

        if (pets.isEmpty()) {
            EmptyState(
                "No pets yet",
                "Scan a vet or groomer's QR to import your dog's first verified record — your pets " +
                    "appear here automatically.",
                onScan,
            )
            return@Column
        }

        // Pet selector (only if more than one pet).
        if (pets.size > 1) {
            PetChips(pets, currentPet?.dogTagId) { selectedPetId = it }
        }

        val pet = currentPet
        if (pet != null) {
            PetIdentity(pet)
            PetPhotoCard(onScan)

            val petCreds = creds.filter { it.dogTagId == pet.dogTagId }
            SectionTitle("Credentials", "${petCreds.size} total")
            if (petCreds.isEmpty()) {
                EmptyState(
                    "No credentials yet",
                    "Scan a vet's QR to import a record for ${pet.name}.",
                    onScan,
                )
            } else {
                GroupCard(CredentialGroup.Health, Icons.Filled.Favorite, c.healthTint, c.danger,
                    petCreds, expanded == CredentialGroup.Health) {
                    expanded = if (expanded == CredentialGroup.Health) null else CredentialGroup.Health
                }
                GroupCard(CredentialGroup.Service, Icons.Filled.Shield, c.serviceTint, c.success,
                    petCreds, expanded == CredentialGroup.Service) {
                    expanded = if (expanded == CredentialGroup.Service) null else CredentialGroup.Service
                }
                GroupCard(CredentialGroup.Travel, Icons.Filled.Flight, c.travelTint, Color(0xFF2F6BFF),
                    petCreds, expanded == CredentialGroup.Travel) {
                    expanded = if (expanded == CredentialGroup.Travel) null else CredentialGroup.Travel
                }
            }
        }
        Spacer(Modifier.size(24.dp))
    }
}

@Composable
fun PetChips(pets: List<Pet>, selectedId: String?, onSelect: (String?) -> Unit) {
    val c = DogTagTheme.colors
    val row = rememberScrollState()
    Row(
        Modifier.fillMaxWidth().horizontalScroll(row),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        pets.forEach { p ->
            val sel = p.dogTagId == selectedId
            Box(
                Modifier.clip(CircleShape)
                    .background(if (sel) c.accent else c.surfaceVariant)
                    .clickable { onSelect(p.dogTagId) }
                    .padding(horizontal = 14.dp, vertical = 8.dp),
            ) {
                Text(p.name, fontSize = 13.sp, fontWeight = FontWeight.SemiBold,
                    color = if (sel) c.onAccent else c.onBackground)
            }
        }
    }
}

@Composable
private fun PetIdentity(pet: Pet) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth()) {
        Column(Modifier.weight(1f)) {
            Text("NAME", fontSize = 11.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
            Text(pet.name, fontSize = 22.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text("DogTag #${pet.dogTagId}", fontSize = 12.sp, color = c.muted)
        }
        if (pet.breed.isNotBlank()) {
            Column(horizontalAlignment = Alignment.End) {
                Text("BREED", fontSize = 11.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
                Text(pet.breed, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                if (pet.ageLabel.isNotBlank()) Text(pet.ageLabel, fontSize = 12.sp, color = c.muted)
            }
        }
    }
}

@Composable
private fun PetPhotoCard(onScan: () -> Unit) {
    val c = DogTagTheme.colors
    Box(
        Modifier.fillMaxWidth().aspectRatio(1.15f)
            .clip(RoundedCornerShape(180.dp))
            .background(c.accent.copy(alpha = 0.18f)),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.fillMaxWidth(0.62f).aspectRatio(1f).clip(CircleShape).background(c.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Filled.Pets, "Pet", tint = c.accent, modifier = Modifier.size(72.dp)) }
        Box(
            Modifier.align(Alignment.BottomCenter).padding(bottom = 8.dp)
                .size(44.dp).clip(CircleShape).background(c.danger).clickable { onScan() },
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Filled.Add, "Add record", tint = Color.White) }
    }
}

@Composable
fun EmptyState(title: String, body: String, onScan: () -> Unit) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Box(
            Modifier.size(56.dp).clip(CircleShape).background(c.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Filled.QrCodeScanner, null, tint = c.accent, modifier = Modifier.size(28.dp)) }
        Text(title, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text(body, fontSize = 13.sp, color = c.muted, modifier = Modifier.fillMaxWidth())
        Box(
            Modifier.clip(CircleShape).background(c.accent).clickable { onScan() }
                .padding(horizontal = 18.dp, vertical = 10.dp),
        ) { Text("Scan a QR", color = c.onAccent, fontSize = 13.sp, fontWeight = FontWeight.SemiBold) }
    }
}

@Composable
private fun GroupCard(
    group: CredentialGroup,
    icon: ImageVector,
    tint: Color,
    iconTint: Color,
    creds: List<Credential>,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    val c = DogTagTheme.colors
    val items = creds.filter { it.group == group }
    val count = items.size
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(tint)
            .clickable { onToggle() }.padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(
                Modifier.size(38.dp).clip(CircleShape).background(c.surface),
                contentAlignment = Alignment.Center,
            ) { Icon(icon, group.title, tint = iconTint, modifier = Modifier.size(20.dp)) }
            Spacer(Modifier.size(12.dp))
            Column(Modifier.weight(1f)) {
                Text(group.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                Text("$count record${if (count == 1) "" else "s"}", fontSize = 12.sp, color = c.muted)
            }
            Icon(Icons.Filled.ChevronRight, "Open", tint = c.muted)
        }
        if (expanded) {
            items.forEach { cred ->
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.surface).padding(12.dp),
                ) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    Text("${cred.recordType} · ${cred.verdict}", fontSize = 12.sp, color = c.muted)
                    if (cred.issuer.isNotBlank()) Text(cred.issuer, fontSize = 11.sp, color = c.muted)
                }
            }
        }
    }
}
