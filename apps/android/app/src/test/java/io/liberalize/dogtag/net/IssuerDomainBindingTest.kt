package io.liberalize.dogtag.net

import io.liberalize.dogtag.data.WrappedDoc
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The issuer↔domain binding: the three-state DNS rule, the ABI string decode, and the copy discipline.
 *
 * The load-bearing properties, in order of what a regression would cost:
 *
 *  * **SERVFAIL and "no record" are not the same state.** Collapsing them is the fail-open shape this app
 *    already had elsewhere (`else -> "VALID"` in the importer). [servfail_and_absence_differ] is the guard.
 *  * **A provenance failure never reads as a DNS one.** "This contract did not come from the DogTag
 *    factory" is categorically stronger than "no record published" and must not be softened.
 *  * **No verdict or alarm language, in any state.** A missing DNS record says nothing about the
 *    credential, whose validity is proven on-chain; telling a holder their valid credential FAILED would
 *    be worse than showing nothing.
 *
 * Kept in lockstep with `apps/ios/DogTagTests/IssuerDomainBindingTests.swift` and the Rust
 * `dogtag-dns-rs` unit tests — all three classify the same DoH bodies the same way.
 */
class IssuerDomainBindingTest {
    private val clone = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F"
    private val cloneLc = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f"

    // ---- the normative record name -------------------------------------------------------------

    @Test
    fun txtName_lowercases_a_checksummed_address() {
        assertEquals(
            "$cloneLc._dogtag.moh.gov.sg",
            IssuerBindingResolver.txtName(clone, "moh.gov.sg"),
        )
    }

    @Test
    fun txtName_tolerates_padding_and_a_trailing_dot() {
        assertEquals(
            "$cloneLc._dogtag.moh.gov.sg",
            IssuerBindingResolver.txtName("  $clone ", " MOH.GOV.SG. "),
        )
    }

    @Test
    fun txtName_rejects_unusable_inputs() {
        assertNull(IssuerBindingResolver.txtName("", "moh.gov.sg"))
        assertNull(IssuerBindingResolver.txtName(clone, ""))
        assertNull(IssuerBindingResolver.txtName("0xnothex", "moh.gov.sg"))
        assertNull(IssuerBindingResolver.txtName("0xb5d6", "moh.gov.sg"))
        assertNull(IssuerBindingResolver.txtName(clone, "localhost"))
        assertNull(IssuerBindingResolver.txtName(clone, "https://moh.gov.sg"))
        assertNull(IssuerBindingResolver.txtName(clone, "moh.gov.sg:443"))
        assertNull(IssuerBindingResolver.txtName(clone, "moh gov sg"))
    }

    /** The address can live in the NAME only because it fits a DNS label; that is what frees the VALUE. */
    @Test
    fun address_label_fits_a_dns_label() {
        assertEquals(42, cloneLc.length)
        assertTrue(cloneLc.length <= 63)
    }

    /** The three implementations must agree on the record shape, or a phone and a portal disagree. */
    @Test
    fun txtName_matches_the_normative_convention() {
        val name = IssuerBindingResolver.txtName(clone, "moh.gov.sg")!!
        assertTrue(name.contains(".${IssuerBindingResolver.LABEL}."))
        assertEquals(cloneLc, name.substringBefore('.'))
        assertTrue(name.endsWith("moh.gov.sg"))
    }

    // ---- the three DNS states -------------------------------------------------------------------

    private fun doh(status: Int, answers: String? = null): JSONObject =
        JSONObject("""{"Status":$status${if (answers != null) ",\"Answer\":[$answers]" else ""}}""")

    private fun txt(name: String, data: String) =
        """{"name":"$name","type":16,"TTL":300,"data":"$data"}"""

    @Test
    fun verified_when_the_record_exists() {
        val s = IssuerBindingResolver.classifyDoh(
            doh(0, txt("$cloneLc._dogtag.moh.gov.sg", "\\\"Travel clearance issuance\\\"")),
        )
        assertEquals(IssuerBindingState.Verified("Travel clearance issuance"), s)
    }

