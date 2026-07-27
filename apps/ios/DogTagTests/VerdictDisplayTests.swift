import XCTest

/// AUDIT RECS 2 AND 7 (2026-07-27 mock audit), the half #87 and #94 left open — the iOS half.
///
/// #87 gave the app a refresh and a `lastCheckedAt`; #94 made the verdict fold monotone. Neither
/// touched what the BADGE renders. Documents, Home, the credential detail and Travel all drew a
/// full-strength green `VALID` straight from the stored verdict string, so:
///
///  * a credential revoked after the last check kept a green badge until someone tapped refresh, and
///  * an EXPIRED credential read green on every one of those surfaces, because `validity.validUntil`
///    was enforced in exactly three places and no badge was one of them.
///
/// These cases pin the rule: **a surface states an accurate observation, or says it could not check.**
/// A green VALID on the strength of a check run days ago is a claim stronger than the evidence — and a
/// verdict that has merely gone stale must not flip to INVALID either, because "I have not looked
/// recently" is its own state and may not render as either neighbour.
///
/// This file is the case #94's body filed as a follow-up: *"extract the pure verdict logic into an
/// FFI-free source so the existing XCTest target can pin it the way `RoaxRpcSelectorTest` pins
/// Android."* `VerdictDisplay.swift` is Foundation-only, so the host-less bundle compiles it directly
/// and the iOS half of this change is genuinely tested rather than argued. It is the same case list as
/// Android's `VerdictBadgeStalenessTest` + `CredentialExpiryFromDocTest`, name for name.
final class VerdictDisplayTests: XCTestCase {

    private let now = Date(timeIntervalSince1970: 1785240000)   // 2026-07-28T12:00:00Z

    /// The exact shape `Stamp` writes and parses, so these fixtures are the strings the app stores.
    private let iso = ISO8601DateFormatter()

    private func stamp(_ date: Date) -> String { iso.string(from: date) }
    private func ago(_ seconds: TimeInterval) -> String { stamp(now.addingTimeInterval(-seconds)) }

    private func badge(
        _ verdict: String,
        lastCheckedAt: String? = nil,
        validUntil: String? = nil
    ) -> VerdictDisplay.Badge {
        VerdictDisplay.badge(
            verdict: verdict,
            lastCheckedAt: lastCheckedAt ?? ago(60),
            validUntil: validUntil,
            now: now
        )
    }

    /// The fixture instant really is the UTC day the expiry cases assume.
    func test_theFixtureClockIsTheDayTheExpiryCasesAssume() {
        XCTAssertEqual(VerdictDisplay.dateOnlyUtc(now), "2026-07-28")
    }

    // MARK: - staleness: only ever weakens a claim

    /// A verdict read off the chain minutes ago may still be asserted — the case that stops an
    /// over-eager staleness rule turning every badge grey.
    func test_aRecentlyCheckedValidStillReadsValid() {
        let out = badge("VALID", lastCheckedAt: ago(5 * 60))
        XCTAssertEqual(out.label, "VALID")
        XCTAssertEqual(out.tone, .positive)
    }

    /// THE defect, stated. A chain read from three days ago is not evidence about the chain now, and a
    /// revocation in between is exactly the case the audit named. The badge must stop asserting.
    func test_aValidCheckedDaysAgoNoLongerAssertsValid() {
        let out = badge("VALID", lastCheckedAt: ago(3 * 86400))
        XCTAssertEqual(out.label, "VALID · STALE")
        XCTAssertEqual(out.tone, .neutral)
    }

    /// The other direction of the same rule, and the mistake this codebase has already made twice. A
    /// badge that has not been refreshed must NOT read as revoked: "could not check" is its own state
    /// and must never render as either neighbour.
    func test_aStaleValidNeverRendersAsEitherNeighbour() {
        let out = badge("VALID", lastCheckedAt: ago(30 * 86400))
        XCTAssertNotEqual(out.tone, .positive, "must not claim validity")
        XCTAssertNotEqual(out.tone, .negative, "must not accuse")
        XCTAssertTrue(out.label.contains("VALID"), "the last answer stays visible")
        XCTAssertTrue(out.label.contains("STALE"), "and is visibly discounted")
    }

    /// #94's rule, pointed at the badge. Staleness is a NON-ANSWER, and a non-answer may not raise a
    /// verdict's severity — nor lower it. An established INVALID stays INVALID however old it is.
    func test_anEstablishedInvalidIsNotSoftenedByAge() {
        let out = badge("INVALID", lastCheckedAt: ago(400 * 86400))
        XCTAssertEqual(out.label, "INVALID")
        XCTAssertEqual(out.tone, .negative)
    }

