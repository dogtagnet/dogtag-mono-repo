// The provider's own half of the typed-resolver pair: SELECTING one.
//
// WHAT THIS COVERS AND WHY IT IS NOT OPTIONAL. Flows 3 and 4 of the provider page were gated on a
// selection the provider had no way to make - `setDirectoryResolver` and `setDomainResolver` existed
// only in the contracts and their forge tests. Measured on the live set before this shipped: both
// resolvers approved fleet-wide, every provider's `directoryResolver` and every service's
// `domainResolver` still the zero address. So a vet could not publish a listing, which means they
// never appear in the directory and nothing a pet owner searches can find them.
//
// The reader is UNSCRIPTED BY DEFAULT - every method throws unless a case supplies it - so a case
// cannot pass because a read quietly defaulted. That matters most for the standing rows: a fake that
// answered `true` for everything would make the standing-before-authority ordering untestable, and
// that ordering is what keeps this surface from telling a provider their key is at fault for a
// refusal only DogTag can clear.
import { describe, expect, it } from "vitest";
import {
  assessResolverSelection,
  canStopUsing,
  describeSelection,
  kindForScope,
  ResolverKind,
  selectionChangeRefusal,
  Standing,
  ZERO_ADDR,
  type Address,
  type HexWord,
  type ProviderChainReader,
  type ResolverListing,
  type ResolverSelectionScope,
} from "../src/provider";

const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const SERVICE: Address = "0x1111111111111111111111111111111111111111";
const CALLER: Address = "0x2222222222222222222222222222222222222222";
/** The approved register - the one a provider may choose. */
const APPROVED: Address = "0xda784f9b9d54684882210facc2c38d9a9d259f78";
/** Listed by the registry and its approval WITHDRAWN. Still in `resolverPage`, and not a choice. */
const WITHDRAWN: Address = "0xbbe7922d13e992022915c972522deb76b54ab3f4";
/** Never named by the registry at all - what a hand-typed or pasted address looks like. */
const UNLISTED: Address = "0xcccccccccccccccccccccccccccccccccccccccc";

const BOTH_LISTED: readonly ResolverListing[] = [
  { resolver: APPROVED, approved: true },
  { resolver: WITHDRAWN, approved: false },
];

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
    approvedResolvers: unscripted("approvedResolvers"),
    canWriteProviderDirectoryResolver: unscripted("canWriteProviderDirectoryResolver"),
    canWriteServiceDomainResolver: unscripted("canWriteServiceDomainResolver"),
    ...overrides,
  } as ProviderChainReader;
}

/** A reader whose every read succeeds, for the directory scope. */
function directoryReader(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  return reader({
    approvedResolvers: async () => BOTH_LISTED,
    provider: async () => ({
      controller: CALLER,
      directoryResolver: ZERO_ADDR,
      standing: Standing.ACTIVE,
    }),
    canWriteProviderDirectoryResolver: async () => true,
    ...overrides,
  });
}