    /**
     * A >255-byte description arrives as multiple quoted chunks; RFC 1035 says CONCATENATE, so
     * `"part-one" "part-two"` is `part-onepart-two`, not `part-one part-two`.
     */
    @Test
    fun verified_concatenates_chunked_txt_strings() {
        val s = IssuerBindingResolver.classifyDoh(doh(0, txt("x", "\\\"part-one\\\" \\\"part-two\\\"")))
        assertEquals(IssuerBindingState.Verified("part-onepart-two"), s)
    }

    @Test
    fun not_listed_on_noerror_with_no_answer_section() {
        assertEquals(IssuerBindingState.NotListed, IssuerBindingResolver.classifyDoh(doh(0)))
    }

    @Test
    fun not_listed_on_noerror_with_only_non_txt_records() {
        val cname = """{"name":"x","type":5,"TTL":300,"data":"elsewhere.example."}"""
        assertEquals(IssuerBindingState.NotListed, IssuerBindingResolver.classifyDoh(doh(0, cname)))
    }

    /** NXDOMAIN is a DEFINITIVE "this name does not exist", not a failure. */
    @Test
    fun not_listed_on_nxdomain() {
        assertEquals(IssuerBindingState.NotListed, IssuerBindingResolver.classifyDoh(doh(3)))
    }

    @Test
    fun could_not_check_on_servfail_and_refused() {
        assertEquals(IssuerBindingState.CouldNotCheck, IssuerBindingResolver.classifyDoh(doh(2)))
        assertEquals(IssuerBindingState.CouldNotCheck, IssuerBindingResolver.classifyDoh(doh(5)))
    }

    /**
     * A 200 whose body is not a DoH answer must NOT be read as NOERROR — that would turn a
     * wrong-endpoint misconfiguration into a confident "absent".
     */
    @Test
    fun could_not_check_when_there_is_no_status_field() {
        assertEquals(
            IssuerBindingState.CouldNotCheck,
            IssuerBindingResolver.classifyDoh(JSONObject("""{"hello":"world"}""")),
        )
    }

    /**
     * A CNAME chain answers under the TARGET's name, so a lone record is accepted whatever name it
     * echoes — the resolver answered the question it was asked.
     */
    @Test
    fun a_single_record_is_accepted_even_when_the_echoed_name_differs() {
        val s = IssuerBindingResolver.classifyDoh(
            doh(0, txt("somewhere-else.example", "\\\"Travel clearance issuance\\\"")),
            queriedName = "$cloneLc._dogtag.moh.gov.sg",
        )
        assertEquals(IssuerBindingState.Verified("Travel clearance issuance"), s)
    }

    /**
     * With TWO OR MORE records the echoed name is the only way to tell which one answers OUR query, so
     * it must be matched. Taking whichever happened to be first would display an unrelated record (an
     * SPF line at a CNAME target, say) as this domain's description of the issuer. Rust's
     * `select_binding` is the normative rule; this pins the Kotlin port to it.
     */
    @Test
    fun multiple_records_require_the_echoed_name_to_match() {
        val queried = "$cloneLc._dogtag.moh.gov.sg"
        val mixed = txt("unrelated.example", "\\\"v=spf1 -all\\\"") + "," +
            txt(queried, "\\\"Travel clearance issuance\\\"")
        assertEquals(
            IssuerBindingState.Verified("Travel clearance issuance"),
            IssuerBindingResolver.classifyDoh(doh(0, mixed), queriedName = queried),
        )
        val none = txt("a.example", "\\\"one\\\"") + "," + txt("b.example", "\\\"two\\\"")
        assertEquals(
            IssuerBindingState.NotListed,
            IssuerBindingResolver.classifyDoh(doh(0, none), queriedName = queried),
        )
    }

    /**
     * DNS 0x20 randomises the echoed case, and a zone file may or may not carry the trailing root dot.
     * Neither may change the answer.
     */
    @Test
    fun the_echoed_name_match_ignores_case_and_the_trailing_dot() {
        val queried = "$cloneLc._dogtag.moh.gov.sg"
        val answers = txt("unrelated.example", "\\\"v=spf1 -all\\\"") + "," +
            txt("${cloneLc.uppercase()}._DogTag.MOH.gov.SG.", "\\\"Travel clearance issuance\\\"")
        assertEquals(
            IssuerBindingState.Verified("Travel clearance issuance"),
            IssuerBindingResolver.classifyDoh(doh(0, answers), queriedName = queried),
        )
    }

