// `AddressRef`'s inert branch, and the one way an operator can take an unusable address away.
//
// Withholding the link is the right call - the address addresses nothing, so there is no explorer page
// to open - but it removes the escape hatch the LINKED branch quietly provides: an anchor carries the
// full value in its `href`, so a truncated link is still copyable. The inert branch has no `href`, and
// `shortAddr` truncates the STRING, so the elided characters are not in the DOM at all. That left the
// full address reachable only by hovering, and a tooltip survives neither a screenshot, nor a touch
// device, nor a paste into an incident report - which is precisely what an operator does with an
// address they have just been told cannot be looked up.
//
// So the value the copy button writes has to be asserted, not just the button's presence. A button
// wired to the truncated form would satisfy "there is a copy affordance" and still lose the fact.
//
// The second half is the `copyable` opt-out. Two call sites already render their own copy control
// (`Governance`'s `AddrLink` with `withCopy`, the predicted-clone block in `Issuers`), and one value
// offering two copy buttons is its own confusion. Both directions are asserted, because a default that
// silently won either way would make the prop meaningless.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { AddressRef } from "../src/components/ChainRef";

const GOOD_ADDRESS = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F";
/** Present, and unusable: 39 hex digits, so no explorer page can exist for it. */
const BAD_ADDRESS = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9";

let container: HTMLDivElement;
let root: Root;
let written: string[];

async function settle(turns = 4) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(node: Parameters<Root["render"]>[0]) {
  root.render(node);
  await settle();
}

/** Every copy affordance rendered anywhere under the component - the duplication guard needs all. */
function copyButtons(): HTMLButtonElement[] {
  return [...container.querySelectorAll<HTMLButtonElement>('[data-testid="copy-button"]')];
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  written = [];
  // jsdom has neither `navigator.clipboard` nor `document.execCommand`, so without this the button
  // lands in its `failed` state and an assertion about what it copied would pass for the wrong reason.
  vi.stubGlobal("navigator", {
    ...navigator,
    clipboard: {
      writeText: async (v: string) => {
        written.push(v);
      },
    },
  });
});

afterEach(() => {
  root.unmount();
  container.remove();
  vi.unstubAllGlobals();
});

describe("AddressRef — an address that cannot be looked up", () => {
  it("hands over the FULL address without a hover", async () => {
    await mount(createElement(AddressRef, { address: BAD_ADDRESS, testId: "addr" }));

    const inert = container.querySelector<HTMLElement>('[data-testid="addr-inert"]');
    expect(inert).not.toBeNull();
    // The premise: what is rendered is NOT the whole value, so a copy affordance is the only route to
    // it. If this ever stops holding, the assertion below stops being about anything.
    expect(inert!.textContent).not.toBe(BAD_ADDRESS);

    const buttons = copyButtons();
    expect(buttons).toHaveLength(1);
    buttons[0].click();
    await settle();

    // The whole address, not the truncated form the operator can already see.
    expect(written).toEqual([BAD_ADDRESS]);
  });

  it("adds no copy button when the caller already renders one", async () => {
    await mount(
      createElement(AddressRef, { address: BAD_ADDRESS, copyable: false, testId: "addr" }),
    );

    // `Governance`'s `AddrLink` with `withCopy` and the predicted-clone block in `Issuers` supply their
    // own; a second one beside it is not a stronger affordance, it is an ambiguous one.
    expect(copyButtons()).toHaveLength(0);
    // Opting out of the copy button must not opt out of the state itself.
    expect(container.querySelector('[data-testid="addr-inert"]')).not.toBeNull();
  });
});

describe("AddressRef — the other two states", () => {
  it("keeps a linkable address a link, whose href is the full value", async () => {
    await mount(createElement(AddressRef, { address: GOOD_ADDRESS, testId: "addr" }));

    const link = container.querySelector<HTMLAnchorElement>('[data-testid="addr"]');
    expect(link).not.toBeNull();
    expect(link!.tagName).toBe("A");
    // This is why the copy affordance is the inert branch's alone rather than both: the full value is
    // already in the DOM here, and the anchor is a way to take it away.
    expect(link!.getAttribute("href")).toContain(GOOD_ADDRESS);
    expect(container.querySelector('[data-testid="addr-inert"]')).toBeNull();
  });

  it("offers nothing to copy when there is no address at all", async () => {
    await mount(createElement(AddressRef, { address: null, testId: "addr" }));

    // Absence claims nothing and has nothing to hand over - a copy button here would be a control that
    // copies an em dash.
    expect(copyButtons()).toHaveLength(0);
    expect(container.querySelector('[data-testid="addr-none"]')).not.toBeNull();
  });
});
