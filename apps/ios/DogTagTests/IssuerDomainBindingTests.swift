import XCTest

/// The issuer↔domain binding: the three-state DNS rule, the ABI string decode, and the copy discipline.
///
/// The load-bearing properties, in order of what a regression would cost:
///
///  * **SERVFAIL and "no record" are not the same state.** Collapsing them is the fail-open shape this
///    app already had elsewhere (`.unknown -> VALID` in the importer). `servfail_and_absence_differ`
///    is the guard.
///  * **A provenance failure never reads as a DNS one.** "This contract did not come from the DogTag
///    factory" is a categorically stronger statement than "no record published" and must not be softened.
///  * **No verdict or alarm language, in any state.** A missing DNS record says nothing about the
///    credential, whose validity is proven on-chain; telling a holder their valid credential FAILED would
///    be worse than showing nothing.
final class IssuerDomainBindingTests: XCTestCase {
    private let clone = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F"
    private let cloneLc = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f"

    // MARK: - the normative record name

    func test_txtName_lowercases_a_checksummed_address() {
        XCTAssertEqual(
            IssuerBindingResolver.txtName(clone: clone, domain: "moh.gov.sg"),
            "\(cloneLc)._dogtag.moh.gov.sg"
        )
    }

    func test_txtName_tolerates_padding_and_a_trailing_dot() {
        XCTAssertEqual(
            IssuerBindingResolver.txtName(clone: "  \(clone) ", domain: " MOH.GOV.SG. "),
            "\(cloneLc)._dogtag.moh.gov.sg"
        )
    }

