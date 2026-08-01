// Flows 1 and 3 (registry-plan S-15): deploy-your-own-clone, and the TYPED domain claim.
//
// The captain's domain ruling is that "nobody has said", "deliberately no domain" and "a claim was
// withdrawn" are three different facts, where the predecessor could express only one of them as
// `""` - which every consumer then rendered as "this issuer has published no domain on-chain", a
// publication decision the issuer may never have made. So the assertions below are that the four
// dispositions produce four DISTINCT sentences, not merely that each produces some sentence.
import { describe, expect, it, vi } from "vitest";
import {
  assessDomainClaim,
  canWithdraw,
  createLiveProviderReader,
  describeDisposition,
  DomainDisposition,
  planCloneDeployment,
  Standing,
  validateDomain,
  ZERO_ADDR,
  type Address,
  type HexWord,
  type ProviderChainReader,
} from "../src/provider";

const PROVIDER: HexWord = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const RECORD_TYPE: HexWord = `0x${"ab".repeat(32)}`;
const CALLER: Address = "0x2222222222222222222222222222222222222222";
const SERVICE: Address = "0x1111111111111111111111111111111111111111";
const PREDICTED: Address = "0x9999999999999999999999999999999999999999";

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

// -------------------------------------------------------------------------------------------------
// Flow 1 - deploy
// -------------------------------------------------------------------------------------------------

const deployBase = { providerId: PROVIDER, recordType: RECORD_TYPE, caller: CALLER, cloneNonce: 0n };

describe("deploy preflight", () => {
  const ok = (extra: Partial<ProviderChainReader> = {}) =>
    reader({
      provider: async () => ({
        controller: CALLER,
        directoryResolver: ZERO_ADDR,
        standing: Standing.ACTIVE,
      }),
      canCreateService: async () => true,
      predictIssuer: async () => PREDICTED,
      isFactoryClone: async () => false,
      ...extra,
    });

  it("shows the exact address before the transaction", async () => {
    const plan = await planCloneDeployment(deployBase, ok());
    expect(plan.canDeploy).toBe(true);
    expect(plan.predictedAddress).toBe(PREDICTED);
    expect(plan.nextStep).toMatch(/You will own the contract/i);
  });

  it("says the attachment step is DogTag's, not the provider's", async () => {
    const plan = await planCloneDeployment(deployBase, ok());
    expect(plan.nextStep).toMatch(/DogTag then attaches it/i);
  });

  it("refuses a nonce whose address already exists", async () => {
    const plan = await planCloneDeployment(deployBase, ok({ isFactoryClone: async () => true }));
    expect(plan.canDeploy).toBe(false);
    expect(plan.verdict).toBe("refused");
    expect(plan.nextStep).toMatch(/different contract number/i);
  });

  it("refuses when the provider is not cleared, naming the standing", async () => {
    const plan = await planCloneDeployment(
      deployBase,
      ok({
        provider: async () => ({
          controller: CALLER,
          directoryResolver: ZERO_ADDR,
          standing: Standing.SUSPENDED,
        }),
      }),
    );
    expect(plan.verdict).toBe("refused");
    expect(plan.checks.find((c) => c.id === "provider-standing")!.finding).toMatch(/suspended/i);
  });

  it("an unreadable approval is indeterminate, not a refusal", async () => {
    const plan = await planCloneDeployment(
      deployBase,
      ok({
        canCreateService: async () => {
          throw new Error("HTTP request failed: 429");
        },
      }),
    );
    expect(plan.verdict).toBe("indeterminate");
    expect(plan.canDeploy).toBe(false);
    expect(plan.checks.find((c) => c.id === "deploy-authority")!.couldNotRunReason).toContain("429");
  });

  it("a prediction that could not be read leaves the address UNDEFINED, not blank-looking", async () => {
    // A blank where a provider expects an address reads as "no address yet", which is a different
    // and wrong fact from "we could not compute it".
    const plan = await planCloneDeployment(
      deployBase,
      ok({
        predictIssuer: async () => {
          throw new Error("reverted");
        },
      }),
    );
    expect(plan.predictedAddress).toBeUndefined();
    expect(plan.verdict).toBe("indeterminate");
  });
});

// -------------------------------------------------------------------------------------------------
// The msg.sender trap - the sharpest thing in this slice
// -------------------------------------------------------------------------------------------------

