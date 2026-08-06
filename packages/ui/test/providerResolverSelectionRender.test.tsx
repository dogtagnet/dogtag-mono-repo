/**
 * @vitest-environment jsdom
 */
// The register-selection controls, MOUNTED.
//
// WHY MOUNTED. Every claim here is about what a provider can REACH and what a control SAYS when it
// cannot be used, and neither is a property of a pure function. The engine's own suite
// (`providerResolverSelection.test.ts`) covers the decisions; this covers the four honesty rules that
// live in the component:
//
//   1. NEVER A DEAD BUTTON. A disabled control says why AND what is needed first.
//   2. THREE STATES AFTER A SIGNATURE - it worked, it failed, we could not tell - never a blank.
//   3. A REFUSAL IS VISIBLE. Not 12pt grey under a button; a previous version of this exact page did
//      that and a captain read it as nothing happening.
//   4. NOT YET IS NOT NO. A pending read is not a refusal, and an empty dropdown is not "DogTag has
//      approved nothing".
//
// The stale-plan guard gets its own case here rather than in `providerSendGuards.test.tsx` because a
// DROPDOWN is precisely the input that makes it easy to forget: check register A, switch the select
// to B, and without the choice in the plan key the button stays enabled while the CHECKED plan still
// carries A.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package. `act()` drains React's work into its own queue and reorders promise
// continuations against passive effects, which is exactly what would hide a stale-plan defect.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import { ResolverKind, Standing, ZERO_ADDR, type ProviderContracts } from "../src/provider";

const CALLER = "0x2222222222222222222222222222222222222222";
const PROVIDER = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const CLONE = "0x1111111111111111111111111111111111111111";
const APPROVED_DIR = "0xda784f9b9d54684882210facc2c38d9a9d259f78";
const APPROVED_DOM = "0xbbe7922d13e992022915c972522deb76b54ab3f4";
const SECOND_DIR = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const WITHDRAWN_DIR = "0xdddddddddddddddddddddddddddddddddddddddd";
const HASH = `0x${"cd".repeat(32)}`;

const writeContractAsync = vi.fn(async () => HASH);
let chainId: number | undefined = 135;
let connected = true;
vi.mock("wagmi", () => ({
  useAccount: () => ({ address: connected ? CALLER : undefined, isConnected: connected, chainId }),
  useWriteContract: () => ({ writeContractAsync }),
}));

/** Overridable per case, so a scenario changes only what it is about. */
let approvedResolvers = vi.fn(async (kind: ResolverKind) =>
  kind === ResolverKind.DIRECTORY
    ? [
        { resolver: APPROVED_DIR, approved: true },
        { resolver: SECOND_DIR, approved: true },
        { resolver: WITHDRAWN_DIR, approved: false },
      ]
    : [{ resolver: APPROVED_DOM, approved: true }],
);
let directorySelected = ZERO_ADDR;
let domainSelected = ZERO_ADDR;
let providerStanding = Standing.ACTIVE;
let canWriteDirectory = vi.fn(async () => true);
let canWriteDomainResolver = vi.fn(async () => true);

vi.mock("../src/provider/liveReader", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/provider/liveReader")>();
  return {
    ...actual,
    createLiveProviderReader: () => ({
      isFactoryClone: async () => true,
      cloneOwner: async () => CALLER,
      service: async () => ({
        providerId: PROVIDER,
        factoryGeneration: `0x${"11".repeat(32)}`,
        recordType: `0x${"ab".repeat(32)}`,
        confirmedOwner: CALLER,
        domainResolver: domainSelected,
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
      provider: async () => ({
        controller: CALLER,
        directoryResolver: directorySelected,
        standing: providerStanding,
      }),
      currentService: async () => ZERO_ADDR,
      canWriteServiceRepoint: async () => true,
      approvedResolvers,
      canWriteProviderDirectoryResolver: canWriteDirectory,
      canWriteServiceDomainResolver: canWriteDomainResolver,
      issuerCreations: async () => [],
      providerProfileAnchor: async () => ({
        digest: `0x${"0".repeat(64)}`,
        schema: 0,
        codec: 0,
        hashAlgorithm: 0,
        revision: 0n,
      }),
    }),
  };
});

const receipt = vi.fn(async () => ({ status: "success" }));
vi.mock("../src/wallet/contracts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/wallet/contracts")>();
  return { ...actual, roaxPublicClient: () => ({ waitForTransactionReceipt: receipt }) };
});

