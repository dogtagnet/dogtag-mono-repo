/**
 * @vitest-environment jsdom
 */
// The two guards that sit between a preflight and a transaction (registry-plan S-15).
//
// Both close the same defect - a verdict stated before the fact is established - and both live in
// the COMPONENT rather than in the pure engine, which is why they need a mounted test:
//
//   1. A plan stops gating anything once the inputs it was computed from change. Every send handler
//      used to read current form state while its button was gated on a plan computed earlier, so
//      editing an input after pressing Check sent an unchecked value. On flow 2 that was not merely
//      confusing: pasting a DIFFERENT clone the caller also owns would SUCCEED and silently move
//      the wrong record type's pointer.
//   2. A submitted transaction is reported as submitted. `writeContractAsync` resolves on a hash and
//      does not throw on a revert, so a reverted write used to leave a message asserting the action
//      had succeeded.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import { Standing, ZERO_ADDR, type ProviderContracts } from "../src/provider";

const CALLER = "0x2222222222222222222222222222222222222222";
const PROVIDER = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const CLONE_A = "0x1111111111111111111111111111111111111111";
const CLONE_B = "0x7777777777777777777777777777777777777777";
const RECORD_TYPE = `0x${"ab".repeat(32)}`;
const HASH = `0x${"cd".repeat(32)}`;

const writeContractAsync = vi.fn(async () => HASH);
vi.mock("wagmi", () => ({
  useAccount: () => ({ address: CALLER, isConnected: true }),
  useWriteContract: () => ({ writeContractAsync }),
}));

// Both attached clones are genuine, owned by this key and repointable, so NOTHING but the guard
// under test distinguishes them - which is exactly the state that made the stale-input send
// dangerous rather than merely wrong.
vi.mock("../src/provider/liveReader", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/provider/liveReader")>();
  return {
    ...actual,
    createLiveProviderReader: () => ({
      isFactoryClone: async () => true,
      cloneOwner: async () => CALLER,
      service: async () => ({
        providerId: PROVIDER,
        factoryGeneration: RECORD_TYPE,
        recordType: RECORD_TYPE,
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
      canWriteServiceRepoint: async () => true,
      currentService: async () => ZERO_ADDR,
    }),
  };
});

const receipt = vi.fn(async () => ({ status: "success" }));
vi.mock("../src/wallet/contracts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/wallet/contracts")>();
  return { ...actual, roaxPublicClient: () => ({ waitForTransactionReceipt: receipt }) };
});

const CONTRACTS: ProviderContracts = {
  core: "0xA4916d75722cf7d39a8E030cFbAee30a411aAEa9",
  factory: "0x4CBfF4Cf47c313C9Df9689dd2A47eC71675233c6",
  domainResolver: "0x4AB4a70CFa9CE9415B96dF543C218F90a2619c33",
  directory: "0x25a318a0Bf83a7ea64fB0a7b1cDe8847722C7bC0",
};

let root: Root | null = null;
let host: HTMLElement;

const turn = () => new Promise((r) => setTimeout(r, 0));
const settle = async () => {
  for (let i = 0; i < 12; i++) await turn();
};

function type(id: string, value: string) {
  const input = host.querySelector<HTMLInputElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const button = (testId: string) => host.querySelector<HTMLButtonElement>(`[data-testid='${testId}']`)!;
const byText = (text: string) =>
  Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes(text))!;

beforeEach(async () => {
  writeContractAsync.mockClear();
  receipt.mockClear();
  receipt.mockImplementation(async () => ({ status: "success" }));
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(
    createElement(ProviderSelfServiceFlows, {
      contracts: CONTRACTS,
      missingConfig: [],
      capabilities: { issuance: true, listing: true },
    }),
  );
  await settle();
  type("providerId", PROVIDER);
  await settle();
});

afterEach(() => {
  root?.unmount();
  host.remove();
  root = null;
});

