package io.liberalize.dogtag.data

import android.content.Context
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import io.liberalize.dogtag.net.RoaxRpc
import io.liberalize.dogtag.ui.ThemeId
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import java.io.IOException
import kotlin.coroutines.cancellation.CancellationException

private val Context.dataStore by preferencesDataStore(name = "dogtag_settings")

/** Tri-state dark preference: follow system, force light, force dark. */
enum class DarkPref { System, Light, Dark }

data class AppSettings(
    val themeId: ThemeId,
    val darkPref: DarkPref,
    /** User-selected chain peer. Central/indexer service URLs are deliberately not settings. */
    val rpcUrl: String,
)

/** Persisted appearance and chain-endpoint selection (survives relaunch). */
class SettingsStore(private val context: Context) {
    private val keyTheme = intPreferencesKey("theme_id")
    private val keyDark = stringPreferencesKey("dark_pref")
    private val keyRpcUrl = stringPreferencesKey("rpc_url")

    /**
     * An unreadable preferences file degrades to the shipped defaults rather than throwing into
     * every collector: a settings read that cannot answer is not evidence about the endpoint, and a
     * broken preference must never abort a blockchain read that the bundled endpoint could serve.
     */
    val settings: Flow<AppSettings> = context.dataStore.data.catch { cause ->
        if (cause is IOException) emit(emptyPreferences()) else throw cause
    }.map { p ->
        val theme = ThemeId.entries.getOrElse(p[keyTheme] ?: ThemeId.Pink.ordinal) { ThemeId.Pink }
        val dark = runCatching { DarkPref.valueOf(p[keyDark] ?: DarkPref.System.name) }
            .getOrDefault(DarkPref.System)
        val rpcUrl = p[keyRpcUrl]?.trim().takeUnless { it.isNullOrBlank() } ?: RoaxRpc.DEFAULT_RPC
        AppSettings(theme, dark, rpcUrl)
    }

    suspend fun setTheme(id: ThemeId) {
        context.dataStore.edit { it[keyTheme] = id.ordinal }
    }

    suspend fun setDark(pref: DarkPref) {
        context.dataStore.edit { it[keyDark] = pref.name }
    }

    /** Persist only the chain peer. The central provider/indexer API remains fixed in [AppConfig]. */
    suspend fun setRpcUrl(url: String) {
        val selected = url.trim()
        context.dataStore.edit {
            if (selected.isBlank() || selected == RoaxRpc.DEFAULT_RPC) {
                it.remove(keyRpcUrl)
            } else {
                it[keyRpcUrl] = selected
            }
        }
    }

    /**
     * The requested peer. [RoaxRpc] applies the chain guard at its raw transport seam immediately
     * before every blockchain read; callers must not pre-resolve this into an unguarded URL.
     *
     * Resolves to the bundled default when the preference cannot be read at all, so a corrupt
     * settings file costs the endpoint choice and not the read.
     */
    suspend fun selectedRpcUrl(): String =
        try {
            settings.first().rpcUrl
        } catch (cancellation: CancellationException) {
            throw cancellation
        } catch (failure: Exception) {
            RoaxRpc.DEFAULT_RPC
        }
}
