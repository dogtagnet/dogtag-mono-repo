import XCTest

/// Covers the sweep behind every "delete my …" action. The interesting cases are not "removeItem
/// works" but the ones a reset gets wrong quietly: the staging siblings that hold the same secrets as
/// the canonical file, idempotence, and honest reporting when something survives.
final class LocalDataSweepTests: XCTestCase {
    private var dir: URL!

    override func setUpWithError() throws {
        try super.setUpWithError()
        dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("LocalDataSweepTests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dir)
        try super.tearDownWithError()
    }

    private func write(_ name: String) throws {
        try Data("secret".utf8).write(to: dir.appendingPathComponent(name))
    }

    private func exists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: dir.appendingPathComponent(name).path)
    }

    func testRemovesNamedFile() throws {
        try write("pets.json")
        let outcome = LocalDataSweep.remove(from: dir, files: ["pets.json"])
        XCTAssertTrue(outcome.isComplete)
        XCTAssertEqual(outcome.removed, ["pets.json"])
        XCTAssertFalse(exists("pets.json"))
    }

    /// The one that matters for secrets: `ProfileTreeStore.write` stages through
    /// `.<name>.<token>.tmp` / `.bak` siblings holding the same owner-secret material, and a crash can
    /// leave one behind. Removing only the canonical file would leave the secret readable on disk.
    func testRemovesStagingSiblingsOfSweptFiles() throws {
        let file = "dogtag-owner-secrets.json"
        try write(file)
        try write(".\(file).ABC123.tmp")
        try write(".\(file).ABC123.bak")

        let outcome = LocalDataSweep.remove(from: dir, files: [file])

        XCTAssertTrue(outcome.isComplete)
        XCTAssertFalse(exists(file))
        XCTAssertFalse(exists(".\(file).ABC123.tmp"))
        XCTAssertFalse(exists(".\(file).ABC123.bak"))
    }

    func testLeavesUnrelatedAndSimilarlyNamedFilesAlone() throws {
        try write("dogtag-owner-secrets.json")
        try write("credentials.json")
        // Shares a prefix but is NOT a staging sibling: the sibling pattern requires the leading dot.
        try write("dogtag-owner-secrets.json.old")

        let outcome = LocalDataSweep.remove(from: dir, files: ["dogtag-owner-secrets.json"])

        XCTAssertTrue(outcome.isComplete)
        XCTAssertFalse(exists("dogtag-owner-secrets.json"))
        XCTAssertTrue(exists("credentials.json"))
        XCTAssertTrue(exists("dogtag-owner-secrets.json.old"))
    }

    func testRemovesDirectoryRecursively() throws {
        let photos = dir.appendingPathComponent("pet-photos", isDirectory: true)
        try FileManager.default.createDirectory(at: photos, withIntermediateDirectories: true)
        try Data("jpeg".utf8).write(to: photos.appendingPathComponent("42.jpg"))

        let outcome = LocalDataSweep.remove(from: dir, directories: ["pet-photos"])

        XCTAssertTrue(outcome.isComplete)
        XCTAssertEqual(outcome.removed, ["pet-photos"])
        XCTAssertFalse(exists("pet-photos"))
    }

    /// A reset must be re-runnable: the second pass over an already-clean directory succeeds and
    /// reports nothing removed, rather than failing on the absent files.
    func testIsIdempotentAndAbsentFilesAreNotFailures() throws {
        try write("pets.json")
        XCTAssertTrue(LocalDataSweep.remove(from: dir, files: ["pets.json"]).isComplete)

        let second = LocalDataSweep.remove(from: dir, files: ["pets.json"], directories: ["pet-photos"])

        XCTAssertTrue(second.isComplete)
        XCTAssertTrue(second.removed.isEmpty)
    }

    func testMissingDirectoryYieldsNoFailures() {
        let absent = dir.appendingPathComponent("does-not-exist", isDirectory: true)
        let outcome = LocalDataSweep.remove(from: absent, files: ["pets.json"])
        XCTAssertTrue(outcome.isComplete)
        XCTAssertTrue(outcome.removed.isEmpty)
    }

    func testStagingSiblingsRequireANamedFileToMatchAgainst() throws {
        try write(".dogtag-owner-secrets.json.ABC.tmp")
        XCTAssertTrue(LocalDataSweep.stagingSiblings(in: dir, of: []).isEmpty)
        XCTAssertEqual(
            LocalDataSweep.stagingSiblings(in: dir, of: ["dogtag-owner-secrets.json"]),
            [".dogtag-owner-secrets.json.ABC.tmp"])
    }

    /// Merging is what lets `AppReset` combine several sweeps into one verdict; a failure anywhere must
    /// dominate, so the UI cannot report a partial reset as done.
    func testMergePropagatesFailuresAndRemovals() {
        var a = LocalDataSweep.Outcome(removed: ["pets.json"])
        let b = LocalDataSweep.Outcome(
            removed: ["credentials.json"],
            failures: [.init(name: "pet-photos", error: CocoaError(.fileWriteNoPermission))])

        a.merge(b)

        XCTAssertEqual(a.removed, ["pets.json", "credentials.json"])
        XCTAssertFalse(a.isComplete)
        XCTAssertTrue(a.failureSummary?.contains("pet-photos") == true)
    }

    func testFailureSummaryIsNilWhenComplete() {
        XCTAssertNil(LocalDataSweep.Outcome(removed: ["pets.json"]).failureSummary)
    }
}
