// @vitest-environment jsdom
//
// THE RULE OF SLICE S-17, pinned where it can actually break: **an unverified logo renders NOTHING.**
//
// Not a placeholder, not a broken-image icon, not a generic avatar, not an initials block. A logo is
// the strongest visual claim of legitimacy in the product, so a stand-in shown for one that could
// not be verified is precisely how a forged provider comes to look real.
//
// This has to be a MOUNTED test rather than a pure one, for a reason that is specific and not
// stylistic: the way the rule breaks is TIMING. An `<img>` whose `src` points at unverified bytes is
// fetched — and may be PAINTED — before any check lands, so a component that mounts the element and
// corrects itself afterwards has already shown the thing it exists to withhold. A pure assertion
// over a returned state object cannot see that; only the DOM can.
//
// DO NOT rewrite this with `act()`. Same reason `rpcSettingsVerdict.test.ts` gives: `act()` drains
// React's work into its own queue, reordering promise continuations against passive effects — which
// is exactly the ordering this file is about. Real macrotask turns instead, and no
// `IS_REACT_ACT_ENVIRONMENT`.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { ProviderLogo } from "../src/mirror";
import type { LogoState } from "../src/mirror";

const LOGO_BYTES = new TextEncoder().encode("the genuine clinic logo bytes");

let container: HTMLDivElement;
let root: Root;
/** Every object URL this render minted. The list being EMPTY is the assertion that matters most. */
let minted: string[];

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  minted = [];
});

afterEach(async () => {
  root.unmount();
  container.remove();
  await settle();
});

/** Real macrotask turns, so passive effects run on React's own scheduler rather than in `act`'s. */
async function settle(turns = 4, ms = 5): Promise<void> {
  for (let turn = 0; turn < turns; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve, ms));
  }
}

function objectUrlFor(blob: Blob): string {
  const url = `blob:mock/${minted.length}`;
  void blob;
  minted.push(url);
  return url;
}

async function render(logo: LogoState | undefined): Promise<void> {
  root.render(
    createElement(ProviderLogo, {
      logo,
      providerName: "Seaport Veterinary",
      objectUrlFor,
      revokeObjectUrl: () => {},
    }),
  );
  await settle();
}

const images = () => container.querySelectorAll("img");
const text = () => container.textContent ?? "";

describe("an unverified logo renders NOTHING", () => {
  it("mounts no <img> at all for a hash mismatch", async () => {
    await render({
      state: "unverified",
      reason: "the logo does not match the address this provider published",
    });

    // The absence must be STRUCTURAL. A hidden or zero-sized image would still have been fetched,
    // and a `src` is a request the moment it is in the DOM.
    expect(images()).toHaveLength(0);
    expect(container.innerHTML).not.toContain("src=");
    expect(minted).toEqual([]);
  });

  it("mints no object URL, so the unverified bytes are never addressable by the browser", async () => {
    await render({ state: "unverified", reason: "the mirror could not be reached" });
    expect(minted).toEqual([]);
  });

  it("SAYS WHY, in visible text rather than a tooltip", async () => {
    // A tooltip is not a state: hover survives neither a screenshot nor a touch device. This repo
    // has ruled that shortcut insufficient twice already.
    const reason = "the logo does not match the address this provider published";
    await render({ state: "unverified", reason });

    expect(text()).toContain("Logo not shown.");
    expect(text()).toContain(reason);
    // And it is rendered as text, not stashed in an attribute nobody sees without a pointer.
    expect(container.querySelector("[title]")).toBeNull();
  });

  it("renders no placeholder, avatar, initials block or broken-image icon", async () => {
    await render({ state: "unverified", reason: "unreachable" });
    expect(images()).toHaveLength(0);
    expect(container.querySelectorAll("svg[data-avatar]")).toHaveLength(0);
    // The provider's name must not be rendered AS the image - an initials block is a stand-in too.
    expect(text()).not.toContain("SV");
  });
});

describe("a verified logo renders, and only after it verified", () => {
  it("mounts the image from bytes that already verified", async () => {
    await render({ state: "verified", bytes: LOGO_BYTES, mediaType: "image/png" });

    const img = images();
    expect(img).toHaveLength(1);
    expect(img[0]!.getAttribute("src")).toBe("blob:mock/0");
    expect(img[0]!.getAttribute("alt")).toBe("Seaport Veterinary logo");
    expect(minted).toHaveLength(1);
  });

  it("mounts NO image while the resolution is still pending", async () => {
    // The ordering property, stated directly. `undefined` is the pending state, and it must render
    // nothing at all - the only thing worse than a placeholder for an unverified logo is one shown
    // before anybody has looked.
    await render(undefined);
    expect(images()).toHaveLength(0);
    expect(text()).toBe("");
    expect(minted).toEqual([]);

    // ...and it appears once, and only once, verification has landed.
    await render({ state: "verified", bytes: LOGO_BYTES, mediaType: "image/png" });
    expect(images()).toHaveLength(1);
  });

  it("takes the image DOWN when a later resolution fails", async () => {
    await render({ state: "verified", bytes: LOGO_BYTES, mediaType: "image/png" });
    expect(images()).toHaveLength(1);

    await render({ state: "unverified", reason: "the mirror could not be reached" });
    expect(images()).toHaveLength(0);
    expect(text()).toContain("Logo not shown.");
  });
});

describe("notPublished is ORDINARY and must not read as a failure", () => {
  it("renders no image and a neutral line", async () => {
    await render({ state: "notPublished" });

    expect(images()).toHaveLength(0);
    expect(text()).toContain("No logo published");
    expect(text()).not.toContain("Logo not shown.");
  });

  it("is visually distinguishable from unverified, not merely differently worded", async () => {
    // Both render no image, so tone is the only thing separating them. This repo's standing rule:
    // facts render neutral, failures warn. A reader scanning the page must be able to tell "this
    // provider published no logo" from "this provider's logo did not verify" without hovering
    // anything.
    await render({ state: "notPublished" });
    const absent = container.querySelector("[data-logo-state='notPublished']");
    expect(absent).not.toBeNull();
    const absentClasses = absent!.className;

    await render({ state: "unverified", reason: "unreachable" });
    const unverified = container.querySelector("[data-logo-state='unverified']");
    expect(unverified).not.toBeNull();
    const unverifiedClasses = unverified!.className;

    expect(unverifiedClasses).not.toBe(absentClasses);
    // The failure warns; the fact does not.
    expect(unverifiedClasses).toContain("amber");
    expect(absentClasses).not.toContain("amber");
    // And the failure carries an icon the ordinary case does not.
    expect(container.querySelectorAll("svg").length).toBeGreaterThan(0);
  });
});

describe("the three states are exhaustive and mutually exclusive", () => {
  it("renders exactly one of image / unverified / notPublished, never two", async () => {
    const cases: Array<[string, LogoState | undefined]> = [
      ["verified", { state: "verified", bytes: LOGO_BYTES, mediaType: "image/png" }],
      ["unverified", { state: "unverified", reason: "unreachable" }],
      ["notPublished", { state: "notPublished" }],
      ["pending", undefined],
    ];
    for (const [name, logo] of cases) {
      await render(logo);
      const rendered = [
        container.querySelectorAll("[data-testid='provider-logo']").length,
        container.querySelectorAll("[data-testid='provider-logo-unverified']").length,
        container.querySelectorAll("[data-testid='provider-logo-absent']").length,
      ];
      const total = rendered.reduce((a, b) => a + b, 0);
      expect(total, `${name} rendered ${total} states`).toBe(name === "pending" ? 0 : 1);
    }
  });
});