    /** THE regression guard. */
    @Test
    fun servfail_and_absence_differ() {
        val absent = IssuerBindingResolver.classifyDoh(doh(0))
        val broken = IssuerBindingResolver.classifyDoh(doh(2))
        assertNotEquals(absent, broken)
        assertFalse(IssuerBinding(absent).isVerified)
        assertFalse(IssuerBinding(broken).isVerified)
    }

    // ---- the ABI string decode ------------------------------------------------------------------

    @Test
    fun decodes_an_abi_encoded_string() {
        val hex =
            "0000000000000000000000000000000000000000000000000000000000000020" +
                "000000000000000000000000000000000000000000000000000000000000000a" +
                "6d6f682e676f762e736700000000000000000000000000000000000000000000"
        assertEquals("moh.gov.sg", RoaxRpc.decodeAbiString(hex))
    }

    /**
     * An ABI-encoded EMPTY string is a real answer ("no claim published"). It is NOT the same thing as an
     * empty `eth_call` result, which means there is no contract at the address — the resolver keeps those
     * apart via `StringRead.NoContract`.
     */
    @Test
    fun decodes_an_abi_encoded_empty_string() {
        val hex =
            "0000000000000000000000000000000000000000000000000000000000000020" +
                "0000000000000000000000000000000000000000000000000000000000000000"
        assertEquals("", RoaxRpc.decodeAbiString(hex))
    }

    @Test
    fun decode_returns_null_on_a_truncated_body_rather_than_guessing() {
        assertNull(RoaxRpc.decodeAbiString(""))
        assertNull(RoaxRpc.decodeAbiString("0020"))
        // A declared length that runs past the payload must not be honoured.
        val bad =
            "0000000000000000000000000000000000000000000000000000000000000020" +
                "00000000000000000000000000000000000000000000000000000000000000ff"
        assertNull(RoaxRpc.decodeAbiString(bad))
    }

    /**
     * An offset/length word with a HIGH byte set must be REJECTED, not throw. `Int` is 32 bits here, so
     * the old `offset + 32 > bytes.size` guard WRAPPED negative near `Int.MAX_VALUE`, passed, and the
     * decoder threw `ArrayIndexOutOfBounds` / `StringIndexOutOfBounds` instead of returning null as its
     * doc promises. Swift's `Int` is 64-bit and cannot wrap — which is exactly how the two ports drifted
     * while the Swift test claimed Kotlin already enforced this band. Mirror of
     * `test_decode_rejects_an_oversized_offset_or_length_without_trapping`.
     */
    @Test
    fun decode_rejects_an_oversized_offset_or_length_without_throwing() {
        val payload = "00".repeat(32)
        // offset with bit 63 set
        assertNull(
            RoaxRpc.decodeAbiString(
                "0000000000000000000000000000000000000000000000008000000000000000" + payload,
            ),
        )
        // offset with a byte set above the low 4 (the reject band `beInt` enforces)
        assertNull(
            RoaxRpc.decodeAbiString(
                "0000000000000000000000000000000000000000000000000000010000000000" + payload,
            ),
        )
        // THE wrap case: an offset just below Int.MAX_VALUE, where `offset + 32` goes negative.
        assertNull(
            RoaxRpc.decodeAbiString(
                "000000000000000000000000000000000000000000000000000000007ffffff0" + payload,
            ),
        )
        // a sane offset, but a length word in the same wrap band
        assertNull(
            RoaxRpc.decodeAbiString(
                "0000000000000000000000000000000000000000000000000000000000000020" +
                    "000000000000000000000000000000000000000000000000000000007ffffff0",
            ),
        )
        // and the full-width word
        assertNull(RoaxRpc.decodeAbiString("ff".repeat(32) + payload))
    }

    // ---- provenance line ---------------------------------------------------------------------------

    private fun bound(
        state: IssuerBindingState,
        block: Long? = 283207L,
        checkedAt: Long? = 1_000_000_000L,
        now: Long = 1_000_000_000L,
    ) = IssuerBinding(
        state = state,
        domain = "moh.gov.sg",
        blockNumber = block,
        checkedAt = checkedAt,
    ).provenanceLine(now)

