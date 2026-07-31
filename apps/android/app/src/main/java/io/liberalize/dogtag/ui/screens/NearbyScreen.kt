package io.liberalize.dogtag.ui.screens

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.Uri
import android.os.CancellationSignal
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.Business
import androidx.compose.material.icons.filled.Directions
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Language
import androidx.compose.material.icons.filled.LocationOff
import androidx.compose.material.icons.filled.LocationOn
import androidx.compose.material.icons.filled.MyLocation
import androidx.compose.material.icons.filled.Phone
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.location.LocationManagerCompat
import io.liberalize.dogtag.nearby.ContactDirectoryPresentation
import io.liberalize.dogtag.nearby.CallerPosition
import io.liberalize.dogtag.nearby.DEFAULT_PROVIDER_PAGE_SIZE
import io.liberalize.dogtag.nearby.DirectoryObservation
import io.liberalize.dogtag.nearby.DirectoryProvider
import io.liberalize.dogtag.nearby.DistanceClaim
import io.liberalize.dogtag.nearby.GeoPoint
import io.liberalize.dogtag.nearby.NearbyDecision
import io.liberalize.dogtag.nearby.NearbyOriginState
import io.liberalize.dogtag.nearby.NearbyPresentation
import io.liberalize.dogtag.nearby.NearbyRow
import io.liberalize.dogtag.nearby.ProviderDirectories
import io.liberalize.dogtag.nearby.ProviderDirectory
import io.liberalize.dogtag.nearby.ProviderDirectoryQuery
import io.liberalize.dogtag.nearby.ProviderDirectoryResult
import io.liberalize.dogtag.nearby.StoredProviderRecords
import io.liberalize.dogtag.nearby.appendDirectoryPage
import io.liberalize.dogtag.net.BindingTone
import io.liberalize.dogtag.net.IssuerBinding
import io.liberalize.dogtag.ui.DogTagTheme
import java.util.Locale
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private enum class DirectoryScope { Nearby, Contacts }
private val OWNER_PROVIDER_KINDS = listOf("vet", "groomer")

/**
 * List-first provider discovery. There is deliberately no map, autocomplete, geocoder, manual
 * coordinate input, or map-app handoff.
 */
