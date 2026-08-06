// @vitest-environment jsdom
//
// "Fill demo data" on the registrar's Register-a-provider dialog must NOT supply a controller.
//
// The controller is the address that will act AS the provider - it deploys and owns that provider's
// issuing contracts - so a prefilled one is a single address shared by every reader of the demo, and
// the only address a shipped preset can name is one whose private key is published. This field held
// anvil's account 0 until 2026-08-06, which is exactly that: a key with no secrecy at all, so anyone
// in the world could act as any provider registered with it.
//
// The claim needs MOUNTING rather than a pure assertion on the constant, because what a reader is
// handed is the field's state after pressing the button - a call site that filled it from anywhere
// else, or a later "helpful" default, would leave `DEMO_PROVIDER_REGISTRATION.controller` empty and
// the screen still prefilled.
//
// This file is separate from `providersPage.test.tsx` because it needs `VITE_DEMO_MODE` SET, while
// every case there needs it unset (the button does not render at all otherwise). `env` is a
// module-level literal evaluated at import time, so the stub must precede the import - hence
// `resetModules` + dynamic import rather than a top-level `import`.
//
// Deliberately no `act()`, matching its sibling: it drains React's work into its own queue and
// reorders promise continuations against passive effects.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement, type ComponentType } from "react";
import { createRoot, type Root } from "react-dom/client";

const PID = `0x${"a1".repeat(20)}`;
const REGISTRY = `0x${"c3".repeat(20)}`;

let container: HTMLDivElement;
let root: Root;

async function settle(turns = 8) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

/** The three orthogonal reads the page issues on mount, plus an empty provider list. */
function stubFetch() {
  const json = (body: unknown) =>
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/verifier-capabilities")) return json({ registry: REGISTRY, purposes: [] });
      if (url.includes("/resolvers")) return json({ registry: REGISTRY, kinds: [] });
      if (url.includes("/services"))
        return json({ registry: REGISTRY, providerId: PID, services: [] });
      return json({
        registry: REGISTRY,
        providers: [],
        authority: {
          target: REGISTRY,
          owner: `0x${"ad".repeat(20)}`,
          hostedSigner: `0x${"ad".repeat(20)}`,
          heldByHosted: true,
          capability: "registerProvider / setProviderStanding / setServiceCreationApproval",
        },
        identitySchema: {
          schema: 1,
          schemaId: "dogtag/provider-identity/1",
          hashAlgorithm: 0x1b,
        },
      });
    }),
  );
}

/**
 * Import the page with demo mode ON, mount it, and open the register dialog.
 *
 * `ToastProvider` is imported HERE rather than at the top of the file: `resetModules` gives the
 * dynamic imports a fresh `@dogtag/ui` module graph, so a statically imported provider would create
 * a DIFFERENT React context from the one `AppProvider`'s `useToast` reads, and every case would die
 * with "useToast must be used within a <ToastProvider>".
 */
async function openDialogInDemoMode() {
  vi.stubEnv("VITE_DEMO_MODE", "1");
  vi.resetModules();
  const { ToastProvider } = (await import("@dogtag/ui")) as { ToastProvider: ComponentType<never> };
  const { AppProvider } = await import("../src/app/AppContext");
  const { Providers } = (await import("../src/pages/Providers")) as {
    Providers: ComponentType;
  };
  stubFetch();
  root.render(
    createElement(ToastProvider, null, createElement(AppProvider, null, createElement(Providers))),
  );
  await settle();
  buttonWithText("Register provider")!.click();
  await settle();
}

function byId(id: string) {
  const el = document.body.querySelector<HTMLInputElement>(`#${id}`);
  if (!el) throw new Error(`no input #${id} is rendered`);
  return el;
}

function buttonWithText(text: string) {
  return [...document.body.querySelectorAll("button")].find(
    (b) => (b.textContent ?? "").trim().toLowerCase() === text.toLowerCase(),
  );
}