/** A reader whose every read succeeds, for the domain scope. */
function domainReader(overrides: Partial<ProviderChainReader> = {}): ProviderChainReader {
  return reader({
    approvedResolvers: async () => BOTH_LISTED,
    service: async () => ({
      providerId: PROVIDER,
      factoryGeneration: `0x${"11".repeat(32)}`,
      recordType: `0x${"22".repeat(32)}`,
      confirmedOwner: CALLER,
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
    canWriteServiceDomainResolver: async () => true,
    ...overrides,
  });
}

const dir = (chosen: Address, r: ProviderChainReader) =>
  assessResolverSelection({ scope: "directory", subject: PROVIDER, chosen, caller: CALLER, reader: r });
const dom = (chosen: Address, r: ProviderChainReader) =>
  assessResolverSelection({ scope: "domain", subject: SERVICE, chosen, caller: CALLER, reader: r });

/** One check by id, with a message naming the id when it is absent rather than a bare undefined. */
function check(
  plan: { checks: readonly { id: string; outcome: string; finding: string }[] },
  id: string,
) {
  const found = plan.checks.find((c) => c.id === id);
  expect(found, `no check with id ${id}; got ${plan.checks.map((c) => c.id).join(", ")}`).toBeDefined();
  return found!;
}

// -------------------------------------------------------------------------------------------------
// The register list is APPEND-ONLY, and this is the trap most likely to ship
// -------------------------------------------------------------------------------------------------

describe("what may be chosen comes from the chain, and a withdrawn register is never a choice", () => {
  it("offers only the APPROVED entries, although the registry still lists the withdrawn one", async () => {
    // `setResolverApproved` pushes onto `_resolverAddresses[kind]` the first time it sees an address
    // and NEVER removes it - pulling a register only flips the approval mapping. So `resolverPage`
    // answers "every register ever named", which is not "what may I choose". Offering a withdrawn
    // entry is a button whose write reverts `ResolverNotApproved`, and the mistake is invisible on
    // today's chain, where each kind has exactly one entry and it is approved.
    const plan = await dir(APPROVED, directoryReader());
    expect(plan.choices).toEqual([APPROVED]);
    expect(plan.withdrawn).toEqual([WITHDRAWN]);
  });

  it("reports the withdrawn count in the finding, so a short list is legible rather than mysterious", async () => {
    const plan = await dir(APPROVED, directoryReader());
    expect(check(plan, "resolver-choices").finding).toMatch(/approves 1 provider directory/i);
    expect(check(plan, "resolver-choices").finding).toMatch(/1 more whose approval it has withdrawn/i);
  });

  it("an UNREADABLE list leaves `choices` undefined, never an empty array", async () => {
    // An empty array says DogTag has approved nothing - a statement about DogTag that a read which
    // failed is in no position to make. The distinction has to survive into the plan, because the
    // renderer's only other option is an empty dropdown, which a reader takes at face value.
    const plan = await dir(APPROVED, directoryReader({
      approvedResolvers: async () => {
        throw new Error("rate limited");
      },
    }));
    expect(plan.choices).toBeUndefined();
    expect(plan.withdrawn).toBeUndefined();
    const row = check(plan, "resolver-choices");
    expect(row.outcome).toBe("could-not-run");
    expect(plan.verdict).not.toBe("ready");
  });

  it("an EMPTY approved list is a definite answer, and says nothing is selectable yet", async () => {
    const plan = await dir(APPROVED, directoryReader({ approvedResolvers: async () => [] }));
    expect(plan.choices).toEqual([]);
    expect(check(plan, "resolver-choices").outcome).toBe("pass");
    expect(check(plan, "resolver-choices").finding).toMatch(/approved no provider directory/i);
  });

  it("asks the right ResolverKind for each scope", async () => {
    // The two kinds are separate keyspaces on the contract, so asking the wrong one returns another
    // kind's allowlist - and every choice would then be refused for a reason nothing on screen could
    // explain.
    const seen: number[] = [];
    const spy = (r: ProviderChainReader) =>
      ({ ...r, approvedResolvers: async (k: ResolverKind) => (seen.push(k), BOTH_LISTED) }) as ProviderChainReader;
    await dir(APPROVED, spy(directoryReader()));
    await dom(APPROVED, spy(domainReader()));
    expect(seen).toEqual([ResolverKind.DIRECTORY, ResolverKind.DOMAIN]);
    expect(kindForScope("directory")).toBe(ResolverKind.DIRECTORY);
    expect(kindForScope("domain")).toBe(ResolverKind.DOMAIN);
  });
});

// -------------------------------------------------------------------------------------------------
// Three selection states, and the third is the one that would otherwise be invisible
// -------------------------------------------------------------------------------------------------

describe("what is selected now is three states, and a failed read is none of them", () => {
  it("the zero address is `none`", async () => {
    const plan = await dir(APPROVED, directoryReader());
    expect(plan.selection).toEqual({ kind: "none" });
  });

  it("an approved selector is `selected`", async () => {
    const plan = await dir(ZERO_ADDR, directoryReader({
      provider: async () => ({ controller: CALLER, directoryResolver: APPROVED, standing: Standing.ACTIVE }),
    }));
    expect(plan.selection).toEqual({ kind: "selected", resolver: APPROVED });
  });

  it("a selector whose approval was WITHDRAWN is its own state, not `none`", async () => {
    // REACHABLE, and the contract says so: the core never clears a stored selection when the
    // registrar deapproves a register, and `setDomainResolver`'s own doc calls the stored selector a
    // historical record for exactly this reason. So a provider can be pointed at a register that
    // answers nothing - which needs repairing - and rendering that as "nothing is selected" would
    // describe a provider who has never started.
    const plan = await dir(ZERO_ADDR, directoryReader({
      provider: async () => ({ controller: CALLER, directoryResolver: WITHDRAWN, standing: Standing.ACTIVE }),
    }));
    expect(plan.selection).toEqual({ kind: "selectedButPulled", resolver: WITHDRAWN });
  });

  it("the three states produce three DISTINCT sentences", async () => {
    const sentences = new Set([
      describeSelection("directory", { kind: "none" }),
      describeSelection("directory", { kind: "selected", resolver: APPROVED }),
      describeSelection("directory", { kind: "selectedButPulled", resolver: WITHDRAWN }),
    ]);
    expect(sentences.size).toBe(3);
  });

  it("the withdrawn-underneath sentence says the content is not deleted, and what fixes it", async () => {
    const words = describeSelection("directory", { kind: "selectedButPulled", resolver: WITHDRAWN });
    expect(words).toMatch(/withdrawn its approval/i);
    expect(words).toMatch(/not deleted/i);
  });

  it("a non-zero selector plus an unreadable list is could-not-run, NOT `selected`", async () => {
    // The one case that could quietly over-claim: the SELECTOR read succeeded and names a register,
    // but whether it is still approved did not - so the state is genuinely between `selected` and
    // `selectedButPulled`, and reporting either invents one.
    const plan = await dir(ZERO_ADDR, directoryReader({
      approvedResolvers: async () => {
        throw new Error("no");
      },
      provider: async () => ({ controller: CALLER, directoryResolver: APPROVED, standing: Standing.ACTIVE }),
    }));
    expect(plan.selection).toBeUndefined();
    expect(plan.description).toBeUndefined();
    const row = check(plan, "resolver-selection-current");
    expect(row.outcome).toBe("could-not-run");
    // Names the address anyway, because it is the one thing that WAS established.
    expect(row.finding).toMatch(/0xda784f9b/i);
  });

  it("an unreadable SELECTOR read is could-not-run and never `none`", async () => {
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => {
        throw new Error("down");
      },
    }));
    expect(plan.selection).toBeUndefined();
    expect(check(plan, "resolver-selection-current").outcome).toBe("could-not-run");
  });

  it("reads the SERVICE's selector for the domain scope and the PROVIDER's for the directory scope", async () => {
    // Keyed differently on purpose: one listing per provider however many contracts it holds, one
    // domain per contract. Reading the wrong one would report another subject's selection entirely.
    const domPlan = await dom(APPROVED, domainReader({
      service: async () => ({
        providerId: PROVIDER,
        factoryGeneration: `0x${"11".repeat(32)}`,
        recordType: `0x${"22".repeat(32)}`,
        confirmedOwner: CALLER,
        domainResolver: WITHDRAWN,
        ownerEpoch: 1n,
        standing: Standing.ACTIVE,
      }),
    }));
    expect(domPlan.selection).toEqual({ kind: "selectedButPulled", resolver: WITHDRAWN });
    expect(domPlan.subject).toBe(SERVICE);
  });
});