    /// UNVERIFIED is already a non-asserting state; age cannot make it any weaker.
    func test_unverifiedIsUnchangedByAge() {
        let out = badge("UNVERIFIED", lastCheckedAt: ago(9 * 86400))
        XCTAssertEqual(out.label, "UNVERIFIED")
        XCTAssertEqual(out.tone, .neutral)
    }

    /// A record stored before `lastCheckedAt` shipped has no stamp. Absent evidence is not fresh
    /// evidence — inventing freshness for exactly the oldest records is the same over-claim.
    func test_aRecordWithNoCheckStampIsNotFresh() {
        for raw in [nil, "", "not-a-date"] as [String?] {
            XCTAssertEqual(
                VerdictDisplay.badge(verdict: "VALID", lastCheckedAt: raw, validUntil: nil, now: now).label,
                "VALID · STALE"
            )
        }
    }

    /// A device clock set forward is not evidence that a check ran recently. Under-claim instead.
    func test_aCheckStampInTheFutureIsNotFresh() {
        let ahead = stamp(now.addingTimeInterval(2 * 3600))
        XCTAssertFalse(VerdictDisplay.isFresh(ahead, now: now))
        XCTAssertEqual(badge("VALID", lastCheckedAt: ahead).label, "VALID · STALE")
    }

    /// The window boundary, exactly. Pins `VerdictDisplay.freshFor` as the single knob.
    func test_theFreshnessWindowBoundaryIsExact() {
        let edge = VerdictDisplay.freshFor
        XCTAssertTrue(VerdictDisplay.isFresh(ago(edge - 1), now: now))
        XCTAssertFalse(VerdictDisplay.isFresh(ago(edge), now: now), "the window is exclusive at its far edge")
        XCTAssertFalse(VerdictDisplay.isFresh(ago(edge + 1), now: now))
    }

    // MARK: - expiry: a definite, offline, root-covered negative

    /// AUDIT REC 7. `validity.validUntil` is a Merkle-covered leaf, so it is tamper-evident and readable
    /// with no chain access at all — yet every list badge ignored it.
    func test_anExpiredCredentialDoesNotReadAsValid() {
        let out = badge("VALID", lastCheckedAt: ago(1), validUntil: "2026-07-01")
        XCTAssertEqual(out.label, "EXPIRED")
        XCTAssertEqual(out.tone, .warning)
    }

    /// Expiry needs no chain read, so it outranks staleness: it is a definite fact where staleness is
    /// the absence of one.
    func test_expiryOutranksStalenessBecauseItIsKnowableOffline() {
        XCTAssertEqual(badge("VALID", lastCheckedAt: ago(5 * 86400), validUntil: "2020-01-01").label, "EXPIRED")
    }

    /// …and it outranks UNVERIFIED for the same reason: "expired" is strictly more informative than
    /// "I could not check".
    func test_expiryIsReportedEvenWhenTheChainCouldNotBeReached() {
        XCTAssertEqual(badge("UNVERIFIED", validUntil: "2026-07-27").label, "EXPIRED")
    }

    /// Revoked beats expired. An expired credential is not authorised; a revoked one is repudiated.
    func test_revocationOutranksExpiry() {
        XCTAssertEqual(badge("INVALID", validUntil: "2020-01-01").label, "INVALID")
    }

    /// `validUntil` is the INCLUSIVE last day: a credential good "until 2026-07-28" is good ON the 28th.
    func test_theValidUntilDayItselfIsStillValid() {
        XCTAssertEqual(badge("VALID", validUntil: "2026-07-28").label, "VALID")
        XCTAssertEqual(badge("VALID", validUntil: "2026-07-27").label, "EXPIRED")
    }

    /// A full timestamp is compared at day granularity, because the leaf is a date claim.
    func test_aFullTimestampIsComparedAtDayGranularity() {
        XCTAssertFalse(VerdictDisplay.lapsed("2026-07-28T00:00:01Z", now: now))
        XCTAssertTrue(VerdictDisplay.lapsed("2026-07-27T23:59:59Z", now: now))
    }

