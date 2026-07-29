import XCTest

private actor RpcTransportLog {
    struct Entry: Equatable {
        let url: String
        let body: String
    }

    private var probeEntries: [Entry] = []
    private var requestEntries: [Entry] = []

    func recordProbe(url: String, body: String) {
        probeEntries.append(Entry(url: url, body: body))
    }

    func recordRequest(url: String, body: String) {
        requestEntries.append(Entry(url: url, body: body))
    }

    func probes() -> [Entry] { probeEntries }
    func requests() -> [Entry] { requestEntries }
}

/// The user-selected CHAIN endpoint and its fail-closed chain guard. These tests are host-safe:
/// URLSession is never touched; the injected probe supplies exact JSON-RPC envelopes.
final class RpcEndpointSettingsTests: XCTestCase {
    private let custom = "https://rpc.example/roax?token=keep-me"
    private let chainId = 135

    private func chainResponse(_ value: String, code: Int = 200) -> Http.Response {
        Http.Response(code: code, body: #"{"jsonrpc":"2.0","id":1,"result":"\#(value)"}"#)
    }

    private func defaults() -> (UserDefaults, String) {
        let name = "RpcEndpointSettingsTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defaults.removePersistentDomain(forName: name)
        return (defaults, name)
    }

    func testPersistenceKeepsACompleteCustomURLIncludingPathAndQuery() {
        let (defaults, name) = defaults()
        defer { defaults.removePersistentDomain(forName: name) }

        XCTAssertTrue(RpcEndpointSettings.saveChecked(
            "  \(custom) \n", route: .custom(custom), in: defaults))
        XCTAssertEqual(RpcEndpointSettings.customRpc(in: defaults), custom)
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), custom)