@Composable
fun NearbyScreen(onBack: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val directory = remember { ProviderDirectories.central }
    // The offline fallback. Records only: identity and contacts, never a distance or the ranking.
    val recordCache = remember { ProviderDirectories.recordCache(context.cacheDir) }
    var directoryResult by remember { mutableStateOf<ProviderDirectoryResult?>(null) }
    var storedRecords by remember { mutableStateOf<StoredProviderRecords?>(null) }
    var refreshKey by remember { mutableIntStateOf(0) }
    var scope by rememberSaveable { mutableStateOf(DirectoryScope.Nearby) }
    // Search text and the current fix are process-memory only. Neither is saved into the activity's
    // recreation Bundle, and the directory adapter never caches a position-keyed response.
    var query by remember { mutableStateOf("") }
    var origin by remember { mutableStateOf<NearbyOriginState>(NearbyOriginState.AwaitingChoice) }
    var actionError by remember { mutableStateOf<String?>(null) }
    var requestGeneration by remember { mutableIntStateOf(0) }
    var loadingMore by remember { mutableStateOf(false) }
    val coroutineScope = rememberCoroutineScope()

    val locationManager = remember {
        context.getSystemService(Context.LOCATION_SERVICE) as LocationManager
    }
    val cancellation = remember { mutableStateOf<CancellationSignal?>(null) }
    DisposableEffect(Unit) {
        onDispose {
            cancellation.value?.cancel()
            cancellation.value = null
        }
    }

    fun hasCoarseGrant(): Boolean =
        ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    fun resolveCurrentLocation() {
        // Supersede any earlier request before checking providers. Cancellation can race a callback
        // already queued on the main executor, so that callback also checks this signal's identity.
        cancellation.value?.cancel()
        cancellation.value = null
        if (!LocationManagerCompat.isLocationEnabled(locationManager)) {
            origin = NearbyOriginState.LocationUnavailable
            return
        }
        val provider = listOf(LocationManager.NETWORK_PROVIDER, LocationManager.GPS_PROVIDER)
            .firstOrNull { name ->
                LocationManagerCompat.hasProvider(locationManager, name) &&
                    runCatching { locationManager.isProviderEnabled(name) }.getOrDefault(false)
            }
        if (provider == null) {
            origin = NearbyOriginState.LocationUnavailable
            return
        }

        cancellation.value?.cancel()
        val signal = CancellationSignal()
        cancellation.value = signal
        origin = NearbyOriginState.Locating
        try {
            LocationManagerCompat.getCurrentLocation(
                locationManager,
                provider,
                signal,
                ContextCompat.getMainExecutor(context),
            ) { location ->
                if (cancellation.value === signal) {
                    cancellation.value = null
                    val point = location?.let { GeoPoint(it.latitude, it.longitude) }
                    origin = if (point?.isUsable == true) {
                        NearbyOriginState.Available(
                            point,
                            // The fix's own horizontal uncertainty, carried so no row can render a
                            // distance finer than this grant supports. It is the only such bound now:
                            // the request sends the exact fix. Absent is not zero.
                            accuracyMetres = location.takeIf { it.hasAccuracy() }
                                ?.accuracy
                                ?.toDouble(),
                        )
                    } else {
                        NearbyOriginState.LocationUnavailable
                    }
                }
            }
        } catch (_: SecurityException) {
            if (cancellation.value === signal) {
                cancellation.value = null
                // With coarse-only collection this provider may simply refuse a caller holding a
                // perfectly good coarse grant. Only an actually absent grant is a refusal; calling
                // the other case "refused" would accuse the owner of a choice they never made.
                origin = if (hasCoarseGrant()) {
                    NearbyOriginState.LocationUnavailable
                } else {
                    NearbyOriginState.PermissionRefused
                }
            }
        } catch (_: Exception) {
            if (cancellation.value === signal) {
                cancellation.value = null
                origin = NearbyOriginState.LocationUnavailable
            }
        }
    }

    // Coarse only, deliberately: the service needs no more precision than this to rank providers, and
    // the one feature whose own copy promises privacy must not ask for precise GPS. Whatever this
    // grant yields is what the request sends, unmodified - see NearbyDecision.LOCATION_DISCLOSURE.
    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) resolveCurrentLocation() else origin = NearbyOriginState.PermissionRefused
    }

    fun useCurrentLocation() {
        // This function is reached only from the explicit button below. Merely opening Nearby never
        // triggers a permission prompt or a location read.
        if (hasCoarseGrant()) {
            resolveCurrentLocation()
        } else {
            permissionLauncher.launch(Manifest.permission.ACCESS_COARSE_LOCATION)
        }
    }

    val callerPosition = remember(origin) {
        (origin as? NearbyOriginState.Available)
            ?.point
            ?.let(CallerPosition::from)
    }
    val trimmedQuery = query.trim()

    LaunchedEffect(scope, trimmedQuery, callerPosition?.lat, callerPosition?.lng, refreshKey) {
        requestGeneration += 1
        val generation = requestGeneration
        loadingMore = false
        directoryResult = null
        actionError = null
        if (scope == DirectoryScope.Nearby && callerPosition == null) {
            return@LaunchedEffect
        }
        if (trimmedQuery.isNotEmpty()) delay(300)
        val page = loadDirectoryPage(
            directory = directory,
            scope = scope,
            name = trimmedQuery,
            position = callerPosition,
            offset = 0,
        )
        if (generation == requestGeneration) {
            directoryResult = page
            // Remember what the owner just saw, and reach for the remembered set ONLY when the live
            // read could not answer at all. An `Empty` is an answer, so it is not a fallback case.
            when (page) {
                is ProviderDirectoryResult.Found ->
                    recordCache.remember(page.providers, System.currentTimeMillis())
                is ProviderDirectoryResult.Unavailable ->
                    storedRecords = recordCache.recall(System.currentTimeMillis())
                is ProviderDirectoryResult.Empty -> Unit
            }
        }
    }

    fun loadMore() {
        val current = directoryResult as? ProviderDirectoryResult.Found ?: return
        if (!current.hasMore || loadingMore) return
        val generation = requestGeneration
        val nextOffset = current.offset + current.providers.size
        loadingMore = true
        coroutineScope.launch {
            val next = loadDirectoryPage(
                directory = directory,
                scope = scope,
                name = trimmedQuery,
                position = callerPosition,
                offset = nextOffset,
            )
            if (generation == requestGeneration) {
                directoryResult = appendDirectoryPage(current, next)
                loadingMore = false
            }
        }
    }

    val unitSystem = remember {
        NearbyDecision.unitSystemForRegion(Locale.getDefault().toLanguageTag())
    }
    val livePresentation = NearbyDecision.nearby(
        directory = directoryResult,
        origin = origin,
        query = query,
        unit = unitSystem,
    )
    // The remembered set may only stand in when the live read could not answer. It never overrides a
    // real answer, and when there is nothing relevant remembered the live "could not check" stands -
    // a fallback that answered an empty list would turn could-not-check into an established absence.
    val nearbyPresentation = if (livePresentation is NearbyPresentation.DirectoryUnavailable) {
        NearbyDecision.storedFallback(storedRecords, query, System.currentTimeMillis())
            ?: livePresentation
    } else {
        livePresentation
    }
    val contactPresentation = NearbyDecision.contacts(directoryResult, query)
    val fixAccuracyNote = (origin as? NearbyOriginState.Available)
        ?.let { NearbyDecision.accuracyNote(it.accuracyMetres, unitSystem) }

    Column(Modifier.fillMaxSize().background(c.background)) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back", tint = c.onBackground)
            }
            Text(
                "Nearby providers",
                fontSize = 24.sp,
                fontWeight = FontWeight.Bold,
                color = c.onBackground,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { refreshKey += 1 }) {
                Icon(Icons.Filled.Refresh, "Refresh provider directory", tint = c.accent)
            }
        }

        ScopePicker(scope = scope, onSelect = { scope = it })

        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
            singleLine = true,
            leadingIcon = { Icon(Icons.Filled.Search, null) },
            label = { Text("Search by provider name") },
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Text,
                autoCorrectEnabled = false,
            ),
        )

        if (scope == DirectoryScope.Nearby) {
            OriginPicker(
                origin = origin,
                fixAccuracyNote = fixAccuracyNote,
                onUseCurrent = ::useCurrentLocation,
            )
        }

        actionError?.let { error ->
            Text(
                error,
                color = c.danger,
                fontSize = 12.sp,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
            )
        }

        if (scope == DirectoryScope.Nearby) {
            NearbyResults(
                presentation = nearbyPresentation,
                hasMore = (directoryResult as? ProviderDirectoryResult.Found)?.hasMore == true,
                pageLoadFailure =
                    (directoryResult as? ProviderDirectoryResult.Found)?.pageLoadFailure,
                loadingMore = loadingMore,
                onLoadMore = ::loadMore,
                onOpen = { uri, dial ->
                    actionError = if (openExternal(context, uri, dial)) {
                        null
                    } else {
                        openFailureMessage(uri)
                    }
                },
                modifier = Modifier.weight(1f),
            )
        } else {
            ContactResults(
                presentation = contactPresentation,
                hasMore = (directoryResult as? ProviderDirectoryResult.Found)?.hasMore == true,
                pageLoadFailure =
                    (directoryResult as? ProviderDirectoryResult.Found)?.pageLoadFailure,
                loadingMore = loadingMore,
                onLoadMore = ::loadMore,
                onOpen = { uri, dial ->
                    actionError = if (openExternal(context, uri, dial)) {
                        null
                    } else {
                        openFailureMessage(uri)
                    }
                },
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun ScopePicker(scope: DirectoryScope, onSelect: (DirectoryScope) -> Unit) {
    val c = DogTagTheme.colors
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        DirectoryScope.entries.forEach { candidate ->
            val selected = scope == candidate
            Box(
                Modifier.weight(1f).clip(CircleShape)
                    .background(if (selected) c.accent else c.surfaceVariant)
                    .clickable { onSelect(candidate) }
                    .padding(vertical = 9.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    if (candidate == DirectoryScope.Nearby) "Nearby" else "Provider contacts",
                    color = if (selected) c.onAccent else c.onBackground,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }
    }
}

@Composable
private fun OriginPicker(
    origin: NearbyOriginState,
    fixAccuracyNote: String?,
    onUseCurrent: () -> Unit,
) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(16.dp)).background(c.surface).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Text(
            "Find providers near you",
            fontSize = 14.sp,
            fontWeight = FontWeight.Bold,
            color = c.onBackground,
        )
        Text(
            NearbyDecision.LOCATION_DISCLOSURE,
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold,
            color = c.onBackground,
        )
        Button(
            onClick = onUseCurrent,
            colors = ButtonDefaults.buttonColors(
                containerColor = c.accent,
                contentColor = c.onAccent,
            ),
            modifier = Modifier.fillMaxWidth(),
        ) {
            Icon(Icons.Filled.MyLocation, null, modifier = Modifier.size(17.dp))
            Spacer(Modifier.size(6.dp))
            Text("Use my current location", fontSize = 12.sp)
        }
        when (origin) {
            NearbyOriginState.Locating ->
                Text("Getting your current position…", fontSize = 12.sp, color = c.muted)
            is NearbyOriginState.Available ->
                Text(
                    fixAccuracyNote?.let {
                        "Using your current location (device accuracy $it)."
                    } ?: "Using your current location.",
                    fontSize = 12.sp,
                    color = c.success,
                )
            else -> Unit
        }
    }
}