describe("a plan stops gating anything once its inputs change", () => {
  it("disables the send and says why when the checked address is edited", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();

    // The control: a checked, attached, repointable clone really is offered.
    expect(button("repoint-send").disabled).toBe(false);

    // Now paste a DIFFERENT address. It is just as genuine, just as owned and just as attached, so
    // nothing but this guard stands between the provider and moving the wrong record type's pointer.
    type("candidate", CLONE_B);
    await settle();

    expect(button("repoint-send").disabled).toBe(true);
    expect(host.querySelector("[data-testid='repoint-stale']")).not.toBeNull();
    expect(host.textContent).toContain("Check again before sending");
  });

  it("sends the address that was CHECKED, not whatever the field holds", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(writeContractAsync).toHaveBeenCalledTimes(1);
    expect(writeContractAsync.mock.calls[0]![0]).toMatchObject({
      functionName: "repointService",
      args: [CLONE_A],
    });
  });

  it("re-checking the new address clears the staleness rather than stranding the flow", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    type("candidate", CLONE_B);
    await settle();
    byText("Check this contract").click();
    await settle();

    expect(host.querySelector("[data-testid='repoint-stale']")).toBeNull();
    expect(button("repoint-send").disabled).toBe(false);
  });
});

describe("a plan does not outlive its own transaction", () => {
  // The other set of inputs. A plan is keyed on the FORM, so an untouched form leaves the key
  // matching - but the answers came from the CHAIN, and submitting moves that. Without this, Check
  // -> Send -> Send fires a second transaction against state nobody re-read.
  it("disables the send after one transaction, with the form untouched", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    expect(button("repoint-send").disabled).toBe(false);

    button("repoint-send").click();
    await settle();

    expect(writeContractAsync).toHaveBeenCalledTimes(1);
    expect(button("repoint-send").disabled).toBe(true);
  });

  it("says the plan was ACTED ON, not that an input was edited - different remedies", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='repoint-stale-spent']")).not.toBeNull();
    expect(host.querySelector("[data-testid='repoint-stale']")).toBeNull();
    expect(host.textContent).toContain("already been acted on");
  });

  it("a second press really does send nothing", async () => {
    // Non-vacuous: the button is clicked again rather than merely inspected, because a disabled
    // attribute that some future handler ignored would still let the transaction through.
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(writeContractAsync).toHaveBeenCalledTimes(1);
  });

  it("retires the plan after a REVERT too - the chain may have moved precisely because it reverted", async () => {
    receipt.mockImplementation(async () => ({ status: "reverted" }));
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(button("repoint-send").disabled).toBe(true);
    // The outcome is still reported: retiring the plan must not hide the result the provider needs.
    expect(host.querySelector("[data-testid='sent-reverted']")).not.toBeNull();
  });

  it("retires the plan after an UNFETCHABLE receipt - an unestablished outcome authorizes nothing", async () => {
    receipt.mockImplementation(async () => {
      throw new Error("Timed out while waiting for transaction");
    });
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(button("repoint-send").disabled).toBe(true);
    expect(host.querySelector("[data-testid='sent-unknown']")).not.toBeNull();
  });

  it("re-checking after a send restores the button", async () => {
    // The flow must not be stranded: retiring is a demand to re-read, not a dead end.
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();
    byText("Check this contract").click();
    await settle();

    expect(button("repoint-send").disabled).toBe(false);
    expect(host.querySelector("[data-testid='repoint-stale-spent']")).toBeNull();
  });
});

describe("a submitted transaction is reported as submitted, and settled by its receipt", () => {
  it("reports a successful transaction as succeeded, with a link", async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='sent-succeeded']")).not.toBeNull();
    expect(host.querySelector(`a[href$='${HASH}']`)).not.toBeNull();
  });

  it("REPORTS A REVERT rather than the action it was attempting", async () => {
    // `writeContractAsync` does not throw on status 0, so without the receipt read this would have
    // rendered exactly like the success above.
    receipt.mockImplementation(async () => ({ status: "reverted" }));
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='sent-reverted']")).not.toBeNull();
    expect(host.querySelector("[data-testid='sent-succeeded']")).toBeNull();
    expect(host.textContent).toContain("reverted on chain");
  });

  it("an unfetchable receipt is NEITHER neighbour, and prints its reason", async () => {
    receipt.mockImplementation(async () => {
      throw new Error("Timed out while waiting for transaction");
    });
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='sent-unknown']")).not.toBeNull();
    expect(host.querySelector("[data-testid='sent-succeeded']")).toBeNull();
    expect(host.querySelector("[data-testid='sent-reverted']")).toBeNull();
    // Printed, not hovered - the sentence that distinguishes "we could not follow it" from "it failed".
    expect(host.textContent).toContain("Why the outcome is not known");
    expect(host.textContent).toContain("Timed out while waiting");
    // And it is not routed into the page-level error, which would put it in the same bucket as a
    // rejected signature and lose which transaction it was about.
    expect(host.querySelector("[data-testid='page-error']")).toBeNull();
  });
});
