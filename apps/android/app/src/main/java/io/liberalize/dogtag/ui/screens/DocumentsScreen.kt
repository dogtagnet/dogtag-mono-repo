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
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.QrCode2
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.liberalize.dogtag.data.Credential
import io.liberalize.dogtag.data.DemoData
import io.liberalize.dogtag.qr.QrImage
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import org.json.JSONObject

@Composable
fun DocumentsScreen() {
    val c = DogTagTheme.colors
    val scroll = rememberScrollState()
    var shareOf by remember { mutableStateOf<Credential?>(null) }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Documents", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        SectionTitle("All records", "${DemoData.credentials.size}")
        DemoData.credentials.forEach { cred ->
            Row(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(c.surface)
                    .clickable { shareOf = cred }.padding(14.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.size(38.dp).clip(CircleShape).background(c.surfaceVariant), contentAlignment = Alignment.Center) {
                    Icon(Icons.Filled.Description, cred.title, tint = c.accent, modifier = Modifier.size(18.dp))
                }
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(cred.title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = c.onBackground)
                    Text("${cred.group.title} · ${cred.recordType}", fontSize = 12.sp, color = c.muted)
                }
                Icon(Icons.Filled.QrCode2, "Share", tint = c.muted)
            }
        }
        Spacer(Modifier.size(24.dp))
    }

    val sel = shareOf
    if (sel != null) ShareSheet(sel) { shareOf = null }
}

@Composable
private fun ShareSheet(cred: Credential, onClose: () -> Unit) {
    val c = DogTagTheme.colors
    val payload = remember(cred.id) {
        JSONObject().apply {
            put("type", "dogtag.credential.share")
            put("dogTagId", DemoData.pet.dogTagId)
            put("credentialId", cred.id)
            put("title", cred.title)
            put("recordType", cred.recordType)
            put("issuer", cred.issuer)
            put("issuedOn", cred.issuedOn)
        }.toString()
    }
    Box(
        Modifier.fillMaxSize().background(androidx.compose.ui.graphics.Color(0xCC000000))
            .clickable { onClose() },
        contentAlignment = Alignment.Center,
    ) {
        Column(
            Modifier.fillMaxWidth(0.86f).clip(RoundedCornerShape(20.dp)).background(c.surface).padding(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("Share credential", fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
                Spacer(Modifier.weight(1f))
                Icon(Icons.Filled.Close, "Close", tint = c.muted, modifier = Modifier.clickable { onClose() })
            }
            Text(cred.title, fontSize = 14.sp, color = c.muted)
            QrImage(payload, modifier = Modifier.size(240.dp))
            Text("Scan to share this credential reference", fontSize = 12.sp, color = c.muted)
        }
    }
}
