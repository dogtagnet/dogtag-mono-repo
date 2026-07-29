package io.liberalize.dogtag.ui.screens

import android.Manifest
import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.location.Location
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
import androidx.compose.material3.OutlinedButton
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
import io.liberalize.dogtag.nearby.DirectoryObservation
import io.liberalize.dogtag.nearby.DirectoryProvider
import io.liberalize.dogtag.nearby.GeoPoint
import io.liberalize.dogtag.nearby.NearbyDecision
import io.liberalize.dogtag.nearby.NearbyOriginState
import io.liberalize.dogtag.nearby.NearbyPresentation
import io.liberalize.dogtag.nearby.NearbyRow
import io.liberalize.dogtag.nearby.OriginSource
import io.liberalize.dogtag.nearby.ProviderDirectories
import io.liberalize.dogtag.nearby.ProviderDirectoryResult
import io.liberalize.dogtag.nearby.ProviderMeasurement
import io.liberalize.dogtag.net.BindingTone
import io.liberalize.dogtag.net.IssuerBinding
import io.liberalize.dogtag.ui.DogTagTheme
import java.util.Locale

private enum class DirectoryScope { Nearby, Contacts }

/**
 * List-first provider discovery. There is deliberately no map view: a destination leaves DogTag only
 * after the owner taps Directions, and the handoff contains that public destination but no origin.
 */
