// The pure decisions behind the service half of the registrar screen.
//
// These are rules about WHAT THE SCREEN SAYS, which is exactly the class of thing worth pinning
// independently of layout: a lifecycle term rendered as the wrong tone, or a preflight that refuses
// what the chain would accept, are both defects no typecheck can see.
import { describe, expect, it } from "vitest";
import {
  attachKey,
  attachSendable,
  effectiveTerms,
  issuanceBlocker,
  preflightPermitsSend,
  resolvedGenerationId,
  resolvedOwner,
  termTone,
} from "../src/lib/providerServices";
import type { AttachPreflightResp, ServiceEffective } from "@dogtag/ui";

const ALL_HELD: ServiceEffective = {
  providerStanding: "active",
  serviceStanding: "active",
  factoryActive: true,
  ownerConfirmed: true,
  hasActiveIssuer: true,
};

function preflight(over: Partial<AttachPreflightResp>): AttachPreflightResp {
  return {
    registry: "0xreg",
    providerId: "0xpid",
    serviceAddress: "0xsvc",
    alreadyAttached: null,
    generation: { state: "resolved", generationId: "0xgen", factory: "0xfac" },
    metadata: { state: "resolved", owner: "0xowner", recordTypeKey: "0xrt", recordType: "VACCINATION" },
    verdict: "ready",
    reason: "",
    ...over,
  } as AttachPreflightResp;
}

describe("the five lifecycle terms are reported APART", () => {
  it("gives every term its own remedy, because each has a different fix", () => {
    const terms = effectiveTerms(ALL_HELD);
    expect(terms).toHaveLength(5);
    const remedies = new Set(terms.map((t) => t.remedy));
    expect(remedies.size, "a shared remedy would make the split pointless").toBe(5);
  });

  it("all five held means nothing is blocking issuance", () => {
    expect(issuanceBlocker(effectiveTerms(ALL_HELD))).toBeNull();
  });

  // The whole reason the terms are not pre-ANDed: the blocker names an ACTIONABLE remedy rather
  // than reporting that something, somewhere, is wrong.
  it("names the failing term's own remedy rather than a generic failure", () => {
    const blocker = issuanceBlocker(
      effectiveTerms({ ...ALL_HELD, serviceStanding: "pending" }),
    );
    expect(blocker).toContain("standing to Active");
    const owner = issuanceBlocker(effectiveTerms({ ...ALL_HELD, ownerConfirmed: false }));
    expect(owner).toContain("confirmServiceOwner");
    expect(owner).not.toEqual(blocker);
  });

  // An unreadable read is could-not-check, never "no". Rendering it as a definite failure would
  // accuse a healthy service on the strength of a read that never happened.
  it("an unavailable read yields all five as null, never as false", () => {
    const terms = effectiveTerms({ unavailable: "rpc died" });
    expect(terms.every((t) => t.held === null)).toBe(true);
    expect(terms.some((t) => t.held === false)).toBe(false);
    expect(issuanceBlocker(terms)).toContain("not established");
  });

  /**
   * `hasActiveIssuer` is the one term the chain does not answer independently: it re-folds the
   * owner, both standings AND the provider's current pointer. So it is the state a registrar
   * reaches when it has finished, and the remedy that unblocks it is the PROVIDER's repoint - a
   * sentence naming another registrar grant sends the admin back to the step they just completed.
   */
  it("names the provider's repoint when the pointer is what is missing", () => {
    const terms = effectiveTerms({ ...ALL_HELD, hasActiveIssuer: false }, {
      state: "resolved",
      service: `0x${"0".repeat(40)}`,
      isCurrent: false,
    });
    const blocker = issuanceBlocker(terms);
    expect(blocker).toContain("repoint");
    expect(blocker).toContain("provider");
    expect(blocker, "the registrar cannot fix this by granting again").not.toContain(
      "Grant issuance capability",
    );
  });

  /**
   * The pointer is TRI-state, so the remedy is too. An unreadable pointer read leaves both causes
   * possible, and naming either would be a definite remedy derived from a read that never happened
   * - beside a `service-pointer` line that already says the read did not complete.
   */
  it("asserts no remedy at all when the pointer read itself failed", () => {
    const blocker = issuanceBlocker(
      effectiveTerms({ ...ALL_HELD, hasActiveIssuer: false }, {
        state: "unavailable",
        reason: "rpc died",
      }),
    );
    expect(blocker).toContain("pointer read did not complete");
    expect(blocker, "we did not establish that nobody is granted").not.toContain(
      "Grant issuance capability",
    );
    expect(blocker, "nor that the provider failed to repoint").not.toContain("Only the provider");
  });

  it("falls back to the grant remedy when the provider HAS repointed", () => {
    const blocker = issuanceBlocker(
      effectiveTerms({ ...ALL_HELD, hasActiveIssuer: false }, {
        state: "resolved",
        service: "0xsvc",
        isCurrent: true,
      }),
    );
    expect(blocker).toContain("Grant issuance capability");
  });

  // The pointer informs the REMEDY only. Folding it into a term's `held` would make one of the five
  // stop meaning what the wire says it means, and the chain already folds it inside hasActiveIssuer.
  it("never lets the pointer change which terms are held", () => {
    const withPointer = effectiveTerms(ALL_HELD, {
      state: "resolved",
      service: `0x${"0".repeat(40)}`,
      isCurrent: false,
    });
    expect(withPointer.map((t) => t.held)).toEqual(effectiveTerms(ALL_HELD).map((t) => t.held));
    expect(issuanceBlocker(withPointer)).toBeNull();
  });

  it("tones a could-not-establish term as a warning, never as the failure red", () => {
    expect(termTone(null)).toBe("warning");
    expect(termTone(false)).toBe("danger");
    expect(termTone(true)).toBe("success");
  });
});

