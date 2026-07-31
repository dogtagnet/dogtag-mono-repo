// The adversarial half of the verification bench: fraudulent records driven through the REAL verify
// path, each asserting WHICH check refused it.
//
// Every case asserts the FULL outcome vector, never a single row. "Something went red" is satisfied by
// a fixture that happens to be expired, or by a chain that could not be reached - so a suite built on
// it stays green while the check it is named after is dead. The vector is what makes a scenario that
// starts passing for a different reason fail.
//
// No network: every scenario carries its own scripted, address-keyed chain (see `benchScenarios.ts`).
import { describe, expect, it } from "vitest";
import {
  BENCH_SCENARIOS,
  batchInclusionProof,
  foreignRegistry,
  genuineCredential,
  hostileIssuerContract,
  relabelledRecordType,
  revokedPresentedAsLive,
  runBenchScenario,
  signerDelistedAfterIssuance,
  signerDelistedBeforeIssuance,
  tamperedCoveredField,
  unanchoredSelfConsistentForgery,
  wrongChainEndpoint,
  type BenchScenario,
} from "../src/wallet/benchScenarios";
import type { BenchCheckId, BenchReport, CheckOutcome } from "../src/wallet/verificationBench";

const outcomes = (r: BenchReport) =>
  Object.fromEntries(r.checks.map((c) => [c.id, c.outcome])) as Record<BenchCheckId, CheckOutcome>;

const outcome = (r: BenchReport, id: BenchCheckId): CheckOutcome => {
  const c = r.checks.find((x) => x.id === id);
  if (!c) throw new Error(`no check ${id} in report`);
  return c.outcome;
};

// ── the catalogue's own contract ────────────────────────────────────────────────────────────────

describe("every scenario declares a complete, self-consistent expectation", () => {
  it("expects an outcome for EVERY check the bench emits - no partial vectors", async () => {
    const emitted = (await runBenchScenario(genuineCredential)).checks.map((c) => c.id).sort();
    for (const s of BENCH_SCENARIOS) {
      expect(Object.keys(s.expected).sort(), `${s.id} expectation is not exhaustive`).toEqual(emitted);
    }
  });

  it("names only real check ids, and never both refuses and excuses the same check", () => {
    const ids = new Set(Object.keys(genuineCredential.expected) as BenchCheckId[]);
    for (const s of BENCH_SCENARIOS) {
      for (const id of s.refusedBy) {
        expect(ids, `${s.id} refusedBy ${id}`).toContain(id);
        expect(s.expected[id], `${s.id} says ${id} refuses it but expects a non-fail`).toBe("fail");
      }
      for (const b of s.blindSpots) {
        expect(ids, `${s.id} blindSpot ${b.id}`).toContain(b.id);
        expect(s.refusedBy, `${s.id} cannot both be refused by and blind to ${b.id}`).not.toContain(b.id);
        expect(b.why.length, `${s.id} blindSpot ${b.id} needs a real reason`).toBeGreaterThan(20);
      }
    }
  });

  it("a scenario that must verify names no refusing check, and vice versa", () => {
    for (const s of BENCH_SCENARIOS) {
      if (s.mustVerify) expect(s.refusedBy, `${s.id}`).toEqual([]);
    }
  });

  it("keeps `expected` as the CORRECT answer - a scenario that must verify never expects a refusal", () => {
    // The catalogue's honesty rule, made mechanical. An expectation edited down to whatever the
    // implementation prints is a test that certifies the bug, so where the code is wrong the
    // expectation stays right and the gap goes in `knownDefect`.
    for (const s of BENCH_SCENARIOS) {
      if (!s.mustVerify) continue;
      const refusals = (Object.entries(s.expected) as Array<[BenchCheckId, CheckOutcome]>)
        .filter(([, o]) => o === "fail")
        .map(([id]) => id);
      expect(
        refusals,
        `${s.id} must verify, so its EXPECTATION may not carry a failure - record the defect in knownDefect instead`,
      ).toEqual([]);
      expect(s.expectedVerdict, `${s.id} must verify, so its expected verdict is true`).toBe(true);
    }
  });

  it("declares a known defect for exactly the case the implementation gets wrong", () => {
    expect(BENCH_SCENARIOS.filter((s) => s.knownDefect).map((s) => s.id)).toEqual([
      "signer-delisted-after-issuance",
    ]);
    for (const s of BENCH_SCENARIOS) {
      if (!s.knownDefect) continue;
      // A defect that does not actually differ from the expectation is not a defect - it is a stale
      // field that would keep reporting a fixed bug forever.
      expect(
        s.knownDefect.observed,
        `${s.id} declares a defect identical to its expectation`,
      ).not.toEqual(s.expected);
      expect(s.knownDefect.statement.length).toBeGreaterThan(80);
    }
  });
});

