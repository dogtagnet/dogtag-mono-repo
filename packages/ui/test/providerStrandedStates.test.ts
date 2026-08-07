// The four states that stranded a captain mid-journey on the provider self-service page.
//
// All four are the same defect wearing different clothes: the page said something it was in no
// position to say, or said the same thing twice, or named a remedy that was not available. None of
// them is a chain fault - he had done everything right, and on the live chain his contract was
// exactly where it should have been.
//
//   1. the superseded banner said "Check again before sending" over a Check button the page itself
//      had disabled;
//   2. one obstacle printed the identical four-line sentence under every control it blocked;
//   3. the step-2 banner claimed there was no page for attaching a contract, when the admin portal
//      has carried one under Providers -> Attached services since C-2;
//   4. `attachService` leaves a service PENDING BY DESIGN, and the page reported that ordinary,
//      transient state as "Frozen" - then blamed his key for a refusal that was about standing.
import { describe, expect, it } from "vitest";
import { blankContactFields } from "../src/directory/registration";
import {
  assessCandidateClone,
  assessDomainClaim,
  ATTACHMENT_IS_A_DOGTAG_STEP,
  DIRECTORY_NEEDS_TURNING_ON,
  DOMAIN_REGISTER_NEEDS_TURNING_ON,
  DomainDisposition,
  planDirectoryPublication,
  describeActionBlock,
  describePlanRetirement,
  firstSentence,
  renderReason,
  sequenceReasons,
  Standing,
  type Address,
  type ActionBlock,
  type EffectiveService,
  type HexWord,
  type ProviderChainReader,
  type ProviderCheck,
  type ServiceRecord,
} from "../src/provider";

const CLONE: Address = "0x14a090086a6fd747840b003a9c09521d09ddef3a";
const OWNER: Address = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
const PROVIDER: HexWord = "0x7b160cb6dd3f8690247093c16d16a21e61d98eea";
const DOG_PROFILE: HexWord = "0x501883bc66249712c0662ee63b45b38088e876bff90ccfb60d6d0778a245683d";