// -------------------------------------------------------------------------------------------------
// Is the choice one the chain would accept
// -------------------------------------------------------------------------------------------------

describe("the choice is checked against the registry's own allowlist", () => {
  it("an approved register passes", async () => {
    const plan = await dir(APPROVED, directoryReader());
    expect(check(plan, "resolver-choice-approved").outcome).toBe("pass");
    expect(plan.verdict).toBe("ready");
  });

  it("a WITHDRAWN register is refused, and says it was approved once", async () => {
    const plan = await dir(WITHDRAWN, directoryReader());
    const row = check(plan, "resolver-choice-approved");
    expect(row.outcome).toBe("fail");
    expect(row.finding).toMatch(/approval has been withdrawn/i);
    expect(plan.canSelect).toBe(false);
  });

  it("an UNLISTED register is refused, and says it was never approved", async () => {
    const plan = await dir(UNLISTED, directoryReader());
    const row = check(plan, "resolver-choice-approved");
    expect(row.outcome).toBe("fail");
    expect(row.finding).toMatch(/is not a provider directory DogTag has approved/i);
  });

  it("those two refusals are DIFFERENT sentences - the remedies differ", async () => {
    // One was approved and DogTag pulled it; the other was never approved, which is what a pasted or
    // mistyped address looks like. Telling the second person to ask DogTag why they withdrew an
    // approval that never existed sends them to the wrong place.
    //
    // ASSERTED ON THE DISTINGUISHING CLAUSE, NOT ON WHOLE-STRING INEQUALITY. A mutation run caught the
    // weaker version: both sentences EMBED THE ADDRESS, and the two addresses differ, so
    // `expect(a).not.toBe(b)` held even after both branches were made to produce identical wording.
    // A test whose subject is a distinction has to name the distinction.
    const pulled = check(await dir(WITHDRAWN, directoryReader()), "resolver-choice-approved").finding;
    const never = check(await dir(UNLISTED, directoryReader()), "resolver-choice-approved").finding;
    expect(pulled).toMatch(/approval has been withdrawn/i);
    expect(pulled).not.toMatch(/is not a provider directory DogTag has approved/i);
    expect(never).toMatch(/is not a provider directory DogTag has approved/i);
    expect(never).not.toMatch(/approval has been withdrawn/i);
  });

  it("STOPPING is always allowed, whatever the registry approves", async () => {
    // `setDirectoryResolver` checks approval only for a NON-ZERO resolver, so the deselect is
    // accepted even when every register has been withdrawn - and it must be, or a provider pointed at
    // a dead register would have no way out.
    const plan = await dir(ZERO_ADDR, directoryReader({
      approvedResolvers: async () => [{ resolver: WITHDRAWN, approved: false }],
      provider: async () => ({ controller: CALLER, directoryResolver: WITHDRAWN, standing: Standing.ACTIVE }),
    }));
    expect(check(plan, "resolver-choice-approved").outcome).toBe("pass");
    expect(plan.canSelect).toBe(true);
  });

  it("cannot answer the choice question when the list is unreadable, and does not guess either way", async () => {
    const plan = await dir(APPROVED, directoryReader({
      approvedResolvers: async () => {
        throw new Error("nope");
      },
    }));
    expect(check(plan, "resolver-choice-approved").outcome).toBe("could-not-run");
  });

  it("compares addresses case-insensitively - hex casing is a checksum, not an identity", async () => {
    const plan = await dir(APPROVED.toUpperCase().replace("0X", "0x") as Address, directoryReader());
    expect(check(plan, "resolver-choice-approved").outcome).toBe("pass");
  });
});

