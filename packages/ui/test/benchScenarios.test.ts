// The adversarial half of the verification bench: fraudulent records driven through the REAL verify
// path, each asserting WHICH check refused it.
//
// Every case asserts the FULL outcome vector, never a single row. "Something went red" is satisfied by
// a fixture that happens to be expired, or by a chain that could not be reached - so a suite built on
// it stays green while the check it is named after is dead. The vector is what makes a scenario that
// starts passing for a different reason fail.
//
// No network: every scenario carries its own scripted chain, keyed on the contract each read is put to
// - the getters on the contract address, and the three registry reads on the registry address (see
// `benchScenarios.ts`). That keying is asserted directly below, because a fake that discards its
// address agrees with itself and no assertion about the resulting report can notice.
import { describe, expect, it } from "vitest";
import {
  BENCH_SCENARIOS,
  batchInclusionProof,
  foreignRegistry,
  FOREIGN_REGISTRY,
  genuineCredential,
  hostileIssuerContract,
  RECORD_TYPE_KEY,
  relabelledRecordType,
  revokedPresentedAsLive,
  runBenchScenario,
  SCENARIO_REGISTRY,
  signerDelistedAfterIssuance,
  signerDelistedBeforeIssuance,
  SIGNER,
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

  it("declares a known defect only where one still reproduces", () => {
    // EMPTY today: the delisting defect this field was introduced for has been fixed, so its pin was
    // deleted rather than left to report a bug that no longer exists. The machinery below stays,
    // because the next finding the catalogue turns up will need exactly it.
    expect(BENCH_SCENARIOS.filter((s) => s.knownDefect).map((s) => s.id)).toEqual([]);
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

  it("AND DOES - the defect is fixed, so no pin remains to excuse a refusal", async () => {
    // This assertion used to say the opposite: it pinned the refusal as `knownDefect` so the finding
    // could not be lost while the fix was out of scope. The fix landed, so the pin is gone and the
    // requirement stands on its own - which is exactly the transition the pin existed to force.
    expect(
      signerDelistedAfterIssuance.knownDefect,
      "the defect no longer reproduces; a lingering pin would report a fixed bug forever",
    ).toBeUndefined();
    const r = await runBenchScenario(signerDelistedAfterIssuance);
    expect(outcome(r, "issuer-whitelisted")).toBe("pass");
    expect(r.verdict, "a key rotation must not render a genuine credential a forgery").toBe(true);
    expect(r.verdict).toBe(signerDelistedAfterIssuance.expectedVerdict);
  });

  it("both whitelist rows clear it, and the historical one cites the rule that makes it so", async () => {
    // The two rows are still separate, and still answer for different parties: the gating one is the
    // VERIFIER's own answer, the advisory one the bench's INDEPENDENT reconstruction from the same
    // log. Their agreeing is corroboration - and the day the verifier regresses to a current-state
    // read, the gating row turns red beside a green historical one and says so in place.
    const r = await runBenchScenario(signerDelistedAfterIssuance);
    const gating = r.checks.find((c) => c.id === "issuer-whitelisted");
    const historical = r.checks.find((c) => c.id === "whitelisted-at-issuance");
    expect(gating?.gatesVerdict).toBe(true);
    expect(gating?.outcome).toBe("pass");
    expect(historical?.gatesVerdict).toBe(false);
    expect(historical?.outcome).toBe("pass");
    // The rule and its source stay on the row, not only in a commit message: an operator who knows
    // the signer is delisted today needs to be told, in place, why that does not matter here.
    expect(gating?.finding).toContain("forward-only");
    expect(gating?.finding).toContain("DogTagIssuer.sol:82");
  });

  it("distinguishes the two delisting cases - the check is not simply reading the current state", async () => {
    // THE test for the whole change. Both scenarios present a signer that is delisted NOW. A verdict
    // formula reading the current getter would refuse them identically; reading the grant history at
    // the anchoring point separates them - and it must separate them on BOTH rows, because both now
    // ask the historical question.
    const after = await runBenchScenario(signerDelistedAfterIssuance);
    const before = await runBenchScenario(signerDelistedBeforeIssuance);
    expect(outcome(after, "issuer-whitelisted")).toBe("pass");
    expect(outcome(before, "issuer-whitelisted")).toBe("fail");
    expect(outcome(after, "whitelisted-at-issuance")).toBe("pass");
    expect(outcome(before, "whitelisted-at-issuance")).toBe("fail");
    expect(after.verdict).toBe(true);
    expect(before.verdict).toBe(false);
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

describe("a client configured with a registry that does not govern the issuer", () => {
  it("is flagged by the registry row while the whitelist row beside it correctly reads GREEN", async () => {
    const r = await runBenchScenario(foreignRegistry);
    expect(outcome(r, "registry-governs-issuer")).toBe("fail");
    // The pillar passes, and correctly: it asks the registry the CLONE names, which is the only
    // authority whose `_wl` gated this contract's `issue()`, and that registry's log holds the grant.
    // The row above reports the mis-pairing because it matters to every OTHER surface aimed at the
    // configured registry - it no longer voids this one.
    expect(outcome(r, "issuer-whitelisted")).toBe("pass");
    const row = r.checks.find((c) => c.id === "registry-governs-issuer");
    expect(row?.finding).toContain("MISCONFIGURED");
  });

  it("reads the grant history from the GOVERNING registry, not the one this client is configured with", async () => {
    // The rule the historical row rests on. Under this misconfiguration the grant is in the registry
    // the CLONE names, and only there - `issue()` is `onlyWhitelisted` against that slot, so an honest
    // anchoring cannot rest on any other log. A row that asked the configured registry would find
    // nothing and print a definite refusal of a genuine credential: our own misconfiguration turned
    // into an accusation, the fail-closed mirror of the fail-open bug this surface exists to prevent.
    const r = await runBenchScenario(foreignRegistry);
    expect(outcome(r, "whitelisted-at-issuance")).toBe("pass");
    expect(outcome(r, "whitelisted-at-issuance")).not.toBe("fail");
    const read = r.reads.find((x) => x.method === "whitelistHistory");
    expect(read?.contract.toLowerCase(), "the grant log was read from the wrong contract").toBe(
      FOREIGN_REGISTRY.toLowerCase(),
    );
    expect(read?.contract.toLowerCase()).not.toBe(SCENARIO_REGISTRY.toLowerCase());
    // ...and the row CITES that authority, so a reader can see which contract answered.
    const row = r.checks.find((c) => c.id === "whitelisted-at-issuance");
    expect(row?.evidence.some((e) => e.source.includes(FOREIGN_REGISTRY))).toBe(true);
  });

  it("keys its scripted registry reads on the REGISTRY, so a wrong-contract read finds nothing", async () => {
    // The fake-integrity guard. Both registry-scoped reads used to discard their registry argument and
    // answer identically whichever authority was asked - so this scenario went green while the
    // production code read the wrong contract, exactly the `MockChain` trap recorded in
    // `crates/dogtag-standard-rs/src/verify.rs`. Asserted against the readers directly, because a fake
    // that agrees with itself cannot be caught by any assertion about the report.
    const w = foreignRegistry.build();
    expect(await w.grantHistoryReader.grants(FOREIGN_REGISTRY, RECORD_TYPE_KEY, SIGNER)).not.toEqual([]);
    expect(
      await w.grantHistoryReader.grants(SCENARIO_REGISTRY, RECORD_TYPE_KEY, SIGNER),
      "a grant log read against a registry that never recorded it must come back empty",
    ).toEqual([]);
    // The VERIFIER's own reader is keyed the same way and reads the same map, so the gating row and
    // the advisory row cannot be shown different chains - and a wrong-authority read finds nothing on
    // that side too.
    const honest = genuineCredential.build();
    expect(
      await honest.reader.grantHistory(SCENARIO_REGISTRY, RECORD_TYPE_KEY, SIGNER),
    ).not.toEqual([]);
    expect(await honest.reader.grantHistory(FOREIGN_REGISTRY, RECORD_TYPE_KEY, SIGNER)).toEqual([]);
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

  it("says the factory was never REACHED, not that it answered with nothing", async () => {
    // The two are different facts and this is the scenario that separates them. Every read threw, so
    // no factory answered at all - reporting "none was resolved" would describe a contract that
    // answered with an empty index, which is the could-not-ask/asked-and-got-nothing collapse this
    // module exists to prevent, arriving from the environment rather than the document.
    const r = await runBenchScenario(wrongChainEndpoint);
    for (const id of ["registry-governs-issuer", "whitelisted-at-issuance"] as const) {
      const reason = r.checks.find((c) => c.id === id)?.couldNotRunReason ?? "";
      expect(reason, `${id} must name the read that failed`).toContain("rootIssuer");
      expect(reason).toContain("failed");
      expect(reason, `${id} must not report an unreachable factory as one that resolved nothing`).not.toContain(
        "None was resolved",
      );
    }
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
    // Nothing may be unanswered on the control: every row the catalogue carries is answerable from
    // its scripted chain, so a could-not-run here is a broken fixture rather than an honest limit.
    expect(r.checks.filter((c) => c.outcome === "could-not-run").map((c) => c.id).sort()).toEqual([]);
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

  it("runs delisted-after INDISTINGUISHABLY from the control - both are genuine credentials", async () => {
    // The complement of the exclusion above, and the assertion that flipped when the defect was fixed.
    // It used to say the two DIFFER, pinning the refusal; a genuine credential whose signer was later
    // rotated is a genuine credential, so every row and the verdict must now match the control exactly.
    // Nothing about the delisting is visible in the report, because nothing about it is wrong.
    const control = await runBenchScenario(genuineCredential);
    const rotated = await runBenchScenario(signerDelistedAfterIssuance);
    expect(signerDelistedAfterIssuance.expected).toEqual(outcomes(control));
    expect(outcomes(rotated)).toEqual(outcomes(control));
    expect(rotated.verdict).toBe(control.verdict);
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