@Composable
fun NearbyScreen(onBack: () -> Unit) {
    val c = DogTagTheme.colors
    val context = LocalContext.current
    val directory = remember { ProviderDirectories.central }
    var directoryResult by remember { mutableStateOf<ProviderDirectoryResult?>(null) }
    var refreshKey by remember { mutableIntStateOf(0) }
    var scope by rememberSaveable { mutableStateOf(DirectoryScope.Nearby) }
    // Search and chosen coordinates are deliberately process-memory only. Saving them into the
    // activity's recreation Bundle would turn private inputs into a second cache.
    var query by remember { mutableStateOf("") }
    var origin by remember { mutableStateOf<NearbyOriginState>(NearbyOriginState.AwaitingChoice) }
    var showCoordinates by rememberSaveable { mutableStateOf(false) }
    var latitude by remember { mutableStateOf("") }
    var longitude by remember { mutableStateOf("") }
    var handoffError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(refreshKey) {
        directoryResult = null
        directoryResult = directory.read()
    }

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
                        NearbyOriginState.Available(point, OriginSource.CurrentLocation)
                    } else {
                        NearbyOriginState.LocationUnavailable
                    }
                }
            }
        } catch (_: SecurityException) {
            if (cancellation.value === signal) {
                cancellation.value = null
                origin = NearbyOriginState.PermissionRefused
            }
        } catch (_: Exception) {
            if (cancellation.value === signal) {
                cancellation.value = null
                origin = NearbyOriginState.LocationUnavailable
            }
        }
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants ->
        val granted = grants[Manifest.permission.ACCESS_COARSE_LOCATION] == true ||
            grants[Manifest.permission.ACCESS_FINE_LOCATION] == true
        if (granted) resolveCurrentLocation() else origin = NearbyOriginState.PermissionRefused
    }

    fun useCurrentLocation() {
        // This function is reached only from the explicit button below. Merely opening Nearby never
        // triggers a permission prompt or a location read.
        val granted =
            ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED ||
                ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION) ==
                PackageManager.PERMISSION_GRANTED
        if (granted) {
            resolveCurrentLocation()
        } else {
            permissionLauncher.launch(
                arrayOf(
                    Manifest.permission.ACCESS_COARSE_LOCATION,
                    Manifest.permission.ACCESS_FINE_LOCATION,
                ),
            )
        }
    }

    val measurements = remember(directoryResult, origin) {
        val found = directoryResult as? ProviderDirectoryResult.Found
        val available = origin as? NearbyOriginState.Available
        if (found == null || available == null) {
            emptyList()
        } else {
            found.providers.mapNotNull { provider ->
                measure(available.point, provider)
            }
        }
    }
    val nearbyPresentation = NearbyDecision.nearby(
        directory = directoryResult,
        origin = origin,
        measurements = measurements,
        query = query,
    )
    val contactPresentation = NearbyDecision.contacts(directoryResult, query)
    val unitSystem = remember {
        NearbyDecision.unitSystemForRegion(Locale.getDefault().toLanguageTag())
    }
    val offerOriginChoice = when (nearbyPresentation) {
        NearbyPresentation.AwaitingOrigin,
        NearbyPresentation.Locating,
        NearbyPresentation.PermissionRefused,
        NearbyPresentation.LocationUnavailable,
        NearbyPresentation.InvalidChosenLocation,
        is NearbyPresentation.ProvidersFound,
        -> true
        // Keep a chosen/current origin editable after a result changes. If the phone-local list
        // proves there is no possible candidate before an origin exists, do not invite a pointless
        // permission request.
        is NearbyPresentation.NoneWithinRange,
        is NearbyPresentation.NoNameMatch,
        -> origin is NearbyOriginState.Available
        NearbyPresentation.LoadingDirectory,
        is NearbyPresentation.DirectoryUnavailable,
        is NearbyPresentation.DirectoryEmpty,
        -> false
    }

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

        if (scope == DirectoryScope.Nearby && offerOriginChoice) {
            OriginPicker(
                origin = origin,
                showCoordinates = showCoordinates,
                latitude = latitude,
                longitude = longitude,
                onUseCurrent = ::useCurrentLocation,
                onToggleCoordinates = { showCoordinates = !showCoordinates },
                onLatitude = { latitude = it },
                onLongitude = { longitude = it },
                onApplyCoordinates = {
                    // A chosen origin deliberately supersedes any current-location request. The
                    // callback's identity check keeps an already-queued result from winning the race.
                    cancellation.value?.cancel()
                    cancellation.value = null
                    val point = NearbyDecision.parseChosenOrigin(latitude, longitude)
                    origin = if (point == null) {
                        NearbyOriginState.InvalidChosenLocation
                    } else {
                        NearbyOriginState.Available(point, OriginSource.ChosenCoordinates)
                    }
                },
            )
        }

        handoffError?.let { error ->
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
                unitSystem = unitSystem,
                onDirections = { provider ->
                    handoffError = if (openDirections(context, provider)) {
                        null
                    } else {
                        "No maps app could open this destination."
                    }
                },
                modifier = Modifier.weight(1f),
            )
        } else {
            ContactResults(
                presentation = contactPresentation,
                onOpen = { uri, dial ->
                    handoffError = if (openExternal(context, uri, dial)) {
                        null
                    } else {
                        "No app could open this contact method."
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
    showCoordinates: Boolean,
    latitude: String,
    longitude: String,
    onUseCurrent: () -> Unit,
    onToggleCoordinates: () -> Unit,
    onLatitude: (String) -> Unit,
    onLongitude: (String) -> Unit,
    onApplyCoordinates: () -> Unit,
) {
    val c = DogTagTheme.colors
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)
            .clip(RoundedCornerShape(16.dp)).background(c.surface).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(9.dp),
    ) {
        Text("Distance from", fontSize = 14.sp, fontWeight = FontWeight.Bold, color = c.onBackground)
        Text(
            "DogTag uses your position only on this phone. It is never sent to DogTag, the provider " +
                "directory, or a provider.",
            fontSize = 12.sp,
            color = c.muted,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(
                onClick = onUseCurrent,
                colors = ButtonDefaults.buttonColors(
                    containerColor = c.accent,
                    contentColor = c.onAccent,
                ),
                modifier = Modifier.weight(1f),
            ) {
                Icon(Icons.Filled.MyLocation, null, modifier = Modifier.size(17.dp))
                Spacer(Modifier.size(6.dp))
                Text("Use my location", fontSize = 12.sp)
            }
            OutlinedButton(onClick = onToggleCoordinates, modifier = Modifier.weight(1f)) {
                Icon(Icons.Filled.LocationOn, null, modifier = Modifier.size(17.dp))
                Spacer(Modifier.size(6.dp))
                Text("Coordinates", fontSize = 12.sp)
            }
        }
        when (origin) {
            NearbyOriginState.Locating ->
                Text("Getting your current position…", fontSize = 12.sp, color = c.muted)
            is NearbyOriginState.Available ->
                Text(
                    if (origin.source == OriginSource.CurrentLocation) {
                        "Using your current position on this phone."
                    } else {
                        "Using your chosen coordinates on this phone."
                    },
                    fontSize = 12.sp,
                    color = c.success,
                )
            else -> Unit
        }
        if (showCoordinates) {
            Text(
                "Enter decimal coordinates. They are parsed here and never geocoded or sent anywhere.",
                fontSize = 11.sp,
                color = c.muted,
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(
                    value = latitude,
                    onValueChange = onLatitude,
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    label = { Text("Latitude") },
                    // Decimal keyboards commonly omit a minus key, making half the globe
                    // impossible to enter. Parsing and validation still happen locally on submit.
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Text,
                        autoCorrectEnabled = false,
                    ),
                )
                OutlinedTextField(
                    value = longitude,
                    onValueChange = onLongitude,
                    modifier = Modifier.weight(1f),
                    singleLine = true,
                    label = { Text("Longitude") },
                    keyboardOptions = KeyboardOptions(
                        keyboardType = KeyboardType.Text,
                        autoCorrectEnabled = false,
                    ),
                )
            }
            Button(onClick = onApplyCoordinates, modifier = Modifier.fillMaxWidth()) {
                Text("Use chosen location")
            }
        }
    }
}