function reader(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
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

function serviceRecord(standing: Standing): ServiceRecord {
  return {
    providerId: PROVIDER,
    factoryGeneration: `0x${"4a".repeat(32)}`,
    recordType: DOG_PROFILE,
    confirmedOwner: OWNER,
    domainResolver: `0x${"0".repeat(40)}`,
    ownerEpoch: 1n,
    standing,
  };
}

function effective(overrides: Partial<EffectiveService> = {}): EffectiveService {
  return {
    providerStanding: Standing.ACTIVE,
    serviceStanding: Standing.ACTIVE,
    factoryActive: true,
    ownerConfirmed: true,
    hasActiveIssuer: false,
    ...overrides,
  };
}

/**
 * The chain exactly as it stood after DogTag attached the captain's contract: attached to HIS
 * provider, owned by HIS key, and service standing PENDING - the state `attachService` writes.
 * `canWriteService` returns false on `!_serviceStandingIsEffective(...)` BEFORE it looks at the
 * caller, so it answers false here for him and for everybody.
 */
function justAttached(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  return reader({
    isFactoryClone: async () => true,
    cloneOwner: async () => OWNER,
    service: async () => serviceRecord(Standing.PENDING),
    effectiveService: async () => effective({ serviceStanding: Standing.PENDING }),
    canWriteServiceRepoint: async () => false,
    currentService: async () => `0x${"0".repeat(40)}` as Address,
    ...overrides,
  });
}

const assess = (r: ProviderChainReader) =>
  assessCandidateClone({ candidate: CLONE, caller: OWNER, providerId: PROVIDER }, r);

// Typed against the real `ProviderCheck` rather than a hand-written shape, so a field renamed in
// the engine is a compile error here instead of an `undefined` that no assertion can catch.
const rowFor = (checks: readonly ProviderCheck[], id: ProviderCheck["id"]): ProviderCheck => {
  const found = checks.find((c) => c.id === id);
  if (!found) throw new Error(`no ${id} row: the assessment did not produce one`);
  return found;
};

describe("a contract DogTag has just attached", () => {
  it("is NOT reported as frozen - it is waiting on a step DogTag has still to take", async () => {
    // The state that stranded him. `attachService` writes PENDING by design, so this is the
    // ORDINARY state right after an attach and the likeliest one a provider ever checks in.
    // "Frozen" describes the opposite: RETIRED is terminal and a deprecated generation cannot be
    // undeprecated, so both really are permanent and there is nothing to wait for.
    const { checks } = await assess(justAttached());
    const standing = rowFor(checks, "clone-standing");
    expect(standing.outcome).toBe("fail");
    expect(standing.finding).not.toMatch(/frozen/i);
    expect(standing.finding).toMatch(/waiting on dogtag/i);
    // The remedy has to name who does it and where, or it is a wall with a nicer wall next to it.
    expect(standing.finding).toMatch(/standing to active/i);
    expect(standing.finding).toMatch(/providers page/i);
  });

  it("says WAIT in the one-line summary, not 'the failed checks above say why'", async () => {
    // The summary is what a first-time provider reads and stops at. Sending them to read five rows
    // to discover the answer is "wait for DogTag" is a wall with a nicer wall behind it - and this
    // is the single most common state a provider is in right after their contract is attached.
    const { nextStep } = await assess(justAttached());
    expect(nextStep).toMatch(/nothing for you to do here yet/i);
    expect(nextStep).toMatch(/standing to active/i);
    expect(nextStep).not.toMatch(/failed checks above say why/i);
  });

  it("still refuses the send, because the chain would refuse it too", async () => {
    // The SENTENCE changed and the verdict did not. A pass here would offer a transaction that
    // cannot succeed, and a could-not-run would claim we failed to read what we read perfectly well.
    const assessment = await assess(justAttached());
    expect(assessment.canRepoint).toBe(false);
  });

  it("does not blame the operator's key for a refusal that is about standing", async () => {
    // He IS the owner on file. `canWriteService` folds standing before it looks at the caller, so
    // its `false` here is the same `false` the owner and every delegate would get - and the row used
    // to read it as a fact about his key and tell him the owner on file could do this instead.
    const { checks } = await assess(justAttached());
    const authority = rowFor(checks, "clone-write-authority");
    expect(authority.outcome).toBe("could-not-run");
    expect(authority.finding).not.toMatch(/does not accept a selection from your key/i);
    expect(authority.couldNotRunReason).toMatch(/says nothing about your key/i);
  });

  it("DOES blame the key when standing is established as fine, so the row still refuses a real refusal", async () => {
    // The other half, and without it the fix would have made the authority row unable to refuse
    // anything at all - a check that can never fail is not a check.
    const { checks } = await assess(
      justAttached({
        service: async () => serviceRecord(Standing.ACTIVE),
        effectiveService: async () => effective(),
        canWriteServiceRepoint: async () => false,
      }),
    );
    const authority = rowFor(checks, "clone-write-authority");
    expect(authority.outcome).toBe("fail");
    expect(authority.finding).toMatch(/does not accept a selection from your key/i);
  });

  it("still calls a RETIRED contract frozen, because that one really is permanent", async () => {
    const { checks } = await assess(
      justAttached({
        service: async () => serviceRecord(Standing.RETIRED),
        effectiveService: async () => effective({ serviceStanding: Standing.RETIRED }),
      }),
    );
    const standing = rowFor(checks, "clone-standing");
    expect(standing.finding).toMatch(/frozen/i);
    expect(standing.finding).not.toMatch(/waiting on dogtag/i);
  });

  it("still calls a deprecated factory generation frozen", async () => {
    const { checks } = await assess(
      justAttached({
        service: async () => serviceRecord(Standing.ACTIVE),
        effectiveService: async () => effective({ factoryActive: false }),
      }),
    );
    expect(rowFor(checks, "clone-standing").finding).toMatch(/frozen/i);
  });
});

describe("the SAME false accusation, in the other two flows that gate on a composed predicate", () => {
  // `canWriteDomain` composes `isAuthoritativeFor`, and `canWriteProvider` returns false on a
  // non-ACTIVE provider standing BEFORE it looks at the caller. Both therefore answer one `false`
  // for two reasons, exactly like `canWriteService` - and both rows read it as a fact about the key.
  const SERVICE: Address = "0xd6c312c59404e9c8b6b68a936d412273605da9f8";

  it("flow 3 does not blame the key when the domain register is not live for the contract", async () => {
    const { checks } = await assessDomainClaim(
      SERVICE,
      OWNER,
      reader({
        domainClaimStanding: async () => ({
          disposition: DomainDisposition.UNSET,
          domain: "",
          lineageRecognizesService: true,
          registryApprovesThisResolver: true,
          // Not selected: DogTag's to fix, and nothing about this key.
          coreSelectsThisResolver: false,
          serviceStandingEffective: true,
        }),
        canWriteDomain: async () => false,
      }),
    );
    const authority = rowFor(checks, "domain-write-authority");
    expect(authority.outcome).toBe("could-not-run");
    expect(authority.finding).not.toMatch(/owner on file/i);
    expect(authority.couldNotRunReason).toMatch(/says nothing about your key/i);
  });

  it("flow 3 DOES blame the key when the register is live, so the row can still refuse", async () => {
    const { checks } = await assessDomainClaim(
      SERVICE,
      OWNER,
      reader({
        domainClaimStanding: async () => ({
          disposition: DomainDisposition.UNSET,
          domain: "",
          lineageRecognizesService: true,
          registryApprovesThisResolver: true,
          coreSelectsThisResolver: true,
          serviceStandingEffective: true,
        }),
        canWriteDomain: async () => false,
      }),
    );
    const authority = rowFor(checks, "domain-write-authority");
    expect(authority.outcome).toBe("fail");
    expect(authority.finding).toMatch(/owner on file/i);
  });

  it("flow 4 names a PENDING provider record rather than accusing the key - the state every new provider starts in", async () => {
    // `registerProvider` writes PENDING and `setProviderStanding(ACTIVE)` is a separate registrar
    // call, so this is the FIRST state a new provider ever sees on this page. It used to read "your
    // key may not publish into your provider record", and the flow did not read the standing at all,
    // so nothing on screen named the real cause.
    const plan = await planDirectoryPublication(
      {
        providerId: PROVIDER,
        caller: OWNER,
        latInput: "",
        lngInput: "",
        contacts: { ...blankContactFields(), phone: "+65 6123 4567" },
        locationKind: 0,
        locationActive: true,
        logo: null,
      },
      reader({
        provider: async () => ({
          controller: OWNER,
          directoryResolver: `0x${"0".repeat(40)}` as Address,
          standing: Standing.PENDING,
        }),
        directoryIsLiveFor: async () => true,
        canWriteProviderRecord: async () => false,
        providerProfileAnchor: async () => ({
          digest: `0x${"0".repeat(64)}` as HexWord,
          schema: 0,
          codec: 0,
          hashAlgorithm: 0,
          revision: 0n,
        }),
        providerPinCount: async () => 0,
      }),
      (() => `0x${"ab".repeat(32)}` as HexWord) as never,
    );
    const standing = rowFor(plan.checks, "provider-standing");
    expect(standing.outcome).toBe("fail");
    expect(standing.finding).toMatch(/pending review/i);
    expect(standing.finding).toMatch(/nothing for you to change/i);
    const authority = rowFor(plan.checks, "domain-write-authority");
    expect(authority.outcome).toBe("could-not-run");
    expect(authority.finding).not.toMatch(/provider's controller/i);
    expect(authority.couldNotRunReason).toMatch(/says nothing about your key/i);
  });
});

describe("what the page claims exists elsewhere in the product", () => {
  it("does not claim there is no page for attaching a contract - the admin portal has one", () => {
    // It said "there is no page for it yet", which was true when written and is now false: the admin
    // Providers page carries "Attach a contract" under Attached services. A captain read that
    // sentence, went looking anyway, found the control and used it - so the page had told him the
    // thing he had just done was impossible.
    expect(ATTACHMENT_IS_A_DOGTAG_STEP).not.toMatch(/no page/i);
    expect(ATTACHMENT_IS_A_DOGTAG_STEP).not.toMatch(/not.{0,12}(built|exist)/i);
  });

  it("names BOTH steps DogTag takes, because naming only the attach stranded him at the second", () => {
    expect(ATTACHMENT_IS_A_DOGTAG_STEP).toMatch(/attach/i);
    expect(ATTACHMENT_IS_A_DOGTAG_STEP).toMatch(/standing to active/i);
  });

  it("does not claim the resolver steps have no page - DogTag's half of each one does", () => {
    // Same rot, two more sentences. `setResolverApproved` is on the admin Providers page, so
    // "neither has a page here yet" was half false in both of these. The SELECTION half genuinely
    // has no surface anywhere, which is why neither sentence claims one exists either.
    for (const notice of [DOMAIN_REGISTER_NEEDS_TURNING_ON, DIRECTORY_NEEDS_TURNING_ON]) {
      expect(notice).not.toMatch(/no page/i);
      expect(notice).not.toMatch(/neither has a page/i);
      // Still says whose move it is, which is the half a provider actually needs.
      expect(notice).toMatch(/dogtag/i);
    }
  });
});

describe("a reason is said once in full, and never left unsaid", () => {
  const notConnected: ActionBlock = { kind: "notConnected" };
  const missing: ActionBlock = { kind: "missingInput", needs: "Enter your provider id to check." };

  it("says a repeated obstacle briefly rather than printing it verbatim again", () => {
    // What he saw: the identical four-line sentence twice in a row.
    const [first, second] = sequenceReasons([notConnected, notConnected]);
    expect(first!.style).toBe("full");
    expect(second!.style).toBe("brief");
    expect(renderReason(first!)).toBe(describeActionBlock(notConnected));
    expect(renderReason(second!)).not.toBe(renderReason(first!));
  });

  it("never leaves a control silent, which is the older and more important rule", () => {
    // Suppressing the repeat outright is the obvious fix and it re-opens the defect this module was
    // written for: a disabled control with nothing on screen saying why.
    for (const reason of sequenceReasons([notConnected, notConnected, notConnected])) {
      expect(reason).not.toBeNull();
      expect(renderReason(reason!).trim().length).toBeGreaterThan(0);
    }
  });

  it("keeps the brief form specific to its obstacle, so it is still an explanation", () => {
    const [, repeat] = sequenceReasons([notConnected, notConnected]);
    expect(renderReason(repeat!)).toMatch(/wallet is not connected/i);
  });

  it("compares the SENTENCE, not the kind, so two different demands are both said in full", () => {
    // `missingInput` carries a whole sentence supplied by the call site, and each flow's is about a
    // different field. Deduping on the kind would silence flow 3's "enter your contract address"
    // because flow 1 had already asked for a provider id.
    const other: ActionBlock = { kind: "missingInput", needs: "Enter your contract address." };
    const [a, b] = sequenceReasons([missing, other]);
    expect(a!.style).toBe("full");
    expect(b!.style).toBe("full");
  });

  it("keeps the FIELD in a deduped missing-input reason, because it is the reason's whole identity", () => {
    // What a captain hit on flow 2: provider id empty, the address field on THAT card filled, and
    // the deduped reason reading "a field it needs is still empty" - the one obstacle whose short
    // form named nothing, on the one kind whose identity is supplied per call site and so cannot be
    // inferred from context. The brief form is now the sentence's own first sentence.
    const [, repeat] = sequenceReasons([missing, missing]);
    expect(repeat!.style).toBe("brief");
    expect(renderReason(repeat!)).toBe("Enter your provider id to check.");
    expect(renderReason(repeat!)).not.toMatch(/a field it needs is still empty/i);
  });

  it("shortens a multi-sentence demand to its first sentence, which still names the field", () => {
    const flow3: ActionBlock = {
      kind: "missingInput",
      needs:
        "Enter your contract address in step 2 first. A domain is published for a contract, so "
        + "there is nothing to check until this page knows which one.",
    };
    const [, repeat] = sequenceReasons([flow3, flow3]);
    expect(renderReason(repeat!)).toBe("Enter your contract address in step 2 first.");
  });

  it("keeps a non-field obstacle routed through missingInput honest in brief form", () => {
    // The register reads arrive through the same parameter, and "a field it needs is still empty"
    // was FALSE for them outright - nothing was empty, a read was pending or failed. The first
    // sentence carries whatever the obstacle actually was.
    const pending: ActionBlock = {
      kind: "missingInput",
      needs:
        "Still reading which registers DogTag approves. This needs your wallet connected, because "
        + "there is no backend here to read the chain for you.",
    };
    const [, repeat] = sequenceReasons([pending, pending]);
    expect(renderReason(repeat!)).toBe("Still reading which registers DogTag approves.");
  });

  it("does not let an interpolated error's own punctuation truncate the sentence around it", () => {
    // One call site interpolates a read failure into a parenthetical. An error message is free text
    // and may carry periods; the sentence boundary is outside the parentheses.
    expect(
      firstSentence(
        "The registers you may choose from could not be read (HTTP 500. Internal error), so there "
          + "is nothing to check against. That is this page's connection to the chain.",
      ),
    ).toBe(
      "The registers you may choose from could not be read (HTTP 500. Internal error), so there "
        + "is nothing to check against.",
    );
  });

  it("names the check in a deduped never-checked reason, for the same reason", () => {
    // Reachable, not hypothetical: flow 3 renders three sends against one plan state, so the second
    // and third dedupe. "The check is unavailable" named no check.
    const never: ActionBlock = { kind: "neverChecked", check: "Check the domain record" };
    const [, repeat] = sequenceReasons([never, never]);
    expect(renderReason(repeat!)).toMatch(/"Check the domain record"/);
  });

  it("gives the full sentence to the FIRST control, not a later one", () => {
    const [first, second] = sequenceReasons([notConnected, notConnected]);
    expect(first!.style).toBe("full");
    expect(second!.style).toBe("brief");
  });
});

describe("the superseded banner", () => {
  it("does not tell you to check again when Check is unavailable", () => {
    // A screen that instructs an action it has disabled is a dead end: it names one remedy, that
    // remedy is greyed out, and nothing connects the two.
    const text = describePlanRetirement("spent", { kind: "notConnected" });
    expect(text).not.toMatch(/check again before sending/i);
    expect(text).toMatch(/not available while your wallet is not connected/i);
  });

  it("does tell you to check again when Check is available", () => {
    const text = describePlanRetirement("spent", null);
    expect(text).toMatch(/check again before sending another/i);
  });

  it("names the obstacle briefly, because the control above has already said it in full", () => {
    const text = describePlanRetirement("edited", { kind: "wrongChain", expected: 135, actual: 1 });
    expect(text).toMatch(/chain 1 and this deployment is on chain 135/);
    // Not the four-line sentence again - that would be the duplication defect inside the fix.
    expect(text.length).toBeLessThan(describeActionBlock({ kind: "notConnected" }).length * 2);
  });

  it("names the FIELD when the blocked check wants one, in the obstacle's own words", () => {
    // Check once, then clear the provider id: the plan is edited AND the check is blocked by the
    // now-empty field. The banner used to say "not available while a field it needs is still
    // empty" - the same nameless shortening the control reasons had, one surface over.
    const text = describePlanRetirement("edited", {
      kind: "missingInput",
      needs: "Enter your provider id to check.",
    });
    expect(text).toMatch(/Enter your provider id to check\.$/);
    expect(text).not.toMatch(/a field it needs is still empty/i);
  });
});