        RpcEndpointSettings.useBundled(in: defaults)
        XCTAssertNil(RpcEndpointSettings.customRpc(in: defaults))
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), AppConfig.roaxRpc)
    }

    func testInvalidURLClearsThePreviousChoiceAndFailsClosedToBundled() {
        let (defaults, name) = defaults()
        defer { defaults.removePersistentDomain(forName: name) }

        XCTAssertTrue(RpcEndpointSettings.saveChecked(
            custom, route: .custom(custom), in: defaults))
        XCTAssertFalse(RpcEndpointSettings.saveChecked(
            "file:///tmp/not-json-rpc", route: .custom(custom), in: defaults))
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), AppConfig.roaxRpc)

        XCTAssertTrue(RpcEndpointSettings.saveChecked(
            custom, route: .custom(custom), in: defaults))
        XCTAssertFalse(RpcEndpointSettings.saveChecked(
            "rpc.example/no-scheme", route: .custom(custom), in: defaults))
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), AppConfig.roaxRpc)

        XCTAssertTrue(RpcEndpointSettings.saveChecked(
            custom, route: .custom(custom), in: defaults))
        XCTAssertFalse(RpcEndpointSettings.saveChecked(
            "https://rpc.example/#not-sent", route: .custom(custom), in: defaults))
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), AppConfig.roaxRpc)
    }

    func testRejectedChainCheckClearsAnOlderCustomChoice() {
        let (defaults, name) = defaults()
        defer { defaults.removePersistentDomain(forName: name) }

        XCTAssertTrue(RpcEndpointSettings.saveChecked(
            custom, route: .custom(custom), in: defaults))
        XCTAssertFalse(RpcEndpointSettings.saveChecked(
            custom,
            route: .bundledFallback(.wrongChain(reported: 1)),
            in: defaults
        ))
        XCTAssertNil(RpcEndpointSettings.customRpc(in: defaults))
        XCTAssertEqual(RpcEndpointSettings.rpcUrl(in: defaults), AppConfig.roaxRpc)
    }

    func testRequestGenerationRejectsAnOlderSaveAndInvalidatesOnDisappear() {
        var generation = RpcEndpointRequestGeneration()

        let firstSave = generation.begin()
        XCTAssertTrue(generation.accepts(firstSave))

        let laterSelection = generation.begin()
        XCTAssertFalse(
            generation.accepts(firstSave),
            "an older Save & check completion must not overwrite a later selection"
        )
        XCTAssertTrue(generation.accepts(laterSelection))

        generation.invalidate()
        XCTAssertFalse(
            generation.accepts(laterSelection),
            "leaving Profile must invalidate an in-flight completion even if transport cancellation is late"
        )
    }

    func testMatchingCustomEndpointIsSelectedUsingEthChainId() async {
        let route = await RoaxRpc.endpointRoute(
            rpcUrl: custom,
            expectedChainId: chainId,
            probe: { _, body in
                guard body.contains(#""method":"eth_chainId""#) else {
                    return Http.Response(code: 200, body: "{}")
                }
                return self.chainResponse("0x87")
            }
        )

        XCTAssertEqual(route, .custom(custom))
        XCTAssertEqual(route.rpcUrl, custom)
    }

    func testDifferentChainFallsBackToGuardedBundledEndpoint() async {
        let route = await RoaxRpc.endpointRoute(
            rpcUrl: custom,
            expectedChainId: chainId,
            probe: { url, _ in
                url == self.custom
                    ? self.chainResponse("0x1")
                    : self.chainResponse("0x87")
            }
        )

        XCTAssertEqual(route, .bundledFallback(.wrongChain(reported: 1)))
        XCTAssertEqual(route.rpcUrl, AppConfig.roaxRpc)
    }

    func testUnreachableOrMalformedCustomEndpointFallsBackToBundledEndpoint() async {
        let unreachable = await RoaxRpc.endpointRoute(
            rpcUrl: custom,
            expectedChainId: chainId,
            probe: { url, _ in
                url == self.custom
                    ? Http.Response(code: -1, body: "offline")
                    : self.chainResponse("0x87")
            }
        )
        XCTAssertEqual(unreachable, .bundledFallback(.unavailable))

        let malformed = await RoaxRpc.endpointRoute(
            rpcUrl: custom,
            expectedChainId: chainId,
            probe: { url, _ in
                url == self.custom
                    ? Http.Response(code: 200, body: #"{"result":"135"}"#)
                    : self.chainResponse("0x87")
            }
        )
        XCTAssertEqual(malformed, .bundledFallback(.invalidChainIdResponse))
    }

    func testBundledEndpointMustAlsoPassTheChainGuard() async {
        let route = await RoaxRpc.endpointRoute(
            rpcUrl: AppConfig.roaxRpc,
            expectedChainId: chainId,
            probe: { _, _ in self.chainResponse("0x1") }
        )

        XCTAssertEqual(
            route,
            .unavailable(custom: nil, bundled: .wrongChain(reported: 1))
        )
        XCTAssertNil(route.rpcUrl, "a wrong-chain bundled peer must not receive a contract read")
    }

    func testNoReadRouteWhenCustomAndBundledPeersBothFail() async {
        let route = await RoaxRpc.endpointRoute(
            rpcUrl: custom,
            expectedChainId: chainId,
            probe: { url, _ in
                url == self.custom
                    ? self.chainResponse("0xaa36a7")
                    : Http.Response(code: -1, body: "offline")
            }
        )

        XCTAssertEqual(
            route,
            .unavailable(custom: .wrongChain(reported: 11_155_111), bundled: .unavailable)
        )
        XCTAssertNil(route.rpcUrl)
    }

    func testWrongChainCustomGetsNoContractCallAndGuardedBundledDoes() async {
        let log = RpcTransportLog()
        let contractBody = #"{"jsonrpc":"2.0","id":7,"method":"eth_call","params":[]}"#

        let response = await RoaxRpc.guardedPostJSON(
            rpcUrl: custom,
            body: contractBody,
            expectedChainId: chainId,
            probe: { url, body in
                await log.recordProbe(url: url, body: body)
                return url == self.custom
                    ? self.chainResponse("0x1")
                    : self.chainResponse("0x87")
            },
            request: { url, body in
                await log.recordRequest(url: url, body: body)
                return Http.Response(code: 200, body: #"{"result":"0x01"}"#)
            }
        )

        let probes = await log.probes()
        let requests = await log.requests()
        XCTAssertTrue(response.ok)
        XCTAssertEqual(probes.map(\.url), [custom, AppConfig.roaxRpc])
        XCTAssertTrue(probes.allSatisfy { $0.body.contains(#""method":"eth_chainId""#) })
        XCTAssertEqual(
            requests,
            [.init(url: AppConfig.roaxRpc, body: contractBody)],
            "a wrong-chain custom peer must see only eth_chainId, never the address-bound request"
        )
    }

    func testNoContractCallWhenNeitherEndpointPassesTheGuard() async {
        let log = RpcTransportLog()
        let contractBody = #"{"jsonrpc":"2.0","id":8,"method":"eth_call","params":[]}"#

        let response = await RoaxRpc.guardedPostJSON(
            rpcUrl: custom,
            body: contractBody,
            expectedChainId: chainId,
            probe: { url, body in
                await log.recordProbe(url: url, body: body)
                return url == self.custom
                    ? self.chainResponse("0x1")
                    : Http.Response(code: -1, body: "offline")
            },
            request: { url, body in
                await log.recordRequest(url: url, body: body)
                return Http.Response(code: 200, body: #"{"result":"must-not-run"}"#)
            }
        )

        let probes = await log.probes()
        let requests = await log.requests()
        XCTAssertFalse(response.ok)
        XCTAssertEqual(probes.map(\.url), [custom, AppConfig.roaxRpc])
        XCTAssertTrue(requests.isEmpty, "no address-bound call is safe when neither peer establishes chain identity")
    }

    func testCustomReadTransportFailureGetsIndependentlyGuardedBundledRetry() async {
        let log = RpcTransportLog()
        let contractBody = #"{"jsonrpc":"2.0","id":9,"method":"eth_call","params":[]}"#

        let response = await RoaxRpc.guardedPostJSON(
            rpcUrl: custom,
            body: contractBody,
            expectedChainId: chainId,
            probe: { url, body in
                await log.recordProbe(url: url, body: body)
                return self.chainResponse("0x87")
            },
            request: { url, body in
                await log.recordRequest(url: url, body: body)
                return url == self.custom
                    ? Http.Response(code: -1, body: "transport failed")
                    : Http.Response(code: 200, body: #"{"result":"0x01"}"#)
            }
        )

        let probes = await log.probes()
        let requests = await log.requests()
        XCTAssertTrue(response.ok)
        XCTAssertEqual(
            probes.map(\.url),
            [custom, AppConfig.roaxRpc],
            "the fallback must receive its own eth_chainId probe after the custom read fails"
        )
        XCTAssertEqual(
            requests,
            [
                .init(url: custom, body: contractBody),
                .init(url: AppConfig.roaxRpc, body: contractBody),
            ]
        )
    }
}
