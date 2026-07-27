import { describe, expect, it } from "vitest";
import {
  bindingExplanation,
  bindingLine,
  bindingProvenanceLine,
  bindingTone,
  displayIssuerName,
  expectedTxtName,
  isDomainVerified,
  type IssuerDomainBinding,
  type IssuerDomainBindingState,
} from "../src/domain/issuerDomainBinding";

const ALL_STATES: IssuerDomainBindingState[] = [
  "verified",
  "notADogTagIssuer",
  "notListed",
  "couldNotCheck",
  "noDomainClaimed",
  "unavailable",
  "pending",
];

const b = (state: IssuerDomainBindingState, extra: Partial<IssuerDomainBinding> = {}) =>
  ({ state, domain: "moh.gov.sg", ...extra }) as IssuerDomainBinding;

describe("tone", () => {
  it("makes only verified positive and only notListed negative", () => {
    expect(bindingTone("verified")).toBe("positive");
    expect(bindingTone("notListed")).toBe("negative");
  });

  // The captain's explicit instruction: a resolver timeout is not evidence either way, so it must be
  // neither green nor red. Colouring it as a failure would be a lie of emphasis.
  it("keeps could-not-check neutral, never negative", () => {
    expect(bindingTone("couldNotCheck")).toBe("neutral");
    expect(bindingTone("couldNotCheck")).not.toBe("negative");
    expect(bindingTone("couldNotCheck")).not.toBe("positive");
  });

  // Link 1 failing is a categorically stronger statement than a missing DNS record. It is red, like
  // notListed, but its COPY must not be confusable — see the copy-discipline block.
  it("gives a provenance failure a negative tone", () => {
    expect(bindingTone("notADogTagIssuer")).toBe("negative");
  });

  it("keeps the other unknown states neutral too", () => {
    expect(bindingTone("noDomainClaimed")).toBe("neutral");
    expect(bindingTone("unavailable")).toBe("neutral");
  });

  it("never gives a non-verified state a positive tone", () => {
    for (const s of ALL_STATES.filter((s) => s !== "verified")) {
      expect(bindingTone(s)).not.toBe("positive");
    }
  });
});

describe("copy discipline", () => {
  // The load-bearing guard. The copy must state the OBSERVATION, never a verdict: a missing DNS record
  // does not mean the credential is bad, and the credential's validity is separately proven on-chain.
  // Telling a user their perfectly valid credential "FAILED" is worse than showing nothing.
  const FORBIDDEN = [
    "verification failed",
    "verification fail",
    "failed",
    "failure",
    "invalid",
    "untrusted",
    "not trusted",
    "warning",
    "danger",
    "insecure",
    "fraud",
    "fake",
    "suspicious",
    "unsafe",
    "error",
    "rejected",
  ];

  it("uses no verdict or alarm words in any state's line", () => {
    for (const s of ALL_STATES) {
      const line = bindingLine(b(s, { description: "Travel clearance issuance" })).toLowerCase();
      for (const word of FORBIDDEN) {
        expect(line, `state "${s}" line: "${line}"`).not.toContain(word);
      }
    }
  });

  it("uses no verdict or alarm words in any state's explanation", () => {
    for (const s of ALL_STATES) {
      const text = bindingExplanation(b(s)).toLowerCase();
      for (const word of FORBIDDEN) {
        expect(text, `state "${s}" explanation: "${text}"`).not.toContain(word);
      }
    }
  });

  it("states what was looked at and what was found", () => {
    expect(bindingLine(b("verified"))).toBe("This address is listed in moh.gov.sg's DNS records");
    expect(bindingLine(b("notListed"))).toBe(
      "This address is not listed in moh.gov.sg's DNS records",
    );
  });

  // The two states differ by one word, which is the point: same size, same register, an observation
  // either way. Neither is a badge of trust or of distrust.
  it("keeps verified and notListed symmetric", () => {
    const yes = bindingLine(b("verified"));
    const no = bindingLine(b("notListed"));
    expect(no).toBe(yes.replace("is listed", "is not listed"));
  });

  it("describes could-not-check as our failure to reach DNS, not the issuer's", () => {
    const line = bindingLine(b("couldNotCheck"));
    expect(line).toBe("We could not reach DNS to check this domain");
    expect(line.toLowerCase()).not.toContain("not listed");
  });

  it("never claims the organisation's identity was verified by us", () => {
    const verified = bindingExplanation(b("verified")).toLowerCase();
    expect(verified).toContain("domain owner vouches");
    expect(verified).toContain("not a check of the organisation's identity");
  });

  it("says an absent record is unremarkable rather than a problem", () => {
    const text = bindingExplanation(b("notListed")).toLowerCase();
    expect(text).toContain("most issuers have not published one");
    expect(text).toContain("proven on-chain");
  });

  // The captain's instruction: a non-clone must NEVER render as merely "not listed in DNS". The two are
  // different claims about different things.
  it("never describes a provenance failure in terms of DNS", () => {
    const line = bindingLine(b("notADogTagIssuer"));
    expect(line).toBe("This contract was not deployed by the DogTag factory");
    expect(line.toLowerCase()).not.toContain("dns");
    expect(line.toLowerCase()).not.toContain("listed");
    expect(line).not.toBe(bindingLine(b("notListed")));
  });

  it("gives every state distinct copy, so no two states read the same", () => {
    const lines = ALL_STATES.map((s) => bindingLine(b(s)));
    expect(new Set(lines).size).toBe(ALL_STATES.length);
  });

  it("falls back to a neutral phrase when the domain is unknown", () => {
    expect(bindingLine({ state: "notListed" })).toContain("this domain");
  });
});