// -------------------------------------------------------------------------------------------------
// Standing BEFORE authority, reported apart - the trap a dropdown makes easy to skip
// -------------------------------------------------------------------------------------------------

describe("standing is read before authority, and a refusal is never blamed on the key", () => {
  it("a PENDING provider record fails on standing and says DogTag activates it", async () => {
    // THE STATE EVERY NEW PROVIDER IS IN. `registerProvider` writes `PENDING` and activation is a
    // separate registrar call, so this is the first thing a provider meets - and it is not their
    // doing.
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => ({ controller: CALLER, directoryResolver: ZERO_ADDR, standing: Standing.PENDING }),
      canWriteProviderDirectoryResolver: async () => false,
    }));
    const standing = check(plan, "provider-standing");
    expect(standing.outcome).toBe("fail");
    expect(standing.finding).toMatch(/DogTag activates it/i);
    expect(standing.finding).toMatch(/nothing you have set up is wrong/i);
  });

  it("and the authority row is could-not-run, NOT a refusal of the key", async () => {
    // `canWriteProvider` returns false on `standing != ACTIVE` BEFORE it looks at the caller, so a
    // bare `false` here says nothing about this key. Rendering it as "your key may not" is an
    // accusation nobody was in a position to make - the same defect as the issuer-whitelist pillar's,
    // pointed at the provider's own property.
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => ({ controller: CALLER, directoryResolver: ZERO_ADDR, standing: Standing.PENDING }),
      canWriteProviderDirectoryResolver: async () => false,
    }));
    const auth = check(plan, "resolver-write-authority");
    expect(auth.outcome).toBe("could-not-run");
    expect(auth.couldNotRunReason).toMatch(/says nothing about your key/i);
    // `refused` still outranks `indeterminate`: the standing row is a definite failure.
    expect(plan.verdict).toBe("refused");
  });

  it("refuses the key ONLY when standing definitely holds", async () => {
    const plan = await dir(APPROVED, directoryReader({
      canWriteProviderDirectoryResolver: async () => false,
    }));
    const auth = check(plan, "resolver-write-authority");
    expect(auth.outcome).toBe("fail");
    expect(auth.finding).toMatch(/controller on your provider record/i);
  });

  it("an unreadable standing makes the authority row could-not-run for its OWN reason", async () => {
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => {
        throw new Error("down");
      },
      canWriteProviderDirectoryResolver: async () => false,
    }));
    const auth = check(plan, "resolver-write-authority");
    expect(auth.outcome).toBe("could-not-run");
    expect(auth.couldNotRunReason).toMatch(/could not be read/i);
    expect(auth.couldNotRunReason).not.toMatch(/refuses this write from ANY key/i);
  });

  it("an unreadable AUTHORITY read is could-not-run rather than a refusal", async () => {
    const plan = await dir(APPROVED, directoryReader({
      canWriteProviderDirectoryResolver: async () => {
        throw new Error("boom");
      },
    }));
    expect(check(plan, "resolver-write-authority").outcome).toBe("could-not-run");
  });

  it("names the CONTRACT OWNER for the domain scope and the CONTROLLER for the directory scope", async () => {
    const d = check(await dir(APPROVED, directoryReader({ canWriteProviderDirectoryResolver: async () => false })), "resolver-write-authority");
    const s = check(await dom(APPROVED, domainReader({ canWriteServiceDomainResolver: async () => false })), "resolver-write-authority");
    expect(d.finding).toMatch(/controller on your provider record/i);
    expect(s.finding).toMatch(/owner of this contract/i);
  });
});

