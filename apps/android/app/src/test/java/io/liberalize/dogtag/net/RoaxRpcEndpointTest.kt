package io.liberalize.dogtag.net

import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Test
import java.io.IOException

/**
 * Pins endpoint selection, not 4-byte function selection (that is [RoaxRpcSelectorTest]).
 *
 * Contract addresses come from one bundled chain, so a custom JSON-RPC URL is usable only after its
 * reported `eth_chainId` matches that bundle. Every other custom outcome falls back to the compiled
 * default; no test here treats the peer as trusted merely because it reports the expected number.
 */
class RoaxRpcEndpointTest {
    private val expectedChainId = 135L
    private val custom = "https://rpc.example/v1/key"

    private fun chainResponse(chainId: Long): Http.Response =
        Http.Response(200, """{"jsonrpc":"2.0","id":1,"result":"0x${chainId.toString(16)}"}""")

    @Test
    fun matchingCustomEndpointIsSelected() = runBlocking {
        val calls = mutableListOf<String>()
        val out = RoaxRpc.endpointRoute(
            "  $custom  ",
            expectedChainId,
        ) { url, body ->
            calls += url
            assertEquals("eth_chainId", JSONObject(body).getString("method"))
            chainResponse(expectedChainId)
        }

        assertEquals(RoaxRpc.EndpointRoute.Custom(custom), out)
        assertEquals(listOf(custom), calls)
    }

    @Test
    fun endpointReportingAnotherChainFallsBackToDefault() = runBlocking {
        val out = RoaxRpc.endpointRoute(
            custom,
            expectedChainId,
        ) { url, _ ->
            if (url == custom) chainResponse(1L) else chainResponse(expectedChainId)
        }

        assertEquals(RoaxRpc.DEFAULT_RPC, out.rpcUrl)
        assertEquals(
            RoaxRpc.EndpointRoute.BundledFallback(
                RoaxRpc.EndpointFailure.WrongChain(1L),
            ),
            out,
        )
    }

    @Test
    fun unreachableCustomEndpointFallsBackToDefault() = runBlocking {
        val out = RoaxRpc.endpointRoute(
            custom,
            expectedChainId,
        ) { url, _ ->
            if (url == custom) Http.Response(503, "") else chainResponse(expectedChainId)
        }

        assertEquals(RoaxRpc.DEFAULT_RPC, out.rpcUrl)
        assertEquals(
            RoaxRpc.EndpointRoute.BundledFallback(RoaxRpc.EndpointFailure.Unavailable),
            out,
        )
    }

    @Test
    fun malformedChainIdResponseFallsBackToDefault() = runBlocking {
        val out = RoaxRpc.endpointRoute(
            custom,
            expectedChainId,
        ) { url, _ ->
            if (url == custom) {
                Http.Response(200, """{"jsonrpc":"2.0","id":1,"result":"135"}""")
            } else {
                chainResponse(expectedChainId)
            }
        }

        assertEquals(
            RoaxRpc.EndpointRoute.BundledFallback(
                RoaxRpc.EndpointFailure.InvalidChainIdResponse,
            ),
            out,
        )
    }

    @Test
    fun invalidUrlSkipsThatPeerAndGuardsTheBundledFallback() = runBlocking {
        val calls = mutableListOf<String>()
        val out = RoaxRpc.endpointRoute(
            "file:///tmp/not-a-peer",
            expectedChainId,
        ) { url, _ ->
            calls += url
            chainResponse(expectedChainId)
        }

        assertEquals(
            RoaxRpc.EndpointRoute.BundledFallback(RoaxRpc.EndpointFailure.InvalidUrl),
            out,
        )
        assertEquals(listOf(RoaxRpc.DEFAULT_RPC), calls)
    }

    @Test
    fun defaultEndpointIsGuardedBeforeUse() = runBlocking {
        val calls = mutableListOf<String>()
        val out = RoaxRpc.endpointRoute(
            RoaxRpc.DEFAULT_RPC,
            expectedChainId,
        ) { url, body ->
            calls += url
            assertEquals("eth_chainId", JSONObject(body).getString("method"))
            chainResponse(expectedChainId)
        }

        assertSame(RoaxRpc.EndpointRoute.Bundled, out)
        assertEquals(listOf(RoaxRpc.DEFAULT_RPC), calls)
    }

    @Test
    fun neitherPeerPassingTheGuardProducesNoRoute() = runBlocking {
        val out = RoaxRpc.endpointRoute(
            custom,
            expectedChainId,
        ) { _, _ -> Http.Response(503, "") }

        assertEquals(
            RoaxRpc.EndpointRoute.Unavailable(
                customFailure = RoaxRpc.EndpointFailure.Unavailable,
                bundledFailure = RoaxRpc.EndpointFailure.Unavailable,
            ),
            out,
        )
        assertNull(out.rpcUrl)
    }

