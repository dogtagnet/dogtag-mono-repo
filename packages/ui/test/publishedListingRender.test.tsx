// @vitest-environment jsdom
//
// `PublishedListingCard` — the surface that tells a provider what a READER sees of their listing.
//
// It gets its own mounted suite because it RE-DERIVES the resolution's states rather than passing
// them through, and it is the surface that decides whether contacts render as published facts. The
// logo rule itself is `ProviderLogo`'s and is pinned in providerLogoRender.test.tsx; what is pinned
// here is that this card routes each state to the right rendering, and in particular that it never
// presents an unverified document's contents as published.
//
// The case that motivated it: `unconfigured` and `pending` both had the spelling `undefined`, so a
// deployment with no content mirror rendered a spinner that could never resolve - "a spinner that
// never resolves and a provider who cannot tell whether anything happened", which this component's
// own neighbour already names as a defect. The publish path refuses loudly in that same deployment,
// so the read path failing silently made the two halves of one surface disagree.
//
// No `act()`, same reason as the sibling suites: it reorders promise continuations against passive
// effects, which is the ordering these components are about.
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { PublishedListingCard } from "../src/domain/ProviderSelfServicePanel";
import { blankContactFields } from "../src/directory/registration";
import type { ProfileResolution } from "../src/mirror";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  root.unmount();
  container.remove();
  await settle();
});

async function settle(turns = 4, ms = 5): Promise<void> {
  for (let turn = 0; turn < turns; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve, ms));
  }
}

async function render(
  resolution: ProfileResolution | undefined,
  unconfigured = false,
): Promise<void> {
  root.render(
    createElement(PublishedListingCard, {
      resolution,
      providerName: "0x3f5c9a1e77b204d8e6130fa95c8b47e2d61099af",
      unconfigured,
    }),
  );
  await settle();
}

const has = (testId: string) => container.querySelector(`[data-testid='${testId}']`) !== null;
const text = () => container.textContent ?? "";

const RESOLVED: ProfileResolution = {
  state: "resolved",
  address: `0x${"ab".repeat(32)}`,
  profile: {
    contact: { ...blankContactFields(), phone: "+65 6123 4567" },
    logo: null,
  },
  logo: { state: "notPublished" },
};

describe("cannot-start is its own state, never the pending spinner", () => {
  it("says a mirror is not configured rather than spinning forever", async () => {
    await render(undefined, true);

    expect(has("listing-unconfigured")).toBe(true);
    expect(has("listing-pending")).toBe(false);
    expect(container.querySelector(".animate-spin")).toBeNull();
    expect(text()).toContain("VITE_CONTENT_MIRROR_BASE");
  });

  it("still spins while a resolution that CAN start has not finished", async () => {
    // The distinction the bug collapsed. `undefined` means "not finished yet" and nothing else.
    await render(undefined, false);

    expect(has("listing-pending")).toBe(true);
    expect(has("listing-unconfigured")).toBe(false);
  });

  it("prefers the unconfigured line over the spinner when both could apply", async () => {
    // `unconfigured` implies the resolution never started, so `resolution` is `undefined` in that
    // deployment. If the pending branch won, the spinner would come back for the exact case this
    // state was added for.
    await render(undefined, true);
    expect(has("listing-pending")).toBe(false);
  });
});

describe("an unverified document's contents are never rendered as published facts", () => {
  it("shows the failure and NO contacts", async () => {
    await render({
      state: "unverified",
      reason: "the profile document does not match the address this provider published",
      logo: { state: "unverified", reason: "unreachable" },
    });

    expect(has("listing-unverified")).toBe(true);
    expect(has("listing-resolved")).toBe(false);
    expect(text()).toContain("could not be confirmed");
    expect(text()).not.toContain("+65");
  });

  it("renders that failure in the WARNING tone, not the ordinary one", async () => {
    // The chain says something IS published and a reader cannot confirm it, which looks from the
    // outside exactly like publishing nothing. Quiet grey would understate it.
    await render({
      state: "unverified",
      reason: "unreachable",
      logo: { state: "unverified", reason: "unreachable" },
    });
    const el = container.querySelector("[data-testid='listing-unverified']");
    expect(el).not.toBeNull();
    expect(el!.className).toContain("amber");
  });
});

describe("the ordinary states read as ordinary", () => {
  it("renders the verified contacts once the document resolved", async () => {
    await render(RESOLVED);

    expect(has("listing-resolved")).toBe(true);
    expect(text()).toContain("+65 6123 4567");
    expect(has("listing-unverified")).toBe(false);
  });

  it("omits a channel the provider did not publish rather than showing it blank", async () => {
    await render(RESOLVED);
    expect(text()).toContain("phone");
    expect(text()).not.toContain("whatsapp");
  });

  it("tells never-published from withdrawn, quietly in both cases", async () => {
    await render({ state: "notPublished", withdrawn: false });
    expect(has("listing-not-published")).toBe(true);
    expect(text()).toContain("not published any details yet");
    expect(container.querySelector("[data-testid='listing-not-published']")!.className).not.toContain(
      "amber",
    );

    await render({ state: "notPublished", withdrawn: true });
    expect(text()).toContain("taken your published details down");
  });
});

describe("exactly one state renders at a time", () => {
  it("never shows two", async () => {
    const cases: Array<[string, ProfileResolution | undefined, boolean]> = [
      ["unconfigured", undefined, true],
      ["pending", undefined, false],
      ["resolved", RESOLVED, false],
      ["unverified", { state: "unverified", reason: "x", logo: { state: "unverified", reason: "x" } }, false],
      ["notPublished", { state: "notPublished", withdrawn: false }, false],
    ];
    for (const [name, resolution, unconfigured] of cases) {
      await render(resolution, unconfigured);
      const shown = [
        "listing-unconfigured",
        "listing-pending",
        "listing-resolved",
        "listing-unverified",
        "listing-not-published",
      ].filter(has);
      expect(shown, `${name} rendered ${shown.join(" + ")}`).toHaveLength(1);
    }
  });
});