describe("the domain scope folds FOUR standing terms and deliberately not the fifth", () => {
  it("an unconfirmed owner is HELD PENDING CONFIRMATION, which is not frozen", async () => {
    // A quarantine is cleared by `confirmServiceOwner`; a freeze is cleared by nothing. Calling one
    // the other is how this page told a captain a contract was frozen when it was merely pending.
    const plan = await dom(APPROVED, domainReader({
      effectiveService: async () => ({
        providerStanding: Standing.ACTIVE,
        serviceStanding: Standing.ACTIVE,
        factoryActive: true,
        ownerConfirmed: false,
        hasActiveIssuer: true,
      }),
      canWriteServiceDomainResolver: async () => false,
    }));
    const row = check(plan, "clone-standing");
    expect(row.outcome).toBe("fail");
    expect(row.finding).toMatch(/pending confirmation/i);
    expect(row.finding).not.toMatch(/frozen/i);
  });

  it("a retired factory generation has its own clause", async () => {
    const plan = await dom(APPROVED, domainReader({
      effectiveService: async () => ({
        providerStanding: Standing.ACTIVE,
        serviceStanding: Standing.ACTIVE,
        factoryActive: false,
        ownerConfirmed: true,
        hasActiveIssuer: true,
      }),
      canWriteServiceDomainResolver: async () => false,
    }));
    expect(check(plan, "clone-standing").finding).toMatch(/generation that deployed this contract has been retired/i);
  });

  it("`hasActiveIssuer` false does NOT block a register choice", async () => {
    // THE FOUR-NOT-FIVE RULE. `canWriteService` folds provider standing, service standing, factory
    // generation and the confirmed-owner match - and deliberately not `hasActiveIssuer`, which is
    // about whether the contract can anchor credentials. Folding it in would refuse a selection the
    // chain would happily accept, and this is the exact state a provider is in before they have
    // repointed: `canIssue` false while everything else holds.
    const plan = await dom(APPROVED, domainReader({
      effectiveService: async () => ({
        providerStanding: Standing.ACTIVE,
        serviceStanding: Standing.ACTIVE,
        factoryActive: true,
        ownerConfirmed: true,
        hasActiveIssuer: false,
      }),
    }));
    expect(check(plan, "clone-standing").outcome).toBe("pass");
    expect(plan.verdict).toBe("ready");
    expect(plan.canSelect).toBe(true);
  });

  it("names EVERY blocker rather than only the first", async () => {
    const plan = await dom(APPROVED, domainReader({
      effectiveService: async () => ({
        providerStanding: Standing.SUSPENDED,
        serviceStanding: Standing.RETIRED,
        factoryActive: true,
        ownerConfirmed: true,
        hasActiveIssuer: true,
      }),
      canWriteServiceDomainResolver: async () => false,
    }));
    const finding = check(plan, "clone-standing").finding;
    expect(finding).toMatch(/suspended/i);
    expect(finding).toMatch(/retired/i);
  });
});