describe("every scenario's run matches its declared expectation, row for row", () => {
  for (const s of BENCH_SCENARIOS) {
    it(`${s.id}: ${s.title}`, async () => {
      const r = await runBenchScenario(s);
      if (s.knownDefect) {
        // A DOCUMENTED DEFECT. Both halves are asserted: what the code does today (so a change is
        // noticed) AND that it still differs from the correct answer (so the day it is fixed this
        // goes red and whoever fixed it must delete the field, rather than the finding evaporating).
        expect(
          outcomes(r),
          `${s.id}: the documented defect no longer reproduces. If the verifier was fixed, DELETE knownDefect and let the expectation stand.`,
        ).toEqual(s.knownDefect.observed);
        expect(r.verdict).toBe(s.knownDefect.observedVerdict);
        expect(
          outcomes(r),
          `${s.id}: observed behaviour now matches the correct expectation - the defect is fixed, so remove knownDefect`,
        ).not.toEqual(s.expected);
        return;
      }
      expect(outcomes(r), `${s.id} diverged from its declared vector`).toEqual(s.expected);
      expect(r.verdict, `${s.id} verdict`).toBe(s.expectedVerdict);
      // The named refusing checks are asserted individually too, so a vector that drifts wholesale
      // cannot take the scenario's whole point with it silently.
      for (const id of s.refusedBy) expect(outcome(r, id), `${s.id} expected ${id} to refuse`).toBe("fail");
    });
  }
});

// ── the individual frauds, each naming the check that catches it ────────────────────────────────

describe("a tampered field is caught by integrity ALONE", () => {
  it("goes red on the recompute while every chain row stays green", async () => {
    const r = await runBenchScenario(tamperedCoveredField);
    expect(outcome(r, "integrity")).toBe("fail");
    // The chain is asked about the UNCHANGED claimed root, so it rightly still vouches for it. A
    // surface reporting only the chain's answer would call this credential valid.
    expect(outcome(r, "anchored-on-chain")).toBe("pass");
    expect(outcome(r, "not-revoked")).toBe("pass");
    expect(outcome(r, "issuer-whitelisted")).toBe("pass");
    expect(r.verdict).toBe(false);
    expect(r.response?.status).toBe("integrity_failed");
  });
});

describe("a hostile issuer contract is refused by the factory anchor", () => {
  it("fails the anchor row and never asks the attacker's contract anything", async () => {
    const r = await runBenchScenario(hostileIssuerContract);
    expect(outcome(r, "issuer-descends-from-factory")).toBe("fail");
    // Not one read may reach the address the document nominated: the whole defence is that the
    // hostile contract - which would answer `isValid = true` - is never consulted.
    const hostile = "0x000000000000000000000000000000000000ba0b";
    expect(r.reads.some((x) => x.contract.toLowerCase() === hostile)).toBe(false);
    // ...and no downstream row may soften into a pass on the strength of a read never made.
    expect(outcome(r, "not-revoked")).toBe("could-not-run");
    expect(outcome(r, "anchored-on-chain")).toBe("could-not-run");
    expect(outcome(r, "issuer-whitelisted")).toBe("could-not-run");
    expect(r.verdict).toBe(false);
  });

  it("PASSES integrity - which is exactly why the anchor row has to exist", async () => {
    const r = await runBenchScenario(hostileIssuerContract);
    expect(outcome(r, "integrity")).toBe("pass");
  });
});

