import XCTest

/// Pins the public-signal index constants. These are plain integers, so nothing else in the build
/// catches a wrong one: a Level-A/Level-B mix-up reads a real field element from the wrong slot and
/// fails silently, far from the mistake. The Rust mirror of these assertions lives in
/// `crates/dogtag-standard-rs/src/public_signals.rs`.
final class PublicSignalIndexTests: XCTestCase {

    /// Guards the Level-B constants against accidental drift. The values were transcribed from
    /// `VerificationRegistryConsent.sol:81-87`'s `P_*` constants, which remain the authority - but
    /// this asserts literals and never reads the Solidity, so a CONTRACT-side change would not fail
    /// it. If the circuit's output order ever changes, contract and app have to be moved together by
    /// hand, or on-chain and off-chain silently disagree about what they are comparing.
    func testLevelBMatchesTheOnChainConstants() {
        XCTAssertEqual(PublicSignalIndex.levelB.dogTagId, 0, "P_DOGTAGID")
        XCTAssertEqual(PublicSignalIndex.levelB.purpose, 1, "P_PURPOSE")
        XCTAssertEqual(PublicSignalIndex.levelB.relayer, 2, "P_RELAYER")
        XCTAssertEqual(PublicSignalIndex.levelB.nullifier, 3, "P_NULLIFIER")
        XCTAssertEqual(PublicSignalIndex.levelB.root, 4, "P_ROOT")
        XCTAssertEqual(PublicSignalIndex.levelB.recordType, 5, "P_RECORDTYPE")
        XCTAssertEqual(PublicSignalIndex.levelB.deadline, 6, "P_DEADLINE")
    }

    /// Level-A is what this app actually produces and consumes today.
    func testLevelAMatchesTheShippedProverOutput() {
        XCTAssertEqual(PublicSignalIndex.levelA.dogTagId, 0)
        XCTAssertEqual(PublicSignalIndex.levelA.purpose, 1)
        XCTAssertEqual(PublicSignalIndex.levelA.relayer, 2)
        XCTAssertEqual(PublicSignalIndex.levelA.subject, 3)
        XCTAssertEqual(PublicSignalIndex.levelA.nullifier, 4)
        XCTAssertEqual(PublicSignalIndex.levelA.keyHash, 5)
        XCTAssertEqual(PublicSignalIndex.levelA.root, 6)
    }

    /// The drift that motivated these constants: the orders agree on the first three signals and
    /// diverge from index 3 on. Level-A's NULLIFIER slot is Level-B's ROOT slot - reading one as the
    /// other is exactly the bug that makes a successful verification hang the phone.
    func testTheTwoOrdersDivergeExactlyFromIndexThree() {
        XCTAssertEqual(PublicSignalIndex.levelA.dogTagId, PublicSignalIndex.levelB.dogTagId)
        XCTAssertEqual(PublicSignalIndex.levelA.purpose, PublicSignalIndex.levelB.purpose)
        XCTAssertEqual(PublicSignalIndex.levelA.relayer, PublicSignalIndex.levelB.relayer)
        XCTAssertNotEqual(PublicSignalIndex.levelA.nullifier, PublicSignalIndex.levelB.nullifier)
        XCTAssertEqual(PublicSignalIndex.levelA.nullifier, PublicSignalIndex.levelB.root)
        XCTAssertNotEqual(PublicSignalIndex.levelA.root, PublicSignalIndex.levelB.root)
    }

    /// Both circuits emit the same WIDTH, which is why a length check can never catch an order mix-up.
    func testBothOrdersAreSevenWide() {
        XCTAssertEqual(PublicSignalIndex.count, 7)
        XCTAssertEqual(PublicSignalIndex.levelA.root, PublicSignalIndex.count - 1)
        XCTAssertEqual(PublicSignalIndex.levelB.deadline, PublicSignalIndex.count - 1)
    }
}
