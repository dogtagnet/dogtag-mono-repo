// The portal-layer forgery guard (registry-plan S-15). Hermetic: every chain read is an injected
// fake, so each case scripts a chain state and asserts what the surface concludes from it.
//
// THE POINT OF THIS FILE. A happy-path test proves nothing about a guard - it passes just as well
// with the guard deleted. So the cases that matter here are the refusals, and each of them drives a
// candidate through the SAME `assessCandidateClone` the UI calls rather than through a test-only
// path. Three shapes are covered because the chain produces three and they are not
// interchangeable: a hand-rolled contract that answers every read perfectly, an EOA whose staticcall
// succeeds with empty returndata, and a genuine clone belonging to somebody else.
import { describe, expect, it } from "vitest";
import {
  assessCandidateClone,
  providerCheck,
  Standing,
  ZERO_ADDR,
  ZERO_PROVIDER_ID,
  type Address,
  type HexWord,
  type ProviderChainReader,
} from "../src/provider";

const FACTORY_CLONE: Address = "0x1111111111111111111111111111111111111111";
const PROVIDER_KEY: Address = "0x2222222222222222222222222222222222222222";
const OTHER_KEY: Address = "0x3333333333333333333333333333333333333333";
/** Answers `owner()` and `recordType()` exactly as a genuine clone does. Only provenance refuses it. */
const FORGED: Address = "0x4444444444444444444444444444444444444444";
/** No code at all. A staticcall to it SUCCEEDS with empty returndata - the silent shape. */
const EOA: Address = "0x5555555555555555555555555555555555555555";

const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const OTHER_PROVIDER: HexWord = "0xc4a70bd318e952f6017da84b3c6e29fd50b7143e";
const RECORD_TYPE: HexWord = `0x${"ab".repeat(32)}`;

/** Every read a case did not script THROWS, so no case can pass by leaning on a default. */
function reader(overrides: Partial<ProviderChainReader>): ProviderChainReader {
  const unscripted = (name: string) => async () => {
    throw new Error(`the test did not script ${name}`);
  };
  return {
    isFactoryClone: unscripted("isFactoryClone"),
    cloneOwner: unscripted("cloneOwner"),
    service: unscripted("service"),
    effectiveService: unscripted("effectiveService"),
    provider: unscripted("provider"),
    currentService: unscripted("currentService"),
    canCreateService: unscripted("canCreateService"),
    predictIssuer: unscripted("predictIssuer"),
    issuerCreations: unscripted("issuerCreations"),
    cloneRecordType: unscripted("cloneRecordType"),
    canWriteServiceRepoint: unscripted("canWriteServiceRepoint"),
    domainClaimStanding: unscripted("domainClaimStanding"),
    canWriteDomain: unscripted("canWriteDomain"),
    directoryIsLiveFor: unscripted("directoryIsLiveFor"),
    canWriteProviderRecord: unscripted("canWriteProviderRecord"),
    providerProfileAnchor: unscripted("providerProfileAnchor"),
    providerPinCount: unscripted("providerPinCount"),
    providerNextLocationNumber: unscripted("providerNextLocationNumber"),
    providerHasPin: unscripted("providerHasPin"),
    providerPin: unscripted("providerPin"),
    ...overrides,
  } as ProviderChainReader;
}

/** A genuine clone, attached to PROVIDER, active, owner confirmed. The control. */
function genuineReader(extra: Partial<ProviderChainReader> = {}): ProviderChainReader {
  return reader({
    isFactoryClone: async (a) => a === FACTORY_CLONE,
    cloneOwner: async () => PROVIDER_KEY,
    service: async () => ({
      providerId: PROVIDER,
      factoryGeneration: RECORD_TYPE,
      recordType: RECORD_TYPE,
      confirmedOwner: PROVIDER_KEY,
      domainResolver: ZERO_ADDR,
      ownerEpoch: 1n,
      standing: Standing.ACTIVE,
    }),
    effectiveService: async () => ({
      providerStanding: Standing.ACTIVE,
      serviceStanding: Standing.ACTIVE,
      factoryActive: true,
      ownerConfirmed: true,
      hasActiveIssuer: true,
    }),
    // The chain's own repoint predicate. Scripted here rather than derived from `cloneOwner`,
    // because a fake that agreed with the module's old owner-equality rule could not tell a fixed
    // preflight from a broken one.
    canWriteServiceRepoint: async (_service, who) => who === PROVIDER_KEY,
    currentService: async () => ZERO_ADDR,
    ...extra,
  });
}