@Composable
private fun NearbyResults(
    presentation: NearbyPresentation,
    unitSystem: NearbyDecision.UnitSystem,
    onDirections: (DirectoryProvider) -> Unit,
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
                    "Choose where to measure from",
                    "Use your current position or enter coordinates above. Neither is sent to the directory.",
                )
            }
            NearbyPresentation.Locating -> item { LoadingCard("Getting your current position…") }
            NearbyPresentation.PermissionRefused -> item {
                StateCard(
                    "Location permission refused",
                    "DogTag cannot use your current position. You can still enter coordinates above; " +
                        "that path needs no permission.",
                    danger = true,
                )
            }
            NearbyPresentation.LocationUnavailable -> item {
                StateCard(
                    "Current location unavailable",
                    "Turn on location services and try again, or enter coordinates above.",
                    danger = true,
                )
            }
            NearbyPresentation.InvalidChosenLocation -> item {
                StateCard(
                    "Those coordinates are not valid",
                    "Latitude must be between -90 and 90, and longitude between -180 and 180.",
                    danger = true,
                )
            }
            is NearbyPresentation.NoneWithinRange -> item {
                ObservationBanner(presentation.observation)
                val radius = NearbyDecision.formatDistanceKm(presentation.radiusKm, unitSystem)
                    ?: "${presentation.radiusKm.toInt()} km"
                StateCard(
                    "No providers within range",
                    "No listed vet or groomer is within $radius. " +
                        "Provider contacts may still include businesses without a published location.",
                )
            }
            is NearbyPresentation.NoNameMatch -> item {
                ObservationBanner(presentation.observation)
                StateCard(
                    "No provider named “${presentation.query}”",
                    "The directory was searched on this phone; your search text was not sent anywhere.",
                )
            }
            is NearbyPresentation.ProvidersFound -> {
                item { ObservationBanner(presentation.observation) }
                items(presentation.rows, key = { it.provider.providerId }) { row ->
                    NearbyProviderRow(row, unitSystem, onDirections)
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
        }
    }
}

