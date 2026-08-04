/**
 * @vitest-environment jsdom
 */
// "Who may sign in your name", MOUNTED.
//
// The claims this page makes are claims about what a provider SEES, and a pure assertion over a
// returned object cannot see a paint. Three of them cannot be checked any other way:
//
//   * an unreadable list must not render as an empty one - the defect would be an absent element,
//     which every pure test passes over;
//   * the withdrawn/current distinction must survive being read as PLAIN TEXT, which is precisely
//     what a flattened dump of the DOM is;
//   * a submitted-but-unsettled transaction must never flip a row to "Can issue" - a claim about
//     WHEN state is re-read, which only a render can observe.
//
// Real DOM, real macrotask turns, no `act()` - the convention `rpcSettingsVerdict.test.ts`
// established in this package, and for the reason recorded there: `act()` drains React's work into
// its own queue and reorders promise continuations against passive effects, which can hide exactly
// the ordering defects these cases exist to catch.

import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IssuerSignersPanel } from "../src/domain/IssuerSignersPanel";
import type { IssuanceAllowedResponse } from "../src/signers";

const OWNER = "0x00000000000000000000000000000000000000a1";
const OURS = "0x00000000000000000000000000000000000000b7";
const WITHDRAWN_ADDR = "0x00000000000000000000000000000000000000c3";
const CLONE = "0x00000000000000000000000000000000000000d4";

// The wallet is the OWNER by default, so a case that wants a blocked control has to say so - the
// inverse would make every "the button is disabled" assertion pass for the wrong reason.
let account: string | undefined = OWNER;
let writeResult: () => Promise<`0x${string}`> = async () => `0x${"1".repeat(64)}`;

vi.mock("wagmi", () => ({
  useAccount: () => ({ address: account, isConnected: !!account, chainId: 135 }),
  useWriteContract: () => ({ writeContractAsync: () => writeResult() }),
}));

// The receipt wait is stubbed rather than the whole client, so the component's own decision - only
// an established success re-reads - is what is under test.
let receipt: () => Promise<{ status: string }> = async () => ({ status: "success" });
vi.mock("../src/wallet/contracts", () => ({
  roaxPublicClient: () => ({ waitForTransactionReceipt: () => receipt() }),
}));

let root: Root | null = null;
let host: HTMLElement | null = null;

async function mount(load: () => Promise<IssuanceAllowedResponse>) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  root.render(
    createElement(IssuerSignersPanel, { load, rpcUrl: "http://localhost:0" }),
  );
  await settle();
  return host;
}

/** Several real macrotask turns, so effects and promise continuations interleave as they really do. */
async function settle(turns = 6) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

beforeEach(() => {
  account = OWNER;
  receipt = async () => ({ status: "success" });
  writeResult = async () => `0x${"1".repeat(64)}`;
});

afterEach(() => {
  root?.unmount();
  host?.remove();
  root = null;
  host = null;
});

const text = (el: HTMLElement) => el.textContent ?? "";
const testid = (el: HTMLElement, id: string) => el.querySelector(`[data-testid="${id}"]`);
const all = (el: HTMLElement, id: string) =>
  Array.from(el.querySelectorAll(`[data-testid="${id}"]`));

function response(over: Partial<IssuanceAllowedResponse> = {}): IssuanceAllowedResponse {
  return {
    activeSigner: OURS,
    contracts: [
      {
        recordType: "VACCINATION",
        issuerAddr: CLONE,
        read: {
          state: "resolved",
          owner: OWNER,
          entries: [
            { address: OWNER, allowed: true, everNamed: true },
            { address: WITHDRAWN_ADDR, allowed: false, everNamed: true },
            { address: OURS, allowed: false, everNamed: false },
          ],
          activeSignerAllowed: false,
        },
      },
    ],
    ...over,
  };
}

// -------------------------------------------------------------------------------------------------
// An unreadable list is not an empty one
// -------------------------------------------------------------------------------------------------

