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
// The second half is that the affordance has to WORK where these portals actually run.
// `navigator.clipboard` is undefined in any non-secure context, and the demo/LAN topology is plain
// `http://`, so a control built on it alone is a silent no-op there - clicked, nothing copied, nothing
// said. That is how an opt-out (`copyable={false}`, since removed) turned this guarantee into a
// per-caller convention and then lost it: both callers that took the opt-out supplied exactly such a
// control. The shared `CopyButton` is the one with the hidden-textarea `execCommand` fallback and a
// visible FAILED state, so the non-secure case is asserted here rather than assumed - swap the inert
// branch onto a clipboard-only control and that case goes red while every other assertion in this file
// stays green.
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
  delete (document as unknown as { execCommand?: unknown }).execCommand;
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

  it("still hands it over on a non-secure origin, where navigator.clipboard does not exist", async () => {
    // The plain `http://<lan-ip>` demo topology, reproduced: the global is simply absent. A control
    // built on `navigator.clipboard?.writeText` copies nothing here and says nothing about it, which
    // in the one state whose point is recoverability leaves no route to the value at all.
    vi.stubGlobal("navigator", { ...navigator, clipboard: undefined });
    const execCopied: string[] = [];
    const execCommand = vi.fn(() => {
      const ta = document.querySelector("textarea");
      if (ta) execCopied.push((ta as HTMLTextAreaElement).value);
      return true;
    });
    Object.defineProperty(document, "execCommand", { value: execCommand, configurable: true });

    await mount(createElement(AddressRef, { address: BAD_ADDRESS, testId: "addr" }));

    const buttons = copyButtons();
    expect(buttons).toHaveLength(1);
    buttons[0].click();
    await settle();

    // It fell back to the hidden-textarea path and carried the whole address through it...
    expect(execCommand).toHaveBeenCalled();
    expect(execCopied).toEqual([BAD_ADDRESS]);
    // ...and reported success rather than sitting there looking functional.
    expect(buttons[0].getAttribute("data-copy-state")).toBe("copied");
  });

  it("cannot be told not to offer one — the guarantee is the component's, not the caller's", async () => {
    // There is no prop to decline it. An opt-out existed for one round and both callers that took it
    // supplied a clipboard-only control, so the state that most needs a working route to its value had
    // none. Where a caller renders its own control the inert state now shows two, deliberately.
    await mount(createElement(AddressRef, { address: BAD_ADDRESS, testId: "addr" }));

    expect(copyButtons()).toHaveLength(1);
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
