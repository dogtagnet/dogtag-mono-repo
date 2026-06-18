package io.liberalize.dogtag.data

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import io.liberalize.dogtag.ui.ThemeId
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

private val Context.dataStore by preferencesDataStore(name = "dogtag_settings")

/** Tri-state dark preference: follow system, force light, force dark. */
enum class DarkPref { System, Light, Dark }

data class AppSettings(
    val themeId: ThemeId,
    val darkPref: DarkPref,
)

/** Persisted theme selection + dark preference (survives relaunch). */
class SettingsStore(private val context: Context) {
    private val keyTheme = intPreferencesKey("theme_id")
    private val keyDark = stringPreferencesKey("dark_pref")

    val settings: Flow<AppSettings> = context.dataStore.data.map { p ->
        val theme = ThemeId.entries.getOrElse(p[keyTheme] ?: ThemeId.Pink.ordinal) { ThemeId.Pink }
        val dark = runCatching { DarkPref.valueOf(p[keyDark] ?: DarkPref.System.name) }
            .getOrDefault(DarkPref.System)
        AppSettings(theme, dark)
    }

    suspend fun setTheme(id: ThemeId) {
        context.dataStore.edit { it[keyTheme] = id.ordinal }
    }

    suspend fun setDark(pref: DarkPref) {
        context.dataStore.edit { it[keyDark] = pref.name }
    }
}