describe("isDomainVerified", () => {
  it("is true only for verified", () => {
    expect(isDomainVerified(b("verified"))).toBe(true);
    for (const s of ALL_STATES.filter((s) => s !== "verified")) {
      expect(isDomainVerified(b(s)), s).toBe(false);
    }
    expect(isDomainVerified(null)).toBe(false);
    expect(isDomainVerified(undefined)).toBe(false);
  });
});

describe("expectedTxtName", () => {
  it("builds the normative name, lowercasing an EIP-55 address", () => {
    expect(
      expectedTxtName("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F", "MOH.GOV.SG."),
    ).toBe("0xb5d6654d8b29096c8fcf71d24bbe6f6de86c5f9f._dogtag.moh.gov.sg");
  });

  it("keeps the address label inside the 63-octet DNS label limit", () => {
    const name = expectedTxtName("0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F", "moh.gov.sg");
    expect(name.split(".")[0].length).toBe(42);
    expect(name.split(".")[0].length).toBeLessThanOrEqual(63);
  });
});

describe("displayIssuerName", () => {
  // The document's issuer block is outside the Merkle root. Rendering it as the issuer is what let the
  // audit's relabelled credential display "Ministry of Health of Singapore".
  it("prefers the authoritative on-chain name", () => {
    expect(
      displayIssuerName({
        onchainName: "DogTag Government Authority",
        onchainNameAvailable: true,
        documentName: "Ministry of Health of Singapore",
      }),
    ).toEqual({ name: "DogTag Government Authority", authoritative: true });
  });

  it("marks a document-name fallback as NOT authoritative", () => {
    expect(
      displayIssuerName({
        onchainName: null,
        onchainNameAvailable: false,
        documentName: "Ministry of Health of Singapore",
      }),
    ).toEqual({ name: "Ministry of Health of Singapore", authoritative: false });
  });

  it("never claims authority for an empty on-chain name", () => {
    const r = displayIssuerName({ onchainName: "   ", onchainNameAvailable: true, documentName: "X" });
    expect(r.authoritative).toBe(false);
  });

  it("degrades to a neutral placeholder rather than an empty string", () => {
    expect(displayIssuerName(null)).toEqual({ name: "Unknown issuer", authoritative: false });
  });
});

describe("bindingProvenanceLine", () => {
  // A verdict without a "when" is not auditable against a mutable world.
  it("names the block the chain half was read at", () => {
    expect(bindingProvenanceLine(b("verified", { blockNumber: 283207 }))).toContain(
      "chain read at block 283207",
    );
  });

  // THE asymmetry. Chain state is reproducible with an archive node; DNS has no history at all, so a
  // DNS answer can only be recorded, never recomputed. The copy must say so rather than let a reader
  // assume the DNS half is as replayable as the chain half.
  it("states that DNS has no history and cannot be re-checked for the past", () => {
    const live = bindingProvenanceLine(b("verified", { blockNumber: 1, dnsObservation: "live" }))!;
    expect(live).toContain("DNS checked just now");
    expect(live).toContain("DNS has no history");
  });

  it("distinguishes a recorded observation from a live one", () => {
    const stored = bindingProvenanceLine(b("verified", { blockNumber: 1, dnsObservation: "stored" }))!;
    expect(stored).toContain("as recorded earlier");
    expect(stored).not.toContain("just now");
  });

  it("claims no DNS provenance for states that never made a DNS query", () => {
    for (const s of ["notADogTagIssuer", "noDomainClaimed", "unavailable"] as const) {
      const line = bindingProvenanceLine(b(s, { blockNumber: 5 }));
      expect(line, s).not.toContain("DNS");
    }
  });

  it("says nothing rather than implying an anchor it does not have", () => {
    expect(bindingProvenanceLine({ state: "noDomainClaimed" })).toBeNull();
    expect(bindingProvenanceLine({ state: "noDomainClaimed", blockNumber: null })).toBeNull();
  });

  it("uses no verdict or alarm words", () => {
    for (const s of ALL_STATES) {
      const line = (bindingProvenanceLine(b(s, { blockNumber: 9 })) ?? "").toLowerCase();
      for (const word of ["failed", "invalid", "untrusted", "warning", "error"]) {
        expect(line, `${s}: ${line}`).not.toContain(word);
      }
    }
  });
});