describe("a relabelled record type is refused by the whitelist pillar", () => {
  it("fails on the record type the CLONE declares, not the one the document claims", async () => {
    const r = await runBenchScenario(relabelledRecordType);
    expect(outcome(r, "issuer-whitelisted")).toBe("fail");
    // Everything else about the credential is genuine, so nothing else may object.
    expect(outcome(r, "integrity")).toBe("pass");
    expect(outcome(r, "issuer-descends-from-factory")).toBe("pass");
    expect(outcome(r, "anchored-on-chain")).toBe("pass");
    // The historical row CANNOT run, and that is the right answer rather than a gap: the verifier
    // refuses a relabel before it resolves a signer, so no signer's grant history exists to read.
    // Answering "authorised" from the clone's own signer would read as excusing the relabel.
    expect(outcome(r, "whitelisted-at-issuance")).toBe("could-not-run");
    expect(
      r.checks.find((c) => c.id === "whitelisted-at-issuance")?.couldNotRunReason,
    ).toContain("before resolving a signer");
    expect(r.verdict).toBe(false);
  });
});

// ── the delisting pair: forward-only, and what the verifier actually does about it ───────────────

describe("delisting BEFORE the issuance block - MUST be refused", () => {
  it("is refused, and the scenario declares that refusal as the CORRECT outcome", async () => {
    // Direction one of the pair, asserted as a requirement rather than an observation.
    expect(signerDelistedBeforeIssuance.mustVerify).toBe(false);
    expect(signerDelistedBeforeIssuance.expectedVerdict).toBe(false);
    expect(signerDelistedBeforeIssuance.knownDefect).toBeUndefined();
    const r = await runBenchScenario(signerDelistedBeforeIssuance);
    expect(r.verdict).toBe(false);
  });

  it("is refused by both whitelist rows, for two different reasons", async () => {
    const r = await runBenchScenario(signerDelistedBeforeIssuance);
    // The gating row: not authorised NOW.
    expect(outcome(r, "issuer-whitelisted")).toBe("fail");
    // The historical row: not authorised THEN - the reason that makes the record fraudulent rather
    // than merely stale, because `issue()` is `onlyWhitelisted` and could not have run in that state.
    expect(outcome(r, "whitelisted-at-issuance")).toBe("fail");
    const historical = r.checks.find((c) => c.id === "whitelisted-at-issuance");
    expect(historical?.finding).toContain("DELISTED before this root was anchored");
    expect(r.verdict).toBe(false);
  });
});