// -------------------------------------------------------------------------------------------------
// The no-op the chain refuses
// -------------------------------------------------------------------------------------------------

describe("a write the registry would refuse as a no-op is refused here, with its own sentence", () => {
  it("choosing what is already selected fails", async () => {
    // Both setters `revert NoChange()` when the value is already stored, so offering the button
    // produces a revert the provider cannot interpret.
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => ({ controller: CALLER, directoryResolver: APPROVED, standing: Standing.ACTIVE }),
    }));
    const row = check(plan, "resolver-selection-change");
    expect(row.outcome).toBe("fail");
    expect(row.finding).toMatch(/already your provider directory/i);
    expect(plan.canSelect).toBe(false);
  });

  it("stopping when nothing is selected fails, and says so differently", async () => {
    const plan = await dir(ZERO_ADDR, directoryReader());
    const row = check(plan, "resolver-selection-change");
    expect(row.outcome).toBe("fail");
    expect(row.finding).toMatch(/nothing to stop using/i);
  });

  it("the two no-op sentences differ", () => {
    const a = selectionChangeRefusal("directory", { kind: "selected", resolver: APPROVED }, APPROVED);
    const b = selectionChangeRefusal("directory", { kind: "none" }, ZERO_ADDR);
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();
    expect(a).not.toBe(b);
  });

  it("emits NO no-op row when the current value could not be read", async () => {
    // Refusing on the strength of a read that failed would block a write the chain would have
    // accepted - the inverse of the fail-open, and just as wrong.
    const plan = await dir(APPROVED, directoryReader({
      provider: async () => {
        throw new Error("down");
      },
    }));
    expect(plan.checks.find((c) => c.id === "resolver-selection-change")).toBeUndefined();
    expect(selectionChangeRefusal("directory", undefined, APPROVED)).toBeNull();
  });

  it("a REPLACEMENT is not a no-op", async () => {
    const plan = await dir(APPROVED, directoryReader({
      approvedResolvers: async () => [
        { resolver: APPROVED, approved: true },
        { resolver: WITHDRAWN, approved: false },
      ],
      provider: async () => ({ controller: CALLER, directoryResolver: WITHDRAWN, standing: Standing.ACTIVE }),
    }));
    expect(plan.checks.find((c) => c.id === "resolver-selection-change")).toBeUndefined();
    expect(plan.canSelect).toBe(true);
    expect(plan.nextStep).toMatch(/makes your published content readable again/i);
  });
});

