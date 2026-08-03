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

// Mutable, because the reasons under a disabled control are ORDERED - a disconnected wallet
// outranks an empty field, which outranks an unrun check - so each case has to be driven from its
// own account state rather than asserted all at once.
const account: { address?: string; isConnected: boolean; chainId?: number } = {
  address: undefined,
  isConnected: false,
};
vi.mock("wagmi", () => ({
  useAccount: () => account,
  useWriteContract: () => ({ writeContractAsync: async () => "0x" }),
}));

const CONNECTED = "0x2222222222222222222222222222222222222222";
function connect(chainId?: number) {
  account.address = CONNECTED;
  account.isConnected = true;
  account.chainId = chainId;
}
function disconnect() {
  account.address = undefined;
  account.isConnected = false;
  account.chainId = undefined;
}

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
  disconnect();
});

const reason = (id: string) => host!.querySelector(`[data-testid='${id}']`);
const reasonKind = (id: string) => reason(id)?.getAttribute("data-block") ?? null;

describe("a control that cannot be used says why", () => {
  it("names the field flow 3 actually depends on, which is flow 2's and not the one beside it", async () => {
    // The gate is `!candidate`. A provider typing into the Domain field is looking at the wrong
    // input entirely, so the reason has to name the other step by number AND say the button is not
    // gated on the domain they just typed.
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af");
    await turn();
    const note = el.querySelector("[data-testid='domain-check-reason']")!;
    expect(note.getAttribute("data-block")).toBe("missingInput");
    expect(note.textContent).toMatch(/step 2/i);
    expect(note.textContent).toMatch(/not on the domain above/i);
  });

  it("withdraws that reason once the field it names is filled", async () => {
    // The half that makes the case above non-vacuous: a note rendered unconditionally would satisfy
    // it while telling a provider who has already done the thing to go and do it.
    connect(135);
    await mount({ issuance: true, listing: true });
    type("providerId", "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af");
    await turn();
    expect(reasonKind("domain-check-reason")).toBe("missingInput");
    expect(reasonKind("repoint-check-reason")).toBe("missingInput");

    type("candidate", "0x1111111111111111111111111111111111111111");
    await turn();

    expect(reason("domain-check-reason")).toBeNull();
    expect(reason("repoint-check-reason")).toBeNull();
  });

  it("says the same thing on flow 2, whose Check is gated on that field too", async () => {
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af");
    await turn();
    expect(el.querySelector("[data-testid='repoint-check-reason']")!.textContent).toMatch(
      /contract you deployed in step 1/i,
    );
  });
});

