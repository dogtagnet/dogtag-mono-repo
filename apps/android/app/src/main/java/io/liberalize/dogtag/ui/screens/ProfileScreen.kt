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
import io.liberalize.dogtag.data.LocalStore
import io.liberalize.dogtag.data.RoaxConfig
import io.liberalize.dogtag.data.SettingsStore
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import io.liberalize.dogtag.ui.ThemeId
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.SeedBackup
import io.liberalize.dogtag.wallet.Wallet
import androidx.lifecycle.compose.collectAsStateWithLifecycle
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
    // Whether the user has affirmed they stored this wallet's phrase offline. Gates owner-secret
    // creation (`ProfileTreeStore.buildAndPersist`), so it gets a standing card of its own below.
    //
    // Deliberately TRI-state and deliberately NOT resolved while composing. `SeedBackup.isConfirmed`
    // is bound to a fingerprint of the seed, so answering it means decrypting the 64-byte BIP-39
    // master seed under the biometric-gated Keystore key - which throws when the user has not
    // authenticated recently, and on the ordinary Profile visit would be main-thread Keystore crypto
    // (materialising the master secret as an unzeroable String) for a flag nobody asked for. So it
    // starts `null` = undetermined and is only resolved from an already-authenticated path.
    //
    // `null` counts as UNCONFIRMED everywhere a decision is made, and renders the confirm action just
    // as `false` does: the invariant that matters is that whenever the gate would refuse, the remedy
    // is reachable. It gets its own wording, though, because undetermined is not the same claim as
    // unconfirmed - most users landing here HAVE confirmed, and telling them issuance is blocked would
    // state as fact something this screen has not checked. A cheap "some fingerprint exists" prefs
    // read cannot stand in for the real check either: a restored-prefs / new-Keystore-seed mismatch is
    // precisely what the fingerprint exists to catch, and treating it as confirmed would hide the
    // action while the gate refused forever.
    var seedBackupConfirmed by remember { mutableStateOf<Boolean?>(null) }

    val localStore = remember { LocalStore.get(context) }
    val pets by localStore.pets.collectAsStateWithLifecycle()

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
                                    // A brand-new seed cannot already be confirmed; no need to
                                    // decrypt it again just to learn that.
                                    seedBackupConfirmed = false
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
                                    // Authenticated already, so resolve the backup flag here rather
                                    // than on every composition. Still guarded: a Keystore failure
                                    // must leave it undetermined, not crash the screen.
                                    seedBackupConfirmed = runCatching {
                                        Wallet.seedHex(context)?.let { SeedBackup.isConfirmed(context, it) }
                                    }.getOrNull()
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
                    Spacer(Modifier.size(10.dp))
                    Text(
                        "Write these 24 words down and store them offline. Without them a lost " +
                            "phone permanently destroys every dog tag on it — there is no reset. " +
                            "This is the only time they are shown.",
                        fontSize = 11.sp, color = c.muted,
                    )
                }
            }

            // The gate on owner-secret creation, as a STANDING card rather than a one-shot button on
            // the genesis phrase card. This store is excluded from device backups, so the phrase is
            // the ONLY way to regenerate a tag's owner-secret on a replacement phone; a tag issued to
            // an owner who never saved it is one lost device away from being permanently unprovable,
            // and `profileRoot` is write-once so there is no on-chain repair.
            //
            // It has to live outside `mnemonic?.let` because `mnemonic` is set at exactly one site -
            // the `Wallet.create` success handler - and Android has no `revealMnemonic` (it does not
            // persist BIP-39 entropy, unlike iOS, and adding that was rejected as a wallet-security
            // scope expansion). Gated on the phrase card, a user backgrounded before tapping it could
            // never reach it again: `buildAndPersist` would throw on every future tag, for good, with
            // no in-app remedy. The gate itself is unchanged - this only makes its remedy reachable.
            if (walletExists) {
                Column(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
                        .background(c.surfaceVariant).padding(12.dp),
                ) {
                    Text(
                        "Recovery phrase backup",
                        fontSize = 12.sp, fontWeight = FontWeight.Bold, color = c.onBackground,
                    )
                    Spacer(Modifier.size(4.dp))
                    if (seedBackupConfirmed == true) {
                        Text(
                            "✓ You confirmed your 24-word recovery phrase is saved offline. Dog tags " +
                                "can be issued to this wallet.",
                            fontSize = 11.sp, color = c.muted,
                        )
                    } else {
                        Text(
                            if (seedBackupConfirmed == false) {
                                "Dog tags cannot be issued to this wallet until you confirm your " +
                                    "24-word recovery phrase is written down and stored offline. " +
                                    "Confirm only if you already have that copy — the words are not " +
                                    "shown again here, and this app cannot recover them for you. " +
                                    "Without it, a lost or reset phone permanently destroys every dog " +
                                    "tag on this wallet: the tag's root is written once on-chain and " +
                                    "can never be moved."
                            } else {
                                "Unlock this wallet above to see whether your phrase backup is " +
                                    "already on record, or just confirm here — as long as your " +
                                    "24 words are already written down and stored offline. " +
                                    "Confirming again when you already have is harmless. The words " +
                                    "are not shown again here, and this app cannot recover them for " +
                                    "you: without that copy, a lost or reset phone permanently " +
                                    "destroys every dog tag on this wallet."
                            },
                            fontSize = 11.sp, color = c.muted,
                        )
                        Spacer(Modifier.size(6.dp))
                        Button(
                            onClick = {
                                // `seedHex` decrypts under the biometric-gated Keystore key, so it
                                // succeeds straight after genesis/unlock and throws on a cold visit.
                                // Try it first to avoid a redundant prompt, then authenticate and
                                // retry. Every read is guarded: a Keystore failure must re-prompt,
                                // never crash the screen.
                                val recordBackup = { reportFailure: Boolean ->
                                    val seedHex = runCatching { Wallet.seedHex(context) }.getOrNull()
                                    val ok = seedHex != null && SeedBackup.confirm(context, seedHex)
                                    seedBackupConfirmed = if (ok) true else null
                                    if (!ok && reportFailure) {
                                        walletMsg =
                                            "Could not record the backup — unlock the device and retry."
                                    }
                                    ok
                                }
                                if (!recordBackup(false)) {
                                    Biometric.prompt(
                                        activity,
                                        "Confirm phrase backup",
                                        "Authenticate to record that you saved your recovery phrase",
                                        onSuccess = { recordBackup(true) },
                                        onError = { walletMsg = it },
                                    )
                                }
                            },
                            colors = ButtonDefaults.buttonColors(containerColor = c.danger),
                        ) { Text("I have saved my 24 words offline") }
                    }
                }
            }
            if (walletMsg.isNotBlank()) Text(walletMsg, fontSize = 12.sp, color = c.muted)
        }

        // ---- Dog-tags: dog tags issued to this wallet (scan the vet's /p/<token> QR to issue one) ----
        SectionTitle("Dog-tags")
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            val minted = pets.filter { it.dogTagId.isNotBlank() && it.dogTagId.all { ch -> ch.isDigit() } }
            if (minted.isEmpty()) {
                Text(
                    "No dog tag yet. Scan your vet's dog-tag QR (Scan) to have one issued and bound to " +
                        "this wallet — the dogTagId then appears here.",
                    fontSize = 12.sp, color = c.muted,
                )
            } else {
                minted.forEach { pet ->
                    KV(pet.name.ifBlank { "Pet" }, "dogTagId ${pet.dogTagId}")
                }
            }
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

        // ---- Developer · on-device ZK self-test (debug builds only) ----
        // Exercises the REAL on-device Groth16 prover end to end (see [ZkSelfTestCard]); the Maestro
        // mobile e2e drives this. Never present in a release build.
        if (io.liberalize.dogtag.BuildConfig.DEBUG) {
            ZkSelfTestCard()
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