    /**
     * THE fabrication guard. The resolver returns for these three states BEFORE link 3, so no DNS query
     * was ever fired — a line claiming DNS was checked is exactly the "we did not look rendered as we
     * looked" the three-state design exists to prevent. The chain clause still renders: that read DID
     * happen. Mirror of the Swift and TS equivalents.
     */
    @Test
    fun no_dns_clause_for_states_that_never_queried_dns() {
        val noDns = listOf(
            IssuerBindingState.NotADogTagIssuer,
            IssuerBindingState.Unavailable,
            IssuerBindingState.NoDomainClaimed,
            IssuerBindingState.NoDomainListed,
        )
        for (state in noDns) {
            // `checkedAt` is deliberately supplied: even a stray timestamp must not license the claim.
            val line = bound(state)
            assertNotNull("$state", line)
            assertFalse("$state: $line", line!!.contains("DNS"))
            assertTrue("$state: $line", line.contains("chain read at block 283207"))
        }
    }

    @Test
    fun dns_bearing_states_do_report_the_dns_observation() {
        val withDns = listOf(
            IssuerBindingState.Verified("Travel clearance issuance"),
            IssuerBindingState.NotListed,
            IssuerBindingState.CouldNotCheck,
        )
        for (state in withDns) {
            val line = bound(state)
            assertNotNull("$state", line)
            assertTrue("$state: $line", line!!.contains("DNS checked just now"))
            assertTrue("$state: $line", line.contains("DNS has no history"))
        }
    }

    /**
     * Answers are cached for 15 minutes keeping their ORIGINAL timestamp, so "just now" would be a false
     * claim on a re-render.
     */
    @Test
    fun a_cached_observation_is_not_described_as_just_now() {
        val line = bound(IssuerBindingState.NotListed, checkedAt = 1_000_000_000L, now = 1_000_600_000L)!!
        assertTrue(line, line.contains("DNS as recorded earlier"))
        assertFalse(line, line.contains("just now"))
    }

    /** A DNS-bearing state with no timestamp cannot say WHEN, so it says nothing about DNS at all. */
    @Test
    fun a_dns_state_without_a_timestamp_makes_no_dns_claim() {
        val line = bound(IssuerBindingState.NotListed, checkedAt = null)!!
        assertFalse(line, line.contains("DNS"))
    }

    /** Nothing to anchor and nothing observed: say nothing rather than imply either. */
    @Test
    fun says_nothing_rather_than_implying_an_anchor_it_does_not_have() {
        assertNull(bound(IssuerBindingState.Pending, block = null, checkedAt = null))
        assertNull(IssuerBinding().provenanceLine())
    }

    @Test
    fun provenance_line_uses_no_verdict_or_alarm_words() {
        for (state in allStates) {
            val line = (bound(state) ?: "").lowercase()
            for (word in listOf("failed", "invalid", "untrusted", "warning", "error")) {
                assertFalse("$state: $line", line.contains(word))
            }
        }
    }

    // ---- copy discipline ------------------------------------------------------------------------

    private val allStates = listOf(
        IssuerBindingState.Verified("Travel clearance issuance"),
        IssuerBindingState.NotADogTagIssuer,
        IssuerBindingState.NotListed,
        IssuerBindingState.CouldNotCheck,
        IssuerBindingState.NoDomainClaimed,
        IssuerBindingState.NoDomainListed,
        IssuerBindingState.Unavailable,
        IssuerBindingState.Pending,
    )

    private fun b(state: IssuerBindingState, domain: String = "moh.gov.sg") =
        IssuerBinding(state = state, domain = domain)

    @Test
    fun no_verdict_or_alarm_words_in_any_state() {
        val forbidden = listOf(
            "verification failed", "failed", "failure", "invalid", "untrusted", "not trusted",
            "warning", "danger", "insecure", "fraud", "fake", "suspicious", "unsafe", "error",
            "rejected",
        )
        for (state in allStates) {
            val line = b(state).line.lowercase()
            for (word in forbidden) {
                assertFalse("state $state line \"$line\" contains \"$word\"", line.contains(word))
            }
        }
    }

    @Test
    fun states_what_was_looked_at_and_what_was_found() {
        assertEquals(
            "This address is listed in moh.gov.sg's DNS records",
            b(IssuerBindingState.Verified("x")).line,
        )
        assertEquals(
            "This address is not listed in moh.gov.sg's DNS records",
            b(IssuerBindingState.NotListed).line,
        )
    }