describe("delisting AFTER the issuance block - the finding", () => {
  // `DogTagIssuer.sol:82` states the rule in the contract's own words: "delisting is forward-only".
  // `adminRevoke` exists as the retroactive lever precisely because `delistFor` is not one. So this
  // credential is genuine and the brief says it must still verify.
  it("is a GENUINE credential the historical row correctly clears", async () => {
    const r = await runBenchScenario(signerDelistedAfterIssuance);
    expect(outcome(r, "whitelisted-at-issuance")).toBe("pass");
    expect(outcome(r, "integrity")).toBe("pass");
    expect(outcome(r, "issuer-descends-from-factory")).toBe("pass");
    expect(outcome(r, "anchored-on-chain")).toBe("pass");
    expect(outcome(r, "not-revoked")).toBe("pass");
  });

  it("MUST still verify - the scenario's expectation says so, and is not edited to match the code", async () => {
    // Direction two of the pair, stated as the REQUIREMENT. This is the assertion the captain's
    // ruling turns on: delisting is forward-only, so the correct verdict here is `true`.
    expect(signerDelistedAfterIssuance.mustVerify).toBe(true);
    expect(signerDelistedAfterIssuance.expectedVerdict).toBe(true);
    expect(
      signerDelistedAfterIssuance.expected["issuer-whitelisted"],
      "the expectation must state the CORRECT outcome, never the observed one",
    ).toBe("pass");
  });

  it("DOES NOT - the implementation refuses it, and that gap is the documented defect", async () => {
    // The report, not an endorsement. It pins what the code does today so the defect cannot be lost,
    // while the expectation above goes on stating the right answer.
    const defect = signerDelistedAfterIssuance.knownDefect;
    expect(defect, "the defect must be declared as data, not only in prose").toBeTruthy();
    const r = await runBenchScenario(signerDelistedAfterIssuance);
    expect(outcome(r, "issuer-whitelisted")).toBe("fail");
    expect(r.verdict, "a genuine credential is rendered as a forgery by a key rotation").toBe(false);
    expect(r.verdict).not.toBe(signerDelistedAfterIssuance.expectedVerdict);
    expect(defect?.observedVerdict).toBe(false);
  });

  it("states the contradicted ruling by name, with its source in the contract", async () => {
    // The defect statement is what a reviewer reads first. It has to name the rule it contradicts and
    // where that rule is written down, or the finding degrades into "a test is red for some reason".
    const statement = signerDelistedAfterIssuance.knownDefect?.statement ?? "";
    expect(statement).toContain("FORWARD-ONLY");
    expect(statement).toContain("DogTagIssuer.sol:82");
    expect(statement).toContain("adminRevoke");
    expect(statement).toContain("correct verdict is `true`");
  });

  it("renders the contradiction legibly: one row gates and fails, the other is advisory and passes", async () => {
    // The reason the two rows are separate. An operator seeing a single red "issuer whitelisted" row
    // would read "forgery"; seeing it beside a green historical row, marked advisory, they can read
    // what is actually true - the signer's status changed after a genuine issuance.
    const r = await runBenchScenario(signerDelistedAfterIssuance);
    const gating = r.checks.find((c) => c.id === "issuer-whitelisted");
    const historical = r.checks.find((c) => c.id === "whitelisted-at-issuance");
    expect(gating?.gatesVerdict).toBe(true);
    expect(gating?.outcome).toBe("fail");
    expect(historical?.gatesVerdict).toBe(false);
    expect(historical?.outcome).toBe("pass");
    // The row must cite the rule and its source, not merely report "pass" - an operator staring at a
    // red gating row needs to be told, in place, why the green one beside it outranks their instinct.
    expect(historical?.finding).toContain("forward-only");
    expect(historical?.finding).toContain("DogTagIssuer.sol:82");
    expect(historical?.finding).toContain("adminRevoke");
  });

  it("distinguishes the two delisting cases - the check is not simply reading the current state", async () => {
    // Both scenarios present a signer that is delisted NOW. If `whitelisted-at-issuance` were reading
    // the same getter as the pillar, both would fail identically and the row would be decorative.
    const after = await runBenchScenario(signerDelistedAfterIssuance);
    const before = await runBenchScenario(signerDelistedBeforeIssuance);
    expect(outcome(after, "issuer-whitelisted")).toBe(outcome(before, "issuer-whitelisted"));
    expect(outcome(after, "whitelisted-at-issuance")).not.toBe(
      outcome(before, "whitelisted-at-issuance"),
    );
  });
});

describe("a revoked credential presented as live", () => {
  it("is refused by the revocation row, and NOT by the anchoring row beside it", async () => {
    const r = await runBenchScenario(revokedPresentedAsLive);
    expect(outcome(r, "not-revoked")).toBe("fail");
    // It really was anchored. Collapsing "revoked" into "never issued" would misdescribe a credential
    // that was genuinely issued and later withdrawn - a different fact with a different remedy.
    expect(outcome(r, "anchored-on-chain")).toBe("pass");
    expect(outcome(r, "integrity")).toBe("pass");
    expect(r.verdict).toBe(false);
    expect(r.response?.status).toBe("revoked");
  });
});

