package io.liberalize.dogtag.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Flight
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.VerifiedUser
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
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
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.data.AppSettings
import io.liberalize.dogtag.data.SettingsStore
import io.liberalize.dogtag.ui.screens.DocumentsScreen
import io.liberalize.dogtag.ui.screens.HomeScreen
import io.liberalize.dogtag.ui.screens.ProfileScreen
import io.liberalize.dogtag.ui.screens.TravelScreen
import io.liberalize.dogtag.ui.screens.VerifyScreen

enum class Tab(val label: String, val icon: ImageVector) {
    Verify("Verify", Icons.Filled.VerifiedUser),
    Travel("Travel", Icons.Filled.Flight),
    Home("Home", Icons.Filled.Home),
    Documents("Documents", Icons.Filled.Description),
    Profile("Profile", Icons.Filled.Person),
}

@Composable
fun DogTagApp(store: SettingsStore, settings: AppSettings, activity: FragmentActivity) {
    val c = DogTagTheme.colors
    var tab by remember { mutableStateOf(Tab.Home) }

    Surface(modifier = Modifier.fillMaxSize(), color = c.background) {
        Box(Modifier.fillMaxSize()) {
            Box(Modifier.fillMaxSize().padding(bottom = 72.dp)) {
                when (tab) {
                    Tab.Verify -> VerifyScreen(activity)
                    Tab.Travel -> TravelScreen()
                    Tab.Home -> HomeScreen(onScan = { tab = Tab.Verify })
                    Tab.Documents -> DocumentsScreen()
                    Tab.Profile -> ProfileScreen(store, settings, activity)
                }
            }
            BottomBar(
                current = tab,
                onSelect = { tab = it },
                modifier = Modifier.align(Alignment.BottomCenter),
            )
        }
    }
}

@Composable
private fun BottomBar(current: Tab, onSelect: (Tab) -> Unit, modifier: Modifier = Modifier) {
    val c = DogTagTheme.colors
    Surface(
        modifier = modifier.fillMaxWidth().height(72.dp),
        color = c.surface,
        shadowElevation = 8.dp,
    ) {
        Row(
            Modifier.fillMaxSize().padding(horizontal = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Tab.entries.forEach { t ->
                val selected = t == current
                val isHome = t == Tab.Home
                Box(
                    Modifier
                        .weight(1f)
                        .fillMaxSize()
                        .clickable(
                            interactionSource = remember { MutableInteractionSource() },
                            indication = null,
                        ) { onSelect(t) },
                    contentAlignment = Alignment.Center,
                ) {
                    androidx.compose.foundation.layout.Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        if (isHome) {
                            Box(
                                Modifier
                                    .size(40.dp)
                                    .clip(CircleShape)
                                    .background(if (selected) c.accent else c.surfaceVariant),
                                contentAlignment = Alignment.Center,
                            ) {
                                Icon(
                                    t.icon, t.label,
                                    tint = if (selected) c.onAccent else c.muted,
                                    modifier = Modifier.size(22.dp),
                                )
                            }
                        } else {
                            Icon(
                                t.icon, t.label,
                                tint = if (selected) c.accent else c.muted,
                                modifier = Modifier.size(22.dp),
                            )
                            Text(
                                t.label,
                                fontSize = 10.sp,
                                color = if (selected) c.accent else c.muted,
                                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                            )
                        }
                    }
                }
            }
        }
    }
}

/** Shared simple section header used across screens. */
@Composable
fun SectionTitle(text: String, trailing: String? = null) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(text, fontSize = 18.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        if (trailing != null) {
            androidx.compose.foundation.layout.Spacer(Modifier.weight(1f))
            Text(trailing, fontSize = 13.sp, color = c.muted)
        }
    }
}
