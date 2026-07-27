package io.liberalize.dogtag.net

import io.liberalize.dogtag.data.WrappedDoc
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
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

    // ---- copy discipline ------------------------------------------------------------------------

    private val allStates = listOf(
        IssuerBindingState.Verified("Travel clearance issuance"),
        IssuerBindingState.NotADogTagIssuer,
        IssuerBindingState.NotListed,
        IssuerBindingState.CouldNotCheck,
        IssuerBindingState.NoDomainClaimed,
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
