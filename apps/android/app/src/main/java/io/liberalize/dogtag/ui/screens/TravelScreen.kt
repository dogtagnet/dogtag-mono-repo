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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Flight
import androidx.compose.material.icons.filled.Verified
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.liberalize.dogtag.ui.DogTagTheme

private data class DocType(val title: String, val subtitle: String, val detail: String)

@Composable
fun TravelScreen() {
    val c = DogTagTheme.colors
    val scroll = rememberScrollState()
    var selected by remember { mutableIntStateOf(0) }
    val types = listOf(
        DocType("CDC Dog Import Form", "Required for U.S. entry (as of Aug 2024)", "Required for all dogs entering the United States as of August 1, 2024. Dogs must be at least 6 months old and have a microchip."),
        DocType("DOT Service Dog Form", "DOT Service Animal Air Transportation Form", "Required for flying with a service animal on U.S. airlines. Airlines must receive this form at least 48 hours before departure."),
        DocType("Other Document", "Other travel document", "Add any other travel-related document for your dog."),
    )

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(36.dp).clip(CircleShape).background(c.travelTint), contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.Flight, "Travel", tint = c.accent, modifier = Modifier.size(20.dp))
            }
            Spacer(Modifier.size(10.dp))
            Column {
                Text("Add Travel Document", fontSize = 20.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                Text("for ${io.liberalize.dogtag.data.DemoData.pet.name}", fontSize = 12.sp, color = c.muted)
            }
        }
        Text("Document Type", fontSize = 22.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text("What type of travel document are you adding?", fontSize = 13.sp, color = c.muted)

        types.forEachIndexed { i, t ->
            DocRow(t, selected == i) { selected = i }
        }

        Spacer(Modifier.size(8.dp))
        Button(
            onClick = {},
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
        ) { Text("Continue to ${types[selected].title.substringBefore(" ")} Form") }
        Spacer(Modifier.size(24.dp))
    }
}

@Composable
private fun DocRow(t: DocType, selected: Boolean, onClick: () -> Unit) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth()
            .clip(RoundedCornerShape(14.dp))
            .background(if (selected) c.accent.copy(alpha = 0.14f) else c.surface)
            .border(
                width = if (selected) 1.5.dp else 1.dp,
                color = if (selected) c.accent else c.outline,
                shape = RoundedCornerShape(14.dp),
            )
            .clickable { onClick() }
            .padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Description, t.title, tint = c.accent, modifier = Modifier.size(20.dp))
            Spacer(Modifier.size(10.dp))
            Column(Modifier.weight(1f)) {
                Text(t.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                Text(t.subtitle, fontSize = 11.sp, color = c.muted)
            }
            if (selected) Icon(Icons.Filled.CheckCircle, "Selected", tint = c.accent)
        }
        if (selected) Text(t.detail, fontSize = 12.sp, color = c.muted)
    }
}
