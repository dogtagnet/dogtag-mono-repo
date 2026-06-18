package io.liberalize.dogtag

import android.os.Bundle
import androidx.activity.compose.setContent
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.fragment.app.FragmentActivity
import io.liberalize.dogtag.data.DarkPref
import io.liberalize.dogtag.data.SettingsStore
import io.liberalize.dogtag.ui.DogTagApp
import io.liberalize.dogtag.ui.DogTagAppTheme

/**
 * FragmentActivity (required by AndroidX BiometricPrompt) hosting the full Compose app. The verify
 * core (Rust SDK parity over UniFFI) lives on the Verify tab; everything else — Home/pet card,
 * grouped credentials, Travel, Documents, Profile, themes, embedded wallet, consent signing — is
 * built around it.
 */
class MainActivity : FragmentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val store = SettingsStore(applicationContext)
        setContent {
            val settings by store.settings.collectAsState(initial = null)
            val s = settings
            if (s != null) {
                DogTagAppTheme(
                    themeId = s.themeId,
                    darkMode = when (s.darkPref) {
                        DarkPref.System -> null
                        DarkPref.Light -> false
                        DarkPref.Dark -> true
                    },
                ) {
                    DogTagApp(store = store, settings = s, activity = this)
                }
            }
        }
    }
}