@Composable
private fun NearbyResults(
    presentation: NearbyPresentation,
    hasMore: Boolean,
    pageLoadFailure: String?,
    loadingMore: Boolean,
    onLoadMore: () -> Unit,
    onOpen: (Uri, Boolean) -> Unit,
    modifier: Modifier,
) {
    LazyColumn(
        modifier = modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        when (presentation) {
            NearbyPresentation.LoadingDirectory ->
                item { LoadingCard("Loading provider directory…") }
            is NearbyPresentation.DirectoryUnavailable -> item {
                StateCard(
                    "Provider directory unavailable",
                    "We could not reach the provider directory. This does not mean there are no vets " +
                        "or groomers nearby. ${presentation.detail}",
                    danger = true,
                )
            }
            is NearbyPresentation.DirectoryEmpty -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "The directory is empty",
                    "The provider directory was reached successfully, but currently contains no providers.",
                )
            }
            NearbyPresentation.AwaitingOrigin -> item {
                StateCard(
                    "Use your current location",
                    "DogTag sends your current location to the provider service to find the nearest listed vets and groomers. It is not stored.",
                )
            }
            NearbyPresentation.Locating -> item { LoadingCard("Getting your current position…") }
            NearbyPresentation.PermissionRefused -> item {
                StateCard(
                    "Location permission refused",
                    "DogTag cannot find nearest providers without your current location. " +
                        "You can still search Provider contacts by name.",
                    danger = true,
                )
            }
            NearbyPresentation.LocationUnavailable -> item {
                StateCard(
                    "Current location unavailable",
                    "Turn on location services and try again, or search Provider contacts by name.",
                    danger = true,
                )
            }
            is NearbyPresentation.NoNearbyProviders -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "No nearby providers found",
                    "The directory returned no located vet or groomer for this search. Provider contacts " +
                        "may still include businesses without a published location.",
                )
            }
            is NearbyPresentation.NoNameMatch -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "No provider named “${presentation.query}”",
                    "The provider directory found no matching vet or groomer.",
                )
            }
            is NearbyPresentation.StoredProvidersOnly -> {
                item {
                    StateCard(
                        "Showing providers saved on this phone",
                        buildString {
                            append("We could not reach the provider directory, so these are providers ")
                            append("this phone saw before")
                            presentation.storedAge?.let { append(" (last updated $it)") }
                            append(". They are NOT sorted by distance and no distance is shown: ")
                            append("that needs the service. Contact details may be out of date.")
                        },
                    )
                }
                // The contact row deliberately: it renders identity and contacts and makes no
                // proximity claim, which is exactly what a remembered record can support. It DOES
                // offer Directions here (captain's ruling, 2026-07-30) - an owner with no signal is
                // exactly who needs it, and the coordinate is part of the saved provider record
                // rather than anything derived from the owner's position. `storedRecord` also
                // carries the row's own stored-not-current note, so the offer stays honest.
                items(presentation.providers, key = { it.providerId }) { provider ->
                    ContactProviderRow(provider, onOpen, storedRecord = true)
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
            is NearbyPresentation.ProvidersFound -> {
                item { ObservationBanner(presentation.observation) }
                items(presentation.rows, key = { it.provider.providerId }) { row ->
                    NearbyProviderRow(row, onOpen)
                }
                if (hasMore) {
                    item {
                        PageButton(
                            loading = loadingMore,
                            pageLoadFailure = pageLoadFailure,
                            onLoadMore = onLoadMore,
                        )
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
        }
    }
}

@Composable
private fun ContactResults(
    presentation: ContactDirectoryPresentation,
    hasMore: Boolean,
    pageLoadFailure: String?,
    loadingMore: Boolean,
    onLoadMore: () -> Unit,
    onOpen: (Uri, Boolean) -> Unit,
    modifier: Modifier,
) {
    LazyColumn(
        modifier = modifier.fillMaxWidth(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        when (presentation) {
            ContactDirectoryPresentation.LoadingDirectory ->
                item { LoadingCard("Loading provider directory…") }
            is ContactDirectoryPresentation.DirectoryUnavailable -> item {
                StateCard(
                    "Provider directory unavailable",
                    "We could not reach the provider directory. This is not the same as finding no providers. " +
                        presentation.detail,
                    danger = true,
                )
            }
            is ContactDirectoryPresentation.DirectoryEmpty -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "No provider contacts",
                    "The provider directory was reached successfully, but has no eligible providers.",
                )
            }
            is ContactDirectoryPresentation.NoNameMatch -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "No provider named “${presentation.query}”",
                    "The provider directory found no matching vet or groomer.",
                )
            }
            is ContactDirectoryPresentation.ProvidersFound -> {
                item {
                    ObservationBanner(presentation.observation)
                    Text(
                        "This unranked directory includes providers that publish contact details but " +
                            "no location. DogTag never invents a placeholder location for them.",
                        fontSize = 12.sp,
                        color = DogTagTheme.colors.muted,
                        modifier = Modifier.padding(vertical = 4.dp),
                    )
                }
                items(presentation.providers, key = { it.providerId }) { provider ->
                    ContactProviderRow(provider, onOpen)
                }
                if (hasMore) {
                    item {
                        PageButton(
                            loading = loadingMore,
                            pageLoadFailure = pageLoadFailure,
                            onLoadMore = onLoadMore,
                        )
                    }
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
        }
    }
}

@Composable
private fun NearbyProviderRow(
    row: NearbyRow,
    onOpen: (Uri, Boolean) -> Unit,
) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ProviderHeading(row.provider)
        when (val distance = row.distance) {
            is DistanceClaim.Measured -> Text(
                distance.display,
                fontSize = 15.sp,
                fontWeight = FontWeight.Bold,
                color = c.onBackground,
            )
            // Never a confident number the origin cannot support, and never a silent blank either.
            is DistanceClaim.Uncertain -> Text(
                distance.reason,
                fontSize = 12.sp,
                color = c.muted,
            )
        }
        ProviderBindingChip(row.provider)
        ProviderDirectionsAction(row.provider, onOpen)
        ProviderContactActions(row.provider, onOpen)
    }
}

