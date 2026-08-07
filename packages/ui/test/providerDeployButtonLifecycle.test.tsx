/**
 * @vitest-environment jsdom
 */
// FLOW 1's Deploy control across the WHOLE LIFE of a send, mounted.
//
// THE CAPTAIN'S REPORT: "after i deployed the deploy button is still blue it should have been
// disabled." On a FRESH wallet at contract number 0 - the first-deploy path, not a repeat.
//
// What the reproduction established, before any fix: the button IS `disabled` after a settled deploy,
// and it IS `disabled` while the send is in flight. So the state machine was right and the DEFECT WAS
// THE APPEARANCE. `disabled:opacity-50` on a `primary` button leaves a saturated blue at half opacity,
// which reads as live - and there is no reason for a reader to doubt it, because a disabled control
// that still looks pressable is indistinguishable from a page that has stopped responding. The captain
// described exactly what he saw.
//
// So this suite pins BOTH halves, deliberately, because either alone would let the other regress:
//
//   1. THE STATE at each stage - idle, awaiting the wallet, broadcast-and-unconfirmed, mined, failed.
//      A `disabled` attribute is the only thing between a second send and the first one's `NoChange`
//      or a duplicate deploy at the same contract number.
//   2. THE APPEARANCE - a disabled send must not render in the primary fill. Asserted on the CLASS
//      LIST rather than on a screenshot, so it is repeatable; `opacity-50` alone is not enough and is
//      asserted NOT to be the whole of it.
//
// Real DOM, real macrotask turns, no `act()` - the convention this package uses, and load-bearing here:
// `act()` would reorder the wallet promise's continuation against the passive effects that re-read the
// deployment record, which is the second half of what is under test.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ProviderSelfServiceFlows } from "../src/domain/ProviderSelfServiceFlows";
import { keccak256, toHex } from "viem";
import { Standing, ZERO_ADDR, type ProviderContracts } from "../src/provider";

/** The captain's fresh wallet, which had deployed nothing. */
const CALLER = "0xaD64000000000000000000000000000000006171";
const PROVIDER = "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af";
const PREDICTED = "0x9999999999999999999999999999999999999999";
/**
 * The keccak of the form's DEFAULT record type, not an arbitrary word.
 *
 * `nextContractNumber` counts only the deployments whose record type matches the one in the form, so a
 * fixture with a different key leaves every row filtered out and the suggested number stuck at 0 -
 * which looks exactly like the page failing to advance it. Fixture faults that mimic the defect under
 * test are the expensive kind.
 */
const RECORD_TYPE_KEY = keccak256(toHex("VACCINATION"));
const HASH = `0x${"cd".repeat(32)}`;

const writeContractAsync = vi.fn(async () => HASH);
vi.mock("wagmi", () => ({
  useAccount: () => ({ address: CALLER, isConnected: true, chainId: 135 }),
  useWriteContract: () => ({ writeContractAsync }),
}));

/** What the factory's creation log says this wallet has deployed. Grows after a settled deploy. */
let creations: { clone: string; cloneNonce: bigint; providerId: string; blockNumber: bigint }[] = [];
const issuerCreations = vi.fn(async () => creations);

