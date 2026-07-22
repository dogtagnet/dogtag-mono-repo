import XCTest

/// Pure tests for the single bundled consent artifact descriptor.
final class ZkeyAssetTests: XCTestCase {
    func testCurrentDescriptorIsOwnerHiddenConsent() {
        let descriptor = ZkeyAsset.current()
        XCTAssertEqual(descriptor.version, "dogtag-levelb/1")
        XCTAssertEqual(descriptor.zkey.name, "consent_final")
        XCTAssertEqual(descriptor.zkey.ext, "zkey")
        XCTAssertEqual(descriptor.graph.name, "consent")
        XCTAssertEqual(descriptor.graph.ext, "graph")
        XCTAssertEqual(ZkeyAsset.resolve()?.version, descriptor.version)
    }

    func testRegistryContainsOnlyTheConsentDescriptor() {
        XCTAssertEqual(ZkeyAsset.registry.count, 1)
        XCTAssertEqual(ZkeyAsset.resolve(version: "dogtag-levelb/1")?.version, "dogtag-levelb/1")
        XCTAssertNil(ZkeyAsset.resolve(version: "dogtag-levela/1"))
        XCTAssertNil(ZkeyAsset.resolve(version: "dogtag-levelc/9"))
    }
}
