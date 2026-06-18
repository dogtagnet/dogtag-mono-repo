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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.liberalize.dogtag.data.CredentialGroup
import io.liberalize.dogtag.data.DemoData
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle

@Composable
fun HomeScreen(onScan: () -> Unit) {
    val c = DogTagTheme.colors
    val pet = DemoData.pet
    val scroll = rememberScrollState()
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
                Modifier.size(40.dp).clip(CircleShape).background(c.accent)
                    .clickable { onScan() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.Add, "Add", tint = c.onAccent) }
        }

        // Pet identity row.
        Row(Modifier.fillMaxWidth()) {
            Column(Modifier.weight(1f)) {
                Text("NAME", fontSize = 11.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
                Text(pet.name, fontSize = 22.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                Text("DogTag #${pet.dogTagId}", fontSize = 12.sp, color = c.muted)
            }
            Column(horizontalAlignment = Alignment.End) {
                Text("BREED", fontSize = 11.sp, color = c.muted, fontWeight = FontWeight.SemiBold)
                Text(pet.breed, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                Text(pet.ageLabel, fontSize = 12.sp, color = c.muted)
            }
        }

        // Pet photo card (placeholder ring with dog glyph, accent-tinted).
        Box(
            Modifier.fillMaxWidth().aspectRatio(1.15f)
                .clip(RoundedCornerShape(180.dp))
                .background(c.accent.copy(alpha = 0.18f)),
            contentAlignment = Alignment.Center,
        ) {
            Box(
                Modifier.fillMaxWidth(0.62f).aspectRatio(1f).clip(CircleShape)
                    .background(c.surfaceVariant),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Filled.Pets, "Pet", tint = c.accent, modifier = Modifier.size(72.dp))
            }
            Box(
                Modifier.align(Alignment.BottomCenter).padding(bottom = 8.dp)
                    .size(44.dp).clip(CircleShape).background(c.danger)
                    .clickable { onScan() },
                contentAlignment = Alignment.Center,
            ) { Icon(Icons.Filled.Add, "Add record", tint = Color.White) }
        }

        SectionTitle("Credentials", "${DemoData.credentials.size} total")

        CredentialGroupCard(
            group = CredentialGroup.Health, icon = Icons.Filled.Favorite, tint = c.healthTint,
            iconTint = c.danger, expanded = expanded == CredentialGroup.Health,
            onToggle = { expanded = if (expanded == CredentialGroup.Health) null else CredentialGroup.Health },
        )
        CredentialGroupCard(
            group = CredentialGroup.Service, icon = Icons.Filled.Shield, tint = c.serviceTint,
            iconTint = c.success, expanded = expanded == CredentialGroup.Service,
            onToggle = { expanded = if (expanded == CredentialGroup.Service) null else CredentialGroup.Service },
        )
        CredentialGroupCard(
            group = CredentialGroup.Travel, icon = Icons.Filled.Flight, tint = c.travelTint,
            iconTint = Color(0xFF2F6BFF), expanded = expanded == CredentialGroup.Travel,
            onToggle = { expanded = if (expanded == CredentialGroup.Travel) null else CredentialGroup.Travel },
        )

        Spacer(Modifier.size(24.dp))
    }
}

@Composable
private fun CredentialGroupCard(
    group: CredentialGroup,
    icon: ImageVector,
    tint: Color,
    iconTint: Color,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    val c = DogTagTheme.colors
    val count = DemoData.countFor(group)
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
            DemoData.credentials.filter { it.group == group }.forEach { cred ->
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
                        .background(c.surface).padding(12.dp),
                ) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    Text(cred.recordType + " · " + cred.subtitle, fontSize = 12.sp, color = c.muted)
                    Text("${cred.issuer} · ${cred.issuedOn}", fontSize = 11.sp, color = c.muted)
                }
            }
        }
    }
}
