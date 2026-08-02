/**
 * @vitest-environment jsdom
 */
// Whether the provider self-service page explains ITSELF (registry-plan S-15).
//
// The page is the primary surface and the click-through guide is the backup, so a first-time
// provider must not need the guide open to use it. That is a claim about what is RENDERED before
// any wallet is connected and before any check is run, which is why this is mounted: every sentence
// here type-checks whether or not it reaches the screen, and the browser walk that found the gaps
// is not repeatable.
//
// Four things are pinned, each of which was a real hole rather than a hypothetical one:
//
//   1. A DISABLED CONTROL SAYS WHY. Flow 3's Check is gated on the CONTRACT ADDRESS in flow 2, not
//      on the domain field beside it, so typing a domain and finding the button still dead was the
//      page's most confusing state - and nothing said a word about it. Worse than a refusal, which
//      at least names itself.
//   2. A FLOW THAT WAITS ON DOGTAG SAYS SO BEFORE IT IS TRIED. Stated as a DEPENDENCY, which is
//      permanent, never as a status, which would go stale the day the step is taken.
//   3. THE PREDICTED ADDRESS IS LABELLED AS A PREDICTION. An unlabelled address on a page that has
//      deployed nothing reads as something that already exists.
//   4. PROGRESSIVE DISCLOSURE, NOT A WALL. The mechanism is present and CLOSED by default - a page
//      nobody reads because it is dense fails the same way an unexplained one does.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DeployPlanCard } from "../src/domain/ProviderSelfServicePanel";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import {
  ATTACHMENT_IS_A_DOGTAG_STEP,
  ATTACHMENT_IS_NOT_SELF_SERVICE,
  DIRECTORY_NEEDS_TURNING_ON,
  DOMAIN_REGISTER_NEEDS_TURNING_ON,
  type DeployPlan,
  type ProviderContracts,
} from "../src/provider";

// Nothing under test reads more of wagmi than these two hooks, and the claims here are about copy
// rendered before a wallet is involved at all.
vi.mock("wagmi", () => ({
  useAccount: () => ({ address: undefined, isConnected: false }),
  useWriteContract: () => ({ writeContractAsync: async () => "0x" }),
}));

const CONTRACTS: ProviderContracts = {
  core: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9",
  factory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6",
  domainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33",
  directory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0",
};

let root: Root | null = null;
let host: HTMLElement | null = null;

const turn = () => new Promise((r) => setTimeout(r, 0));

async function render(node: ReturnType<typeof createElement>) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(node);
  await turn();
  await turn();
  return host;
}

const mount = (capabilities: { issuance: boolean; listing: boolean }) =>
  render(
    createElement(ProviderSelfServiceFlows, {
      contracts: CONTRACTS,
      missingConfig: [],
      capabilities,
    }),
  );

function type(id: string, value: string) {
  const input = host!.querySelector<HTMLInputElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const testId = (id: string) => host!.querySelector(`[data-testid='${id}']`);

afterEach(() => {
  root?.unmount();
  host?.remove();
  root = null;
  host = null;
});

describe("a control that cannot be used says why", () => {
  it("names the field flow 3 actually depends on, which is flow 2's and not the one beside it", async () => {
    // The gate is `!candidate`. A provider typing into the Domain field is looking at the wrong
    // input entirely, so the reason has to name the other step by number.
    const el = await mount({ issuance: true, listing: true });
    const note = el.querySelector("[data-testid='candidate-required-domain']");
    expect(note).not.toBeNull();
    expect(note!.textContent).toMatch(/step 2/i);
  });

  it("withdraws that reason once the field it names is filled", async () => {
    // The half that makes the case above non-vacuous: a note rendered unconditionally would satisfy
    // it while telling a provider who has already done the thing to go and do it.
    await mount({ issuance: true, listing: true });
    expect(testId("candidate-required-domain")).not.toBeNull();
    expect(testId("candidate-required-repoint")).not.toBeNull();

    type("candidate", "0x1111111111111111111111111111111111111111");
    await turn();

    expect(testId("candidate-required-domain")).toBeNull();
    expect(testId("candidate-required-repoint")).toBeNull();
  });

  it("says the same thing on flow 2, whose Check is gated on that field too", async () => {
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='candidate-required-repoint']")).not.toBeNull();
  });
});