    /// The non-answer rule, pointed at expiry. A document that makes NO validity claim is not an expired
    /// document — reading one as expired would manufacture a negative out of missing data.
    func test_aMissingOrUnusableValidUntilIsNotAnExpiryClaim() {
        for v in [nil, "", "   ", "2026", "2026-07"] as [String?] {
            XCTAssertFalse(VerdictDisplay.lapsed(v, now: now), "\(String(describing: v)) is not an expiry claim")
        }
        XCTAssertEqual(badge("VALID", validUntil: nil).label, "VALID")
        XCTAssertEqual(badge("VALID", validUntil: "").label, "VALID")
    }

    /// UTC, not device-local: two phones either side of the date line must agree about the same document.
    func test_theExpiryComparisonIsUtc() {
        XCTAssertEqual(VerdictDisplay.dateOnlyUtc(Date(timeIntervalSince1970: 1785283199)), "2026-07-28")
        XCTAssertEqual(VerdictDisplay.dateOnlyUtc(Date(timeIntervalSince1970: 1785283200)), "2026-07-29")
    }

    // MARK: - where the expiry actually comes from

    private func docJson(_ validityBlock: String) -> String {
        """
        {"version":"dogtag/1.0",
         "data":{"credentialSubject":{"dogTagId":"aa:2:42"\(validityBlock)}},
         "signature":{"type":"DogTagMerkleProof","targetHash":"0x11","proof":[],"merkleRoot":"0x11"},
         "privacy":{"obfuscated":[]},
         "issuer":{"documentStore":"0xabc","name":"Gov","domain":"gov.example","recordType":"TRAVEL_CLEARANCE"}}
        """
    }

    private let lapsedBlock = ##","validity":{"issuedOn":"bb:2:2026-01-01","validUntil":"cc:2:2026-06-30"}"##
    private let currentBlock = ##","validity":{"issuedOn":"bb:2:2026-01-01","validUntil":"cc:2:2027-06-30"}"##

    /// EU_HEALTH_CERT has NO `validity` block: its window is the flat Annex-IV `rabiesValidUntil`.
    private let rabiesLapsedBlock = ##","rabiesValidUntil":"dd:2:2026-06-30""##
    private let rabiesCurrentBlock = ##","rabiesValidUntil":"dd:2:2027-06-30""##

    private func credential(_ validityBlock: String, verdict: String = "VALID") -> Credential {
        Credential(
            id: "rec-1",
            dogTagId: "42",
            group: .travel,
            recordType: "TRAVEL_CLEARANCE",
            title: "Travel Clearance",
            subtitle: "TRAVEL_CLEARANCE",
            issuer: "DogTag Government Authority",
            issuedOn: "",
            credentialRoot: "0x11",
            verdict: verdict,
            wrappedDocJson: docJson(validityBlock),
            importedAt: nil,
            lastCheckedAt: ago(60),
            verdictReason: "anchored on ROAX and not revoked"
        )
    }

    /// The packed `"<salt>:<tag>:<value>"` leaf is unwrapped to its bare value, as the receipt reads it.
    func test_readsThePackedValidUntilLeaf() {
        XCTAssertEqual(WrappedDoc(json: docJson(lapsedBlock))?.validUntil, "2026-06-30")
    }

    /// A document with no validity window makes no expiry claim — blank, never "expired".
    func test_aDocumentWithNoValidityBlockMakesNoExpiryClaim() {
        XCTAssertEqual(WrappedDoc(json: docJson(""))?.validUntil, "")
        XCTAssertNil(credential("").validUntil)
    }

    /// THE point of deriving `validUntil` from the stored document rather than persisting a copy: this
    /// credential carries no expiry column of its own — it is a record as stored before any of this
    /// shipped — and the badge still finds the lapse.
    func test_anAlreadyStoredRecordStillExpires() {
        let cred = credential(lapsedBlock)
        XCTAssertEqual(cred.validUntil, "2026-06-30")
        XCTAssertEqual(cred.badge(now: now).label, "EXPIRED")
        XCTAssertEqual(cred.badge(now: now).tone, .warning)
    }

    /// …and one still inside its window is untouched: the rule only ever fires on a real lapse.
    func test_aCredentialInsideItsWindowIsUnaffected() {
        XCTAssertEqual(credential(currentBlock).badge(now: now).label, "VALID")
    }