@Composable
private fun ContactResults(
    presentation: ContactDirectoryPresentation,
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
                    "The directory was searched on this phone; your search text was not sent anywhere.",
                )
            }
            is ContactDirectoryPresentation.ProvidersFound -> {
                item {
                    ObservationBanner(presentation.observation)
                    Text(
                        "This unranked directory includes providers that publish contact details but " +
                            "no location. It never gives them a placeholder pin or Directions button.",
                        fontSize = 12.sp,
                        color = DogTagTheme.colors.muted,
                        modifier = Modifier.padding(vertical = 4.dp),
                    )
                }
                items(presentation.providers, key = { it.providerId }) { provider ->
                    ContactProviderRow(provider, onOpen)
                }
                item { Spacer(Modifier.height(16.dp)) }
            }
        }
    }
}

@Composable
private fun NearbyProviderRow(
    row: NearbyRow,
    unitSystem: NearbyDecision.UnitSystem,
    onDirections: (DirectoryProvider) -> Unit,
) {
    val c = DogTagTheme.colors
    val distance = NearbyDecision.formatDistanceKm(row.distanceKm, unitSystem)
    val bearing = NearbyDecision.formatBearing(row.bearingDegrees)
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(c.surface).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ProviderHeading(row.provider)
        Text(
            listOfNotNull(distance, bearing).joinToString(" · "),
            fontSize = 15.sp,
            fontWeight = FontWeight.Bold,
            color = c.onBackground,
        )
        ProviderBindingChip(row.provider)
        Button(
            onClick = { onDirections(row.provider) },
            enabled = row.canOpenDirections,
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.buttonColors(
                containerColor = c.accent,
                contentColor = c.onAccent,
            ),
        ) {
            Icon(Icons.Filled.Directions, null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.size(7.dp))
            Text("Directions")
        }
    }
}

@Composable
private fun ContactProviderRow(
    provider: DirectoryProvider,
    onOpen: (Uri, Boolean) -> Unit,
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
        if (!contact.hasAny) {
            Text("No contact details published.", fontSize = 12.sp, color = c.muted)
        }
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

private fun measure(origin: GeoPoint, provider: DirectoryProvider): ProviderMeasurement? {
    val destination = provider.geo?.takeIf { it.isUsable } ?: return null
    val results = FloatArray(2)
    return runCatching {
        // Platform primitive: no second haversine/bearing implementation lives in the app.
        Location.distanceBetween(
            origin.lat,
            origin.lng,
            destination.lat,
            destination.lng,
            results,
        )
        ProviderMeasurement(
            providerId = provider.providerId,
            distanceKm = results[0].toDouble() / 1_000.0,
            // Android reports a plausible 0° for geometries where a heading is undefined. Preserve
            // the canonical geo rule instead of rendering that non-answer as "N".
            bearingDegrees = if (
                (origin.lat == destination.lat && origin.lng == destination.lng) ||
                kotlin.math.abs(origin.lat) == 90.0
            ) {
                null
            } else {
                results.getOrNull(1)?.toDouble()
            },
        )
    }.getOrNull()
}

private fun openDirections(context: Context, provider: DirectoryProvider): Boolean {
    val destination = provider.geo?.takeIf { it.isUsable } ?: return false
    val coordinates = "${destination.lat},${destination.lng}"
    val label = Uri.encode("$coordinates (${provider.name})")
    // Both URI coordinate occurrences are the PUBLIC DESTINATION. The current/chosen origin is never
    // placed in the intent; the external maps app decides what to do only after this deliberate tap.
    val uri = Uri.parse("geo:$coordinates?q=$label")
    val intent = Intent.createChooser(Intent(Intent.ACTION_VIEW, uri), "Open directions")
    return try {
        context.startActivity(intent)
        true
    } catch (_: ActivityNotFoundException) {
        false
    } catch (_: SecurityException) {
        false
    }
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