vi.mock("../src/provider/liveReader", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../src/provider/liveReader")>();
  return {
    ...actual,
    createLiveProviderReader: () => ({
      provider: async () => ({
        controller: CALLER,
        directoryResolver: ZERO_ADDR,
        standing: Standing.ACTIVE,
      }),
      canCreateService: async () => true,
      predictIssuer: async () => PREDICTED,
      // Not yet deployed at the predicted address, which is what makes contract number 0 available.
      isFactoryClone: async (a: string) => creations.some((c) => c.clone === a),
      issuerCreations,
      cloneRecordType: async () => RECORD_TYPE_KEY,
      service: async () => ({
        providerId: PROVIDER,
        factoryGeneration: RECORD_TYPE_KEY,
        recordType: RECORD_TYPE_KEY,
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
      currentService: async () => ZERO_ADDR,
      canWriteServiceRepoint: async () => true,
      approvedResolvers: async () => [],
      canWriteProviderDirectoryResolver: async () => true,
      canWriteServiceDomainResolver: async () => true,
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
  for (let i = 0; i < 16; i++) await turn();
};

const button = (id: string) => host.querySelector<HTMLButtonElement>(`[data-testid='${id}']`)!;
const el = (id: string) => host.querySelector<HTMLElement>(`[data-testid='${id}']`);
/** The Check buttons carry no testid, so they are reached by their own label - as the page shows it. */
const byText = (text: string) =>
  Array.from(host.querySelectorAll("button")).find((b) => b.textContent?.includes(text))!;

function type(id: string, value: string) {
  const input = host.querySelector<HTMLInputElement>(`#${id}`)!;
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Mount, name the provider, and run the deploy check - the state the Deploy button becomes live in. */
async function checked() {
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
  byText("Check what this would deploy").click();
  await settle();
}

beforeEach(() => {
  writeContractAsync.mockClear();
  writeContractAsync.mockImplementation(async () => HASH);
  receipt.mockClear();
  receipt.mockImplementation(async () => ({ status: "success" }));
  issuerCreations.mockClear();
  creations = [];
});

afterEach(() => {
  root?.unmount();
  host.remove();
  root = null;
  // The provider id is remembered in localStorage per wallet; a test must not inherit one.
  window.localStorage.clear();
});

// -------------------------------------------------------------------------------------------------
// 1. The state, at every stage of a send
// -------------------------------------------------------------------------------------------------

describe("the Deploy control is pressable in exactly one state", () => {
  it("is live once, after a check that approved it", async () => {
    await checked();
    expect(button("deploy-send").disabled).toBe(false);
    expect(el("deploy-send-reason")).toBeNull();
  });

  it("is NOT pressable while the wallet has been asked and has not answered", async () => {
    // The window a person spends reading a transaction or fetching a hardware key. A second press
    // here is a second `createIssuer` at the SAME contract number - the first to mine takes the
    // address and the second reverts, having cost gas.
    let release!: (h: string) => void;
    writeContractAsync.mockImplementationOnce(
      () => new Promise<string>((res) => { release = res; }),
    );
    await checked();
    button("deploy-send").click();
    await settle();
    expect(button("deploy-send").disabled).toBe(true);
    expect(el("sent-awaitingWallet")).not.toBeNull();
    release(HASH);
    await settle();
  });

  it("is NOT pressable while broadcast and unconfirmed", async () => {
    let releaseReceipt!: (r: { status: string }) => void;
    receipt.mockImplementationOnce(
      () => new Promise<{ status: string }>((res) => { releaseReceipt = res; }),
    );
    await checked();
    button("deploy-send").click();
    await settle();
    expect(button("deploy-send").disabled).toBe(true);
    expect(el("sent-submitted")).not.toBeNull();
    releaseReceipt({ status: "success" });
    await settle();
  });

  it("is NOT pressable after the deploy has MINED", async () => {
    // The captain's case. A contract number is single-use: `createIssuer` derives the address from
    // (recordType, wallet, cloneNonce), so a second send at the same number can only ever produce the
    // address that now exists.
    await checked();
    button("deploy-send").click();
    await settle();
    expect(el("sent-succeeded")).not.toBeNull();
    expect(button("deploy-send").disabled).toBe(true);
  });

  it("is NOT pressable after a REVERT either", async () => {
    // A revert does not restore the plan: a write can revert precisely BECAUSE the chain moved, so the
    // answers the plan was computed against are no longer necessarily current.
    receipt.mockImplementation(async () => ({ status: "reverted" }));
    await checked();
    button("deploy-send").click();
    await settle();
    expect(el("sent-reverted")).not.toBeNull();
    expect(button("deploy-send").disabled).toBe(true);
  });

  it("is NOT pressable after an outcome nobody could establish", async () => {
    receipt.mockImplementation(async () => {
      throw new Error("timeout");
    });
    await checked();
    button("deploy-send").click();
    await settle();
    expect(el("sent-unknown")).not.toBeNull();
    expect(button("deploy-send").disabled).toBe(true);
  });

  it("BECOMES pressable again after a rejected signature, because nothing was submitted", async () => {
    // The one direction that must NOT be sticky. A wallet refusal means no transaction exists, so the
    // checked plan still describes exactly what a send would do - and a page that locked the button
    // here would strand a provider who fat-fingered Reject.
    writeContractAsync.mockImplementationOnce(async () => {
      throw new Error("User rejected the request");
    });
    await checked();
    button("deploy-send").click();
    await settle();
    expect(el("sent-succeeded")).toBeNull();
    expect(button("deploy-send").disabled).toBe(false);
  });

  it("says WHY it is no longer pressable, rather than only going quiet", async () => {
    await checked();
    button("deploy-send").click();
    await settle();
    expect(el("deploy-send-reason")!.textContent).toMatch(/already been sent/i);
    // `-spent` rather than `deploy-stale`: `PlanNotice` suffixes the testid for a spent plan so an EDIT
    // and a SENT TRANSACTION cannot be read as the same banner. They have different remedies.
    expect(el("deploy-stale-spent")!.textContent).toMatch(/transaction has already been sent/i);
  });
});

// -------------------------------------------------------------------------------------------------
// 2. The appearance - the half the captain actually saw
// -------------------------------------------------------------------------------------------------

describe("a disabled send does not LOOK pressable", () => {
  const classes = () => button("deploy-send").className;
  /**
   * Whether a class list carries the UNPREFIXED fill - the one that actually paints the button.
   *
   * Tokenized rather than matched as a substring, because `hover:bg-primary/90` contains "bg-primary"
   * and is inert on a disabled control (`disabled:pointer-events-none`). A substring check reported the
   * fixed button as still filled, which is a test failing for a reason that has nothing to do with what
   * the captain saw - and the version of this assertion that passes for the wrong reason is worse than
   * the version that fails for the wrong one.
   */
  const hasFill = (className: string) =>
    className.split(/\s+/).some((c) => c === "bg-primary" || c === "bg-danger" || c === "bg-success");

  it("renders in the primary fill while it is live", async () => {
    await checked();
    expect(classes()).toMatch(/bg-primary/);
  });

  it("drops the primary fill once it is disabled", async () => {
    // THE DEFECT. `disabled:opacity-50` leaves a saturated blue at half opacity, which on a bright
    // display reads as a live button - so the state machine was right and the page still told the
    // captain he could press it again. A control's appearance is part of what it claims.
    await checked();
    button("deploy-send").click();
    await settle();
    expect(button("deploy-send").disabled).toBe(true);
    expect(hasFill(classes())).toBe(false);
    // And it positively takes the inert treatment, so "no fill" is not achieved by rendering nothing.
    expect(classes()).toMatch(/bg-surface-muted/);
  });

  it("does not rest on opacity alone", async () => {
    // Asserted so a future "simplification" back to opacity-only reddens. Half-opacity is a hint, not
    // a state: it is invisible against a light background and it is the exact treatment that failed.
    await checked();
    button("deploy-send").click();
    await settle();
    const opacityOnly = classes().includes("opacity-50") && hasFill(classes());
    expect(opacityOnly, "a disabled primary send is opacity-50 blue - the reported defect").toBe(false);
  });

  it("applies to EVERY send on the page, not only Deploy", async () => {
    // Enumerated rather than hand-listed, and asserted on the count first so an empty match cannot
    // satisfy `every`. A per-button fix is how the next flow ships with the same defect.
    await checked();
    const sends = Array.from(host.querySelectorAll<HTMLButtonElement>("[data-testid$='-send']"));
    expect(sends.length).toBeGreaterThan(2);
    for (const b of sends.filter((x) => x.disabled)) {
      expect(hasFill(b.className), `${b.getAttribute("data-testid")} is a disabled filled button`).toBe(
        false,
      );
    }
  });
});

// -------------------------------------------------------------------------------------------------
// 3. The second defect the captain asked about: does the record update?
// -------------------------------------------------------------------------------------------------

describe("the deployed-contract record reflects a settled deploy", () => {
  /**
   * Make the chain hold the contract only ONCE THE RECEIPT RESOLVES, which is when it really does.
   *
   * Load-bearing rather than tidy. Seeding the creation log BEFORE the send lets a read triggered at
   * ANY point find the contract - so a re-read wired to fire at `submitted` instead of at settlement
   * passes, and the very defect PR #155 closed becomes unobservable. A mutation run caught exactly
   * that: the fixture, not the page, was answering the question.
   */
  const minesInto = (rows: typeof creations) => {
    receipt.mockImplementation(async () => {
      creations = rows;
      return { status: "success" };
    });
  };

  it("re-reads the factory's creation log AFTER the receipt, not at submission", async () => {
    // PR #155 made this re-read fire on SETTLEMENT rather than on submission, because a read triggered
    // at `submitted` comes back without the very thing it was triggered to find and, being the last
    // trigger, leaves the page stale exactly after the deploy the operator is watching for.
    //
    // DRIVEN BY HOLDING THE RECEIPT OPEN, which is the only way to observe the difference. If the
    // receipt is allowed to resolve first, its microtask beats the effect and the read finds the new
    // contract whichever trigger fired - so the fixture answers the question and a re-read wired to
    // the wrong moment passes. That is what a mutation run showed.
    let releaseReceipt!: (r: { status: string }) => void;
    receipt.mockImplementationOnce(
      () =>
        new Promise<{ status: string }>((res) => {
          releaseReceipt = res;
        }),
    );
    await checked();
    button("deploy-send").click();
    await settle();

    // Broadcast and unconfirmed. Nothing has mined, so the page must NOT claim the contract exists -
    // and a read triggered here would find exactly that.
    expect(el("sent-submitted")).not.toBeNull();
    expect(host.querySelector<HTMLInputElement>("#cloneNonce")!.value).toBe("0");

    // Now it mines. Only a re-read at SETTLEMENT can see this.
    creations = [{ clone: PREDICTED, cloneNonce: 0n, providerId: PROVIDER, blockNumber: 100n }];
    releaseReceipt({ status: "success" });
    await settle();
    expect(el("sent-succeeded")).not.toBeNull();
    expect(el("deployed-contracts")!.textContent).toMatch(/0x9999/i);
    expect(host.querySelector<HTMLInputElement>("#cloneNonce")!.value).toBe("1");
  });

  it("shows the contract it just deployed, and moves the number on to the next free one", async () => {
    // The state that makes a second press meaningless: number 0 is taken, so the next deploy is 1.
    await checked();
    expect(host.querySelector<HTMLInputElement>("#cloneNonce")!.value).toBe("0");
    minesInto([{ clone: PREDICTED, cloneNonce: 0n, providerId: PROVIDER, blockNumber: 100n }]);
    button("deploy-send").click();
    await settle();
    expect(el("deployed-contracts")!.textContent).toMatch(/0x9999/i);
    expect(host.querySelector<HTMLInputElement>("#cloneNonce")!.value).toBe("1");
  });

  it("and the send stays disabled at the new number until it is checked again", async () => {
    // The number moving is not permission. Its plan was computed for number 0.
    await checked();
    minesInto([{ clone: PREDICTED, cloneNonce: 0n, providerId: PROVIDER, blockNumber: 100n }]);
    button("deploy-send").click();
    await settle();
    expect(button("deploy-send").disabled).toBe(true);
    expect(el("deploy-send-reason")!.textContent).toMatch(/Check what this would deploy/i);
  });
});