describe("a whitelist answer from a registry that does not govern the issuer", () => {
  it("is flagged by the registry row while the whitelist row it voids reads GREEN", async () => {
    const r = await runBenchScenario(foreignRegistry);
    expect(outcome(r, "registry-governs-issuer")).toBe("fail");
    // The hazard, stated: the foreign registry lists this signer, so the pillar passes on an
    // authority that governs nothing here. Without the row above, nothing on the page would say so.
    expect(outcome(r, "issuer-whitelisted")).toBe("pass");
    const row = r.checks.find((c) => c.id === "registry-governs-issuer");
    expect(row?.finding).toContain("MISCONFIGURED");
  });

  it("does NOT convert this client's own misconfiguration into an accusation", async () => {
    // A configuration fault is not evidence about a credential - the same rule `FACTORY_ADDR` follows.
    // The row is advisory, so the verdict is untouched and the operator is told what is void.
    const r = await runBenchScenario(foreignRegistry);
    const row = r.checks.find((c) => c.id === "registry-governs-issuer");
    expect(row?.gatesVerdict).toBe(false);
    expect(r.verdict).toBe(true);
  });
});

describe("a self-consistent forgery that was never anchored", () => {
  it("PASSES integrity and is refused only by the chain", async () => {
    const r = await runBenchScenario(unanchoredSelfConsistentForgery);
    // The forger did the maths properly. Integrity proves internal consistency and nothing else.
    expect(outcome(r, "integrity")).toBe("pass");
    expect(outcome(r, "issuer-descends-from-factory")).toBe("fail");
    expect(r.verdict).toBe(false);
    expect(r.response?.status).toBe("not_issued");
  });

  it("was refused by a factory that WAS reachable and answering", async () => {
    // Otherwise this scenario would be indistinguishable from an unreachable chain, and would go
    // green for the one reason that proves nothing.
    const r = await runBenchScenario(unanchoredSelfConsistentForgery);
    const anchor = r.reads.find((x) => x.method === "rootIssuer");
    expect(anchor?.outcome).toBe("ok");
    expect(anchor?.value?.toLowerCase()).toBe("0x0000000000000000000000000000000000000000");
  });
});

describe("a record claiming batch inclusion", () => {
  it("is refused by integrity rather than folded", async () => {
    const r = await runBenchScenario(batchInclusionProof);
    expect(outcome(r, "integrity")).toBe("fail");
    // The root it names IS anchored - that is what makes the shape persuasive, and what makes the
    // refusal to fold an opaque proof the only thing standing between it and a pass.
    expect(outcome(r, "anchored-on-chain")).toBe("pass");
    expect(outcome(r, "issuer-whitelisted")).toBe("pass");
    expect(r.verdict).toBe(false);
  });
});

describe("a wrong-chain endpoint", () => {
  it("makes every on-chain row could-not-run, and NOT ONE of them a failure", async () => {
    // The distinction the whole surface exists for. "The factory has no record of this root" is an
    // accusation about a credential; on an endpoint we were refused permission to ask, it would be an
    // accusation nobody was in a position to make.
    const r = await runBenchScenario(wrongChainEndpoint);
    expect(r.checks.filter((c) => c.outcome === "fail").map((c) => c.id)).toEqual([]);
    expect(outcome(r, "issuer-descends-from-factory")).toBe("could-not-run");
    expect(outcome(r, "anchored-on-chain")).toBe("could-not-run");
    expect(outcome(r, "not-revoked")).toBe("could-not-run");
    expect(outcome(r, "issuer-whitelisted")).toBe("could-not-run");
    expect(outcome(r, "whitelisted-at-issuance")).toBe("could-not-run");
    expect(outcome(r, "registry-governs-issuer")).toBe("could-not-run");
  });

  it("reports no verdict at all rather than a refusal", async () => {
    const r = await runBenchScenario(wrongChainEndpoint);
    expect(r.verdict).toBeNull();
    expect(r.verdict).not.toBe(false);
    expect(r.verifierError).toBeTruthy();
  });

  it("names the chain mismatch as the reason, so the operator can act on it", async () => {
    const r = await runBenchScenario(wrongChainEndpoint);
    const reasons = r.checks.map((c) => c.couldNotRunReason ?? "").join("\n");
    expect(reasons).toContain("chain 135");
  });

  it("still answers the two OFFLINE rows, which need no chain", async () => {
    const r = await runBenchScenario(wrongChainEndpoint);
    expect(outcome(r, "integrity")).toBe("pass");
    expect(outcome(r, "not-expired")).toBe("pass");
  });
});

