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
        // The authority's rights-grant topic. Pinned because the failure mode is SILENT: a value
        // derived at the wrong width, or from a drifted signature, matches no log at all - which
        // reads exactly like "this signer was never granted" and refuses every genuine credential.
        XCTAssertEqual(
            RoaxRpc.rightsSetTopic,
            "0xbc9c679fe541a4f3fcf5f2887c4adcd6e7703f7ea9d0933b8862662f8290af7f")
        XCTAssertEqual(
            RoaxRpc.eventTopic("RightsSet(address,uint256)"),
            RoaxRpc.rightsSetTopic)
    }

    // MARK: - reading RIGHT_ISSUE out of the rights mask

    /// The decoder reads a BIT, not the whole word.
    ///
    /// Bit 0 is the only settable right today, so "the word equals 1" and "bit 0 is set" agree on
    /// every mask the contract can currently emit - which is exactly what would let a whole-word
    /// comparison survive review until a second right is allocated. These cases carry masks with
    /// higher bits set, which no honest authority emits YET, precisely so the decoder is pinned
    /// against the day one does.
    func test_theIssueRightIsReadAsABitNotAsTheWholeWord() {
        func word(_ hex: String) -> [String: Any] {
            ["data": "0x" + String(repeating: "0", count: 64 - hex.count) + hex]
        }
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("1")), true)
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("0")), false)
        // A future second right held ALONGSIDE the issue right: still granted.
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("3")), true)
        // A future second right held WITHOUT it: not granted, and emphatically not malformed.
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("2")), false)
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("f")), true)
        XCTAssertEqual(RoaxRpc.issueRightFromLogData(word("e")), false)
    }

    /// A body that is not exactly one 32-byte hex word is a log this build does not understand, and
    /// answering either way would state a grant or a withdrawal that was never recorded.
    func test_aMalformedRightsBodyIsUndecodableRatherThanFalse() {
        XCTAssertNil(RoaxRpc.issueRightFromLogData(["data": "0x"]))
        XCTAssertNil(RoaxRpc.issueRightFromLogData(["data": "0x01"]))
        XCTAssertNil(RoaxRpc.issueRightFromLogData([:]))
        XCTAssertNil(RoaxRpc.issueRightFromLogData(["data": "0x" + String(repeating: "z", count: 64)]))
        // Two words - an event with a widened body is not this one.
        XCTAssertNil(RoaxRpc.issueRightFromLogData(["data": "0x" + String(repeating: "0", count: 128)]))
    }

    /// A topic is 32 bytes and a selector is 4. Stated as its own assertion because the failure mode
    /// of confusing them is silent: the shorter value simply matches nothing.
    func test_anEventTopicIsTheWholeHashNotTheFourByteSelector() {
        let topic = RoaxRpc.rightsSetTopic
        XCTAssertEqual(topic.count, 66)
        // The registry selector this file can reach IS the 4-byte prefix of `registry()`'s hash, so
        // the two derivations are related exactly as expected and neither is the other.
        XCTAssertEqual(RoaxRpc.registrySelector.count, 10)
        XCTAssertEqual(RoaxRpc.registrySelector, String(RoaxRpc.eventTopic("registry()").prefix(10)))
        XCTAssertNotEqual(topic, RoaxRpc.registrySelector)
    }
}