    /** Same size, same register, an observation either way — the two differ by one word. */
    @Test
    fun verified_and_not_listed_are_symmetric() {
        val yes = b(IssuerBindingState.Verified("x")).line
        val no = b(IssuerBindingState.NotListed).line
        assertEquals(yes.replace("is listed", "is not listed"), no)
    }

    /** A non-clone must NEVER read as merely "not listed in DNS": DNS was never the question. */
    @Test
    fun a_provenance_failure_never_mentions_dns() {
        val line = b(IssuerBindingState.NotADogTagIssuer).line
        assertEquals("This contract was not deployed by the DogTag factory", line)
        assertFalse(line.lowercase().contains("dns"))
        assertFalse(line.lowercase().contains("listed"))
        assertNotEquals(b(IssuerBindingState.NotListed).line, line)
    }

    @Test
    fun could_not_check_blames_our_reach_not_the_issuer() {
        val line = b(IssuerBindingState.CouldNotCheck).line
        assertEquals("We could not reach DNS to check this domain", line)
        assertFalse(line.lowercase().contains("not listed"))
    }

    @Test
    fun every_state_has_distinct_copy() {
        assertEquals(allStates.size, allStates.map { b(it).line }.toSet().size)
    }

    @Test
    fun falls_back_to_a_neutral_phrase_with_no_domain() {
        assertTrue(b(IssuerBindingState.NotListed, domain = "").line.contains("this domain"))
    }

    // ---- tone -----------------------------------------------------------------------------------

    @Test
    fun only_verified_is_positive() {
        assertEquals(BindingTone.Positive, b(IssuerBindingState.Verified("x")).tone)
        for (state in allStates.drop(1)) {
            assertNotEquals("$state must not be positive", BindingTone.Positive, b(state).tone)
        }
    }

    /**
     * A resolver timeout says nothing whatsoever about the issuer, so colouring it as a failure would be a
     * lie of emphasis.
     */
    @Test
    fun the_unknown_states_stay_neutral() {
        assertEquals(BindingTone.Neutral, b(IssuerBindingState.CouldNotCheck).tone)
        assertEquals(BindingTone.Neutral, b(IssuerBindingState.NoDomainClaimed).tone)
        assertEquals(BindingTone.Neutral, b(IssuerBindingState.NoDomainListed).tone)
        assertEquals(BindingTone.Neutral, b(IssuerBindingState.Unavailable).tone)
    }

    @Test
    fun definitive_absences_are_negative() {
        assertEquals(BindingTone.Negative, b(IssuerBindingState.NotListed).tone)
        assertEquals(BindingTone.Negative, b(IssuerBindingState.NotADogTagIssuer).tone)
    }

    @Test
    fun pending_is_the_default_state_and_its_own_tone() {
        val fresh = IssuerBinding()
        assertEquals(IssuerBindingState.Pending, fresh.state)
        assertEquals(BindingTone.Pending, fresh.tone)
        assertFalse(fresh.isVerified)
        assertNull(fresh.blockNumber)
    }

    // ---- the published description ---------------------------------------------------------------

    @Test
    fun the_domain_owners_description_is_surfaced_only_when_verified() {
        assertEquals(
            "Travel clearance issuance",
            b(IssuerBindingState.Verified("Travel clearance issuance")).publishedDescription,
        )
        assertNull(b(IssuerBindingState.NotListed).publishedDescription)
        assertNull(b(IssuerBindingState.Verified("   ")).publishedDescription)
    }

    @Test
    fun unquote_txt_handles_bare_and_escaped_values() {
        assertEquals("plain text", IssuerBindingResolver.unquoteTxt("plain text"))
        assertEquals("quoted", IssuerBindingResolver.unquoteTxt("\"quoted\""))
        assertEquals("a\"b", IssuerBindingResolver.unquoteTxt("\"a\\\"b\""))
    }

    // ---- the DID assertion (the other half of issuer identity) -----------------------------------