/**
 * The scope-neutral row: identity and contacts, and no proximity claim.
 *
 * [storedRecord] says this row came off the device's own saved copy rather than a live read. It is
 * what turns the Directions handoff on, and it is deliberately ONE flag doing both jobs - the offer
 * and its stored-not-current labelling arrive together, so the affordance cannot appear without the
 * sentence that qualifies it.
 */
@Composable
private fun ContactProviderRow(
    provider: DirectoryProvider,
    onOpen: (Uri, Boolean) -> Unit,
    storedRecord: Boolean = false,
) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(7.dp),
    ) {
        ProviderHeading(provider)
        if (provider.geo == null) {
            Text(
                "Contact listing · no published location",
                fontSize = 11.sp,
                color = c.muted,
            )
        }
        ProviderBindingChip(provider)
        if (storedRecord) {
            ProviderDirectionsAction(provider, onOpen, storedRecord = true)
        }
        ProviderContactActions(provider, onOpen)
    }
}

/**
 * Hands this provider's published destination to whichever maps app resolves the `geo:` intent.
 *
 * Offered on nearby rows and on OFFLINE STORED rows (captain's ruling, 2026-07-30: an owner with no
 * signal is exactly who most needs directions, the cached coordinate is part of the provider record
 * rather than anything derived from the owner's position, and a handoff to another app does not
 * break the no-embedded-map promise).
 *
 * It is deliberately NOT on the Provider contacts SEARCH scope, which shares [ContactProviderRow]:
 * that list's own copy promises it sends no position and shows no map, and the captain's ruling was
 * about the offline case rather than that promise. `storedRecord` is what tells the two apart, and
 * `apps/ios/maestro/nearby_scope_separation.yaml` pins the separation from both sides.
 *
 * Deliberately its own composable rather than a sixth entry in [ProviderContactActions]: a published
 * location is not a contact channel, and folding it in there would make it count toward the
 * `contact.hasAny` gate, so a location-only provider would stop rendering "No contact details
 * published." while still having published none.
 *
 * Absent when the provider published no usable location - never a dead button, and never a
 * fabricated destination. [NearbyDecision.directionsUri] owns that rule and carries the origin-free
 * guarantee; this composable only renders what it returns.
 */