    @Test
    fun wrongChainCustomPeerReceivesNoContractCall() = runBlocking {
        val calls = mutableListOf<Pair<String, String>>()
        val contractBody =
            """{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":"0xabc"},"latest"]}"""

        val response = RoaxRpc.guardedPostJson(
            custom,
            expectedChainId,
            contractBody,
        ) { url, body ->
            val method = JSONObject(body).getString("method")
            calls += url to method
            when {
                method == "eth_chainId" && url == custom -> chainResponse(1L)
                method == "eth_chainId" && url == RoaxRpc.DEFAULT_RPC ->
                    chainResponse(expectedChainId)
                method == "eth_call" && url == RoaxRpc.DEFAULT_RPC ->
                    Http.Response(200, """{"jsonrpc":"2.0","id":1,"result":"0x01"}""")
                else -> error("unguarded or wrong-chain contract call: $url $method")
            }
        }

        assertEquals(200, response.code)
        assertEquals(
            listOf(
                custom to "eth_chainId",
                RoaxRpc.DEFAULT_RPC to "eth_chainId",
                RoaxRpc.DEFAULT_RPC to "eth_call",
            ),
            calls,
        )
    }

    @Test
    fun noContractCallWhenBundledFallbackAlsoFailsGuard() = runBlocking {
        val methods = mutableListOf<String>()
        val response = RoaxRpc.guardedPostJson(
            custom,
            expectedChainId,
            """{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}""",
        ) { _, body ->
            methods += JSONObject(body).getString("method")
            Http.Response(503, "")
        }

        assertEquals(-1, response.code)
        assertEquals(listOf("eth_chainId", "eth_chainId"), methods)
    }

    @Test
    fun customReadFailureGetsAGuardedBundledRetry() = runBlocking {
        val calls = mutableListOf<Pair<String, String>>()
        val contractBody =
            """{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":"0xabc"},"latest"]}"""
        var customReadAttempted = false

        val response = RoaxRpc.guardedPostJson(
            custom,
            expectedChainId,
            contractBody,
        ) { url, body ->
            val method = JSONObject(body).getString("method")
            calls += url to method
            when {
                method == "eth_chainId" -> chainResponse(expectedChainId)
                url == custom && !customReadAttempted -> {
                    customReadAttempted = true
                    Http.Response(503, "")
                }
                url == RoaxRpc.DEFAULT_RPC ->
                    Http.Response(200, """{"jsonrpc":"2.0","id":1,"result":"0x01"}""")
                else -> error("unexpected request: $url $method")
            }
        }

        assertEquals(200, response.code)
        assertEquals(
            listOf(
                custom to "eth_chainId",
                custom to "eth_call",
                RoaxRpc.DEFAULT_RPC to "eth_chainId",
                RoaxRpc.DEFAULT_RPC to "eth_call",
            ),
            calls,
        )
    }

    @Test
    fun customReadExceptionGetsAGuardedBundledRetry() = runBlocking {
        val calls = mutableListOf<Pair<String, String>>()
        val contractBody =
            """{"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":"0xabc"},"latest"]}"""

        val response = RoaxRpc.guardedPostJson(
            custom,
            expectedChainId,
            contractBody,
        ) { url, body ->
            val method = JSONObject(body).getString("method")
            calls += url to method
            when {
                method == "eth_chainId" -> chainResponse(expectedChainId)
                url == custom -> throw IOException("connection closed")
                url == RoaxRpc.DEFAULT_RPC ->
                    Http.Response(200, """{"jsonrpc":"2.0","id":1,"result":"0x01"}""")
                else -> error("unexpected request: $url $method")
            }
        }

        assertEquals(200, response.code)
        assertEquals(
            listOf(
                custom to "eth_chainId",
                custom to "eth_call",
                RoaxRpc.DEFAULT_RPC to "eth_chainId",
                RoaxRpc.DEFAULT_RPC to "eth_call",
            ),
            calls,
        )
    }

    @Test
    fun bundledReadExceptionReturnsSyntheticFailure() = runBlocking {
        val response = RoaxRpc.guardedPostJson(
            RoaxRpc.DEFAULT_RPC,
            expectedChainId,
            """{"jsonrpc":"2.0","id":1,"method":"eth_blockNumber","params":[]}""",
        ) { _, body ->
            if (JSONObject(body).getString("method") == "eth_chainId") {
                chainResponse(expectedChainId)
            } else {
                throw IOException("connection closed")
            }
        }

        assertEquals(-1, response.code)
    }

    @Test
    fun parsesCanonicalChainIdQuantityAndRejectsMalformedValues() {
        assertEquals(expectedChainId, RoaxRpc.parseChainId("0x87"))
        assertEquals(expectedChainId, RoaxRpc.parseChainId("0X87"))
        assertNull(RoaxRpc.parseChainId(""))
        assertNull(RoaxRpc.parseChainId("135"))
        assertNull(RoaxRpc.parseChainId("0xzz"))
        assertNull(RoaxRpc.parseChainId("0x8000000000000000"))
    }
}
