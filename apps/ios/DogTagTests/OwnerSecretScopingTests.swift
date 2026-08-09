import XCTest

/// The deployment-scoping decisions behind the owner-secret store, mirrored case for case with
/// Android `OwnerSecretRecordsTest` (the `deployment scoping` section) — change both together.
///
/// One redeploy used to poison every low id on a handset permanently: tag 1 on the new deployment
/// collided with tag 1 on the old one and the phone refused "this dogTagId already has different
/// private profile metadata" for a tag that was genuinely free. These are the pure rules that
/// removed that. The STORE half (`ProfileTreeStore.applyUpsert`'s scoped write-once + migration
/// stamp) is FFI-coupled and outside this host-less bundle — Android's JVM suite pins that half on
/// its identical mirror.
final class OwnerSecretScopingTests: XCTestCase {
    private struct StubRecord: DeploymentScopedRecord {
        let dogTagIdDec: String
        let deployment: DeploymentScope?
        let label: String
    }

    private let depA = DeploymentScope(chainId: 135, sbtAddress: "0x00000000000000000000000000000000000000dd")
    private let depB = DeploymentScope(chainId: 135, sbtAddress: "0x00000000000000000000000000000000000000d1")

    // ---- DeploymentScope -----------------------------------------------------------------------

    func test_aBlankBundledSbtYieldsNoScope_theFailClosedSignal() {
        XCTAssertNil(DeploymentScope.of(chainId: 135, sbtAddress: ""))
        XCTAssertNil(DeploymentScope.of(chainId: 135, sbtAddress: "  "))
        XCTAssertEqual(
            DeploymentScope.of(chainId: 135, sbtAddress: depA.sbtAddress),
            depA
        )
    }

    func test_scopeMatchingIsCaseInsensitiveOnTheAddressAndExactOnTheChain() {
        XCTAssertTrue(depA.matches(DeploymentScope(chainId: 135, sbtAddress: depA.sbtAddress.uppercased())))
        XCTAssertFalse(depA.matches(DeploymentScope(chainId: 1, sbtAddress: depA.sbtAddress)))
        XCTAssertFalse(depA.matches(depB))
    }

    /// "Unknown" must not compare equal to any deployment — a legacy record is not evidence about
    /// where its tag lives, and treating it as a member of every deployment is the poisoning.
    func test_sameScopeTreatsUnknownAsItsOwnFact() {
        XCTAssertTrue(OwnerSecretScoping.sameScope(nil, nil))
        XCTAssertFalse(OwnerSecretScoping.sameScope(depA, nil))
        XCTAssertFalse(OwnerSecretScoping.sameScope(nil, depA))
        XCTAssertTrue(OwnerSecretScoping.sameScope(depA, depA))
        XCTAssertFalse(OwnerSecretScoping.sameScope(depA, depB))
    }

    // ---- selection ------------------------------------------------------------------------------

    func test_preferredReturnsTheCurrentDeploymentsRecordThenLegacyAndNeverAForeignOne() {
        let records = [
            StubRecord(dogTagIdDec: "1", deployment: depB, label: "foreign"),
            StubRecord(dogTagIdDec: "1", deployment: nil, label: "legacy"),
            StubRecord(dogTagIdDec: "1", deployment: depA, label: "current"),
        ]
        XCTAssertEqual(
            OwnerSecretScoping.preferred(records, forDogTagIdDec: "1", current: depA)?.label,
            "current"
        )
        XCTAssertEqual(
            OwnerSecretScoping.preferred(Array(records.prefix(2)), forDogTagIdDec: "1", current: depA)?.label,
            "legacy"
        )
        XCTAssertNil(
            OwnerSecretScoping.preferred(Array(records.prefix(1)), forDogTagIdDec: "1", current: depA)
        )
    }

    /// With NO way to establish the current deployment, only a legacy record answers — a scoped
    /// record must not be assumed to belong to a deployment the app cannot name.
    func test_preferredWithAnUnknownCurrentDeploymentReturnsOnlyLegacyRecords() {
        let records = [
            StubRecord(dogTagIdDec: "1", deployment: depA, label: "scoped"),
            StubRecord(dogTagIdDec: "1", deployment: nil, label: "legacy"),
        ]
        XCTAssertEqual(
            OwnerSecretScoping.preferred(records, forDogTagIdDec: "1", current: nil)?.label,
            "legacy"
        )
        XCTAssertNil(
            OwnerSecretScoping.preferred(Array(records.prefix(1)), forDogTagIdDec: "1", current: nil)
        )
    }

    // ---- the bind flow's decision table ----------------------------------------------------------

    func test_reuseDecisionReusesOnContentMatchWhateverTheScopes() {
        // Byte-identical content — wallet plus the vet-salted identity leaves — means the SAME
        // session already built on this phone: reuse, and (for a legacy record) stamp.
        XCTAssertEqual(
            OwnerSecretScoping.reuseDecision(recordScope: nil, current: depA, contentMatches: true),
            .reuse
        )
        XCTAssertEqual(
            OwnerSecretScoping.reuseDecision(recordScope: depA, current: depA, contentMatches: true),
            .reuse
        )
    }

    func test_reuseDecisionFreesALegacyIdOnlyWhenTheCurrentDeploymentIsKnown() {
        // THE REDEPLOY CASE: a legacy record with different content is another deployment's tag —
        // the id is genuinely free here, so build fresh instead of refusing the owner forever.
        XCTAssertEqual(
            OwnerSecretScoping.reuseDecision(recordScope: nil, current: depA, contentMatches: false),
            .buildFresh
        )
        // Same deployment, different content: a real conflict — refuse before the stored witness
        // (unrecoverable salts) is disturbed.
        XCTAssertEqual(
            OwnerSecretScoping.reuseDecision(recordScope: depA, current: depA, contentMatches: false),
            .refuseConflict
        )
        // No way to tell deployments apart: fail closed, exactly as before scoping existed.
        XCTAssertEqual(
            OwnerSecretScoping.reuseDecision(recordScope: nil, current: nil, contentMatches: false),
            .refuseConflict
        )
    }

    // ---- codec shape -----------------------------------------------------------------------------

    /// The scope is Codable with the exact keys Android writes (`chainId`, `sbtAddress`), so one
    /// record shape crosses the two platforms' documented store formats without translation.
    func test_deploymentScopeEncodesWithTheSharedKeyNames() throws {
        let data = try JSONEncoder().encode(depA)
        let obj = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(obj["chainId"] as? Int, 135)
        XCTAssertEqual(obj["sbtAddress"] as? String, depA.sbtAddress)
        let decoded = try JSONDecoder().decode(DeploymentScope.self, from: data)
        XCTAssertEqual(decoded, depA)
    }
}
