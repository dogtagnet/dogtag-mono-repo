// Why a control on the provider page cannot be used (registry-plan S-15).
//
// The pure half of the first-run defect: Deploy is gated on a plan, no plan exists before the first
// check, and nothing said so. `providerSelfServiceExplains.test.tsx` pins that the page RENDERS a
// reason; this pins that the reason is the RIGHT one, which is the half a rendering test cannot
// reach - it would have to reproduce six situations in a browser to distinguish six sentences.
//
// The ordering cases are the ones that matter most. Every reason here is individually plausible, so
// a wrong precedence does not look wrong; it just sends somebody to fix the second thing first.
import { describe, expect, it } from "vitest";
import {
  checkBlock,
  describeActionBlock,
  planGateState,
  sendBlock,
  type ActionBlock,
} from "../src/provider";

const OK = { busy: false, connected: true, hasReader: true };
const ON_CHAIN = { expected: 135, actual: 135 };

describe("planGateState tells never-checked apart from every other empty state", () => {
  const base = { present: true, spent: false, keyMatches: true, canAct: true };

  it("answers never when no plan is held, which is every provider's first minute", () => {
    expect(planGateState({ ...base, present: false, canAct: false })).toBe("never");
  });

  it("keeps never and edited apart - they send a user to opposite places", () => {
    // "Your answers are out of date" describes answers a first-time provider has never seen.
    expect(planGateState({ ...base, present: false, canAct: false })).toBe("never");
    expect(planGateState({ ...base, keyMatches: false, canAct: false })).toBe("edited");
  });

  it("reports spent before edited, matching how a plan is retired", () => {
    // A submitted transaction falsifies a plan whose form is untouched. Calling that an edit
    // describes a change the user did not make.
    expect(planGateState({ ...base, spent: true, keyMatches: true, canAct: false })).toBe("spent");
    expect(planGateState({ ...base, spent: true, keyMatches: false, canAct: false })).toBe("spent");
  });

  it("distinguishes a check that refused from one that could not resolve", () => {
    expect(planGateState({ ...base, canAct: false, verdict: "refused" })).toBe("refused");
    expect(planGateState({ ...base, canAct: false, verdict: "indeterminate" })).toBe(
      "indeterminate",
    );
  });

  it("is ready only when the plan itself permits the action", () => {
    expect(planGateState({ ...base })).toBe("ready");
  });
});

describe("the reasons are ordered by what to fix first", () => {
  const send = (over: Partial<Parameters<typeof sendBlock>[0]> = {}) =>
    sendBlock({ ...OK, chain: ON_CHAIN, check: "Check", plan: "never", ...over });

  it("puts busy above everything, because it is transient", () => {
    // Telling somebody to connect a wallet they have already connected, because a request is in
    // flight, is worse than saying nothing.
    expect(send({ busy: true, connected: false, plan: "refused" })?.kind).toBe("busy");
  });

  it("puts connect-your-wallet above run-the-check", () => {
    // The reverse order sends a disconnected user to press a button that cannot do anything.
    expect(send({ connected: false })?.kind).toBe("notConnected");
  });

  it("puts the chain above the plan, since no plan can fix a wrong network", () => {
    expect(send({ chain: { expected: 135, actual: 1 }, plan: "ready" })?.kind).toBe("wrongChain");
  });

  it("reports a flow-specific refusal only once the plan itself is ready", () => {
    // Otherwise a provider is told to fix a form field while the real blocker is that they have
    // not run the check at all.
    expect(send({ plan: "never", otherwiseBlocked: "bad domain" })?.kind).toBe("neverChecked");
    expect(send({ plan: "ready", otherwiseBlocked: "bad domain" })?.kind).toBe("otherwiseBlocked");
  });

  it("blocks nothing when everything is in order", () => {
    expect(send({ plan: "ready" })).toBeNull();
  });
});

describe("an unreported chain is not the wrong chain", () => {
  it("does not accuse a connector that reported no chain id", () => {
    // Could-not-check rendered as a definite answer is the defect this whole page is written
    // against. A wallet that reports nothing may be perfectly correct.
    const block = sendBlock({
      ...OK,
      chain: { expected: 135, actual: undefined },
      check: "Check",
      plan: "ready",
    });
    expect(block).toBeNull();
  });

  it("still catches a chain that IS reported and differs", () => {
    // The half that keeps the case above from being a blanket exemption.
    expect(
      sendBlock({ ...OK, chain: { expected: 135, actual: 1 }, check: "Check", plan: "ready" })?.kind,
    ).toBe("wrongChain");
  });
});

describe("a Check button's gate is narrower than a send's", () => {
  // The "no chain term" half is STRUCTURAL rather than behavioural: `checkBlock` takes no chain, so
  // no mutation of this module can add one and no test here can pin its absence. What pins it is the
  // COMPONENT - `providerSelfServiceExplains.test.tsx` asserts a wrong-chain wallet leaves the Check
  // button with no reason while the send has one. Stated because a test named for a property it
  // cannot fail is worse than no test.
  it("blocks nothing when its own preconditions are met", () => {
    expect(checkBlock({ ...OK })).toBeNull();
  });

  it("still reports the preconditions it does have", () => {
    expect(checkBlock({ ...OK, connected: false })?.kind).toBe("notConnected");
    expect(checkBlock({ ...OK, missingInput: "Enter your provider id." })?.kind).toBe(
      "missingInput",
    );
  });
});

describe("every reason names an action rather than only an obstacle", () => {
  const ALL: ActionBlock[] = [
    { kind: "busy" },
    { kind: "notConnected" },
    { kind: "readerUnavailable" },
    { kind: "wrongChain", expected: 135, actual: 1 },
    { kind: "missingInput", needs: "Enter your provider id to check." },
    { kind: "neverChecked", check: "Check what this would deploy" },
    { kind: "planEdited", check: "Check what this would deploy" },
    { kind: "planSpent", check: "Check what this would deploy" },
    { kind: "planRefused" },
    { kind: "planIndeterminate" },
    { kind: "otherwiseBlocked", why: "No content mirror is configured." },
  ];

  it("describes all of them, with no empty sentence", () => {
    // A kind added later without a sentence would render an empty line under a dead button, which
    // is the original defect wearing a different hat.
    for (const b of ALL) {
      const text = describeActionBlock(b);
      expect(text.trim().length, `${b.kind} has no sentence`).toBeGreaterThan(10);
    }
  });

  it("gives no two kinds the same sentence", () => {
    // Six situations, six remedies. Collapsing any pair fixes the silence and keeps the part that
    // costs time.
    const texts = ALL.map(describeActionBlock);
    expect(new Set(texts).size).toBe(ALL.length);
  });

  it("names the check to press, and that the check must match the form as it is now", () => {
    const text = describeActionBlock({ kind: "neverChecked", check: "Check what this would deploy" });
    expect(text).toMatch(/Check what this would deploy/);
    expect(text).toMatch(/match the values in the form right now/i);
  });

  it("names both chains, so a wrong network is actionable rather than merely wrong", () => {
    const text = describeActionBlock({ kind: "wrongChain", expected: 135, actual: 1 });
    expect(text).toMatch(/\b1\b/);
    expect(text).toMatch(/\b135\b/);
    // Says the checks are unaffected, so nobody re-runs them hunting for the cause.
    expect(text).toMatch(/checks above are unaffected/i);
  });
});
