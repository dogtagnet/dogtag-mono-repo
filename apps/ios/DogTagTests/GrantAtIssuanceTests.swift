import XCTest

/// DELISTING IS FORWARD-ONLY, at the fold the mobile issuer-whitelist pillar rests on.
///
/// `DogTagIssuer.sol:82` states the rule in the contract's own source and `adminRevoke` is the
/// retroactive lever, so a credential anchored while its signer held the grant stays genuine when that
/// grant is later withdrawn - an ordinary key rotation, a retirement, a lapsed practice licence. Before
/// this rule moved, the pillar read `IssuerRegistry.isWhitelistedFor`, a CURRENT-state getter, so any
/// such withdrawal retroactively rendered every credential that signer had ever issued a forgery.
///
/// Mirrors Android `GrantInForceAtTest` case for case, and narrows the asymmetry CLAUDE.md records
/// under "Mobile `eth_call` selectors": Android pins ten of its derived values and iOS pinned NONE,
/// because `functionSelector` and every selector are `private static` and unreachable from this
/// host-less bundle. `eventTopic` and the fold are deliberately internal instead, so the values whose
/// silent failure mode is worst - a topic derived at the wrong WIDTH matches no log at all, which is
/// indistinguishable from "never granted" - are checked rather than argued about.
final class GrantAtIssuanceTests: XCTestCase {

    private let anchored = RoaxRpc.LogPoint(blockNumber: 200, logIndex: 3)
    private func granted(_ block: UInt64) -> RoaxRpc.GrantEvent {
        RoaxRpc.GrantEvent(at: RoaxRpc.LogPoint(blockNumber: block, logIndex: 0), granted: true)
    }
    private func delisted(_ block: UInt64) -> RoaxRpc.GrantEvent {
        RoaxRpc.GrantEvent(at: RoaxRpc.LogPoint(blockNumber: block, logIndex: 0), granted: false)
    }

    func test_aSignerDelistedAfterTheAnchoringWasStillAuthorisedWhenItActed() {
        XCTAssertEqual(
            RoaxRpc.grantInForceAt([granted(100), delisted(700)], anchoredAt: anchored), .authorized)
    }

    func test_aSignerDelistedBeforeTheAnchoringWasNot() {
        XCTAssertEqual(
            RoaxRpc.grantInForceAt([granted(100), delisted(199)], anchoredAt: anchored), .notAuthorized)
    }

    /// The mirror of the forward-only rule: a later grant cannot authorise an earlier anchoring.
    func test_aGrantIssuedAfterTheAnchoringDoesNotAuthoriseItRetroactively() {
        XCTAssertEqual(RoaxRpc.grantInForceAt([granted(201)], anchoredAt: anchored), .notAuthorized)
    }

    /// An empty history is an ANSWER - the registry recorded no grant - not an absence of one. The
    /// could-not-read case never reaches this function; the caller returns `.undetermined` for it.
    func test_anEmptyHistoryIsADefiniteRefusalNotAnUndeterminedOne() {
        XCTAssertEqual(RoaxRpc.grantInForceAt([], anchoredAt: anchored), .notAuthorized)
    }

    /// `logIndex` is block-scoped and therefore comparable ACROSS contracts within one block, which is
    /// the only reason a registry grant and a clone's issuance landing in the same block can be
    /// sequenced at all. Inclusive at the anchoring point.
    func test_grantsAndAnchoringsInOneBlockAreSequencedByLogIndex() {
        func at(_ logIndex: UInt64, _ granted: Bool) -> RoaxRpc.GrantEvent {
            RoaxRpc.GrantEvent(
                at: RoaxRpc.LogPoint(blockNumber: anchored.blockNumber, logIndex: logIndex),
                granted: granted)
        }
        XCTAssertEqual(
            RoaxRpc.grantInForceAt([at(anchored.logIndex, true)], anchoredAt: anchored), .authorized)
        XCTAssertEqual(
            RoaxRpc.grantInForceAt([at(0, true), at(anchored.logIndex + 1, false)], anchoredAt: anchored),
            .authorized)
        XCTAssertEqual(
            RoaxRpc.grantInForceAt([at(0, true), at(anchored.logIndex - 1, false)], anchoredAt: anchored),
            .notAuthorized)
    }

    /// The fold takes the LAST event at or before the anchoring, whatever order it arrives in.
    func test_theAnswerIsTheLastEventAtOrBeforeTheAnchoringRegardlessOfInputOrder() {
        let events = [delisted(199), granted(100), granted(150)]
        XCTAssertEqual(RoaxRpc.grantInForceAt(events, anchoredAt: anchored), .notAuthorized)
        XCTAssertEqual(RoaxRpc.grantInForceAt(events.reversed(), anchoredAt: anchored), .notAuthorized)
    }

    // MARK: - the derived topic values

    /// Independently confirmed with `cast keccak`, and byte-identical to the values Android's
    /// `RoaxRpcSelectorTest` pins - the two platforms must filter the same logs.
    func test_theGrantHistoryTopicsMatchTheirCanonicalEventSignatures() {
        XCTAssertEqual(
            RoaxRpc.eventTopic("RootIssued(bytes32,address,uint256)"),
            "0xf8cd30a628b432a1200caf81085096c82a5f570da14360572b72d4e0ba57e6d7")
        XCTAssertEqual(
            RoaxRpc.eventTopic("Whitelisted(bytes32,address)"),
            "0x0ed68b47399672cf072b19a599fa9f99cdc79a286bf59bc301ca44b94f589bce")
        XCTAssertEqual(
            RoaxRpc.eventTopic("Delisted(bytes32,address)"),
            "0xf3af84db5dbf726f68c33f3ded733403e15667370ab38e8cb37fdc874835b00e")
    }

    /// A topic is 32 bytes and a selector is 4. Stated as its own assertion because the failure mode
    /// of confusing them is silent: the shorter value simply matches nothing.
    func test_anEventTopicIsTheWholeHashNotTheFourByteSelector() {
        let topic = RoaxRpc.eventTopic("Whitelisted(bytes32,address)")
        XCTAssertEqual(topic.count, 66)
        // The registry selector this file can reach IS the 4-byte prefix of `registry()`'s hash, so
        // the two derivations are related exactly as expected and neither is the other.
        XCTAssertEqual(RoaxRpc.registrySelector.count, 10)
        XCTAssertEqual(RoaxRpc.registrySelector, String(RoaxRpc.eventTopic("registry()").prefix(10)))
        XCTAssertNotEqual(topic, RoaxRpc.registrySelector)
    }
}
