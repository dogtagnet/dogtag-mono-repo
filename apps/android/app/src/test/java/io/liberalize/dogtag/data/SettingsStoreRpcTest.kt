package io.liberalize.dogtag.data

import io.liberalize.dogtag.net.RoaxRpc
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

/** The selected chain peer survives store reconstruction; the centralized API is not part of it. */
@RunWith(RobolectricTestRunner::class)
class SettingsStoreRpcTest {
    @Test
    fun customRpcPersistsAndUseDefaultClearsIt() = runBlocking {
        val context = RuntimeEnvironment.getApplication().applicationContext
        val first = SettingsStore(context)
        val custom = "https://rpc.example/v1/key"

        // Establish a deterministic start even if Robolectric preserves this app sandbox.
        first.setRpcUrl(RoaxRpc.DEFAULT_RPC)
        first.setRpcUrl(custom)

        assertEquals(custom, SettingsStore(context).settings.first().rpcUrl)

        first.setRpcUrl(RoaxRpc.DEFAULT_RPC)
        assertEquals(RoaxRpc.DEFAULT_RPC, SettingsStore(context).settings.first().rpcUrl)
    }
}
