// @vitest-environment jsdom
//
// The issuer-applications queue's approval receipt, rendered for real.
//
// Approving an application whitelists EACH (address, recordType) pair on chain, so this list is the
// operator's only receipt that the grant actually reached the registry. It had the defect in BOTH
// available directions:
//
//   - it linked every returned hash to the explorer unconditionally, so a value that addresses no
//     transaction on any chain was still dressed as something to click and confirm; and
//   - it rendered NOTHING when the backend came back with an empty list (`txs[id]?.length ? … : null`),
//     so "approved, but not one transaction was sent" looked exactly like a clean approval. That is the
//     same defect arrived at by omission - and it is the more dangerous half, because there is no dead
//     link to notice, just an approval that appears to have worked.
//
// `Wizard.tsx` already told operators about the empty case; this page did not, which is why the fix is
// not "hide the link when the value is empty" but "say which of the three situations this is".
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ApprovalTxList } from "../src/pages/IssuerApplications";

const GOOD_TX = `0x${"c3".repeat(32)}`;
const OTHER_GOOD_TX = `0x${"d4".repeat(32)}`;
/** Present and unusable - the shape a scripted/demo feed produces. */
const BAD_TX = "0x0800";

let container: HTMLDivElement;
let root: Root;

async function settle(turns = 4) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(txs?: string[]) {
  root.render(createElement(ApprovalTxList, { txs }));
  await settle();
}

const anchorHrefs = () =>
  [...container.querySelectorAll("a")].map((a) => a.getAttribute("href") ?? "");

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  root.unmount();
  container.remove();
});

describe("IssuerApplications — the approval transaction receipt", () => {
  it("ON CHAIN: a returned transaction is linked", async () => {
    await mount([GOOD_TX]);

    const link = container.querySelector<HTMLAnchorElement>('[data-testid="approval-tx"]');
    expect(link).not.toBeNull();
    expect(link!.tagName).toBe("A");
    expect(link!.getAttribute("href")).toContain(GOOD_TX);
    expect(container.querySelector('[data-testid="approval-no-txs"]')).toBeNull();
  });

  it("NOT ON CHAIN: an approval that sent nothing says so instead of rendering blank", async () => {
    await mount([]);

    // The whole point: an empty list is a REPORT, not an absence of one. Rendering nothing here let an
    // approval that reached no contract read as a success.
    const stated = container.querySelector('[data-testid="approval-no-txs"]');
    expect(stated).not.toBeNull();
    expect(stated!.textContent).toMatch(/no .*whitelistFor.* transactions were returned/i);
    expect(stated!.textContent).toMatch(/nothing reached the chain/i);
    // And it must not be dressed as a link to anything.
    expect(container.querySelectorAll("a")).toHaveLength(0);
  });

  it("CANNOT TELL: a returned hash that cannot be looked up is not offered as a link", async () => {
    await mount([BAD_TX]);

    // No anchor may carry it. A dead link reads as evidence the transaction exists.
    expect(anchorHrefs().some((h) => h.includes(BAD_TX))).toBe(false);

    const inert = container.querySelector('[data-testid="approval-tx-inert"]');
    expect(inert).not.toBeNull();
    expect(inert!.tagName).not.toBe("A");
    // The reason travels with the value, so "not linked" is never mistaken for "we forgot to link it".
    expect(inert!.getAttribute("title")).toMatch(/not a well-formed 32-byte transaction hash/i);
    // The hash itself stays on screen - it is what the backend reported and the operator needs it to
    // go and investigate.
    expect(inert!.textContent).toBeTruthy();
  });

  it("NOT RUN: no approval in this session renders nothing at all", async () => {
    await mount(undefined);

    // The one state that may be silent, and the reason the empty-array case above cannot be folded
    // into it: nothing was claimed here, so there is nothing to explain. Conflating the two is what
    // made "sent no transactions" invisible.
    expect(container.textContent).toBe("");
  });

  it("judges each returned hash on its own, not the batch", async () => {
    await mount([GOOD_TX, BAD_TX, OTHER_GOOD_TX]);

    // A per-value rule. One unusable hash among good ones must be visible as such, and - just as
    // importantly - must not suppress the links for the transactions that did land.
    expect(container.querySelectorAll('[data-testid="approval-tx"]')).toHaveLength(2);
    expect(container.querySelectorAll('[data-testid="approval-tx-inert"]')).toHaveLength(1);
    expect(anchorHrefs().some((h) => h.includes(BAD_TX))).toBe(false);
    expect(anchorHrefs().some((h) => h.includes(GOOD_TX))).toBe(true);
  });
});