    private fun doc(displayedDomain: String, dataIssuer: String?, nested: Boolean = false): WrappedDoc {
        val subject = JSONObject().put("name", "s:0:Max")
        val data = JSONObject().put("credentialSubject", subject)
        if (dataIssuer != null) {
            if (nested) subject.put("issuer", dataIssuer) else data.put("issuer", dataIssuer)
        }
        val root = JSONObject()
            .put("version", "1.0")
            .put("data", data)
            .put(
                "signature",
                JSONObject().put("type", "t").put("targetHash", "0x00")
                    .put("proof", org.json.JSONArray()).put("merkleRoot", "0x00"),
            )
            .put("privacy", JSONObject().put("obfuscated", org.json.JSONArray()))
            .put(
                "issuer",
                JSONObject().put("name", "Example Competent Authority")
                    .put("domain", displayedDomain).put("documentStore", clone)
                    .put("recordType", "TRAVEL_CLEARANCE"),
            )
        return WrappedDoc(root.toString())
    }

    @Test
    fun did_web_host_drops_path_segments_and_ports() {
        assertEquals("example.com", IssuerIdentity.didWebHost("did:web:example.com"))
        assertEquals("example.com", IssuerIdentity.didWebHost("did:web:example.com:dept:vet"))
        assertEquals("example.com", IssuerIdentity.didWebHost("did:web:example.com%3A8443"))
        assertEquals("example.com", IssuerIdentity.didWebHost("did:web:EXAMPLE.com"))
        assertNull(IssuerIdentity.didWebHost("did:key:z6Mk"))
        assertNull(IssuerIdentity.didWebHost("did:web:localhost"))
        assertNull(IssuerIdentity.didWebHost("not a did"))
    }

    @Test
    fun matching_domain_and_did_asserts() {
        val a = IssuerIdentity.assertDomain(doc("gov.example", "abcd:2:did:web:gov.example"))
        assertEquals(IssuerDidAssertion.Match("gov.example"), a)
        assertFalse(a.isMismatch)
    }

    /** The audit's attack, caught by the document alone — no chain, no DNS needed. */
    @Test
    fun the_relabelling_attack_is_detected() {
        val a = IssuerIdentity.assertDomain(doc("moh.gov.sg", "abcd:2:did:web:gov.example"))
        assertEquals(IssuerDidAssertion.Mismatch("moh.gov.sg", "gov.example"), a)
        assertTrue(a.isMismatch)
    }

    @Test
    fun comparison_ignores_case_and_a_trailing_dot() {
        assertEquals(
            IssuerDidAssertion.Match("gov.example"),
            IssuerIdentity.assertDomain(doc("GOV.Example.", "abcd:2:did:web:gov.example")),
        )
    }

    /** A document with no root-covered DID is NOT a pass and NOT a forgery — it is un-assertable. */
    @Test
    fun a_document_without_the_leaf_is_not_assertable_and_not_a_pass() {
        val a = IssuerIdentity.assertDomain(doc("gov.example", null))
        assertEquals(IssuerDidAssertion.NotAssertable, a)
        assertFalse(a.isMismatch)
        assertFalse(a is IssuerDidAssertion.Match)
    }

    @Test
    fun a_bare_unpacked_did_is_still_read() {
        assertEquals(
            IssuerDidAssertion.Match("gov.example"),
            IssuerIdentity.assertDomain(doc("gov.example", "did:web:gov.example")),
        )
    }

    @Test
    fun the_leaf_is_read_from_credential_subject_too() {
        assertEquals(
            IssuerDidAssertion.Match("gov.example"),
            IssuerIdentity.assertDomain(doc("gov.example", "abcd:2:did:web:gov.example", nested = true)),
        )
    }
}

/**
 * Which contract the binding describes — the sharper relabelling attack.
 *
 * The attack: relabel ONLY `issuer.documentStore` to point at ANOTHER authority's real, factory-deployed
 * clone (clone addresses are public on-chain), and leave `data` untouched so integrity still passes.
 * Link 1 (`isClone`) then PASSES — the target genuinely is a factory clone — and a resolver handed the
 * document's claim directly renders that other authority's on-chain name, its claimed domain, and a green
 * DNS badge.
 *
 * The defence is the factory's write-once `rootIssuer[R]`, which names the clone that issued THIS root.
 * [IssuerBindingResolver.chooseClone] is that decision, kept pure so the property is assertable without a
 * network. Mirror of `apps/ios/DogTagTests/IssuerDomainBindingTests.swift`'s
 * `IssuerCloneResolutionTests`.
 */
class IssuerCloneResolutionTest {
    private val ours = "0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f"
    private val otherAuthority = "0x00000000000000000000000000000000000000ee"