describe("canCreateService is asked AS THE FACTORY, because msg.sender is one of its terms", () => {
  it("sends `account` = the configured factory on the eth_call", async () => {
    // `canCreateService`'s first term is `generationOfFactory[msg.sender]`. A plain eth_call carries
    // no `from`, so msg.sender is the zero address, no generation matches, and the answer is FALSE
    // for every provider on earth - a preflight without this would tell an eligible provider it may
    // not deploy. Same shape as `ProviderRegistry.isWhitelistedFor`'s msg.sender branch, which
    // docs/CLIENT_REPOINT.md records as the reason ISSUER_REGISTRY_ADDR cannot move.
    //
    // Asserted on what reaches `readContract`, not on the returned value: a fake that answers
    // `true` would satisfy any assertion about the result while the `account` was silently dropped.
    const readContract = vi.fn().mockResolvedValue(true);
    const contracts = {
      core: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9" as Address,
      factory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6" as Address,
      domainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33" as Address,
      directory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0" as Address,
    };
    const live = createLiveProviderReader({
      contracts,
      client: { readContract } as never,
    });

    await live.canCreateService(PROVIDER, RECORD_TYPE, CALLER);

    expect(readContract).toHaveBeenCalledTimes(1);
    const call = readContract.mock.calls[0]![0] as { account?: string; address: string };
    expect(call.address).toBe(contracts.core);
    expect(call.account).toBe(contracts.factory);
  });

  it("does NOT send an account on the reads that have no msg.sender term", async () => {
    // Scoped deliberately: attaching a `from` everywhere would be cargo-culting the fix onto reads
    // whose answers do not depend on it, and would make the one place it matters unremarkable.
    const readContract = vi.fn().mockResolvedValue(false);
    const live = createLiveProviderReader({
      contracts: {
        core: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9" as Address,
        factory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6" as Address,
        domainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33" as Address,
        directory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0" as Address,
      },
      client: { readContract } as never,
    });

    await live.isFactoryClone(SERVICE);
    await live.canWriteProviderRecord(PROVIDER, CALLER);

    for (const [args] of readContract.mock.calls) {
      expect((args as { account?: string }).account).toBeUndefined();
    }
  });
});

// -------------------------------------------------------------------------------------------------
// Flow 3 - the typed domain claim
// -------------------------------------------------------------------------------------------------

describe("domain state is typed: three absences are three facts", () => {
  it("gives each of the four dispositions its own sentence", () => {
    const sentences = [
      describeDisposition(DomainDisposition.UNSET, ""),
      describeDisposition(DomainDisposition.NO_DOMAIN, ""),
      describeDisposition(DomainDisposition.CLAIMED, "clinic.example.sg"),
      describeDisposition(DomainDisposition.CLEARED, ""),
    ];
    expect(new Set(sentences).size).toBe(4);
    expect(sentences[0]).toMatch(/nobody has said/i);
    expect(sentences[1]).toMatch(/deliberately publishes no domain/i);
    expect(sentences[2]).toContain("clinic.example.sg");
    expect(sentences[3]).toMatch(/withdrawn/i);
  });

  it("never says 'nobody has said' on the strength of a read that failed", async () => {
    // UNSET is a FACT the chain states. A failed read is not that fact, and defaulting to it would
    // re-create exactly the conflation the typed enum was introduced to end.
    const a = await assessDomainClaim(
      SERVICE,
      CALLER,
      reader({
        domainClaimStanding: async () => {
          throw new Error("HTTP request failed: 503");
        },
        canWriteDomain: async () => true,
      }),
    );
    expect(a.standing).toBeUndefined();
    expect(a.description).toBeUndefined();
    expect(a.verdict).toBe("indeterminate");
    expect(a.canWrite).toBe(false);
  });

  it("offers withdrawal only when a claim exists to withdraw", () => {
    const of = (d: DomainDisposition) => ({
      disposition: d,
      domain: d === DomainDisposition.CLAIMED ? "clinic.example.sg" : "",
      lineageRecognizesService: true,
      registryApprovesThisResolver: true,
      coreSelectsThisResolver: true,
      serviceStandingEffective: true,
    });
    expect(canWithdraw(of(DomainDisposition.CLAIMED))).toBe(true);
    expect(canWithdraw(of(DomainDisposition.UNSET))).toBe(false);
    expect(canWithdraw(of(DomainDisposition.NO_DOMAIN))).toBe(false);
    expect(canWithdraw(of(DomainDisposition.CLEARED))).toBe(false);
    expect(canWithdraw(undefined)).toBe(false);
  });

  it("reports a FROZEN service separately, and says its domain is history", async () => {
    // `serviceStandingEffective` is deliberately outside the resolver verdict: a frozen contract's
    // CLAIMED record is history rather than a live claim, and a consumer must be able to tell.
    const a = await assessDomainClaim(
      SERVICE,
      CALLER,
      reader({
        domainClaimStanding: async () => ({
          disposition: DomainDisposition.CLAIMED,
          domain: "clinic.example.sg",
          lineageRecognizesService: true,
          registryApprovesThisResolver: true,
          coreSelectsThisResolver: true,
          serviceStandingEffective: false,
        }),
        canWriteDomain: async () => false,
      }),
    );
    expect(a.verdict).toBe("refused");
    expect(a.checks.find((c) => c.id === "clone-standing")!.finding).toMatch(/history, not a current claim/i);
  });

  it("names WHICH resolver term is missing rather than saying only 'not live'", async () => {
    const a = await assessDomainClaim(
      SERVICE,
      CALLER,
      reader({
        domainClaimStanding: async () => ({
          disposition: DomainDisposition.UNSET,
          domain: "",
          lineageRecognizesService: true,
          registryApprovesThisResolver: false,
          coreSelectsThisResolver: false,
          serviceStandingEffective: true,
        }),
        canWriteDomain: async () => false,
      }),
    );
    const finding = a.checks.find((c) => c.id === "domain-resolver-live")!.finding;
    expect(finding).toMatch(/not approved/i);
    expect(finding).toMatch(/has not selected/i);
    expect(finding).not.toMatch(/not recognized by the issuer lineage/i);
  });
});