describe("the FIRST-RUN state - a disabled Deploy with nothing said, which is the defect", () => {
  // Found by a captain on a live stack. Deploy is gated on a plan, no plan exists before the first
  // check, and the notice that explains a stale plan renders nothing when there is no plan at all -
  // so every new provider met a dead button under a heading reading "You deploy it and you own it".
  const PROVIDER = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";

  it("tells a connected provider who has checked nothing to run the check first", async () => {
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    await turn();

    const send = el.querySelector<HTMLButtonElement>("[data-testid='deploy-send']")!;
    expect(send.disabled).toBe(true);
    // The disabled button MUST be accompanied by a reason. This is the assertion the page failed.
    const why = el.querySelector("[data-testid='deploy-send-reason']");
    expect(why).not.toBeNull();
    expect(why!.getAttribute("data-block")).toBe("neverChecked");
    expect(why!.textContent).toMatch(/check what this would deploy/i);
  });

  it("explains that the check has to match what is in the form now, not merely that one is needed", async () => {
    // Without this half a provider who checks once, edits a field and finds the button dead again
    // has no way to have predicted it - the gate looks arbitrary rather than deliberate.
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    await turn();
    expect(el.querySelector("[data-testid='deploy-send-reason']")!.textContent).toMatch(
      /match the values in the form right now/i,
    );
  });

  it("gives every send button a reason in that state, not only Deploy", async () => {
    // Deploy is where it was found; the same shape held for all four flows.
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    type("candidate", "0x1111111111111111111111111111111111111111");
    await turn();
    // Enumerated from the rendered buttons rather than a hand list, so a send added later is
    // covered without anyone remembering to add it here. A hand list is how flow 3's three sends
    // came to share one reason in the first place.
    const sends = Array.from(el.querySelectorAll<HTMLButtonElement>("[data-testid$='-send']"));
    expect(sends.length).toBeGreaterThanOrEqual(4);
    for (const b of sends) {
      const id = b.getAttribute("data-testid")!.replace(/-send$/, "-send-reason");
      const why = el.querySelector(`[data-testid='${id}']`);
      expect(why, `${id} missing`).not.toBeNull();
      expect(why!.getAttribute("data-block")).toBe("neverChecked");
    }
  });

  it("never leaves a disabled send button silent, in any of the states this page has", async () => {
    // The general form of the rule. A disabled control with no reason is the defect whatever put it
    // in that state, so the invariant is asserted over the buttons rather than over one case.
    for (const setup of [
      () => disconnect(),
      () => connect(135),
      () => connect(1),
    ]) {
      setup();
      const el = await mount({ issuance: true, listing: true });
      const sends = Array.from(el.querySelectorAll<HTMLButtonElement>("[data-testid$='-send']"));
      expect(sends.length).toBeGreaterThan(0);
      for (const b of sends) {
        if (!b.disabled) continue;
        const id = b.getAttribute("data-testid")!.replace(/-send$/, "-send-reason");
        expect(el.querySelector(`[data-testid='${id}']`), `${id} missing`).not.toBeNull();
      }
      root?.unmount();
      host?.remove();
    }
  });

  it("orders the reasons so the first thing to fix is the one shown", async () => {
    // Six situations with six remedies. A disconnected wallet outranks an unrun check, because
    // telling somebody to press Check when nothing can be signed sends them in a circle.
    disconnect();
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='deploy-send-reason']")!.getAttribute("data-block")).toBe(
      "notConnected",
    );
  });

  it("reports the wrong chain as its own reason, and only for sends", async () => {
    // The checks read through the page's own endpoint, so they are correct on any chain; only a
    // transaction is bound to the wallet's. Gating the checks on it would refuse a preflight that
    // would have answered usefully.
    connect(1);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    await turn();
    expect(el.querySelector("[data-testid='deploy-send-reason']")!.getAttribute("data-block")).toBe(
      "wrongChain",
    );
    expect(el.querySelector("[data-testid='deploy-send-reason']")!.textContent).toMatch(/chain 1\b/);
    // The Check button is untouched: no reason at all, because nothing is blocking it.
    expect(reason("deploy-check-reason")).toBeNull();
  });

  it("does not call an unreported chain the wrong one", async () => {
    // A connector that reports no chain id is not a wallet on the wrong network. Accusing it would
    // be could-not-check rendered as a definite answer - the thing this page exists to avoid.
    connect(undefined);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    await turn();
    expect(el.querySelector("[data-testid='deploy-send-reason']")!.getAttribute("data-block")).toBe(
      "neverChecked",
    );
  });
});

describe("a wallet fault is never rendered as a verdict about the provider", () => {
  // The captain saw "The requested method and/or account has not been authorized by the user." in
  // red under his provider id, while the admin portal showed that same provider ACTIVE and approved
  // for VACCINATION. The chain said authorized; the page said not. The page was wrong - that string
  // is EIP-1193 4100, raised by a wallet extension about a SITE, and it establishes nothing about a
  // provider record.
  const PROVIDER = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";

  it("is not rendered inside the provider-record card, where a verdict belongs", async () => {
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    type("providerId", PROVIDER);
    await turn();
    // Position is the half a label cannot fix: under the provider id, the record type and the
    // caller address, any red sentence reads as an answer about that provider.
    const record = el.querySelector("[data-testid='provider-record-card']");
    if (record) expect(record.querySelector("[data-testid='wallet-fault']")).toBeNull();
    // And the element that used to carry it there is gone entirely.
    expect(el.querySelector("[data-testid='page-error']")).toBeNull();
  });

  it("renders nothing at all when there is no fault", async () => {
    connect(135);
    const el = await mount({ issuance: true, listing: true });
    expect(el.querySelector("[data-testid='wallet-fault']")).toBeNull();
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