describe("a flow that waits on a DogTag step says so before it is tried", () => {
  it("states the dependency on flows 2, 3 and 4, with no check run and no wallet connected", async () => {
    // BEFORE clicking is the whole point: hitting the wall first and reading the explanation second
    // is the experience this closes.
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='repoint-dependency']")!.textContent).toBe(
      ATTACHMENT_IS_A_DOGTAG_STEP,
    );
    expect(el.querySelector("[data-testid='domain-dependency']")!.textContent).toBe(
      DOMAIN_REGISTER_NEEDS_TURNING_ON,
    );
    expect(el.querySelector("[data-testid='directory-dependency']")!.textContent).toBe(
      DIRECTORY_NEEDS_TURNING_ON,
    );
  });

  it("tells the provider it is not their doing, which is the half that makes it usable", async () => {
    // A dependency stated without this reads as "you have misconfigured something", and sends a
    // provider hunting for a setting that does not exist.
    //
    // Asserted per notice against its OWN clause rather than against one shared phrase: each names
    // the thing that provider would otherwise go and re-check - what they deployed, how they set
    // themselves up, what they typed into the form - and flattening them onto a common sentence
    // would buy a tidier test by making the copy vaguer where it is most needed. The alternative,
    // one regex loose enough to match all three, would pass on almost any sentence containing
    // "not".
    const el = await mount({ issuance: true, listing: true });
    const notice = (id: string) => el.querySelector(`[data-testid='${id}']`)!.textContent!;
    expect(notice("repoint-dependency")).toMatch(/nothing is wrong with what you deployed/i);
    expect(notice("domain-dependency")).toMatch(/not something you have set up wrongly/i);
    expect(notice("directory-dependency")).toMatch(/what you have filled in is not the problem/i);
  });

  it("names the DogTag step rather than only reporting that something is missing", async () => {
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='repoint-dependency']")!.textContent).toMatch(/attach/i);
    expect(el.querySelector("[data-testid='domain-dependency']")!.textContent).toMatch(/approve/i);
    expect(el.querySelector("[data-testid='directory-dependency']")!.textContent).toMatch(
      /approve/i,
    );
  });

  it("states a DEPENDENCY and never a current status, so it cannot go stale", async () => {
    // The rule these notices are written under. A sentence claiming the flow is blocked RIGHT NOW
    // would be a chain fact asserted from no read at all, and would become false the day the step
    // is taken - with nothing to make it false in the source.
    const el = await mount({ issuance: true, listing: true });
    for (const id of ["repoint-dependency", "domain-dependency", "directory-dependency"]) {
      const text = el.querySelector(`[data-testid='${id}']`)!.textContent!;
      expect(text).not.toMatch(/currently|right now|is blocked|has not been done/i);
    }
  });

  it("shows the groomer only the directory dependency, the other two being inapplicable", async () => {
    const el = await mount({ issuance: false, listing: true });
    expect(el.querySelector("[data-testid='directory-dependency']")).not.toBeNull();
    expect(el.querySelector("[data-testid='repoint-dependency']")).toBeNull();
    expect(el.querySelector("[data-testid='domain-dependency']")).toBeNull();
  });
});

