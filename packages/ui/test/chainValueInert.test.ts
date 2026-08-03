// @vitest-environment jsdom
//
// `ChainValue`'s unlinked branch renders TWO different situations, and this file exists to keep them
// visually apart.
//
//   no reason - an ordinary identifier that simply has no explorer page (a credential root, a
//               record-type key, a purpose key, a `dogTagId`, a nullifier). Nothing is wrong with it.
//   a reason  - something we tried to resolve and could not. That is the same claim `TxRef` makes
//               about an unusable transaction hash, so it gets the same struck-through amber.
//
// Both halves are load-bearing and they fail in OPPOSITE directions. Left plain, the second state was
// discoverable only by hovering, which this repo does not count as reported - so a malformed contract
// address on the Activity feed looked exactly like the perfectly good root beside it. Struck through
// unconditionally, every ordinary identifier in the details column would be marked as broken, which is
// the worse regression of the two: the government Oversight, vet Traceability and groomer
// tag-discovery panels render whole columns of them and pass no reason at all.
//
// So the treatment keys on `reason`, never on the absence of `href`, and both directions are asserted.
// A test that only checked the struck-through case would pass with the unconditional version.
//
// Rendered in a real DOM rather than reasoned about: a class list is a DOM fact, and the defect was
// invisible to a typecheck (`reason` was well-typed all along - it just never reached the class name).
// `createElement` rather than JSX, matching `rpcSettingsVerdict.test.ts`, since this package has no
// vitest React plugin.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ChainValue } from "../src/chain/ChainValue";

const ROOT = `0x${"ab".repeat(32)}`;
const CONTRACT = "0x6F8a0B1c2D3e4F5A6B7c8D9E0F1A2b3C4d5E6f70";
/** What `addressChainRef` attaches to a present-but-unusable address. */
const REASON = "not a well-formed 20-byte address, so it has no explorer page";

let container: HTMLDivElement;
let root: Root;

async function settle(turns = 3) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(node: Parameters<Root["render"]>[0]) {
  root.render(node);
  await settle();
}

function inertOf(testId: string): HTMLElement {
  const el = container.querySelector<HTMLElement>(`[data-testid="${testId}-inert"]`);
  expect(el).not.toBeNull();
  return el!;
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  root.unmount();
  container.remove();
});

describe("ChainValue — the unlinked branch", () => {
  it("leaves an ordinary identifier with no explorer page looking perfectly normal", async () => {
    await mount(
      createElement(ChainValue, { label: "credential root", value: ROOT, testId: "cv-root" }),
    );

    const inert = inertOf("cv-root");
    // The constraint that bounds the fix. A root has no explorer page and never will; marking it as
    // unresolvable would tell the operator a good value is broken.
    expect(inert.className).not.toMatch(/line-through/);
    expect(inert.className).toContain("text-onSurface");
    // Nothing is being withheld, so there is nothing to explain: the hover text is the value alone.
    expect(inert.getAttribute("title")).toBe(ROOT);
    expect(inert.getAttribute("data-reason")).toBeNull();
  });

  it("marks a value it could not resolve, without requiring a hover", async () => {
    await mount(
      createElement(ChainValue, {
        label: "issuer registry",
        value: CONTRACT,
        reason: REASON,
        testId: "cv-contract",
      }),
    );

    const inert = inertOf("cv-contract");
    // The same visual language `TxRef` uses for an unusable hash - one state, one rendering.
    expect(inert.className).toMatch(/line-through/);
    expect(inert.className).toContain("decoration-warning/60");
    expect(inert.className).toContain("text-muted");
    expect(inert.className).not.toContain("text-onSurface");
    expect(inert.getAttribute("title")).toContain(REASON);
    expect(inert.getAttribute("data-reason")).toBe(REASON);
    // Still no link, and the value is still shown: it is what the backend reported.
    expect(inert.tagName).not.toBe("A");
    expect(inert.textContent).toBeTruthy();
  });

  it("keeps the two apart in one row, which is the only place it matters", async () => {
    await mount(
      createElement(
        "div",
        null,
        createElement(ChainValue, { label: "credential root", value: ROOT, testId: "cv-root" }),
        createElement(ChainValue, {
          label: "issuer registry",
          value: CONTRACT,
          reason: REASON,
          testId: "cv-contract",
        }),
      ),
    );

    // A per-VALUE rule, not a per-page one. Rendered side by side is exactly the Activity details
    // column, where the unresolvable contract sits among ordinary identifiers.
    expect(inertOf("cv-root").className).not.toMatch(/line-through/);
    expect(inertOf("cv-contract").className).toMatch(/line-through/);
  });

  it("ignores a reason on a value it CAN link — a linked value withholds nothing", async () => {
    await mount(
      createElement(ChainValue, {
        label: "issuer registry",
        value: CONTRACT,
        href: `https://explorer.roax.net/address/${CONTRACT}`,
        reason: REASON,
        testId: "cv-linked",
      }),
    );

    const link = container.querySelector<HTMLAnchorElement>('[data-testid="cv-linked-link"]');
    expect(link).not.toBeNull();
    expect(link!.tagName).toBe("A");
    expect(link!.className).not.toMatch(/line-through/);
    // Not merely unstruck: the inert form must be absent, or the row would make both claims at once.
    expect(container.querySelector('[data-testid="cv-linked-inert"]')).toBeNull();
  });
});
