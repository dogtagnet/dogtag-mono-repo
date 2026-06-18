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
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.data.AppSettings
import io.liberalize.dogtag.data.DarkPref
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.SettingsStore
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import io.liberalize.dogtag.ui.ThemeId
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.Wallet
import kotlinx.coroutines.launch

@Composable
fun ProfileScreen(store: SettingsStore, settings: AppSettings, activity: FragmentActivity) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()
    val roax = remember { RoaxConfig.load(context) }

    var walletExists by remember { mutableStateOf(Wallet.exists(context)) }
    var ethAddr by remember { mutableStateOf<String?>(null) }
    var consentAx by remember { mutableStateOf<String?>(null) }
    var consentKeyHash by remember { mutableStateOf<String?>(null) }
    var mnemonic by remember { mutableStateOf<String?>(null) }
    var walletMsg by remember { mutableStateOf("") }

    Column(
        Modifier.fillMaxSize().verticalScroll(scroll).padding(20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("Profile", fontSize = 26.sp, fontWeight = FontWeight.Bold, color = c.onBackground)

        // ---- Appearance ----
        SectionTitle("Appearance")
        Text("Theme", fontSize = 13.sp, color = c.muted)
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            ThemeId.entries.forEach { t ->
                val selected = t == settings.themeId
                Box(
                    Modifier.weight(1f).size(36.dp).clip(CircleShape).background(t.accent)
                        .border(
                            width = if (selected) 3.dp else 0.dp,
                            color = c.onBackground, shape = CircleShape,
                        )
                        .clickable { scope.launch { store.setTheme(t) } },
                    contentAlignment = Alignment.Center,
                ) {
                    if (selected) Icon(Icons.Filled.Check, "Selected", tint = Color.White, modifier = Modifier.size(18.dp))
                }
            }
        }
        Spacer(Modifier.size(4.dp))
        Text("Brightness", fontSize = 13.sp, color = c.muted)
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            val opts = listOf(DarkPref.System to "System", DarkPref.Light to "Light", DarkPref.Dark to "Dark")
            opts.forEachIndexed { i, (pref, label) ->
                SegmentedButton(
                    selected = settings.darkPref == pref,
                    onClick = { scope.launch { store.setDark(pref) } },
                    shape = SegmentedButtonDefaults.itemShape(i, opts.size),
                ) { Text(label) }
            }
        }

        // ---- Embedded wallet ----
        SectionTitle("Embedded wallet")
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "A self-custodial key: BIP-39 seed → secp256k1 wallet + a distinct BabyJubjub consent " +
                    "key (derived in Rust). The seed is encrypted behind the Android Keystore " +
                    "(StrongBox when available), biometric-gated.",
                fontSize = 12.sp, color = c.muted,
            )
            if (!walletExists) {
                Button(
                    onClick = {
                        Biometric.prompt(
                            activity, "Create wallet", "Authenticate to generate your keys",
                            onSuccess = {
                                try {
                                    val id = Wallet.create(context)
                                    walletExists = true
                                    ethAddr = id.ethAddress
                                    consentAx = id.consent.axHex
                                    consentKeyHash = id.consent.keyHashHex
                                    mnemonic = id.mnemonic
                                    walletMsg = "Wallet created. Back up your recovery phrase now."
                                } catch (e: Exception) { walletMsg = "create failed: ${e.message}" }
                            },
                            onError = { walletMsg = it },
                        )
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
                ) { Text("Create embedded wallet") }
            } else {
                Button(
                    onClick = {
                        Biometric.prompt(
                            activity, "Unlock wallet", "Authenticate to reveal your keys",
                            onSuccess = {
                                try {
                                    val id = Wallet.load(context)
                                    ethAddr = id?.ethAddress
                                    consentAx = id?.consent?.axHex
                                    consentKeyHash = id?.consent?.keyHashHex
                                    walletMsg = "Unlocked."
                                } catch (e: Exception) { walletMsg = "unlock failed: ${e.message}" }
                            },
                            onError = { walletMsg = it },
                        )
                    },
                    colors = ButtonDefaults.buttonColors(containerColor = c.accent, contentColor = c.onAccent),
                ) { Text("Unlock & show keys") }
            }

            ethAddr?.let { KV("Wallet", it) }
            consentAx?.let { KV("Consent Ax", it.take(22) + "…") }
            consentKeyHash?.let { KV("keyHash", it.take(22) + "…") }
            consentKeyHash?.let {
                Text(
                    "Bind on-chain: ConsentKeyRegistry.bindConsentKey(keyHash) @ ${roax.consentKeyRegistry.take(10)}…",
                    fontSize = 11.sp, color = c.muted,
                )
            }
            mnemonic?.let {
                Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(c.surfaceVariant).padding(12.dp)) {
                    Text("Recovery phrase (24 words)", fontSize = 12.sp, fontWeight = FontWeight.Bold, color = c.danger)
                    Text(it, fontSize = 12.sp, fontFamily = FontFamily.Monospace, color = c.onBackground)
                }
            }
            if (walletMsg.isNotBlank()) Text(walletMsg, fontSize = 12.sp, color = c.muted)
        }

        // ---- Network ----
        SectionTitle("Network")
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            KV("Chain", "ROAX (chainId ${roax.chainId})")
            KV("DogTagSBT", roax.dogTagSbt.take(16) + "…")
            KV("VerificationRegistry", roax.verificationRegistry.take(16) + "…")
            KV("ConsentKeyRegistry", roax.consentKeyRegistry.take(16) + "…")
            KV("IssuerRegistry", roax.issuerRegistry.take(16) + "…")
        }
        Spacer(Modifier.size(24.dp))
    }
}

@Composable
private fun KV(k: String, v: String) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth()) {
        Text(k, fontSize = 12.sp, color = c.muted, modifier = Modifier.fillMaxWidth(0.4f))
        Text(v, fontSize = 12.sp, color = c.onBackground, fontFamily = FontFamily.Monospace)
    }
}