    /// The sub-line says WHEN, because "EXPIRED" alone is the first thing an owner asks about. The
    /// freshness half is asserted by shape: `lastCheckedLabel` renders against the real wall clock
    /// (#87's behaviour, unchanged here), so pinning its exact words would flake on the minute boundary.
    func test_theStatusLineNamesTheExpiryDate() {
        let line = credential(lapsedBlock).statusLine(now: now)
        XCTAssertTrue(line.hasPrefix("Checked "), line)
        XCTAssertTrue(line.hasSuffix(" · expired 2026-06-30"), line)
    }

    /// …and says nothing about expiry while the window is still open.
    func test_theStatusLineOmitsExpiryWhileTheWindowIsOpen() {
        XCTAssertFalse(credential(currentBlock).statusLine(now: now).contains("expired"))
    }

    /// The reason still travels with a non-VALID verdict, alongside the expiry. #87's line, intact.
    func test_theStatusLineStillCarriesTheVerdictReason() {
        var cred = credential(lapsedBlock, verdict: "UNVERIFIED")
        cred.verdictReason = "could not reach the chain (rpc 502)"
        let line = cred.statusLine(now: now)
        XCTAssertTrue(
            line.hasSuffix("· expired 2026-06-30 · could not reach the chain (rpc 502)"),
            line
        )
    }

    /// EU_HEALTH_CERT carries NO `validity` block at all — the government API issues its window as the
    /// flat Annex-IV `credentialSubject.rabiesValidUntil` leaf. Reading only the nested leaf exempted the
    /// whole record type from the rule, so a lapsed EU health certificate badged a full-strength green
    /// VALID on Documents, Home, Travel and the detail header. `CredentialGroup` maps EU_HEALTH_CERT to
    /// Travel, so those records really do reach the badged surfaces.
    func test_aLapsedEuHealthCertificateExpiresFromTheFlatRabiesLeaf() {
        let cred = credential(rabiesLapsedBlock)
        XCTAssertEqual(cred.validUntil, "2026-06-30")
        XCTAssertEqual(cred.badge(now: now).label, "EXPIRED")
        XCTAssertEqual(cred.badge(now: now).tone, .warning)
    }

    /// …and one still inside its rabies window is untouched, exactly as for the nested leaf.
    func test_anEuHealthCertificateInsideItsWindowIsUnaffected() {
        XCTAssertEqual(credential(rabiesCurrentBlock).validUntil, "2027-06-30")
        XCTAssertEqual(credential(rabiesCurrentBlock).badge(now: now).label, "VALID")
    }

    /// Precedence, asserted in BOTH directions — a one-directional test passes by accident under a
    /// reversed implementation. `validity.validUntil` wins whenever it is present, mirroring the web
    /// wallet's `pick("validity.validUntil") || pick("rabiesValidUntil")`.
    func test_theNestedValidityLeafWinsWhenBothArePresent() {
        XCTAssertEqual(credential(currentBlock + rabiesLapsedBlock).validUntil, "2027-06-30")
        XCTAssertEqual(credential(currentBlock + rabiesLapsedBlock).badge(now: now).label, "VALID")

        XCTAssertEqual(credential(lapsedBlock + rabiesCurrentBlock).validUntil, "2026-06-30")
        XCTAssertEqual(credential(lapsedBlock + rabiesCurrentBlock).badge(now: now).label, "EXPIRED")
    }

    /// The non-answer rule, kept intact through the fallback: a document carrying NEITHER leaf still
    /// claims nothing. Manufacturing an expiry out of missing data is the same defect inverted.
    ///
    /// A present-but-empty nested leaf must fall THROUGH to the flat one rather than suppress it —
    /// emptiness is tested on the unwrapped value, not on the packed `"<salt>:<tag>:<value>"` string.
    func test_anEmptyNestedLeafFallsThroughRatherThanSuppressingTheFallback() {
        XCTAssertNil(credential("").validUntil)
        XCTAssertEqual(credential("").badge(now: now).label, "VALID")

        let emptyNested = ##","validity":{"validUntil":"cc:2:"}"##
        XCTAssertNil(credential(emptyNested).validUntil)
        XCTAssertEqual(credential(emptyNested + rabiesLapsedBlock).validUntil, "2026-06-30")
    }

    /// A record whose document cannot be parsed has no expiry claim, and must not blow up a list render
    /// trying to find one.
    func test_anUnparseableDocumentYieldsNoExpiryClaimRatherThanThrowing() {
        var broken = credential(lapsedBlock)
        broken.wrappedDocJson = "{not json"
        XCTAssertNil(broken.validUntil)
        XCTAssertEqual(broken.badge(now: now).label, "VALID")
    }
}