const request = (candidate: Address) => ({ candidate, caller: PROVIDER_KEY, providerId: PROVIDER });

describe("the forgery guard refuses what our factory did not deploy", () => {
  it("REFUSES a hand-rolled contract that answers every other read perfectly", async () => {
    const r = await assessCandidateClone(
      request(FORGED),
      reader({
        isFactoryClone: async () => false,
        // Scripted to answer, and deliberately so: the forgery is convincing everywhere else.
        cloneOwner: async () => PROVIDER_KEY,
      }),
    );

    expect(r.verdict).toBe("refused");
    expect(r.lifecycle).toBe("notAClone");
    expect(r.canRepoint).toBe(false);
    const provenance = r.checks.find((c) => c.id === "clone-provenance")!;
    expect(provenance.outcome).toBe("fail");
    expect(provenance.finding).toContain(FORGED);
    expect(r.nextStep).toMatch(/not deployed by the DogTag issuer factory/i);
  });

  it("REFUSES an EOA, whose ownership read cannot even be answered", async () => {
    const r = await assessCandidateClone(
      request(EOA),
      reader({
        isFactoryClone: async () => false,
        // viem rejects empty returndata. If this ever became "returns the zero address", the
        // surface would report an EOA as owned-by-nobody rather than as not-answerable.
        cloneOwner: async () => {
          throw new Error("returned no data");
        },
      }),
    );

    expect(r.verdict).toBe("refused");
    expect(r.lifecycle).toBe("notAClone");
    expect(r.canRepoint).toBe(false);
  });

  it("refuses on PROVENANCE without letting a neighbouring non-answer become the reason", async () => {
    // The definite refusal is final, so the assessment stops. If it read on, a could-not-run from
    // the unreadable ownership call would sit beside the refusal and read as its cause - which
    // would send the provider to fix ownership on a contract that is simply not ours.
    const r = await assessCandidateClone(
      request(FORGED),
      reader({
        isFactoryClone: async () => false,
        cloneOwner: async () => {
          throw new Error("nope");
        },
      }),
    );

    expect(r.checks).toHaveLength(1);
    expect(r.checks[0]!.id).toBe("clone-provenance");
    expect(r.checks.some((c) => c.outcome === "could-not-run")).toBe(false);
  });

  it("a FAILED READ is never a refusal - it is indeterminate, and says so", async () => {
    // The inverse mistake, and the one this repo has shipped four times. "We could not reach the
    // factory" is a statement about our connectivity, not about the provider's contract.
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      reader({
        isFactoryClone: async () => {
          throw new Error("HTTP request failed: 503");
        },
        cloneOwner: async () => PROVIDER_KEY,
        service: async () => ({
          providerId: PROVIDER,
          factoryGeneration: RECORD_TYPE,
          recordType: RECORD_TYPE,
          confirmedOwner: PROVIDER_KEY,
          domainResolver: ZERO_ADDR,
          ownerEpoch: 1n,
          standing: Standing.ACTIVE,
        }),
        effectiveService: async () => ({
          providerStanding: Standing.ACTIVE,
          serviceStanding: Standing.ACTIVE,
          factoryActive: true,
          ownerConfirmed: true,
          hasActiveIssuer: true,
        }),
        currentService: async () => ZERO_ADDR,
      }),
    );

    expect(r.verdict).toBe("indeterminate");
    expect(r.lifecycle).not.toBe("notAClone");
    expect(r.canRepoint).toBe(false);
    const provenance = r.checks.find((c) => c.id === "clone-provenance")!;
    expect(provenance.outcome).toBe("could-not-run");
    expect(provenance.couldNotRunReason).toContain("503");
    expect(r.nextStep).toMatch(/could not run/i);
  });

  it("PROVENANCE IS NOT ATTRIBUTION: a genuine clone the chain will not let this key move is refused", async () => {
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        cloneOwner: async () => OTHER_KEY,
        canWriteServiceRepoint: async () => false,
      }),
    );

    expect(r.verdict).toBe("refused");
    expect(r.canRepoint).toBe(false);
    // The refusal comes from the CHAIN's own predicate, which is the only thing entitled to make it.
    const authority = r.checks.find((c) => c.id === "clone-write-authority")!;
    expect(authority.outcome).toBe("fail");
    // Ownership is still REPORTED, and names the owner, because "you are a delegate, not the owner"
    // and "you may not act here" are different things a provider needs told apart.
    const control = r.checks.find((c) => c.id === "clone-control")!;
    expect(control.finding).toContain(OTHER_KEY);
    // Provenance still passed - the answers are kept apart, because "not yours" and "not genuine"
    // send the provider to entirely different remedies.
    expect(r.checks.find((c) => c.id === "clone-provenance")!.outcome).toBe("pass");
  });

  it("ADMITS A DELEGATE THE CHAIN ADMITS - the preflight is never stricter than the contract", async () => {
    // `repointService` is gated by `canWriteService(service, caller, SERVICE_PERMISSION_REPOINT)`,
    // which is true for the confirmed live owner OR an owner-epoch-scoped delegate holding that bit.
    // This surface used to ask `owner() == caller` instead and refuse everything else, so a provider
    // operating through a delegate key was told "the contract is owned by X, not by you" and the
    // button stayed disabled on a transaction the chain would have accepted. Re-deriving an
    // authorization the contract already exposes is the defect; composing it is the fix.
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        cloneOwner: async () => OTHER_KEY,
        canWriteServiceRepoint: async () => true,
      }),
    );

    expect(r.verdict).toBe("ready");
    expect(r.canRepoint).toBe(true);
    expect(r.checks.find((c) => c.id === "clone-write-authority")!.outcome).toBe("pass");
    // Not a pass by silence: the owner is still named, so a delegate can see whose contract it is.
    expect(r.checks.find((c) => c.id === "clone-control")!.finding).toContain(OTHER_KEY);
    // And nothing in the check list refuses. A single `fail` anywhere would have folded to
    // `refused`, which is how a fix at one row can be inert while the button stays disabled.
    expect(r.checks.some((c) => c.outcome === "fail")).toBe(false);
  });

  it("an unreadable authority read is indeterminate - a non-answer is not permission", async () => {
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        canWriteServiceRepoint: async () => {
          throw new Error("HTTP request failed: 429");
        },
      }),
    );

    expect(r.verdict).toBe("indeterminate");
    expect(r.canRepoint).toBe(false);
    const authority = r.checks.find((c) => c.id === "clone-write-authority")!;
    expect(authority.outcome).toBe("could-not-run");
    expect(authority.couldNotRunReason).toContain("429");
  });

  it("a genuine clone attached to ANOTHER provider is refused as foreign, not as a forgery", async () => {
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        service: async () => ({
          providerId: OTHER_PROVIDER,
          factoryGeneration: RECORD_TYPE,
          recordType: RECORD_TYPE,
          confirmedOwner: PROVIDER_KEY,
          domainResolver: ZERO_ADDR,
          ownerEpoch: 1n,
          standing: Standing.ACTIVE,
        }),
      }),
    );

    expect(r.lifecycle).toBe("foreign");
    expect(r.verdict).toBe("refused");
    expect(r.nextStep).toMatch(/belongs to another provider/i);
  });
});