@Composable
private fun ProviderDirectionsAction(
    provider: DirectoryProvider,
    onOpen: (Uri, Boolean) -> Unit,
    storedRecord: Boolean = false,
) {
    val uri = NearbyDecision.directionsUri(provider) ?: return
    val subtitle = if (storedRecord) {
        NearbyDecision.STORED_DIRECTIONS_NOTE
    } else {
        "Open in your maps app"
    }
    ContactAction(Icons.Filled.Directions, "Directions", subtitle) {
        onOpen(Uri.parse(uri), false)
    }
}

@Composable
private fun ProviderContactActions(
    provider: DirectoryProvider,
    onOpen: (Uri, Boolean) -> Unit,
) {
    val c = DogTagTheme.colors
    val contact = provider.contact
    contact.phone?.let {
        val number = it.filter { character ->
            character.isDigit() || character == '+' || character == '*' || character == '#'
        }
        if (number.any(Char::isDigit)) {
            ContactAction(Icons.Filled.Phone, "Phone", it) {
                onOpen(Uri.parse("tel:${Uri.encode(number)}"), true)
            }
        } else {
            PublishedContactValue("Phone", it)
        }
    }
    contact.whatsapp?.let {
        val number = it.filter { character -> character in '0'..'9' }
        if (number.isNotEmpty()) {
            ContactAction(Icons.AutoMirrored.Filled.Chat, "WhatsApp", it) {
                onOpen(Uri.parse("https://wa.me/$number"), false)
            }
        } else {
            PublishedContactValue("WhatsApp", it)
        }
    }
    contact.telegram?.let {
        val handle = it.trim()
            .replace(Regex("^https://t\\.me/", RegexOption.IGNORE_CASE), "")
            .trim('@', '/', ' ')
        if (handle.isNotEmpty() && handle.all { character ->
                character.isLetterOrDigit() || character == '_'
            }
        ) {
            ContactAction(Icons.AutoMirrored.Filled.Chat, "Telegram", it) {
                onOpen(Uri.parse("https://t.me/${Uri.encode(handle)}"), false)
            }
        } else {
            PublishedContactValue("Telegram", it)
        }
    }
    contact.email?.let {
        if ('@' in it) {
            ContactAction(Icons.Filled.Email, "Email", it) {
                onOpen(Uri.parse("mailto:${Uri.encode(it)}"), false)
            }
        } else {
            PublishedContactValue("Email", it)
        }
    }
    contact.website?.let {
        // These are the only caller-controlled URI schemes: explicit HTTP(S) or schemes built here.
        val url = it.trim()
        if (url.startsWith("http://", ignoreCase = true) ||
            url.startsWith("https://", ignoreCase = true)
        ) {
            ContactAction(Icons.Filled.Language, "Website", it) {
                onOpen(Uri.parse(url), false)
            }
        } else {
            PublishedContactValue("Website", it)
        }
    }
    if (!contact.hasAny) {
        Text("No contact details published.", fontSize = 12.sp, color = c.muted)
    }
}