const CONTRACTS: ProviderContracts = {
  core: "0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112",
  factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
  domainResolver: "0x7A9b0C1d2E3F4a5B6C7d8e9f0A1B2c3D4e5F6A7b",
  directory: "0x8b0c1D2E3f4a5B6c7D8E9F0A1B2C3d4E5f6A7b8C",
};

let root: Root | null = null;
let host: HTMLElement;

const turn = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
  for (let i = 0; i < 14; i++) await turn();
};

function type(id: string, value: string) {
  const input = host.querySelector<HTMLInputElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function pick(id: string, value: string) {
  const select = host.querySelector<HTMLSelectElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
  setter.call(select, value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

const el = (testId: string) => host.querySelector<HTMLElement>(`[data-testid='${testId}']`);
const button = (testId: string) => host.querySelector<HTMLButtonElement>(`[data-testid='${testId}']`)!;

async function mount(capabilities = { issuance: true, listing: true }) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(
    createElement(ProviderSelfServiceFlows, {
      contracts: CONTRACTS,
      missingConfig: [],
      capabilities,
    }),
  );
  await settle();
}

beforeEach(() => {
  writeContractAsync.mockClear();
  receipt.mockClear();
  receipt.mockImplementation(async () => ({ status: "success" }));
  chainId = 135;
  connected = true;
  directorySelected = ZERO_ADDR;
  domainSelected = ZERO_ADDR;
  providerStanding = Standing.ACTIVE;
  canWriteDirectory = vi.fn(async () => true);
  canWriteDomainResolver = vi.fn(async () => true);
  approvedResolvers = vi.fn(async (kind: ResolverKind) =>
    kind === ResolverKind.DIRECTORY
      ? [
          { resolver: APPROVED_DIR, approved: true },
          { resolver: SECOND_DIR, approved: true },
          { resolver: WITHDRAWN_DIR, approved: false },
        ]
      : [{ resolver: APPROVED_DOM, approved: true }],
  );
});

afterEach(() => {
  root?.unmount();
  host.remove();
  root = null;
});

// -------------------------------------------------------------------------------------------------
// The selection is reachable at all - the whole point of the branch
// -------------------------------------------------------------------------------------------------

describe("a provider can now make both selections from their own portal", () => {
  it("renders a register picker in the domain flow AND in the listing flow", async () => {
    await mount();
    expect(el("domain-register-select")).not.toBeNull();
    expect(el("directory-register-select")).not.toBeNull();
    expect(button("domain-register-send")).not.toBeNull();
    expect(button("directory-register-send")).not.toBeNull();
  });

  it("sends setDirectoryResolver with the providerId and the chosen register", async () => {
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    button("directory-register-send").click();
    await settle();
    expect(writeContractAsync).toHaveBeenCalledTimes(1);
    const call = writeContractAsync.mock.calls[0]![0] as unknown as {
      functionName: string;
      args: readonly unknown[];
      address: string;
    };
    expect(call.functionName).toBe("setDirectoryResolver");
    // The CORE, not the directory contract: the selection lives in `ProviderRegistry`, and sending it
    // to `ProviderDirectory` would revert at the dispatcher for no visible reason.
    expect(call.address).toBe(CONTRACTS.core);
    expect(call.args).toEqual([PROVIDER, APPROVED_DIR]);
  });

  it("sends setDomainResolver with the CONTRACT address, not the provider id", async () => {
    // Keyed differently on purpose. Passing the provider id here would be a well-formed 20-byte value
    // the chain reads as an unknown service - `UnknownService()`, which reads like the contract does
    // not exist.
    await mount();
    type("providerId", PROVIDER);
    type("candidate", CLONE);
    await settle();
    button("domain-register-check").click();
    await settle();
    button("domain-register-send").click();
    await settle();
    const call = writeContractAsync.mock.calls[0]![0] as unknown as {
      functionName: string;
      args: readonly unknown[];
    };
    expect(call.functionName).toBe("setDomainResolver");
    expect(call.args).toEqual([CLONE, APPROVED_DOM]);
  });
});

// -------------------------------------------------------------------------------------------------
// Rule 1: never a dead button
// -------------------------------------------------------------------------------------------------

describe("no control is ever disabled in silence", () => {
  it("explains the FIRST-RUN state, which is the one every provider begins in", async () => {
    // The state hardest to stumble into deliberately and easiest to ship broken: before any check
    // there is no plan, so the send is gated with nothing on screen saying so.
    await mount();
    type("providerId", PROVIDER);
    await settle();
    expect(button("directory-register-send").disabled).toBe(true);
    expect(el("directory-register-send-reason")!.textContent).toMatch(
      /Run "Check the directory register" first/i,
    );
  });

  it("names the OTHER STEP's field when that is what the domain register check is waiting on", async () => {
    // The one shape a "<field> is required" template cannot carry: the field lives in step 2.
    await mount();
    type("providerId", PROVIDER);
    await settle();
    expect(button("domain-register-check").disabled).toBe(true);
    expect(el("domain-register-check-reason")!.textContent).toMatch(/contract address in step 2/i);
  });

  it("names the wallet as the obstacle, in the BRIEF form the page-wide dedupe gives it", async () => {
    // The page says each obstacle in full ONCE, in source order, and in one line under every later
    // control it also blocks - so a register control that shares the page's wallet obstacle with a
    // control above it gets the short form. That is still a complete sentence NAMING the obstacle,
    // which is the rule; suppressing it entirely would make the control silent again.
    connected = false;
    await mount();
    expect(button("directory-register-check").disabled).toBe(true);
    expect(el("directory-register-check-reason")!.textContent).toMatch(/wallet is not connected/i);
    // And the full sentence really is said somewhere, so the brief form is a shortening rather than
    // the only thing the page ever says about it.
    expect(host.textContent).toMatch(/Connect your wallet first/i);
  });

  it("gates the SEND on the wallet's chain and leaves the CHECK alone", async () => {
    // The checks read through this page's own connection, so they are correct whatever the wallet is
    // pointed at. Gating them would refuse a preflight that would have answered usefully.
    chainId = 1;
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    expect(button("directory-register-send").disabled).toBe(true);
    expect(el("directory-register-send-reason")!.textContent).toMatch(/chain 1/);
    // The CHECK has no reason element at all - `ActionReason` renders nothing when a control is not
    // blocked - so the absence is the assertion, and the check button is genuinely usable.
    expect(el("directory-register-check-reason")).toBeNull();
    expect(button("directory-register-check").disabled).toBe(false);
  });

  it("every disabled register control on the page carries a reason", async () => {
    // Enumerated rather than hand-listed. A hand list is exactly how flow 3's three domain sends came
    // to share one reason, and the enumerating test caught it in the same run.
    connected = false;
    await mount();
    const controls = Array.from(
      host.querySelectorAll<HTMLButtonElement>("[data-testid*='-register-']"),
    ).filter((b) => b.tagName === "BUTTON");
    expect(controls.length).toBeGreaterThan(0);
    for (const b of controls) {
      expect(b.disabled, `${b.getAttribute("data-testid")} is enabled with no wallet`).toBe(true);
      const reason = el(`${b.getAttribute("data-testid")}-reason`);
      expect(reason, `${b.getAttribute("data-testid")} has no reason element`).not.toBeNull();
      expect(reason!.textContent!.trim().length).toBeGreaterThan(0);
    }
  });
});

// -------------------------------------------------------------------------------------------------
// Rule 4: not yet is not no
// -------------------------------------------------------------------------------------------------

describe("an empty dropdown is never how any of three different facts is reported", () => {
  it("says the list is STILL BEING READ rather than showing no options", async () => {
    // A `<select>` with no options is the one thing a reader takes at face value. Held open so the
    // pending state is observed rather than inferred.
    let release!: (v: unknown) => void;
    approvedResolvers = vi.fn(() => new Promise((res) => { release = res; }));
    await mount();
    expect(el("directory-register-options-pending")).not.toBeNull();
    expect(el("directory-register-select")).toBeNull();
    expect(el("directory-register-options-pending")!.textContent).toMatch(/Reading which registers/i);
    release([]);
    await settle();
  });

  it("says the read FAILED, and that it is this page's connection rather than the provider's setup", async () => {
    approvedResolvers = vi.fn(async () => {
      throw new Error("rate limited");
    });
    await mount();
    const notice = el("directory-register-options-unavailable");
    expect(notice).not.toBeNull();
    expect(notice!.textContent).toMatch(/could not be read/i);
    expect(notice!.textContent).toMatch(/not anything you have set up/i);
    expect(notice!.textContent).toMatch(/rate limited/);
    expect(el("directory-register-select")).toBeNull();
  });

  it("says DogTag has approved NONE - a definite fact - and distinguishes it from the other two", async () => {
    approvedResolvers = vi.fn(async () => []);
    await mount();
    expect(el("directory-register-options-none")).not.toBeNull();
    expect(el("directory-register-options-none")!.textContent).toMatch(/approved no directory register/i);
    expect(el("directory-register-options-none")!.textContent).toMatch(/That half is theirs/i);
    expect(el("directory-register-options-pending")).toBeNull();
    expect(el("directory-register-options-unavailable")).toBeNull();
  });

  it("the three states are three different elements, so none can be mistaken for another", async () => {
    // Asserted as a set of testids rather than by reading text, because the whole hazard is that they
    // would LOOK the same.
    const ids = new Set<string>();
    for (const impl of [
      vi.fn(async () => []),
      vi.fn(async () => {
        throw new Error("x");
      }),
    ]) {
      approvedResolvers = impl as never;
      await mount();
      for (const suffix of ["pending", "unavailable", "none"]) {
        if (el(`directory-register-options-${suffix}`)) ids.add(suffix);
      }
      root?.unmount();
      host.remove();
      root = null;
    }
    expect(ids).toEqual(new Set(["none", "unavailable"]));
  });
});

// -------------------------------------------------------------------------------------------------
// Rule 3: a refusal is visible, and the withdrawn entry is not a choice
// -------------------------------------------------------------------------------------------------

describe("what may be chosen is what the chain approves, and the rest is shown as not chooseable", () => {
  it("offers only approved registers as options", async () => {
    await mount();
    const values = Array.from(
      host.querySelectorAll<HTMLOptionElement>("#directory-register option"),
    ).map((o) => o.value);
    expect(values).toContain(APPROVED_DIR);
    expect(values).toContain(SECOND_DIR);
    expect(values).not.toContain(WITHDRAWN_DIR);
  });

  it("lists the withdrawn one separately, struck through, so a short list is legible", async () => {
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    const withdrawn = el("resolver-withdrawn-directory");
    expect(withdrawn).not.toBeNull();
    expect(withdrawn!.textContent).toMatch(/Approval withdrawn/i);
    expect(withdrawn!.querySelector("li")!.className).toMatch(/line-through/);
  });

  it("renders the refusal beside the control, not only inside the card", async () => {
    // A previous version of this page put a refusal in small grey text under a button and a captain
    // read it as nothing happening.
    canWriteDirectory = vi.fn(async () => false);
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    expect(button("directory-register-send").disabled).toBe(true);
    expect(el("directory-register-send-reason")!.textContent).toMatch(/last check refused this/i);
    expect(el("resolver-selection-directory")!.textContent).toMatch(/may not choose it/i);
  });

  it("does not offer STOP USING when nothing is selected", async () => {
    await mount();
    const values = Array.from(
      host.querySelectorAll<HTMLOptionElement>("#directory-register option"),
    ).map((o) => o.value);
    expect(values).not.toContain(ZERO_ADDR);
  });

  it("offers STOP USING once a selection has been read", async () => {
    // Without it the selection would be permanent from this page - the shape of dead end this whole
    // branch exists to remove.
    directorySelected = APPROVED_DIR;
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    const values = Array.from(
      host.querySelectorAll<HTMLOptionElement>("#directory-register option"),
    ).map((o) => o.value);
    expect(values).toContain(ZERO_ADDR);
    pick("directory-register", ZERO_ADDR);
    await settle();
    expect(button("directory-register-send").textContent).toMatch(/Stop using the directory/i);
  });

  it("labels the send by what it will DO, so the button is not a description of the widget", async () => {
    await mount();
    expect(button("directory-register-send").textContent).toMatch(/Use this directory register/i);
    expect(button("domain-register-send").textContent).toMatch(/Use this domain register/i);
  });
});

// -------------------------------------------------------------------------------------------------
// The stale-plan guard, which a dropdown makes easy to forget
// -------------------------------------------------------------------------------------------------

describe("switching the dropdown after Check retires the plan", () => {
  it("disables the send and says the answers describe what was picked before", async () => {
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    expect(button("directory-register-send").disabled).toBe(false);

    pick("directory-register", SECOND_DIR);
    await settle();
    expect(button("directory-register-send").disabled).toBe(true);
    expect(el("directory-register-stale")!.textContent).toMatch(/changed something since this was checked/i);
    expect(writeContractAsync).not.toHaveBeenCalled();
  });

  it("keeps the checked answers ON SCREEN while marking them superseded", async () => {
    // Retiring must not hide the card: what was checked before sending is the thing a provider most
    // wants the moment a transaction goes out.
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    pick("directory-register", SECOND_DIR);
    await settle();
    expect(el("resolver-selection-directory")).not.toBeNull();
    expect(host.querySelector("[data-testid^='verdict-retired-']")).not.toBeNull();
  });

  it("retires the plan on its own transaction too, not only on an edit", async () => {
    // A plan is keyed on the form, so an untouched form leaves the key matching - but every answer in
    // it came from the chain, and submitting moves that.
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    button("directory-register-send").click();
    await settle();
    expect(button("directory-register-send").disabled).toBe(true);
    expect(el("directory-register-send-reason")!.textContent).toMatch(/already been sent/i);
  });
});

// -------------------------------------------------------------------------------------------------
// Rule 2: three states after a signature, never a blank
// -------------------------------------------------------------------------------------------------

describe("a signature is followed by an outcome, never by nothing", () => {
  const send = async () => {
    await mount();
    type("providerId", PROVIDER);
    await settle();
    button("directory-register-check").click();
    await settle();
    button("directory-register-send").click();
    await settle();
  };

  it("reports a mined transaction as succeeded", async () => {
    await send();
    expect(el("sent-transactions")).not.toBeNull();
    expect(el("sent-succeeded")).not.toBeNull();
    expect(el("sent-transactions")!.textContent).toMatch(/Use this directory register/i);
  });

  it("reports a REVERTED transaction as failed rather than as done", async () => {
    // `writeContractAsync` resolves on a hash and does not throw on a revert, so the outcome comes
    // from the receipt.
    receipt.mockImplementation(async () => ({ status: "reverted" }));
    await send();
    expect(el("sent-reverted")).not.toBeNull();
    expect(el("sent-succeeded")).toBeNull();
  });

  it("reports an unfetchable receipt as COULD NOT TELL - neither neighbour", async () => {
    receipt.mockImplementation(async () => {
      throw new Error("timeout");
    });
    await send();
    // `unknown` is NEITHER neighbour: the transaction may well have mined, so calling it reverted
    // would be as wrong as calling it done.
    expect(el("sent-unknown")).not.toBeNull();
    expect(el("sent-succeeded")).toBeNull();
    expect(el("sent-reverted")).toBeNull();
  });
});

// -------------------------------------------------------------------------------------------------
// The capability split, which the selection has to respect
// -------------------------------------------------------------------------------------------------

describe("a groomer gets the directory selection and not the domain one", () => {
  it("renders the register keyed by the provider record and not the one keyed by a contract", async () => {
    // A groomer issues nothing, so it has no `DogTagIssuer` clone - the domain selection is keyed by
    // one and is inapplicable rather than merely hidden. The DIRECTORY selection is keyed by
    // `providerId`, so a groomer needs it exactly as a vet does, and mounting it for vets alone would
    // have left a groomer unable to appear in the directory at all.
    await mount({ issuance: false, listing: true });
    expect(el("directory-register-select")).not.toBeNull();
    expect(el("domain-register-select")).toBeNull();
    expect(button("directory-register-send")).not.toBeNull();
  });
});