describe("the clone lifecycle is an explicit state, because the flow is not one action", () => {
  it("a freshly deployed clone is `deployed`, and the next step belongs to the REGISTRAR", async () => {
    // The step that is easy to model wrong: `attachService` is onlyOwner on the core, so a genuine
    // clone the provider owns is attached to nobody until DogTag attaches it. A surface presenting
    // deploy-and-select as one continuous self-service action describes a journey nobody can walk.
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        service: async () => ({
          providerId: ZERO_PROVIDER_ID as HexWord,
          factoryGeneration: `0x${"00".repeat(32)}`,
          recordType: `0x${"00".repeat(32)}`,
          confirmedOwner: ZERO_ADDR,
          domainResolver: ZERO_ADDR,
          ownerEpoch: 0n,
          standing: Standing.NONE,
        }),
      }),
    );

    expect(r.lifecycle).toBe("deployed");
    expect(r.canRepoint).toBe(false);
    expect(r.nextStep).toMatch(/DogTag to attach it/i);
    expect(r.checks.find((c) => c.id === "clone-provenance")!.outcome).toBe("pass");
    expect(r.checks.find((c) => c.id === "clone-control")!.outcome).toBe("pass");
  });

  it("an attached, cleared clone is repointable", async () => {
    const r = await assessCandidateClone(request(FACTORY_CLONE), genuineReader());
    expect(r.lifecycle).toBe("attached");
    expect(r.verdict).toBe("ready");
    expect(r.canRepoint).toBe(true);
    expect(r.recordType).toBe(RECORD_TYPE);
  });

  it("the clone that is ALREADY current is not offered as a repoint", async () => {
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({ currentService: async () => FACTORY_CLONE }),
    );
    expect(r.lifecycle).toBe("current");
    expect(r.verdict).toBe("ready");
    expect(r.canRepoint).toBe(false);
    expect(r.nextStep).toMatch(/already selected/i);
  });

  it("a frozen clone is refused, and an unconfirmed owner handover is refused DIFFERENTLY", async () => {
    // Two failures with two remedies: a freeze is cleared by nothing, an unconfirmed handover is
    // cleared by the registrar confirming it. One message for both sends the provider to the wrong
    // one.
    // `canWriteServiceRepoint` is scripted false alongside, because the chain agrees: the core's
    // own `canWriteService` requires `_serviceStandingIsEffective`, so a fixture answering `true`
    // beside a RETIRED service would model a state the contract cannot produce.
    const frozen = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        canWriteServiceRepoint: async () => false,
        effectiveService: async () => ({
          providerStanding: Standing.ACTIVE,
          serviceStanding: Standing.RETIRED,
          factoryActive: true,
          ownerConfirmed: true,
          hasActiveIssuer: false,
        }),
      }),
    );
    expect(frozen.verdict).toBe("refused");
    expect(frozen.checks.find((c) => c.id === "clone-standing")!.finding).toMatch(/retired/i);

    const unconfirmed = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        // Same faithfulness point: `canWriteService` also requires `liveOwner == confirmedOwner`.
        canWriteServiceRepoint: async () => false,
        effectiveService: async () => ({
          providerStanding: Standing.ACTIVE,
          serviceStanding: Standing.ACTIVE,
          factoryActive: true,
          ownerConfirmed: false,
          hasActiveIssuer: true,
        }),
      }),
    );
    expect(unconfirmed.verdict).toBe("refused");
    expect(unconfirmed.checks.find((c) => c.id === "clone-standing")!.finding).toMatch(
      /change of owner is waiting/i,
    );
    expect(unconfirmed.checks.find((c) => c.id === "clone-standing")!.finding).not.toMatch(/retired/i);
  });

  it("an unreadable current-pointer leaves what WAS established, rather than discarding it", async () => {
    // The pointer is a display fact no check depends on. Dropping the whole assessment to `unknown`
    // over it would throw away the provenance, control and attachment answers that did resolve.
    const r = await assessCandidateClone(
      request(FACTORY_CLONE),
      genuineReader({
        currentService: async () => {
          throw new Error("timeout");
        },
      }),
    );
    expect(r.lifecycle).toBe("attached");
    expect(r.verdict).toBe("ready");
  });
});

describe("the three-state shape is enforced, not remembered", () => {
  it("a could-not-run MUST carry a reason", () => {
    expect(() =>
      providerCheck("clone-provenance", "q", "could-not-run", "finding"),
    ).toThrow(/must state its reason/);
  });

  it("only a could-not-run MAY carry a reason", () => {
    expect(() => providerCheck("clone-provenance", "q", "pass", "finding", "why")).toThrow(
      /only a could-not-run/,
    );
  });

  it("every check a real assessment emits obeys the reason invariant", async () => {
    const r = await assessCandidateClone(request(FACTORY_CLONE), genuineReader());
    for (const c of r.checks) {
      expect(c.finding.length).toBeGreaterThan(0);
      expect("couldNotRunReason" in c).toBe(c.outcome === "could-not-run");
    }
  });
});

describe("the assessment is answerable about what it judged", () => {
  it("carries the candidate it judged, so a repoint addresses THAT and not a since-edited field", async () => {
    const r = await assessCandidateClone(request(FACTORY_CLONE), genuineReader());
    expect(r.candidate).toBe(FACTORY_CLONE);
  });
});
