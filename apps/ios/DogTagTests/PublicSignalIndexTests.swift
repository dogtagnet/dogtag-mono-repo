import XCTest

/// Pins the frozen consent-circuit order mirrored by VerificationRegistryConsent.
final class PublicSignalIndexTests: XCTestCase {
    func testOwnerHiddenSignalsMatchOnChainConstants() {
        XCTAssertEqual(PublicSignalIndex.ownerHidden.dogTagId, 0, "P_DOGTAGID")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.purpose, 1, "P_PURPOSE")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.relayer, 2, "P_RELAYER")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.nullifier, 3, "P_NULLIFIER")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.root, 4, "P_ROOT")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.recordType, 5, "P_RECORDTYPE")
        XCTAssertEqual(PublicSignalIndex.ownerHidden.deadline, 6, "P_DEADLINE")
        XCTAssertEqual(PublicSignalIndex.count, 7)
    }
}