    func test_txtName_rejects_unusable_inputs() {
        XCTAssertNil(IssuerBindingResolver.txtName(clone: "", domain: "moh.gov.sg"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: clone, domain: ""))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: "0xnothex", domain: "moh.gov.sg"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: "0xb5d6", domain: "moh.gov.sg"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: clone, domain: "localhost"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: clone, domain: "https://moh.gov.sg"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: clone, domain: "moh.gov.sg:443"))
        XCTAssertNil(IssuerBindingResolver.txtName(clone: clone, domain: "moh gov sg"))
    }

    /// The address can live in the NAME only because it fits a DNS label; the VALUE therefore stays free
    /// for the domain owner's own description, which is the whole design.
    func test_address_label_fits_a_dns_label() {
        XCTAssertEqual(cloneLc.count, 42)
        XCTAssertLessThanOrEqual(cloneLc.count, 63)
    }

    // MARK: - the three DNS states

    private func doh(_ status: Int, _ answers: [[String: Any]]? = nil) -> [String: Any] {
        var o: [String: Any] = ["Status": status]
        if let a = answers { o["Answer"] = a }
        return o
    }

    private func txt(_ name: String, _ data: String) -> [String: Any] {
        ["name": name, "type": 16, "TTL": 300, "data": data]
    }

    func test_verified_when_the_record_exists() {
        let s = IssuerBindingResolver.classifyDoh(
            doh(0, [txt("\(cloneLc)._dogtag.moh.gov.sg", "\"Travel clearance issuance\"")])
        )
        XCTAssertEqual(s, .verified(description: "Travel clearance issuance"))
    }

    func test_verified_concatenates_chunked_txt_strings() {
        // A >255-byte description arrives as multiple quoted chunks; RFC 1035 says CONCATENATE, so
        // "part-one" "part-two" is part-onepart-two, not "part-one part-two".
        let s = IssuerBindingResolver.classifyDoh(
            doh(0, [txt("x", "\"part-one\" \"part-two\"")])
        )
        XCTAssertEqual(s, .verified(description: "part-onepart-two"))
    }

    func test_not_listed_on_noerror_with_no_answer_section() {
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(doh(0)), .notListed)
    }

    func test_not_listed_on_noerror_with_only_non_txt_records() {
        let cname: [String: Any] = ["name": "x", "type": 5, "TTL": 300, "data": "elsewhere.example."]
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(doh(0, [cname])), .notListed)
    }

    /// NXDOMAIN is a DEFINITIVE "this name does not exist", not a failure.
    func test_not_listed_on_nxdomain() {
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(doh(3)), .notListed)
    }

    func test_could_not_check_on_servfail_and_refused() {
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(doh(2)), .couldNotCheck)
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(doh(5)), .couldNotCheck)
    }

    /// A 200 whose body is not a DoH answer must NOT be read as NOERROR — that would turn a
    /// wrong-endpoint misconfiguration into a confident "absent".
    func test_could_not_check_when_there_is_no_status_field() {
        XCTAssertEqual(IssuerBindingResolver.classifyDoh(["hello": "world"]), .couldNotCheck)
    }

    /// THE regression guard.
    func test_servfail_and_absence_differ() {
        let absent = IssuerBindingResolver.classifyDoh(doh(0))
        let broken = IssuerBindingResolver.classifyDoh(doh(2))
        XCTAssertNotEqual(absent, broken)
        XCTAssertNotEqual(absent, .verified(description: ""))
        XCTAssertNotEqual(broken, .verified(description: ""))
    }

    // MARK: - the ABI string decode

    func test_decodes_an_abi_encoded_string() {
        // offset 0x20, length 11, "moh.gov.sg" padded — a canonical single-string return.
        let hex =
            "0000000000000000000000000000000000000000000000000000000000000020"
            + "000000000000000000000000000000000000000000000000000000000000000a"
            + "6d6f682e676f762e736700000000000000000000000000000000000000000000"
        XCTAssertEqual(RoaxRpc.decodeAbiString(hex), "moh.gov.sg")
    }

    /// An ABI-encoded EMPTY string is a real answer ("no claim published"). It is not the same thing as
    /// an empty `eth_call` result, which means there is no contract at the address at all — the resolver
    /// keeps those apart via `StringRead.noContract`.
    func test_decodes_an_abi_encoded_empty_string() {
        let hex =
            "0000000000000000000000000000000000000000000000000000000000000020"
            + "0000000000000000000000000000000000000000000000000000000000000000"
        XCTAssertEqual(RoaxRpc.decodeAbiString(hex), "")
    }

    func test_decode_returns_nil_on_a_truncated_body_rather_than_guessing() {
        XCTAssertNil(RoaxRpc.decodeAbiString(""))
        XCTAssertNil(RoaxRpc.decodeAbiString("0020"))
        // A declared length that runs past the payload must not be honoured.
        let bad =
            "0000000000000000000000000000000000000000000000000000000000000020"
            + "00000000000000000000000000000000000000000000000000000000000000ff"
        XCTAssertNil(RoaxRpc.decodeAbiString(bad))
    }

    // MARK: - copy discipline

    private func binding(_ state: IssuerBindingState, domain: String = "moh.gov.sg") -> IssuerBinding {
        var b = IssuerBinding()
        b.state = state
        b.domain = domain
        return b
    }

    private let allStates: [IssuerBindingState] = [
        .verified(description: "Travel clearance issuance"),
        .notADogTagIssuer,
        .notListed,
        .couldNotCheck,
        .noDomainClaimed,
        .unavailable,
        .pending,
    ]

    func test_no_verdict_or_alarm_words_in_any_state() {
        let forbidden = [
            "verification failed", "failed", "failure", "invalid", "untrusted", "not trusted",
            "warning", "danger", "insecure", "fraud", "fake", "suspicious", "unsafe", "error",
            "rejected",
        ]
        for state in allStates {
            let line = binding(state).line.lowercased()
            for word in forbidden {
                XCTAssertFalse(line.contains(word), "state \(state) line \"\(line)\" contains \"\(word)\"")
            }
        }
    }

    func test_states_what_was_looked_at_and_what_was_found() {
        XCTAssertEqual(
            binding(.verified(description: "x")).line,
            "This address is listed in moh.gov.sg's DNS records"
        )
        XCTAssertEqual(
            binding(.notListed).line,
            "This address is not listed in moh.gov.sg's DNS records"
        )
    }

    /// Same size, same register, an observation either way — the two differ by one word.
    func test_verified_and_not_listed_are_symmetric() {
        let yes = binding(.verified(description: "x")).line
        let no = binding(.notListed).line
        XCTAssertEqual(no, yes.replacingOccurrences(of: "is listed", with: "is not listed"))
    }

    /// A non-clone must NEVER read as merely "not listed in DNS": DNS was never the question.
    func test_a_provenance_failure_never_mentions_dns() {
        let line = binding(.notADogTagIssuer).line
        XCTAssertEqual(line, "This contract was not deployed by the DogTag factory")
        XCTAssertFalse(line.lowercased().contains("dns"))
        XCTAssertFalse(line.lowercased().contains("listed"))
        XCTAssertNotEqual(line, binding(.notListed).line)
    }

    func test_could_not_check_blames_our_reach_not_the_issuer() {
        let line = binding(.couldNotCheck).line
        XCTAssertEqual(line, "We could not reach DNS to check this domain")
        XCTAssertFalse(line.lowercased().contains("not listed"))
    }

    func test_every_state_has_distinct_copy() {
        let lines = Set(allStates.map { binding($0).line })
        XCTAssertEqual(lines.count, allStates.count)
    }

    func test_falls_back_to_a_neutral_phrase_with_no_domain() {
        XCTAssertTrue(binding(.notListed, domain: "").line.contains("this domain"))
    }

    // MARK: - tone

    func test_only_verified_is_positive() {
        XCTAssertEqual(binding(.verified(description: "x")).tone, .positive)
        for state in allStates.dropFirst() {
            XCTAssertNotEqual(binding(state).tone, .positive, "\(state) must not be positive")
        }
    }

    /// A resolver timeout says nothing whatsoever about the issuer, so colouring it as a failure would be
    /// a lie of emphasis.
    func test_the_unknown_states_stay_neutral() {
        XCTAssertEqual(binding(.couldNotCheck).tone, .neutral)
        XCTAssertEqual(binding(.noDomainClaimed).tone, .neutral)
        XCTAssertEqual(binding(.unavailable).tone, .neutral)
    }

    func test_definitive_absences_are_negative() {
        XCTAssertEqual(binding(.notListed).tone, .negative)
        XCTAssertEqual(binding(.notADogTagIssuer).tone, .negative)
    }

    func test_pending_is_its_own_tone_and_the_default_state() {
        XCTAssertEqual(IssuerBinding.pending.state, .pending)
        XCTAssertEqual(IssuerBinding.pending.tone, .pending)
        XCTAssertFalse(IssuerBinding.pending.isVerified)
    }

    // MARK: - the published description

    func test_the_domain_owners_description_is_surfaced_only_when_verified() {
        XCTAssertEqual(
            binding(.verified(description: "Travel clearance issuance")).publishedDescription,
            "Travel clearance issuance"
        )
        XCTAssertNil(binding(.notListed).publishedDescription)
        XCTAssertNil(binding(.verified(description: "   ")).publishedDescription)
    }

    func test_unquote_txt_handles_bare_and_escaped_values() {
        XCTAssertEqual(IssuerBindingResolver.unquoteTxt("plain text"), "plain text")
        XCTAssertEqual(IssuerBindingResolver.unquoteTxt("\"quoted\""), "quoted")
        XCTAssertEqual(IssuerBindingResolver.unquoteTxt("\"a\\\"b\""), "a\"b")
    }

    // MARK: - the DID assertion (the other half of issuer identity)

    private func doc(displayedDomain: String, dataIssuer: String?) -> WrappedDoc? {
        var data: [String: Any] = ["credentialSubject": ["name": "s:0:Max"]]
        if let d = dataIssuer { data["issuer"] = d }
        let root: [String: Any] = [
            "version": "1.0",
            "data": data,
            "signature": ["type": "t", "targetHash": "0x00", "proof": [], "merkleRoot": "0x00"],
            "privacy": ["obfuscated": []],
            "issuer": [
                "name": "Example Competent Authority",
                "domain": displayedDomain,
                "documentStore": clone,
                "recordType": "TRAVEL_CLEARANCE",
            ],
        ]
        let json = String(data: try! JSONSerialization.data(withJSONObject: root), encoding: .utf8)!
        return WrappedDoc(json: json)
    }

    func test_did_web_host_drops_path_segments_and_ports() {
        XCTAssertEqual(IssuerIdentity.didWebHost("did:web:example.com"), "example.com")
        XCTAssertEqual(IssuerIdentity.didWebHost("did:web:example.com:dept:vet"), "example.com")
        XCTAssertEqual(IssuerIdentity.didWebHost("did:web:example.com%3A8443"), "example.com")
        XCTAssertEqual(IssuerIdentity.didWebHost("did:web:EXAMPLE.com"), "example.com")
        XCTAssertNil(IssuerIdentity.didWebHost("did:key:z6Mk"))
        XCTAssertNil(IssuerIdentity.didWebHost("did:web:localhost"), "a single label is not a domain")
        XCTAssertNil(IssuerIdentity.didWebHost("not a did"))
    }

    func test_matching_domain_and_did_asserts() {
        let a = IssuerIdentity.assertDomain(doc(displayedDomain: "gov.example", dataIssuer: "abcd:2:did:web:gov.example"))
        XCTAssertEqual(a, .match(domain: "gov.example"))
        XCTAssertFalse(a.isMismatch)
    }

    /// The audit's attack, caught by the document alone — no chain, no DNS needed.
    func test_the_relabelling_attack_is_detected() {
        let a = IssuerIdentity.assertDomain(doc(displayedDomain: "moh.gov.sg", dataIssuer: "abcd:2:did:web:gov.example"))
        XCTAssertEqual(a, .mismatch(displayed: "moh.gov.sg", rootCovered: "gov.example"))
        XCTAssertTrue(a.isMismatch)
    }

    func test_comparison_ignores_case_and_a_trailing_dot() {
        let a = IssuerIdentity.assertDomain(doc(displayedDomain: "GOV.Example.", dataIssuer: "abcd:2:did:web:gov.example"))
        XCTAssertEqual(a, .match(domain: "gov.example"))
    }

    /// A document with no root-covered DID is NOT a pass and NOT a forgery — it is un-assertable.
    func test_a_document_without_the_leaf_is_not_assertable_and_not_a_pass() {
        let a = IssuerIdentity.assertDomain(doc(displayedDomain: "gov.example", dataIssuer: nil))
        XCTAssertEqual(a, .notAssertable)
        XCTAssertFalse(a.isMismatch)
        if case .match = a { XCTFail("notAssertable must never read as a pass") }
    }

    func test_a_bare_unpacked_did_is_still_read() {
        let a = IssuerIdentity.assertDomain(doc(displayedDomain: "gov.example", dataIssuer: "did:web:gov.example"))
        XCTAssertEqual(a, .match(domain: "gov.example"))
    }

    func test_the_leaf_is_read_from_credential_subject_too() {
        var data: [String: Any] = ["credentialSubject": ["name": "s:0:Max", "issuer": "abcd:2:did:web:gov.example"]]
        let root: [String: Any] = [
            "version": "1.0", "data": data,
            "signature": ["type": "t", "targetHash": "0x00", "proof": [], "merkleRoot": "0x00"],
            "privacy": ["obfuscated": []],
            "issuer": ["name": "X", "domain": "gov.example", "documentStore": clone, "recordType": "T"],
        ]
        data = [:]
        let json = String(data: try! JSONSerialization.data(withJSONObject: root), encoding: .utf8)!
        XCTAssertEqual(IssuerIdentity.assertDomain(WrappedDoc(json: json)), .match(domain: "gov.example"))
    }
}
