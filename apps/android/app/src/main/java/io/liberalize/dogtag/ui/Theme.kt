package io.liberalize.dogtag.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/**
 * The seven brand accents the user can pick from (reference: pet-owner app theme dot row).
 * Each theme resolves to a full set of semantic color tokens for BOTH light and dark.
 */
enum class ThemeId(val label: String, val accent: Color) {
    Black("Black", Color(0xFF222222)),
    White("White", Color(0xFF6E7178)),
    Blue("Blue", Color(0xFF2F6BFF)),
    Red("Red", Color(0xFFEF4E4E)),
    Pink("Pink", Color(0xFFEC6FB0)),
    Green("Green", Color(0xFF2FB36B)),
    Yellow("Yellow", Color(0xFFF2B23A)),
}

/**
 * Semantic design tokens — screens reference these, never raw hex. The theme layer maps the chosen
 * (ThemeId, isDark) pair onto a concrete token set so a single picker + a dark switch drive the
 * whole app (7 themes × light/dark = 14 palettes).
 */
data class DogTagColors(
    val accent: Color,
    val onAccent: Color,
    val background: Color,
    val surface: Color,
    val surfaceVariant: Color,
    val onBackground: Color,
    val onSurface: Color,
    val muted: Color,
    val outline: Color,
    val success: Color,
    val danger: Color,
    // Credential-group tints (health / service / travel cards on Home).
    val healthTint: Color,
    val serviceTint: Color,
    val travelTint: Color,
    val isDark: Boolean,
)

val LocalDogTagColors = staticCompositionLocalOf<DogTagColors> {
    error("DogTagColors not provided")
}

/** Convenience accessor: `DogTagTheme.colors.accent`. */
object DogTagTheme {
    val colors: DogTagColors
        @Composable get() = LocalDogTagColors.current
}

private fun tokensFor(id: ThemeId, dark: Boolean): DogTagColors {
    val accent = id.accent
    return if (dark) {
        DogTagColors(
            accent = accent,
            onAccent = Color.White,
            background = Color(0xFF0E1014),
            surface = Color(0xFF181B22),
            surfaceVariant = Color(0xFF222631),
            onBackground = Color(0xFFF2F4F8),
            onSurface = Color(0xFFE6E9EF),
            muted = Color(0xFF9AA0AC),
            outline = Color(0xFF2C313C),
            success = Color(0xFF45C97B),
            danger = Color(0xFFFF6B6B),
            healthTint = Color(0xFF3A1F22),
            serviceTint = Color(0xFF15301F),
            travelTint = Color(0xFF152234),
            isDark = true,
        )
    } else {
        DogTagColors(
            accent = accent,
            onAccent = Color.White,
            background = Color(0xFFF6F7FB),
            surface = Color(0xFFFFFFFF),
            surfaceVariant = Color(0xFFEEF0F6),
            onBackground = Color(0xFF13151A),
            onSurface = Color(0xFF1B1E25),
            muted = Color(0xFF6B7180),
            outline = Color(0xFFE2E5EE),
            success = Color(0xFF1B7F3B),
            danger = Color(0xFFD23B3B),
            healthTint = Color(0xFFFDECEC),
            serviceTint = Color(0xFFE7F6EE),
            travelTint = Color(0xFFE8F1FD),
            isDark = false,
        )
    }
}

/**
 * Root theme. Bridges our semantic tokens onto Material3's ColorScheme (so stock M3 components also
 * pick up the accent) and exposes the richer DogTagColors via a CompositionLocal.
 */
@Composable
fun DogTagAppTheme(
    themeId: ThemeId,
    darkMode: Boolean?, // null = follow system
    content: @Composable () -> Unit,
) {
    val dark = darkMode ?: isSystemInDarkTheme()
    val c = tokensFor(themeId, dark)
    val scheme = if (dark) {
        darkColorScheme(
            primary = c.accent, onPrimary = c.onAccent,
            background = c.background, onBackground = c.onBackground,
            surface = c.surface, onSurface = c.onSurface,
            surfaceVariant = c.surfaceVariant, outline = c.outline,
            error = c.danger,
        )
    } else {
        lightColorScheme(
            primary = c.accent, onPrimary = c.onAccent,
            background = c.background, onBackground = c.onBackground,
            surface = c.surface, onSurface = c.onSurface,
            surfaceVariant = c.surfaceVariant, outline = c.outline,
            error = c.danger,
        )
    }
    CompositionLocalProvider(LocalDogTagColors provides c) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