@Composable
private fun PublishedContactValue(label: String, value: String) {
    val c = DogTagTheme.colors
    Text(
        "$label · $value",
        fontSize = 12.sp,
        color = c.muted,
        maxLines = 2,
    )
}

@Composable
private fun ProviderHeading(provider: DirectoryProvider) {
    val c = DogTagTheme.colors
    Row(verticalAlignment = Alignment.Top) {
        Box(
            Modifier.size(38.dp).clip(CircleShape).background(c.surfaceVariant),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Filled.Business, null, tint = c.accent, modifier = Modifier.size(19.dp))
        }
        Spacer(Modifier.size(10.dp))
        Column(Modifier.weight(1f)) {
            Text(provider.name, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
            Text(
                provider.kind.replaceFirstChar { it.uppercase() },
                fontSize = 12.sp,
                color = c.muted,
            )
            if (provider.services.isNotEmpty()) {
                Text(provider.services.joinToString(" · "), fontSize = 11.sp, color = c.muted)
            }
        }
    }
}

@Composable
private fun ProviderBindingChip(provider: DirectoryProvider) {
    val c = DogTagTheme.colors
    val binding = IssuerBinding(
        state = provider.bindingState,
        domain = provider.domain.orEmpty(),
    )
    val color = when (binding.tone) {
        BindingTone.Positive -> c.success
        BindingTone.Negative -> c.danger
        BindingTone.Neutral, BindingTone.Pending -> c.muted
    }
    Box(
        Modifier.clip(CircleShape).background(color.copy(alpha = 0.12f))
            .padding(horizontal = 10.dp, vertical = 5.dp),
    ) {
        Text(binding.line, fontSize = 10.sp, color = color, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun ContactAction(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    label: String,
    value: String,
    onClick: () -> Unit,
) {
    val c = DogTagTheme.colors
    TextButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Icon(icon, null, modifier = Modifier.size(17.dp), tint = c.accent)
        Spacer(Modifier.size(8.dp))
        Column(Modifier.weight(1f), horizontalAlignment = Alignment.Start) {
            Text(label, fontSize = 11.sp, color = c.muted)
            Text(value, fontSize = 13.sp, color = c.onBackground)
        }
    }
}

@Composable
private fun PageButton(loading: Boolean, pageLoadFailure: String?, onLoadMore: () -> Unit) {
    Column(
        Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        // The failed page is named beside its own retry. Keeping the loaded pages without saying why
        // the next one is missing would leave "Load more" looking as though it did nothing.
        if (pageLoadFailure != null && !loading) {
            Text(
                "The next page could not be loaded. $pageLoadFailure. " +
                    "The providers already listed are unaffected.",
                fontSize = 12.sp,
                color = DogTagTheme.colors.warning,
            )
        }
        Button(
            onClick = onLoadMore,
            enabled = !loading,
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (loading) {
                CircularProgressIndicator(
                    modifier = Modifier.size(16.dp),
                    color = DogTagTheme.colors.onAccent,
                    strokeWidth = 2.dp,
                )
                Spacer(Modifier.size(8.dp))
            }
            Text(
                when {
                    loading -> "Loading more…"
                    pageLoadFailure != null -> "Try loading more again"
                    else -> "Load more"
                }
            )
        }
    }
}

@Composable
private fun ObservationBanner(observation: DirectoryObservation) {
    if (observation != DirectoryObservation.Stored) return
    val c = DogTagTheme.colors
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
            .background(c.surfaceVariant).padding(12.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Icon(Icons.Filled.Info, null, tint = c.muted, modifier = Modifier.size(17.dp))
        Spacer(Modifier.size(8.dp))
        Text(
            "Using a saved, unexpired directory snapshot because the live refresh could not complete.",
            fontSize = 11.sp,
            color = c.muted,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
private fun LoadingCard(message: String) {
    val c = DogTagTheme.colors
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(20.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(modifier = Modifier.size(22.dp), color = c.accent, strokeWidth = 2.dp)
        Spacer(Modifier.size(12.dp))
        Text(message, fontSize = 13.sp, color = c.muted)
    }
}

@Composable
private fun StateCard(title: String, body: String, danger: Boolean = false) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            if (danger) Icons.Filled.LocationOff else Icons.Filled.LocationOn,
            null,
            tint = if (danger) c.danger else c.accent,
            modifier = Modifier.size(30.dp),
        )
        Text(title, fontSize = 16.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text(body, fontSize = 13.sp, color = c.muted)
    }
}

private suspend fun loadDirectoryPage(
    directory: ProviderDirectory,
    scope: DirectoryScope,
    name: String,
    position: CallerPosition?,
    offset: Int,
): ProviderDirectoryResult {
    val query = ProviderDirectoryQuery(
        kinds = OWNER_PROVIDER_KINDS,
        name = name,
        limit = DEFAULT_PROVIDER_PAGE_SIZE,
        offset = offset,
    )
    return when (scope) {
        DirectoryScope.Nearby -> {
            if (position == null) {
                ProviderDirectoryResult.Unavailable(
                    reason = io.liberalize.dogtag.nearby.DirectoryUnavailableReason.InvalidSnapshot,
                    detail = "A current location is required for Nearby",
                    attemptedAt = System.currentTimeMillis(),
                )
            } else {
                directory.nearest(position, query)
            }
        }
        DirectoryScope.Contacts -> directory.contacts(query)
    }
}

/**
 * Names what could not be opened, because the maps handoff and a contact channel are not the same
 * failure and a published location is not a contact method.
 *
 * This is also the one branch that is actually likely: `tel:`, `mailto:` and `https:` resolve on
 * essentially any device, while one with no maps app installed resolves no `geo:` intent at all.
 *
 * The scheme is the discriminator because the `dial` flag cannot be one - website, Telegram,
 * WhatsApp and email all pass `false` exactly as the handoff does. `geo:` is what
 * [NearbyDecision.directionsUri] emits and nothing else on this screen produces: every other action
 * builds `tel:`, `mailto:`, `https://wa.me/`, `https://t.me/`, or an operator-supplied website
 * already gated to an `http(s)://` prefix. `NearbyDecisionTest` asserts that exact URI string, so a
 * change of scheme there fails a test rather than silently misnaming this failure.
 *
 * Neither sentence blames the owner or claims the provider published no location: the location is
 * published and fine, what is missing is an app to show it.
 */
private fun openFailureMessage(uri: Uri): String =
    if (uri.scheme.equals("geo", ignoreCase = true)) {
        "No app on this device can open a map."
    } else {
        "No app could open this contact method."
    }

private fun openExternal(context: Context, uri: Uri, dial: Boolean): Boolean {
    val action = if (dial) Intent.ACTION_DIAL else Intent.ACTION_VIEW
    return try {
        context.startActivity(Intent(action, uri))
        true
    } catch (_: ActivityNotFoundException) {
        false
    } catch (_: SecurityException) {
        false
    }
}