describe("a list that could not be read", () => {
  it("renders as unreadable, and never as an empty list", async () => {
    const el = await mount(async () =>
      response({
        contracts: [
          {
            recordType: "VACCINATION",
            issuerAddr: CLONE,
            read: { state: "unavailable", reason: "eth_getLogs refused" },
          },
        ],
      }),
    );
    expect(testid(el, "roster-unavailable")).not.toBeNull();
    // THE assertion. The empty-list sentence must be absent, and so must the roster itself - a
    // provider deciding who may sign medical records must never read "nobody is admitted" from a
    // question that was never answered.
    expect(testid(el, "roster-empty")).toBeNull();
    expect(testid(el, "roster")).toBeNull();
    expect(text(el)).toContain("eth_getLogs refused");
    expect(text(el)).toMatch(/not the same as nobody being admitted/i);
  });

  it("offers no admit control, because neither the owner nor the list is known", async () => {
    const el = await mount(async () =>
      response({
        contracts: [
          {
            recordType: "VACCINATION",
            issuerAddr: CLONE,
            read: { state: "unavailable", reason: "boom" },
          },
        ],
      }),
    );
    expect(testid(el, "admit-input")).toBeNull();
    expect(testid(el, "remove-signer")).toBeNull();
  });

  it("an EMPTY list is a different rendering - it was read, and it admits nobody", async () => {
    const el = await mount(async () =>
      response({
        activeSigner: null,
        contracts: [
          {
            recordType: "VACCINATION",
            issuerAddr: CLONE,
            read: { state: "resolved", owner: OWNER, entries: [], activeSignerAllowed: null },
          },
        ],
      }),
    );
    expect(testid(el, "roster-empty")).not.toBeNull();
    expect(testid(el, "roster-unavailable")).toBeNull();
  });

  it("a backend that could not be reached says so, and claims nothing about who may issue", async () => {
    const el = await mount(async () => {
      throw new Error("connection refused");
    });
    expect(testid(el, "signers-load-failed")).not.toBeNull();
    expect(text(el)).toMatch(/not a statement that nobody may issue/i);
    expect(testid(el, "roster")).toBeNull();
  });
});

// -------------------------------------------------------------------------------------------------
// Withdrawn survives being read as plain text
// -------------------------------------------------------------------------------------------------

describe("the withdrawn / current distinction", () => {
  it("is carried by a WORD in the DOM, not by styling", async () => {
    const el = await mount(async () => response());
    const rows = all(el, "roster-row") as HTMLElement[];
    const row = rows.find((r) => text(r).includes(WITHDRAWN_ADDR))!;
    const standing = row.querySelector('[data-testid="roster-standing"]')!;

    // The guide-walk crew read a flattened dump and concluded a withdrawn holder was current. A
    // flattened dump of THIS row says "Withdrawn".
    expect(standing.textContent).toMatch(/withdrawn/i);

    // And strip every attribute - which is what a screen reader, a screenshot's OCR and a text
    // extraction all effectively do - and the distinction is still there.
    const stripped = rows.map((r) => (r.textContent ?? "").replace(/\s+/g, " "));
    const withdrawnRow = stripped.find((t) => t.includes(WITHDRAWN_ADDR))!;
    const currentRow = stripped.find((t) => t.includes(OWNER))!;
    expect(withdrawnRow).toMatch(/withdrawn/i);
    expect(currentRow).not.toMatch(/withdrawn/i);
    expect(currentRow).toMatch(/can issue/i);
  });

  it("says never-admitted for an address the list has never held", async () => {
    const el = await mount(async () => response());
    const row = (all(el, "roster-row") as HTMLElement[]).find((r) => text(r).includes(OURS))!;
    expect(text(row)).toMatch(/not admitted/i);
    expect(text(row)).not.toMatch(/withdrawn/i);
  });
});

// -------------------------------------------------------------------------------------------------
// A pending transaction is never a completed grant
// -------------------------------------------------------------------------------------------------

