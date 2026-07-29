package io.liberalize.dogtag.ui.screens

import android.util.Log
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.profile.DogTagCard
import io.liberalize.dogtag.profile.ImportedTag
import io.liberalize.dogtag.profile.OwnedTag
import io.liberalize.dogtag.profile.OwnedTagSource
import io.liberalize.dogtag.profile.OwnerSecretEvidence
import io.liberalize.dogtag.profile.OwnerStoreFailure
import io.liberalize.dogtag.profile.ProfileTreeStore
import io.liberalize.dogtag.ui.DogTagTheme
import io.liberalize.dogtag.ui.SectionTitle
import io.liberalize.dogtag.ui.ThemeId
import io.liberalize.dogtag.wallet.Biometric
import io.liberalize.dogtag.wallet.SeedBackup
import io.liberalize.dogtag.wallet.Wallet
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
fun ProfileScreen(store: SettingsStore, settings: AppSettings, activity: FragmentActivity) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val scroll = rememberScrollState()
    val roax = remember { RoaxConfig.load(context) }
    var rpcInput by remember(settings.rpcUrl) { mutableStateOf(settings.rpcUrl) }
    var rpcMessage by remember { mutableStateOf("") }
    var rpcMessageError by remember { mutableStateOf(false) }
    var checkingRpc by remember { mutableStateOf(false) }

    var walletExists by remember { mutableStateOf(Wallet.exists(context)) }
    var ethAddr by remember { mutableStateOf<String?>(null) }
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

    // The Dog-tags card's OTHER source: the tags this device created by issuance. It is a separate
    // store from `pets` and nothing joins them - custodial issuance writes an owner-secret record
    // and never a pet - so reading only `pets` is what made a freshly minted tag invisible here.
    //
    // Read through the THROWING `load()`, not `all()`: `all()` answers `emptyList()` for a store it
    // could not decrypt, which the card would render as "No dog tag yet" - the same false absence
    // in a second flavour. Off the main thread because it is Keystore AES-GCM plus file I/O, and
    // therefore starting at [OwnedTagSource.Pending] rather than at "none", so the first frame
    // cannot claim an absence nothing has checked yet.
    //
    // Re-runs on every visit to this tab: `DogTagApp` swaps tab content with a `when`, and shows
    // `ScanScreen` INSTEAD of it, so returning here after an issuance composes this screen afresh.
    val treeStore = remember(context) { ProfileTreeStore(context) }
    var ownedTags by remember { mutableStateOf<OwnedTagSource>(OwnedTagSource.Pending) }
    LaunchedEffect(treeStore) {
        ownedTags = withContext(Dispatchers.IO) {
            try {
                OwnedTagSource.Records(
                    treeStore.load().map { OwnedTag(dogTagIdDec = it.dogTagIdDec, rootHex = it.rootHex) },
                )
            } catch (e: Exception) {
                // The CAUSE, never the message. By the time a decode fails the decryption has
                // already succeeded, and `org.json` quotes the input it choked on - which here is
                // the owner-secret store's own plaintext. The screen gets a sentence `DogTagCard`
                // constructed from this cause; see its `reasonText`.
                val kind = (e as? ProfileTreeStore.UnreadableStoreException)?.kind
                // Class and step only - deliberately NOT `Log.w(tag, msg, e)`, which prints the
                // throwable's message and so would move the same plaintext from the screen into
                // logcat, where a bug report collects it. The class still tells support whether
                // this was the Keystore, the filesystem or the parser.
                Log.w(
                    "ProfileScreen",
                    "owner-secret store could not be read: ${e.javaClass.name}" +
                        (kind?.let { " ($it)" } ?: ""),
                )
                OwnedTagSource.Unreadable(
                    when (kind) {
                        ProfileTreeStore.UnreadableStoreException.Kind.CouldNotDecode ->
                            OwnerStoreFailure.CouldNotDecode
                        else -> OwnerStoreFailure.CouldNotRead
                    },
                )
            }
        }
    }

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
                "A self-custodial BIP-39 seed. Rust derives a distinct owner secret and consent key " +
                    "for each dog tag. The seed is encrypted behind the Android Keystore " +
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

        // ---- Dog-tags: owner-hidden trees created from scanned vet issuance sessions ----
        SectionTitle("Dog-tags")
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            // Both sources, folded by `DogTagCard` - see its docs for why an empty card is a claim
            // that has to be earned rather than a default.
            val card = DogTagCard.state(
                owned = ownedTags,
                imported = pets.map { ImportedTag(dogTagIdDec = it.dogTagId, name = it.name) },
            )

            card.rows.forEachIndexed { i, row ->
                if (i > 0) HorizontalDivider(color = c.muted.copy(alpha = 0.3f))
                // The identifier position, always. A tag whose credential has not been imported has
                // no name to show, and inventing one ("Pet", "Unnamed") would read as data.
                KV("dogTagId", row.dogTagIdDec)
                if (row.name != null) KV("Pet", row.name)
                if (row.rootHex != null) KV("Profile root", row.rootHex.take(18) + "…")
                // What the owner-secret record itself proves, and no more. The record is written
                // BEFORE the custodial-bind POST and before the on-chain confirmation poll, and
                // carries no bind or anchor status, so an issuance that died after it was written
                // would render as anchored - the claim this product exists to make truthfully,
                // asserted from evidence that does not establish it.
                if (row.ownerSecret == OwnerSecretEvidence.Held && !row.credentialImported) {
                    Text(
                        "This phone holds this tag's owner-secret and built its profile root. No " +
                            "credential naming this tag has been imported here yet, so its pet " +
                            "details are not known on this phone.",
                        fontSize = 11.sp, color = c.muted,
                    )
                }
                // Only the definite negative is printed. Under Unknown the store never answered, so
                // "holds no owner-secret" would be could-not-check dressed as a fact; the
                // card-level notice below is what speaks for that case.
                if (row.ownerSecret == OwnerSecretEvidence.NotHeld) {
                    Text(
                        "Known from an imported credential. This phone holds no owner-secret for " +
                            "this tag, so it cannot prove consent for it.",
                        fontSize = 11.sp, color = c.muted,
                    )
                }
            }

            // "Could not check" is its own answer and outranks silence: an unread or unreadable
            // owner-secret store leaves any issuance-created tag unlisted, so saying nothing would
            // let the rows above read as the complete set.
            if (card.ownerStorePending) {
                Text("Checking this device for owner-hidden tags…", fontSize = 11.sp, color = c.muted)
            } else if (card.ownerStoreUnavailable != null) {
                // Printed verbatim: the sentence is built by `DogTagCard.reasonText` from a closed
                // set of causes, so there is no caller text here to interpolate.
                Text(card.ownerStoreUnavailable, fontSize = 11.sp, color = c.danger)
            }

            // Only once every source has answered and none knows a tag.
            if (card.establishesNoTags) {
                Text(
                    "No dog tag yet. Scan your vet's dog-tag QR to build its owner-hidden profile " +
                        "tree and have the root issued.",
                    fontSize = 12.sp, color = c.muted,
                )
            }
        }

        // ---- Network ----
        SectionTitle("Network")
        Column(
            Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            KV("Chain", "ROAX (chainId ${roax.chainId})")
            KV("DogTagSBT", roax.dogTagSbt.take(16) + "…")
            KV("IssuerRegistry", roax.issuerRegistry.take(16) + "…")
            KV("ProtocolRegistry", roax.protocolRegistry.ifBlank { "Not deployed" }.take(16) + "…")
            HorizontalDivider(color = c.muted.copy(alpha = 0.3f))
            Text("Blockchain endpoint", fontSize = 13.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text(
                "Choosing a custom JSON-RPC peer can improve endpoint choice and liveness and help " +
                    "resist endpoint censorship. It is not a trust upgrade. DogTag is not a light " +
                    "client, so that peer can fabricate isValid, rootIssuer, profileRoot, and other " +
                    "chain responses. This setting changes blockchain reads only; the centralized " +
                    "provider directory/indexer is not configurable here.",
                fontSize = 11.sp,
                color = c.muted,
            )
            Text(
                "Before every blockchain read, DogTag checks eth_chainId against bundled chainId " +
                    "${roax.chainId}. An invalid, unreachable, or different-chain custom endpoint " +
                    "falls back to the bundled endpoint, which is checked too. If neither endpoint " +
                    "establishes that chainId, no contract read is sent. This prevents accidental " +
                    "cross-chain address use; it cannot prove a peer is honest.",
                fontSize = 11.sp,
                color = c.muted,
            )
            OutlinedTextField(
                value = rpcInput,
                onValueChange = {
                    rpcInput = it
                    rpcMessage = ""
                    rpcMessageError = false
                },
                label = { Text("JSON-RPC URL") },
                placeholder = { Text(RoaxRpc.DEFAULT_RPC) },
                singleLine = true,
                enabled = !checkingRpc,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Button(
                    onClick = {
                        checkingRpc = true
                        rpcMessage = "Checking reported chainId…"
                        rpcMessageError = false
                        scope.launch {
                            val normalized = RoaxRpc.normalizeRpcUrl(rpcInput)
                            if (normalized == null) {
                                // An explicit rejected save must not leave an older custom peer
                                // active behind the error copy. Keep the draft for correction, but
                                // fail the persisted choice closed to the bundled endpoint.
                                store.setRpcUrl(RoaxRpc.DEFAULT_RPC)
                                rpcMessage =
                                    "Enter a valid http(s) JSON-RPC URL. The custom endpoint was " +
                                        "rejected; blockchain reads use the bundled endpoint."
                                rpcMessageError = true
                            } else {
                                val route = withContext(Dispatchers.IO) {
                                    RoaxRpc.endpointRoute(normalized, roax.chainId)
                                }
                                when (route) {
                                    is RoaxRpc.EndpointRoute.Custom -> {
                                        store.setRpcUrl(route.url)
                                        rpcInput = route.url
                                    }
                                    RoaxRpc.EndpointRoute.Bundled -> {
                                        store.setRpcUrl(RoaxRpc.DEFAULT_RPC)
                                        rpcInput = RoaxRpc.DEFAULT_RPC
                                    }
                                    is RoaxRpc.EndpointRoute.BundledFallback,
                                    is RoaxRpc.EndpointRoute.Unavailable ->
                                        store.setRpcUrl(RoaxRpc.DEFAULT_RPC)
                                }
                                rpcMessage = endpointRouteMessage(route, roax.chainId)
                                rpcMessageError = route is RoaxRpc.EndpointRoute.Unavailable ||
                                    route is RoaxRpc.EndpointRoute.BundledFallback
                            }
                            checkingRpc = false
                        }
                    },
                    enabled = !checkingRpc,
                    colors = ButtonDefaults.buttonColors(
                        containerColor = c.accent,
                        contentColor = c.onAccent,
                    ),
                ) {
                    Text(if (checkingRpc) "Checking…" else "Save & check")
                }
                TextButton(
                    onClick = {
                        // Close the interaction gate before the first suspension. Otherwise a quick
                        // default → save sequence can launch two probes whose messages and writes
                        // complete out of order.
                        checkingRpc = true
                        rpcMessage = "Checking the bundled endpoint…"
                        rpcMessageError = false
                        scope.launch {
                            store.setRpcUrl(RoaxRpc.DEFAULT_RPC)
                            rpcInput = RoaxRpc.DEFAULT_RPC
                            val route = withContext(Dispatchers.IO) {
                                RoaxRpc.endpointRoute(RoaxRpc.DEFAULT_RPC, roax.chainId)
                            }
                            rpcMessage = endpointRouteMessage(route, roax.chainId)
                            rpcMessageError = route is RoaxRpc.EndpointRoute.Unavailable
                            checkingRpc = false
                        }
                    },
                    enabled = !checkingRpc,
                ) {
                    Text("Use default")
                }
            }
            if (rpcMessage.isNotBlank()) {
                Text(
                    rpcMessage,
                    fontSize = 11.sp,
                    color = if (rpcMessageError) c.danger else c.success,
                )
            }
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

private fun endpointRouteMessage(route: RoaxRpc.EndpointRoute, expectedChainId: Long): String =
    when (route) {
        RoaxRpc.EndpointRoute.Bundled ->
            "Using the bundled endpoint on chainId $expectedChainId."
        is RoaxRpc.EndpointRoute.Custom ->
            "Saved. The custom endpoint reports chainId $expectedChainId and is active."
        is RoaxRpc.EndpointRoute.BundledFallback ->
            "The custom endpoint ${endpointFailureMessage(route.customFailure)}. It was rejected; " +
                "blockchain reads use the bundled endpoint."
        is RoaxRpc.EndpointRoute.Unavailable -> {
            val custom = route.customFailure?.let {
                "The custom endpoint ${endpointFailureMessage(it)}, and "
            }.orEmpty()
            "${custom}the bundled endpoint ${endpointFailureMessage(route.bundledFailure)}. " +
                "No contract read will be sent until an endpoint establishes chainId $expectedChainId."
        }
    }

private fun endpointFailureMessage(failure: RoaxRpc.EndpointFailure): String =
    when (failure) {
        RoaxRpc.EndpointFailure.InvalidUrl -> "has an invalid URL"
        RoaxRpc.EndpointFailure.Unavailable -> "could not be reached"
        RoaxRpc.EndpointFailure.InvalidChainIdResponse -> "did not return a valid eth_chainId"
        is RoaxRpc.EndpointFailure.WrongChain ->
            "reports chainId ${failure.actualChainId}"
    }

@Composable
private fun KV(k: String, v: String) {
    val c = DogTagTheme.colors
    Row(Modifier.fillMaxWidth()) {
        Text(k, fontSize = 12.sp, color = c.muted, modifier = Modifier.fillMaxWidth(0.4f))
        Text(v, fontSize = 12.sp, color = c.onBackground, fontFamily = FontFamily.Monospace)
    }
}