describe("the domain grammar mirrors the contract, and rejects rather than normalizes", () => {
  it("accepts a canonical domain", () => {
    expect(validateDomain("seaport-vet.example-clinic.sg")).toEqual({
      ok: true,
      domain: "seaport-vet.example-clinic.sg",
    });
  });

  it("REFUSES uppercase instead of silently lowercasing it", () => {
    // The contract rejects rather than normalizes so what a resolver reads is exactly what the
    // writer intended. A convenience lowercaser here would publish a claim the provider did not
    // type, the contract would accept it, and the provider would never learn.
    const r = validateDomain("Seaport-Vet.SG");
    expect(r.ok).toBe(false);
    expect(r.ok === false && r.reason).toMatch(/lowercase/i);
  });

  it("NEVER returns a domain it altered - refusal is the only response to a non-canonical one", () => {
    // The property the case above only half pins. Uppercase is refused twice over (an explicit case
    // check, then the charset rule), so deleting either one still refuses - which makes "it was
    // refused" a weak assertion about NORMALIZING. This is the assertion that is not weak: for every
    // input that is accepted, the returned domain is the input verbatim apart from trimming.
    for (const input of [
      "clinic.example.sg",
      "  clinic.example.sg  ",
      "Seaport-Vet.SG",
      "MiXeD.CaSe.sg",
      "a.sg",
    ]) {
      const r = validateDomain(input);
      if (r.ok) expect(r.domain).toBe(input.trim());
    }
    // And specifically: a lowercasing implementation would ACCEPT this and return a different
    // string. Both halves matter - accepted-at-all, and accepted-as-typed.
    const mixed = validateDomain("MiXeD.CaSe.sg");
    expect(mixed.ok).toBe(false);
  });

  it("refuses every shape the contract's BadDomain refuses", () => {
    for (const bad of [
      "",
      "singlelabel",
      "a..b",
      ".leading.sg",
      "trailing.sg.",
      "-lead.sg",
      "trail-.sg",
      "under_score.sg",
      `${"a".repeat(64)}.sg`,
      `${"a".repeat(250)}.example.sg`,
    ]) {
      expect(validateDomain(bad).ok, `expected ${JSON.stringify(bad)} to be refused`).toBe(false);
    }
  });

  it("accepts a 63-character label, the exact boundary the contract allows", () => {
    expect(validateDomain(`${"a".repeat(63)}.sg`).ok).toBe(true);
  });
});

// -------------------------------------------------------------------------------------------------
// Every plan carries the inputs it was computed FROM
// -------------------------------------------------------------------------------------------------

// The other half of "a send acts on what was checked". The component invalidates a plan when its
// inputs change, and addresses the plan's OWN values - so the plan has to carry them. A plan that
// stopped doing so would leave the send with nothing to address but the live form, which is the
// defect: check one contract, write to another.
describe("a plan is answerable about what it judged", () => {
  it("the deploy plan carries the record type and number it checked", async () => {
    const plan = await planCloneDeployment(
      { ...deployBase, cloneNonce: 7n },
      reader({
        provider: async () => ({
          controller: CALLER,
          directoryResolver: ZERO_ADDR,
          standing: Standing.ACTIVE,
        }),
        canCreateService: async () => true,
        predictIssuer: async () => PREDICTED,
        isFactoryClone: async () => false,
      }),
    );
    expect(plan.request).toEqual({
      providerId: PROVIDER,
      recordType: RECORD_TYPE,
      caller: CALLER,
      cloneNonce: 7n,
    });
  });

  it("the domain assessment carries the contract it judged", async () => {
    const assessment = await assessDomainClaim(
      SERVICE,
      CALLER,
      reader({
        domainClaimStanding: async () => ({
          disposition: DomainDisposition.UNSET,
          domain: "",
          lineageRecognizesService: true,
          registryApprovesThisResolver: true,
          coreSelectsThisResolver: true,
          serviceStandingEffective: true,
        }),
        canWriteDomain: async () => true,
      }),
    );
    expect(assessment.service).toBe(SERVICE);
  });
});