describe("a transaction that has not settled", () => {
  it("does not flip a row to admitted, and the roster is not re-read", async () => {
    let loads = 0;
    // The receipt never resolves within the test, so the send stays `submitted`.
    receipt = () => new Promise(() => {});
    const el = await mount(async () => {
      loads += 1;
      return response();
    });
    expect(loads).toBe(1);

    const input = testid(el, "admit-input") as HTMLInputElement;
    setValue(input, OURS);
    await settle();
    (testid(el, "admit-submit") as HTMLButtonElement).click();
    await settle();

    // A hash exists, so the page reports it - but as submitted, never as done.
    expect(text(el)).toMatch(/submitted, outcome not yet known/i);
    expect(text(el)).not.toMatch(/succeeded/i);
    // The row still says what the CHAIN last said, because nothing has been re-read.
    const row = (all(el, "roster-row") as HTMLElement[]).find((r) => text(r).includes(OURS))!;
    expect(text(row)).toMatch(/not admitted/i);
    expect(loads).toBe(1);
  });

  it("re-reads only once a receipt reports success", async () => {
    let loads = 0;
    const el = await mount(async () => {
      loads += 1;
      return response();
    });
    const input = testid(el, "admit-input") as HTMLInputElement;
    setValue(input, OURS);
    await settle();
    (testid(el, "admit-submit") as HTMLButtonElement).click();
    await settle(12);
    expect(loads).toBe(2);
    expect(text(el)).toMatch(/succeeded/i);
  });

  it("a REVERTED transaction does not re-read either, and says it reverted", async () => {
    let loads = 0;
    receipt = async () => ({ status: "reverted" });
    const el = await mount(async () => {
      loads += 1;
      return response();
    });
    const input = testid(el, "admit-input") as HTMLInputElement;
    setValue(input, OURS);
    await settle();
    (testid(el, "admit-submit") as HTMLButtonElement).click();
    await settle(12);
    expect(text(el)).toMatch(/reverted on chain/i);
    expect(loads).toBe(1);
  });
});

// -------------------------------------------------------------------------------------------------
// The controls say why they are unavailable
// -------------------------------------------------------------------------------------------------

describe("a disabled control renders a reason", () => {
  it("names the owner when the connected wallet is not it", async () => {
    account = "0x00000000000000000000000000000000000000ee";
    const el = await mount(async () => response());
    expect((testid(el, "admit-submit") as HTMLButtonElement).disabled).toBe(true);
    expect(text(testid(el, "admit-blocked") as HTMLElement)).toContain(OWNER);
  });

  it("tells an unconnected visitor to connect, rather than going dead in silence", async () => {
    account = undefined;
    const el = await mount(async () => response());
    expect((testid(el, "admit-submit") as HTMLButtonElement).disabled).toBe(true);
    expect(text(testid(el, "admit-blocked") as HTMLElement)).toMatch(/connect your wallet/i);
  });

  it("marks this shop's own signing key so the provider knows which row to act on", async () => {
    const el = await mount(async () => response());
    const row = (all(el, "roster-row") as HTMLElement[]).find((r) => text(r).includes(OURS))!;
    expect(text(row)).toMatch(/this shop.s signing key/i);
    expect(text(el)).toMatch(/does not admit it/i);
  });

  it("says custody is locked rather than reporting the shop's signer as refused", async () => {
    const el = await mount(async () =>
      response({
        activeSigner: null,
        contracts: [
          {
            recordType: "VACCINATION",
            issuerAddr: CLONE,
            read: {
              state: "resolved",
              owner: OWNER,
              entries: [{ address: OWNER, allowed: true, everNamed: true }],
              activeSignerAllowed: null,
            },
          },
        ],
      }),
    );
    expect(testid(el, "no-active-signer")).not.toBeNull();
    expect(text(testid(el, "backend-signer-verdict") as HTMLElement)).toMatch(/locked/i);
  });
});

/** Set a controlled input's value the way React's synthetic events observe it. */
function setValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}