// -------------------------------------------------------------------------------------------------
// What the send addresses, and the verdict fold
// -------------------------------------------------------------------------------------------------

describe("the plan carries its own inputs back out, so a send acts on what was checked", () => {
  it("captures the subject and the chosen register", async () => {
    const plan = await dir(APPROVED, directoryReader());
    expect(plan.subject).toBe(PROVIDER);
    expect(plan.chosen).toBe(APPROVED);
    expect(plan.scope).toBe("directory");
  });

  it("`canSelect` is true only on a ready verdict", async () => {
    for (const [r, expected] of [
      [directoryReader(), true],
      [directoryReader({ canWriteProviderDirectoryResolver: async () => false }), false],
      [
        directoryReader({
          approvedResolvers: async () => {
            throw new Error("x");
          },
        }),
        false,
      ],
    ] as const) {
      const plan = await dir(APPROVED, r as ProviderChainReader);
      expect(plan.canSelect).toBe(expected);
    }
  });

  it("a definite failure outranks a neighbouring could-not-run", async () => {
    // `refused` beats `indeterminate`, deliberately: a definite failure is not softened by a
    // non-answer beside it, and a non-answer never raises severity to a refusal either.
    const plan = await dir(UNLISTED, directoryReader({
      canWriteProviderDirectoryResolver: async () => {
        throw new Error("x");
      },
    }));
    expect(plan.verdict).toBe("refused");
  });

  it("says what to do next in every verdict, and never instructs an action the plan has refused", async () => {
    const ready = await dir(APPROVED, directoryReader());
    const refused = await dir(UNLISTED, directoryReader());
    const indeterminate = await dir(APPROVED, directoryReader({
      approvedResolvers: async () => {
        throw new Error("x");
      },
    }));
    expect(ready.nextStep).toMatch(/turns the rest of this flow on/i);
    expect(refused.nextStep).toMatch(/failed checks above say why/i);
    expect(indeterminate.nextStep).toMatch(/could not run/i);
    expect(new Set([ready.nextStep, refused.nextStep, indeterminate.nextStep]).size).toBe(3);
  });
});

describe("stopping is offered only when there is something to stop", () => {
  it("is refused for an unread selection and for nothing-selected, and offered for both live states", () => {
    // `undefined` grouped with `none` here on purpose: both mean the button must not appear, and for
    // the unread case that is the honest answer - we cannot say a withdrawal is available.
    expect(canStopUsing(undefined)).toBe(false);
    expect(canStopUsing({ kind: "none" })).toBe(false);
    expect(canStopUsing({ kind: "selected", resolver: APPROVED })).toBe(true);
    expect(canStopUsing({ kind: "selectedButPulled", resolver: WITHDRAWN })).toBe(true);
  });
});

describe("both scopes are exercised, so neither ships on the other's coverage", () => {
  const scopes: readonly ResolverSelectionScope[] = ["directory", "domain"];
  it("produces a ready plan for each", async () => {
    const plans = [await dir(APPROVED, directoryReader()), await dom(APPROVED, domainReader())];
    expect(plans.map((p) => p.scope)).toEqual(scopes);
    expect(plans.every((p) => p.canSelect)).toBe(true);
    // Each names its own register in its own words, so a shared sentence cannot pass for both.
    expect(plans[0]!.description).toMatch(/provider directory/i);
    expect(plans[1]!.nextStep).toMatch(/domain register/i);
  });
});