    /**
     * THE property. A document naming some other factory clone must not cause that clone to be the one
     * the binding describes.
     */
    @Test
    fun a_swapped_document_store_never_becomes_the_resolved_issuer() {
        val choice = IssuerBindingResolver.chooseClone(RoaxRpc.AddressRead.Value(ours), otherAuthority)
        assertEquals(ours, choice.address)
        assertNotEquals(otherAuthority, choice.address)
        assertEquals(IssuerCloneSource.RootIssuer, choice.source)
        assertTrue("and the swap is REPORTED", choice.documentStoreDiffers)
        assertFalse(choice.readFailed)
    }

    /** An agreeing document is not a disagreement, and address CASE is not evidence of one. */
    @Test
    fun an_agreeing_document_store_is_not_reported_as_a_difference() {
        val choice = IssuerBindingResolver.chooseClone(
            RoaxRpc.AddressRead.Value(ours),
            "  0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F ",
        )
        assertEquals(ours, choice.address)
        assertFalse(choice.documentStoreDiffers)
    }

    /**
     * The factory answered, and its answer is "no record of this root". The document's claim is then the
     * only thing available — used, but never labelled authoritative.
     */
    @Test
    fun no_root_issuer_record_falls_back_to_the_document_and_says_so() {
        val choice = IssuerBindingResolver.chooseClone(RoaxRpc.AddressRead.NoRecord, otherAuthority)
        assertEquals(otherAuthority, choice.address)
        assertEquals(IssuerCloneSource.DocumentClaim, choice.source)
        assertFalse("silence is not disagreement", choice.documentStoreDiffers)
        assertFalse(choice.readFailed)
    }

    /** A read we could not make is not "no record", and must not become a licence to trust the document. */
    @Test
    fun a_failed_root_issuer_read_is_not_an_absence() {
        val choice =
            IssuerBindingResolver.chooseClone(RoaxRpc.AddressRead.Failure("rpc 502"), otherAuthority)
        assertTrue("the caller must report 'could not read', not proceed", choice.readFailed)
        assertFalse(choice.documentStoreDiffers)
    }

    /**
     * The line stays an observation: it says what the chain records, and passes no judgement on the
     * credential, whose validity is proven on-chain separately.
     */
    @Test
    fun the_swapped_store_line_is_factual_and_free_of_verdict_words() {
        val line = IssuerBinding(documentStoreDiffers = true).documentStoreLine
        assertEquals("The chain records a different issuing contract than this document names", line)
        val lowered = line.orEmpty().lowercase()
        val forbidden = listOf(
            "verification failed", "failed", "failure", "invalid", "untrusted", "not trusted",
            "warning", "danger", "insecure", "fraud", "fake", "suspicious", "unsafe", "error",
            "rejected",
        )
        for (word in forbidden) {
            assertFalse("\"$lowered\" contains \"$word\"", lowered.contains(word))
        }
        // Nothing to say when the two agree.
        assertNull(IssuerBinding().documentStoreLine)
    }

    // ---- the address word ----------------------------------------------------------------------

    @Test
    fun decodes_a_right_aligned_address_word() {
        val word = "0".repeat(24) + "b5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f"
        assertEquals(ours, RoaxRpc.decodeAbiAddress(word))
        assertEquals(ours, RoaxRpc.decodeAbiAddress("0x" + word.uppercase()))
    }

    /**
     * An EMPTY result is what a call to an address with no code returns. It is not the zero address, and
     * collapsing the two would turn "we could not ask" into a confident "no record".
     */
    @Test
    fun an_unreadable_word_is_null_rather_than_a_guess() {
        assertNull(RoaxRpc.decodeAbiAddress(""))
        assertNull(RoaxRpc.decodeAbiAddress("0x00"))
        // dirty high bytes: not an address word
        assertNull(RoaxRpc.decodeAbiAddress("1".repeat(64)))
        assertNull(RoaxRpc.decodeAbiAddress("z".repeat(64)))
    }

    @Test
    fun the_zero_address_is_an_unset_slot() {
        assertTrue(RoaxRpc.isZeroAddress("0x" + "0".repeat(40)))
        assertFalse(RoaxRpc.isZeroAddress(ours))
        assertFalse(RoaxRpc.isZeroAddress(""))
    }
}