describe("the attach preflight is never stricter than the chain", () => {
  // The `cloneProvenance.ts` lesson: a preflight that refuses what the contract accepts is a worse
  // defect than no preflight, because the admin has no way to override it.
  it("a couldNotRun verdict still permits the send", () => {
    expect(preflightPermitsSend(preflight({ verdict: "couldNotRun" }))).toBe(true);
  });

  it("only a definite refusal blocks the send", () => {
    expect(preflightPermitsSend(preflight({ verdict: "refused" }))).toBe(false);
    expect(preflightPermitsSend(preflight({ verdict: "ready" }))).toBe(true);
    expect(preflightPermitsSend(null), "nothing checked yet").toBe(false);
  });

  // ...but the calldata still cannot be built from nothing. A `couldNotRun` whose missing half is
  // one of the two values a transaction must address is offered as a RETRY, not as a send.
  it("refuses to send when the generation or owner could not be resolved", () => {
    expect(
      attachSendable(
        preflight({ verdict: "couldNotRun", generation: { state: "unavailable", reason: "rpc" } }),
      ),
      "no generation id to address the transaction to",
    ).toBe(false);
    expect(
      attachSendable(
        preflight({ verdict: "couldNotRun", metadata: { state: "unavailable", reason: "rpc" } }),
      ),
      "no owner to guard the transaction with",
    ).toBe(false);
    expect(attachSendable(preflight({})), "both resolved").toBe(true);
  });

  it("resolves the two send values only from a resolved read", () => {
    expect(resolvedGenerationId(preflight({}))).toBe("0xgen");
    expect(resolvedOwner(preflight({}))).toBe("0xowner");
    expect(resolvedGenerationId(preflight({ generation: { state: "none", reason: "x" } }))).toBeNull();
    expect(resolvedOwner(preflight({ metadata: { state: "refused", reason: "x" } }))).toBeNull();
  });
});

describe("a checked plan authorises a send of the values it checked", () => {
  it("changing the address retires the plan", () => {
    expect(attachKey("0xpid", "0xa")).not.toBe(attachKey("0xpid", "0xb"));
  });

  // The provider is in the key because the preflight's answer depends on it - the same address may
  // be attachable for one provider and already bound for another.
  it("changing the provider retires it too", () => {
    expect(attachKey("0xp1", "0xa")).not.toBe(attachKey("0xp2", "0xa"));
  });

  it("is insensitive to case and surrounding whitespace, which change nothing on chain", () => {
    expect(attachKey("0xPID", " 0xABC ")).toBe(attachKey("0xpid", "0xabc"));
  });
});