// ── the control, without which none of the above means anything ─────────────────────────────────

describe("the genuine control", () => {
  it("passes every check that can run, so the frauds above are not passing against a broken verifier", async () => {
    const r = await runBenchScenario(genuineCredential);
    expect(r.verdict).toBe(true);
    expect(r.checks.filter((c) => c.outcome === "fail")).toEqual([]);
    // Only the two issuer-domain rows may be unanswered, and only because no domain registry is
    // configured for the catalogue.
    expect(r.checks.filter((c) => c.outcome === "could-not-run").map((c) => c.id).sort()).toEqual([
      "issuer-domain-claim",
      "issuer-domain-dns",
    ]);
  });

  it("trips a BOUNDED set of checks per fraud - the catalogue is discriminating", async () => {
    // A verifier that refused everything would pass every fraud case above. This asserts the opposite
    // shape: from the same honest baseline, each FRAUD's declared vector differs from the control's in
    // a bounded way rather than collapsing wholesale.
    //
    // `mustVerify` scenarios are excluded because their records are GENUINE - matching the control is
    // the correct answer for them, not a gap. That exclusion is not a loophole: their own cases assert
    // the required verdict directly, and `delisted-after`'s real (defective) behaviour is checked below.
    const control = outcomes(await runBenchScenario(genuineCredential));
    let frauds = 0;
    for (const s of BENCH_SCENARIOS) {
      if (s.mustVerify || s.id === wrongChainEndpoint.id) continue;
      frauds++;
      const differing = (Object.keys(control) as BenchCheckId[]).filter(
        (id) => control[id] !== s.expected[id],
      );
      expect(differing.length, `${s.id} changes nothing at all`).toBeGreaterThan(0);
      expect(differing.length, `${s.id} collapses the whole report rather than tripping a check`).toBeLessThan(
        Object.keys(control).length,
      );
    }
    expect(frauds, "no fraud scenarios were actually examined").toBeGreaterThan(4);
  });

  it("distinguishes the delisted-after DEFECT from the control, even though the record is genuine", async () => {
    // The complement of the exclusion above. `delisted-after` SHOULD be indistinguishable from the
    // control - both are genuine credentials - and the fact that it is not, in the run, is the defect.
    const control = await runBenchScenario(genuineCredential);
    const defective = await runBenchScenario(signerDelistedAfterIssuance);
    expect(signerDelistedAfterIssuance.expected).toEqual(outcomes(control));
    expect(outcomes(defective)).not.toEqual(outcomes(control));
    expect(defective.verdict).not.toBe(control.verdict);
  });
});

// ── the reads a scenario makes are the evidence it is judged on ─────────────────────────────────