function type(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  window.localStorage.setItem("admin.token", "test-token");
});

afterEach(() => {
  root.unmount();
  container.remove();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  window.localStorage.clear();
});

describe("Providers - the demo fill and the controller address", () => {
  it("fills the identity statement and the provider id, but leaves the controller blank", async () => {
    await openDialogInDemoMode();
    // Guard the premise: without the button this case would pass vacuously.
    expect(buttonWithText("Fill demo data")).toBeDefined();

    buttonWithText("Fill demo data")!.click();
    await settle();

    // The two things a demo CAN honestly supply.
    expect(byId("legalName").value).not.toBe("");
    expect(byId("pid").value).toMatch(/^0x[0-9a-f]{40}$/);
    // The one it cannot.
    expect(byId("ctrl").value).toBe("");
  });

  it("CLEARS a controller left over from an earlier attempt rather than leaving it standing", async () => {
    await openDialogInDemoMode();
    type(byId("ctrl"), `0x${"b2".repeat(20)}`);
    await settle();
    expect(byId("ctrl").value).not.toBe("");

    buttonWithText("Fill demo data")!.click();
    await settle();

    // An address left over from an earlier attempt, sitting under freshly filled demo identity data,
    // reads as though the preset supplied it.
    expect(byId("ctrl").value).toBe("");
  });

  it("says on the screen that the blank is deliberate and what to do about it", async () => {
    await openDialogInDemoMode();
    const note = document.body.querySelector('[data-testid="demo-controller-note"]');
    expect(note).not.toBeNull();
    const text = note!.textContent ?? "";
    // States the fact, the reason, and the remedy - a blank field with no explanation reads as a bug.
    expect(text).toContain("blank");
    expect(text.toLowerCase()).toContain("published");
    expect(text).toContain("Generate your own");
  });

  /**
   * The qualifier must be ON the field, not only in the banner beside the button.
   *
   * Seen in a real browser: that banner sits two fields above Controller address and scrolls out of
   * view, so a reader who arrives at an empty box reads only its always-on helper - which says what
   * the field is FOR and nothing about why it is empty - and concludes the fill is broken. Same rule
   * this page already applies to a superseded review verdict.
   */
  it("attaches the qualifier to the empty field itself, and withdraws it once one is entered", async () => {
    await openDialogInDemoMode();
    buttonWithText("Fill demo data")!.click();
    await settle();

    const at = () => document.body.querySelector('[data-testid="demo-controller-field-note"]');
    expect(at()).not.toBeNull();
    expect(at()!.textContent).toContain("Empty on purpose");

    type(byId("ctrl"), `0x${"b2".repeat(20)}`);
    await settle();

    // Once the reader has supplied one there is nothing left to explain, and a note that stayed
    // would read as an unresolved problem with a field that is now correct.
    expect(at()).toBeNull();
  });

  it("cannot be registered past: the review refuses a blank controller", async () => {
    await openDialogInDemoMode();
    buttonWithText("Fill demo data")!.click();
    await settle();
    buttonWithText("Review")!.click();
    await settle();

    // Fail-closed, and it says which field. `Register` was already disabled before the review; what
    // this pins is that reviewing does not ENABLE it, which is the only way to reach a send.
    expect(document.body.textContent).toContain("Controller must be a 0x-prefixed 20-byte address");
    expect(buttonWithText("Register")!.disabled).toBe(true);
  });

  it("reviews and permits the send once a controller is pasted in", async () => {
    await openDialogInDemoMode();
    buttonWithText("Fill demo data")!.click();
    await settle();
    // The reader's own generated address (docs/DEMO_CLICKS.md §1.3).
    type(byId("ctrl"), `0x${"b2".repeat(20)}`);
    await settle();
    buttonWithText("Review")!.click();
    await settle();

    // Without this the blank-controller cases above would be satisfied by a dialog that can never
    // send at all.
    expect(buttonWithText("Register")!.disabled).toBe(false);
  });
});
