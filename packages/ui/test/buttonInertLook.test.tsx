/**
 * @vitest-environment jsdom
 */
// The shared Button's UNAVAILABLE look, and the one state it must NOT apply to.
//
// This component is used by every portal, so both halves below are fleet-wide claims.
//
// HALF ONE - a disabled FILLED button must not keep its fill. A captain deployed a contract, the page
// correctly disabled Deploy, and he reported "the deploy button is still blue it should have been
// disabled": `disabled:opacity-50` leaves `bg-primary` a saturated blue at half opacity, which reads as
// live. Pressing it again is a second `createIssuer` at the same contract number.
//
// HALF TWO - and it must NOT apply while LOADING. A loading button carries the `disabled` attribute and
// is not unavailable: it is busy doing the thing you asked for, which is what its spinner says. Draining
// its fill says the opposite, and there are 73 `loading={…}` call sites across the portals - so folding
// the two states together is a regression on every submit button that has ever spun. The first cut of
// this fix did exactly that (`inert = disabled || loading`), which is why the case exists.
//
// Asserted on the CLASS LIST rather than a screenshot, so it is repeatable - and TOKENIZED, because
// `hover:bg-primary/90` contains the substring "bg-primary" and is inert under
// `disabled:pointer-events-none`. A substring check reported the fixed button as still filled.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { Button } from "../src/components/Button";

let root: Root | null = null;
let host: HTMLElement;

const turn = () => new Promise((r) => setTimeout(r, 0));

async function render(props: Record<string, unknown>) {
  root!.render(createElement(Button, props, "Do the thing"));
  await turn();
  await turn();
  return host.querySelector("button")!;
}

/** The UNPREFIXED fill classes - the ones that actually paint the button. */
const FILLS = new Set(["bg-primary", "bg-danger", "bg-success", "bg-surface-muted"]);
const fillOf = (b: HTMLButtonElement) =>
  b.className.split(/\s+/).filter((c) => FILLS.has(c));

beforeEach(() => {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});

afterEach(() => {
  root?.unmount();
  host.remove();
  root = null;
});

describe("a disabled filled button loses its fill", () => {
  it("keeps the primary fill while it is usable", async () => {
    const b = await render({});
    expect(fillOf(b)).toEqual(["bg-primary"]);
    expect(b.disabled).toBe(false);
  });

  it("drops the primary fill when disabled, and takes the muted surface instead", async () => {
    const b = await render({ disabled: true });
    expect(b.disabled).toBe(true);
    expect(fillOf(b)).toEqual(["bg-surface-muted"]);
    // Positively takes the inert treatment: "no fill" must not be achieved by rendering nothing.
    expect(b.className).toMatch(/border-border/);
  });

  it("does the same for the other filled variants", async () => {
    for (const variant of ["danger", "success", "secondary"] as const) {
      const b = await render({ variant, disabled: true });
      expect(fillOf(b), `${variant} kept a fill`).toEqual(["bg-surface-muted"]);
    }
  });

  it("leaves the FILL-LESS variants alone - they have nothing to remove", async () => {
    // `outline` already renders `bg-surface`, and `ghost`/`link` render no background at all, so for
    // them opacity IS a real change of appearance. Overriding them would be a restyle nobody asked for.
    const outline = await render({ variant: "outline", disabled: true });
    expect(outline.className).toMatch(/bg-surface(?!-muted)/);
    for (const variant of ["ghost", "link"] as const) {
      const b = await render({ variant, disabled: true });
      expect(fillOf(b), `${variant} gained a fill`).toEqual([]);
    }
  });

  it("does not rest on opacity alone", async () => {
    // The treatment that failed. Half-opacity is invisible against a light background, which is exactly
    // how a saturated blue went on reading as pressable.
    const b = await render({ disabled: true });
    const opacityOnlyBlue = b.className.includes("opacity-50") && fillOf(b).includes("bg-primary");
    expect(opacityOnlyBlue).toBe(false);
  });

  it("mentions the fill NOWHERE in the class list, not even behind a hover variant", async () => {
    // So a reader, a grep or a screenshot differ all find the same truth rather than having to reason
    // about whether `hover:bg-primary/90` can fire on a pointer-events-none element.
    const b = await render({ disabled: true });
    expect(b.className).not.toMatch(/bg-primary/);
  });
});

describe("a LOADING button is busy, not unavailable", () => {
  it("keeps its fill while loading, although it is not pressable", async () => {
    // THE REGRESSION THIS CATCHES. Folding `loading` into the unavailable look drains the fill of every
    // spinning submit button in every portal - 73 call sites - and says "this became unavailable" about
    // a control that is doing exactly what was asked of it.
    const b = await render({ loading: true });
    expect(b.disabled, "a loading button must still be un-pressable").toBe(true);
    expect(fillOf(b), "a loading button must keep its fill").toEqual(["bg-primary"]);
  });

  it("renders its spinner, which is what makes the retained fill honest", async () => {
    const b = await render({ loading: true });
    expect(b.querySelector("svg"), "no spinner rendered").not.toBeNull();
  });

  it("keeps its fill even when `disabled` is ALSO set, because the spinner is what a reader sees", async () => {
    // THE TIE-BREAK, and it goes to `loading` deliberately. Whenever the spinner renders it is the
    // dominant signal on the control, and a drained fill beside a spinner is self-contradictory - it
    // says "unavailable" and "working on it" at once. `<Button loading={busy} disabled={!valid}>` is a
    // real shape, so the pair is reachable.
    //
    // The `disabled` ATTRIBUTE is set from either cause regardless, so nothing about pressability turns
    // on this - only which of two appearances a busy control wears.
    const b = await render({ loading: true, disabled: true });
    expect(b.disabled).toBe(true);
    expect(fillOf(b)).toEqual(["bg-primary"]);
    expect(b.querySelector("svg"), "the spinner is what licenses keeping the fill").not.toBeNull();
  });

  it("and the two states are DISTINGUISHABLE, which is the whole point", async () => {
    const loading = (await render({ loading: true })).className;
    root!.unmount();
    host.remove();
    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    const disabled = (await render({ disabled: true })).className;
    expect(loading).not.toBe(disabled);
  });
});
