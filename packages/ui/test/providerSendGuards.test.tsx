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
  core: "0x9309aB1c2d3E4F5061728394A5B6C7D8E9F00112",
  factory: "0x1C9Ac2eB3f1A2D4B5C6d7E8f90A1B2C3D4e5F607",
  domainResolver: "0x7A9b0C1d2E3F4a5B6C7d8e9f0A1B2c3D4e5F6A7b",
  directory: "0x8b0c1D2E3f4a5B6c7D8E9F0A1B2C3d4E5f6A7b8C",
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
  // The provider id is remembered in localStorage per wallet; a test must not inherit one.
  window.localStorage.clear();
});

describe("the wallet window is never a blank page", () => {
  // FOUND ON CHAIN. A captain pressed Deploy, confirmed in his wallet, and the portal showed nothing
  // - ever. The deploy mined, the clone exists, and his account nonce was 1, so it was the only
  // transaction he had ever sent. `sendAndFollow` awaited the wallet FIRST and recorded the attempt
  // only after that promise resolved, so the whole wallet window had no on-screen state and a
  // promise that never resolved left the page blank for good.

  it("records the attempt BEFORE the wallet answers, not after", async () => {
    // The assertion the old code could not pass: state on screen while the wallet is still open.
    let release!: (h: string) => void;
    writeContractAsync.mockImplementationOnce(
      () => new Promise<string>((res) => { release = res; }),
    );
    type("candidate", CLONE_A);
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    // The wallet has not answered. The page must already say what is happening.
    expect(host.querySelector("[data-testid='sent-awaitingWallet']")).not.toBeNull();
    expect(host.textContent).toContain("waiting for your wallet to respond");
    // And it must not invent a transaction id it does not have.
    expect(host.textContent).toContain("No transaction id yet");

    release(HASH);
    await settle();
    expect(host.querySelector("[data-testid='sent-awaitingWallet']")).toBeNull();
    expect(host.querySelector("[data-testid='sent-succeeded']")).not.toBeNull();
  });

  it("withdraws the row entirely when the wallet rejects, since nothing was sent", async () => {
    // The opposite half: a refusal must not leave something on screen that might have happened.
    writeContractAsync.mockRejectedValueOnce(
      Object.assign(new Error("User rejected the request."), { code: 4001 }),
    );
    type("candidate", CLONE_A);
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='sent-awaitingWallet']")).toBeNull();
    expect(host.querySelector("[data-testid='sent-walletSilent']")).toBeNull();
    // It is reported as the wallet fault it is, with the denial that keeps it off the provider.
    const fault = host.querySelector("[data-testid='wallet-fault']");
    expect(fault).not.toBeNull();
    expect(fault!.getAttribute("data-fault")).toBe("walletRejected");
    expect(host.querySelector("[data-testid='wallet-fault-established']")!.textContent).toMatch(
      /nothing about your provider record was checked/i,
    );
  });

  it("a wallet fault is labelled and states what was NOT established", async () => {
    // The 4100 case, which is the one that read as a verdict about the provider.
    writeContractAsync.mockRejectedValueOnce(
      Object.assign(
        new Error("The requested method and/or account has not been authorized by the user."),
        { code: 4100 },
      ),
    );
    type("candidate", CLONE_A);
    byText("Check this contract").click();
    await settle();
    button("repoint-send").click();
    await settle();

    const fault = host.querySelector("[data-testid='wallet-fault']")!;
    expect(fault.getAttribute("data-fault")).toBe("walletUnauthorized");
    expect(fault.textContent).toMatch(/your wallet could not complete this/i);
    expect(fault.textContent).toMatch(/says nothing about whether you are authorized/i);
    // The wallet's own words are kept for diagnosis, but never stand alone.
    expect(fault.textContent).toMatch(/has not been authorized by the user/);
  });
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

describe("a retired plan is SHOWN, and unmistakably labelled as superseded", () => {
  // Dropping the card was the earlier defect twice over: the notice referred to "what is shown
  // below" while nothing was, and it destroyed the one thing a provider most wants right after
  // signing - what they had checked. So the answers stay, and the label is what makes that safe.
  const check = async () => {
    type("candidate", CLONE_A);
    await settle();
    byText("Check this contract").click();
    await settle();
  };

  it("keeps the checked answers on screen after an input is edited", async () => {
    await check();
    expect(host.querySelector("[data-testid='clone-lifecycle']")).not.toBeNull();

    type("candidate", CLONE_B);
    await settle();

    expect(host.querySelector("[data-testid='clone-lifecycle']")).not.toBeNull();
    expect(host.querySelector("[data-testid='provider-checks']")).not.toBeNull();
  });

  it("keeps them on screen after a transaction, which is when they are most wanted", async () => {
    await check();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='clone-lifecycle']")).not.toBeNull();
    expect(host.querySelector("[data-testid='provider-checks']")).not.toBeNull();
  });

  it("STRIKES THROUGH the superseded verdict and names why, on the verdict itself", async () => {
    // A banner above the card is not enough on its own: a reader who scans to a green "Ready" and
    // stops is precisely the reader it misses. The qualifier therefore sits on the badge.
    await check();
    const before = host.querySelector("[data-testid='verdict-ready']")!;
    expect(before.className).not.toContain("line-through");
    expect(host.querySelector("[data-testid='verdict-retired-spent']")).toBeNull();

    button("repoint-send").click();
    await settle();

    const after = host.querySelector("[data-testid='verdict-ready']")!;
    expect(after.className).toContain("line-through");
    const marker = host.querySelector("[data-testid='verdict-retired-spent']")!;
    expect(marker).not.toBeNull();
    expect(marker.textContent).toContain("read before your transaction");
  });

  it("marks an edited plan with its OWN reason, not the transaction one", async () => {
    await check();
    type("candidate", CLONE_B);
    await settle();

    expect(host.querySelector("[data-testid='verdict-retired-edited']")).not.toBeNull();
    expect(host.querySelector("[data-testid='verdict-retired-spent']")).toBeNull();
    expect(host.querySelector("[data-testid='verdict-ready']")!.className).toContain("line-through");
  });

  it("labels it in text a reader cannot miss, not by colour alone", async () => {
    // The label must survive a reader who does not hover, does not click, and does not know that
    // amber means anything. So it is asserted as TEXT.
    await check();
    button("repoint-send").click();
    await settle();

    const notice = host.querySelector("[data-testid='repoint-stale-spent']")!;
    expect(notice).not.toBeNull();
    expect(notice.textContent).toContain("Superseded");
    expect(notice.textContent).toContain("read before your transaction");
    expect(notice.textContent).toContain("Check again before sending another");
    // Not the small-print treatment the earlier version used.
    expect(notice.querySelector(".text-xs")).toBeNull();
  });

  it("does not disturb the send record, which stays separately readable", async () => {
    await check();
    button("repoint-send").click();
    await settle();

    expect(host.querySelector("[data-testid='sent-succeeded']")).not.toBeNull();
    expect(host.querySelector(`a[href$='${HASH}']`)).not.toBeNull();
  });

  it("shows NO marker while the plan is still current", async () => {
    // Non-vacuous: without this, a version that marked everything unconditionally would pass every
    // case above.
    await check();
    expect(host.querySelector("[data-testid='verdict-retired-spent']")).toBeNull();
    expect(host.querySelector("[data-testid='verdict-retired-edited']")).toBeNull();
    expect(host.querySelector("[data-testid='repoint-stale']")).toBeNull();
    expect(host.querySelector("[data-testid='repoint-stale-spent']")).toBeNull();
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
    expect(host.textContent).toContain("A transaction has already been sent against this");
    expect(host.textContent).not.toContain("You have changed something since this was checked");
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
    // And it is not routed into the page-level fault notice, which would put it in the same bucket
    // as a wallet refusal and lose which transaction it was about. (Repointed from the removed
    // `page-error` testid - an absence assertion against an element that no longer exists passes
    // whatever the code does, which is the same vacuity this suite exists to avoid.)
    expect(host.querySelector("[data-testid='wallet-fault']")).toBeNull();
  });
});
