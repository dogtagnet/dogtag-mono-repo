import XCTest

/// Pins the version-keyed artifact resolver's contract. The Kotlin mirror of these assertions lives
/// in `apps/android/.../data/ZkeyAssetTest.kt`, and the Rust one in
/// `crates/dogtag-prover-rs/src/artifact.rs` — the three declare one protocol constant independently
/// and are hand-synced, so a drift is a runtime rejection, not a compile error.
///
/// These are pure resolution tests: they never call `ensure`/`ensureGraph` (the only members that
/// touch `Bundle.main`), so the host-less test bundle can run them.
final class ZkeyAssetTests: XCTestCase {

    /// Back-compat: every caller on this app names no version, so the default MUST stay Level-A.
    func testResolveWithNoVersionIsTheLevelASet() {
        XCTAssertEqual(ZkeyAsset.resolve()?.version, ZkeyAsset.levelAV1.version)
        XCTAssertEqual(ZkeyAsset.current().version, ZkeyAsset.levelAV1.version)
    }

    /// The Level-A descriptor names the bundled assets. If this drifts, the app resolves a resource
    /// the bundle does not ship and proving dies at runtime.
    func testLevelADescriptorNamesTheBundledAssets() {
        let d = ZkeyAsset.current()
        XCTAssertEqual(d.version, "dogtag-levela/1")
        XCTAssertEqual(d.zkey.name, "verification_final")
        XCTAssertEqual(d.zkey.ext, "zkey")
        XCTAssertEqual(d.graph.name, "verification")
        XCTAssertEqual(d.graph.ext, "graph")
    }

    /// M-4: Level-B is REGISTERED and names the consent artifacts the mobile workflow vendors. The
    /// version key and filenames must match the Rust `LEVEL_B_V1_DESCRIPTOR` and Android
    /// `ZkeyAsset.LEVEL_B_V1`.
    func testLevelBDescriptorNamesTheBundledAssets() {
        let d = ZkeyAsset.resolve(version: "dogtag-levelb/1")
        XCTAssertNotNil(d, "Level-B must resolve after M-4 registers it")
        XCTAssertEqual(d?.version, "dogtag-levelb/1")
        XCTAssertEqual(d?.zkey.name, "consent_final")
        XCTAssertEqual(d?.zkey.ext, "zkey")
        XCTAssertEqual(d?.graph.name, "consent")
        XCTAssertEqual(d?.graph.ext, "graph")
    }

    /// Registering Level-B must NOT change the default — the "available, not default" guard for the
    /// artifact resolver, mirroring the server's convenience-tier guard and the Android test.
    func testRegisteringLevelBDoesNotChangeTheDefault() {
        XCTAssertEqual(ZkeyAsset.current().version, "dogtag-levela/1")
        XCTAssertEqual(ZkeyAsset.resolve()?.version, "dogtag-levela/1")
    }

    /// An unknown version fails closed — it must NOT fall back to the current artifact set. A proof
    /// built with the wrong key is rejected by that version's verifier, so a fallback would turn a
    /// clear error into a confusing one. Probes `dogtag-levelc/9` (genuinely unregistered), matching
    /// the Rust `resolve_unknown_version_fails_closed`.
    func testResolveUnknownVersionFailsClosed() {
        XCTAssertNil(ZkeyAsset.resolve(version: "dogtag-levelc/9"))
    }

    /// Every registered version is resolvable by its own key, and keys are unique.
    func testRegistryEntriesAreUniqueAndResolvable() {
        let keys = ZkeyAsset.registry.map { $0.version }
        XCTAssertEqual(keys.count, Set(keys).count, "duplicate version key")
        for d in ZkeyAsset.registry {
            XCTAssertEqual(ZkeyAsset.resolve(version: d.version)?.version, d.version)
        }
    }
}