describe("the contract number explains why it exists, without becoming a wall", () => {
  it("names all three inputs the address is computed from", async () => {
    // The captain's question was "why is there a number", and the answer is the salt: record type,
    // wallet, number. Two are fixed, so this is the only dial - which is the fact the old hint
    // never carried.
    const el = await mount({ issuance: true, listing: true });
    const why = el.querySelector("[data-testid='why-contract-number']")!;
    // The ENUMERATION, not the words scattered anywhere in the disclosure. Asserting
    // /record type/ and /wallet/ alone passed with the three-input sentence deleted outright,
    // because both also occur in the paragraphs below it - a mutation caught that, and a test
    // that survives the removal of the thing it is named for is pinning nothing.
    expect(why.textContent).toMatch(/exactly three things[^.]*record type[^.]*wallet[^.]*number/i);
    expect(why.textContent).toMatch(/only one you can vary/i);
  });

  it("says what is lost without it, and points at the flow that is its other half", async () => {
    const el = await mount({ issuance: true, listing: true });
    const why = el.querySelector("[data-testid='why-contract-number']")!.textContent!;
    expect(why).toMatch(/one possible address/i);
    expect(why).toMatch(/step 2/i);
  });

  it("is CLOSED by default, so the page stays short for someone who does not need it", async () => {
    // Progressive disclosure is the design constraint, not decoration: a page nobody reads because
    // it is dense fails the same way an unexplained one does. `open` absent is what keeps the
    // mechanism one click away rather than in front of everybody.
    const el = await mount({ issuance: true, listing: true });
    const why = el.querySelector<HTMLDetailsElement>("[data-testid='why-contract-number']")!;
    expect(why.tagName).toBe("DETAILS");
    expect(why.open).toBe(false);
    expect(why.querySelector("summary")).not.toBeNull();
  });

  it("keeps a one-line hint outside the disclosure, so a closed page still says something", async () => {
    const el = await mount({ issuance: true, listing: true });
    // Rendered whether or not the disclosure is opened: the field is not left bare.
    expect(el.textContent).toMatch(/only part of your contract's address that you choose/i);
    expect(el.textContent).toMatch(/leave it at 0/i);
  });
});

describe("the predicted address is labelled as a prediction", () => {
  const plan = (predictedAddress?: `0x${string}`): DeployPlan => ({
    request: {
      providerId: "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af",
      recordType: `0x${"ab".repeat(32)}`,
      caller: "0x2222222222222222222222222222222222222222",
      cloneNonce: 0n,
    },
    checks: [],
    verdict: "ready",
    ...(predictedAddress ? { predictedAddress } : {}),
    canDeploy: true,
    nextStep: "Ready to deploy.",
  });

  it("says the address is exact and that nothing has been created yet", async () => {
    // Both halves matter and they pull in opposite directions: "exact" is what makes Check worth
    // reading, and "nothing created yet" is what stops it being read as a receipt.
    const el = await render(
      createElement(DeployPlanCard, {
        plan: plan("0x0505Ac77cb3244936d50665A3636090f05Ef0CC1"),
        attachmentNotice: ATTACHMENT_IS_NOT_SELF_SERVICE,
      }),
    );
    const caption = el.querySelector("[data-testid='predicted-address-caption']")!;
    expect(caption.textContent).toMatch(/exact/i);
    expect(caption.textContent).toMatch(/nothing has been created yet/i);
    expect(el.querySelector("[data-testid='predicted-address']")!.textContent).toBe(
      "0x0505Ac77cb3244936d50665A3636090f05Ef0CC1",
    );
  });

  it("offers no caption when there is no address, rather than captioning an absence", async () => {
    // The existing rule this must not break: a failed prediction renders its own amber line, and a
    // caption promising an exact address beside it would describe something that is not there.
    const el = await render(
      createElement(DeployPlanCard, { plan: plan(), attachmentNotice: ATTACHMENT_IS_NOT_SELF_SERVICE }),
    );
    expect(el.querySelector("[data-testid='predicted-address-caption']")).toBeNull();
    expect(el.querySelector("[data-testid='predicted-unavailable']")).not.toBeNull();
  });
});

describe("a groomer is told why its page is shorter", () => {
  it("explains the absence rather than leaving a page that looks truncated", async () => {
    // Without it a groomer sees one card and cannot tell whether the rest is missing, hidden or
    // broken - the same could-not-tell-what-happened shape the rest of this page is written against.
    const el = await mount({ issuance: false, listing: true });
    const note = el.querySelector("[data-testid='listing-only-note']")!;
    expect(note.textContent).toMatch(/verify/i);
    expect(note.textContent).toMatch(/nothing here for you to deploy/i);
  });

  it("does not show that note to a vet, which has the flows it describes as absent", async () => {
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='listing-only-note']")).toBeNull();
  });
});