describe("the catalogue is hermetic and evidence-backed", () => {
  it("makes no network call - every scenario carries its own scripted chain", async () => {
    // A scenario that fell through to the live viem readers would be asserting about ROAX state
    // nobody in this repo wrote, and would fail in a disconnected worktree.
    const original = globalThis.fetch;
    let calls = 0;
    globalThis.fetch = (async (...args: Parameters<typeof fetch>) => {
      calls++;
      return original(...args);
    }) as typeof fetch;
    try {
      for (const s of BENCH_SCENARIOS) await runBenchScenario(s);
    } finally {
      globalThis.fetch = original;
    }
    expect(calls, "a scenario reached the network").toBe(0);
  });

  it("is DETERMINISTIC - the same scenario twice gives byte-identical outcomes", async () => {
    // What licenses the page leaving a scenario's result on screen while others are re-run: a
    // scenario's world is fixed and network-free, so a result from an earlier click is not stale, it
    // is identical to what a fresh click would produce. Without this the page would be showing
    // answers whose provenance nobody could state.
    for (const s of BENCH_SCENARIOS) {
      const a = await runBenchScenario(s);
      const b = await runBenchScenario(s);
      expect(outcomes(b), `${s.id} is not deterministic`).toEqual(outcomes(a));
      expect(b.verdict).toBe(a.verdict);
      expect(b.reads.map((r) => `${r.method}|${r.contract}|${r.outcome}|${r.value ?? ""}`)).toEqual(
        a.reads.map((r) => `${r.method}|${r.contract}|${r.outcome}|${r.value ?? ""}`),
      );
    }
  });

  it("cites no contract that is absent from its own recorded reads", async () => {
    // THE INVARIANT, applied to the fraud catalogue: an evidence line naming a contract that was
    // never asked hands the reader a citation for a read that did not happen.
    let citations = 0;
    for (const s of BENCH_SCENARIOS) {
      const r = await runBenchScenario(s);
      const observed = new Set(r.reads.map((x) => x.contract.toLowerCase()));
      for (const c of r.checks) {
        for (const e of c.evidence) {
          const m = /on (0x[0-9a-fA-F]{40})$/.exec(e.source);
          if (!m?.[1]) continue;
          citations++;
          expect(
            observed.has(m[1].toLowerCase()),
            `${s.id}/${c.id} cites ${m[1]} as having answered, but no read was recorded against it`,
          ).toBe(true);
        }
      }
    }
    expect(citations).toBeGreaterThan(0);
  });

  it("attaches a reason to every could-not-run and to nothing else", async () => {
    for (const s of BENCH_SCENARIOS) {
      const r = await runBenchScenario(s);
      for (const c of r.checks) {
        if (c.outcome === "could-not-run") {
          expect(c.couldNotRunReason, `${s.id}/${c.id}`).toBeTruthy();
        } else {
          expect(c.couldNotRunReason, `${s.id}/${c.id}`).toBeUndefined();
        }
      }
    }
  });
});

// ── coverage of the brief, stated so a missing scenario is a red test ───────────────────────────

describe("the catalogue covers every fraud the brief names", () => {
  const required: Array<[string, BenchScenario]> = [
    ["a tampered field that breaks the Merkle proof", tamperedCoveredField],
    // Stated precisely rather than as the brief's flat "never whitelisted": for a factory-resolved
    // root the flat case is UNREACHABLE (`issue()` is `onlyWhitelisted` and writes `issuedBy[r]` in
    // the same call, and `rootIssuer[r]` is write-once and writable only from inside a clone's
    // `issue()`), so scripting it would be testing a state the protocol cannot produce.
    ["a record whose issuer was never whitelisted FOR THE TYPE IT CLAIMS", relabelledRecordType],
    ["a signer delisted BEFORE the issuance block (must be refused)", signerDelistedBeforeIssuance],
    ["a signer delisted AFTER the issuance block (must still verify)", signerDelistedAfterIssuance],
    ["a revoked credential presented as live", revokedPresentedAsLive],
    ["a record answered by a registry that does not govern it", foreignRegistry],
    ["a record checked against a different chain", wrongChainEndpoint],
    ["a well-formed record that corresponds to no anchored root", unanchoredSelfConsistentForgery],
  ];

  for (const [what, scenario] of required) {
    it(`covers: ${what}`, () => {
      expect(BENCH_SCENARIOS).toContain(scenario);
    });
  }
});
