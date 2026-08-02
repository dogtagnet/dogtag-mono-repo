// @vitest-environment jsdom
//
// The Providers registrar page, rendered for real.
//
// Two of the three properties this page exists to hold are properties of what an admin can REACH,
// not of any pure function, so only mounting can see them:
//
//   1. An approval log that could not be READ renders as its own state with its reason - never as
//      "approved for nothing" - and the record-type toggles are disabled, because the contract
//      refuses a redundant write and we do not know the current bit.
//   2. A reviewed registration loses its authority to send the moment an input changes, while
//      STAYING VISIBLE with its verdict struck through. `shown` and `fresh` answer different
//      questions; collapsing them either destroys the record of what was reviewed or lets a send
//      carry values nobody looked at.
//
// The page is mounted under its real providers with ONLY `fetch` substituted, in the same spirit as
// `whitelistChainRefs.test.tsx` and `packages/ui/test/rpcSettingsVerdict.test.ts`. Deliberately no
// `act()`: it drains React's work into its own queue and reorders promise continuations against
// passive effects, which is exactly the ordering these cases turn on.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ToastProvider } from "@dogtag/ui";
import { AppProvider } from "../src/app/AppContext";
import { Providers } from "../src/pages/Providers";

const PID = `0x${"a1".repeat(20)}`;
const CONTROLLER = `0x${"b2".repeat(20)}`;
const REGISTRY = `0x${"c3".repeat(20)}`;

let container: HTMLDivElement;
let root: Root;

async function settle(turns = 8) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount() {
  root.render(
    createElement(ToastProvider, null, createElement(AppProvider, null, createElement(Providers))),
  );
  await settle();
}

/** `GET /v1/admin/providers` returning one provider with the given approvals shape. */
function listReturning(approvals: unknown, standing = "active") {
  return vi.fn(async () =>
    new Response(
      JSON.stringify({
        registry: REGISTRY,
        providers: [
          {
            provider: {
              providerId: PID,
              controller: CONTROLLER,
              pendingController: `0x${"0".repeat(40)}`,
              controllerEpoch: 1,
              standing,
              registered: true,
            },
            identityAnchor: {
              digest: `0x${"9".repeat(64)}`,
              schema: 1,
              codec: 0,
              hashAlgorithm: 0x1b,
              revision: 1,
              updatedAtBlock: 10,
            },
            approvals,
          },
        ],
        authority: {
          target: REGISTRY,
          owner: `0x${"ad".repeat(20)}`,
          hostedSigner: `0x${"ad".repeat(20)}`,
          heldByHosted: true,
          capability: "registerProvider / setProviderStanding / setServiceCreationApproval",
        },
        identitySchema: { schema: 1, schemaId: "dogtag/provider-identity/1", hashAlgorithm: 0x1b },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  );
}

const PROPOSED_CALLDATA = `0xdeadbeef${"c7".repeat(100)}`;

/** One provider row, active and approved for nothing, as the list route would ship it. */
function oneProvider(over: Record<string, unknown> = {}) {
  return {
    provider: {
      providerId: PID,
      controller: CONTROLLER,
      pendingController: `0x${"0".repeat(40)}`,
      controllerEpoch: 1,
      standing: "active",
      registered: true,
    },
    identityAnchor: {
      digest: `0x${"9".repeat(64)}`,
      schema: 1,
      codec: 0,
      hashAlgorithm: 0x1b,
      revision: 1,
      updatedAtBlock: 10,
    },
    approvals: { state: "resolved", entries: [] },
    ...over,
  };
}

function listBody(providers: unknown[]) {
  return {
    registry: REGISTRY,
    providers,
    authority: {
      target: REGISTRY,
      owner: `0x${"ad".repeat(20)}`,
      hostedSigner: `0x${"ad".repeat(20)}`,
      heldByHosted: true,
      capability: "registerProvider / setProviderStanding / setServiceCreationApproval",
    },
    identitySchema: { schema: 1, schemaId: "dogtag/provider-identity/1", hashAlgorithm: 0x1b },
  };
}

/** The list route with one provider whose fields are overridden. */
function listWith(over: Record<string, unknown>) {
  return vi.fn(async () =>
    new Response(JSON.stringify(listBody([oneProvider(over)])), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
}

/**
 * A fetch that answers BOTH the list read and the three registrar writes.
 *
 * Writes default to the `proposed_by_design` outcome, because that is the configuration these cases
 * are about: nothing is broadcast, so the returned calldata is the whole deliverable.
 */
function routing(opts: {
  providers: unknown[];
  outcome?: string;
  listFailsAfter?: () => boolean;
}) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), {
        status,
        headers: { "content-type": "application/json" },
      });
    if ((init?.method ?? "GET") === "GET") {
      if (opts.listFailsAfter?.()) return json({ error: "rpc unreachable" }, 502);
      return json(listBody(opts.providers));
    }
    const executed = opts.outcome === "executed";
    return json({
      outcome: opts.outcome ?? "proposed_by_design",
      executed,
      warning: executed ? null : "the hosted key does not own this registry",
      actions: [
        executed
          ? {
              disposition: "executed",
              txHash: `0x${"e1".repeat(32)}`,
              holder: `0x${"ad".repeat(20)}`,
              summary: "registrar action",
            }
          : {
              disposition: "proposed",
              holder: `0x${"ad".repeat(20)}`,
              target: REGISTRY,
              calldata: PROPOSED_CALLDATA,
              authority: "owner",
              summary: "registrar action",
            },
      ],
      ...(url.includes("/service-approval") ? { recordType: "X" } : {}),
    });
  });
}

/** Every value written to the clipboard this case, in order. Installed by `beforeEach`. */
let copied: string[] = [];

/**
 * Click the `CopyButton` for `label` and return what it put on the clipboard.
 *
 * Asserting on the CLIPBOARD rather than on rendered text is the point: `shortAddr`/`slice`
 * truncation removes the elided characters from the DOM entirely, so for a value that has to be
 * signed somewhere else, copy is the operator's only route to it - a rendered prefix proves nothing.
 */
async function copyValueFor(scope: Element, label: string): Promise<string | undefined> {
  const btn = [...scope.querySelectorAll("button")].find((b) =>
    (b.getAttribute("aria-label") ?? "").includes(label),
  );
  if (!btn) return undefined;
  const before = copied.length;
  btn.click();
  await settle();
  return copied[before];
}

function type(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(el, value);
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

// The dialog renders through a portal, so its nodes live under `document.body` rather than inside
// `container`. Everything below therefore queries the document; `afterEach` unmounts, which takes
// the portal with it, so no case can see a previous case's nodes.
function byId(id: string) {
  const el = document.body.querySelector<HTMLInputElement>(`#${id}`);
  if (!el) throw new Error(`no input #${id} is rendered`);
  return el;
}

/** Match on the button's own trimmed label, exactly - "Register" must not find "Register provider". */
function buttonWithText(text: string) {
  return [...document.body.querySelectorAll("button")].find(
    (b) => (b.textContent ?? "").trim().toLowerCase() === text.toLowerCase(),
  );
}

/** The dialog's review panel, wherever the portal put it. */
function reviewPanel() {
  return document.body.querySelector('[data-testid="registration-review"]');
}

/** Open the register dialog, fill a complete statement, review it and send. Assumes a mounted page. */
async function registerOnce() {
  buttonWithText("Register provider")!.click();
  await settle();
  type(byId("ctrl"), CONTROLLER);
  type(byId("legalName"), "Seaport Veterinary Clinic Pte Ltd");
  type(byId("jurisdiction"), "Singapore");
  type(byId("verifiedOn"), "2026-08-02");
  await settle();
  buttonWithText("Review")!.click();
  await settle();
  buttonWithText("Register")!.click();
  await settle();
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  window.localStorage.setItem("admin.token", "test-token");
  // jsdom ships no clipboard, and `CopyButton` falls back to `execCommand`, which jsdom does not
  // implement either - so without this a copy reports FAILED and the assertion would be about the
  // environment rather than about the value being reachable.
  copied = [];
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: async (v: string) => {
        copied.push(v);
      },
    },
  });
});

afterEach(() => {
  root.unmount();
  container.remove();
  vi.unstubAllGlobals();
  window.localStorage.clear();
});

describe("Providers - the approvals column", () => {
  it("says a provider is approved for nothing only when the log actually resolved", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    expect(container.textContent).toContain("Approved for nothing yet");
    expect(container.textContent).not.toContain("Could not be read");
  });

  /**
   * The load-bearing case. An unreadable log is a fact about US; rendering it as "approved for
   * nothing" would state something about the PROVIDER on the strength of a read that never happened.
   */
  it("renders an unreadable log as its own state with its reason, never as no approvals", async () => {
    vi.stubGlobal(
      "fetch",
      listReturning({ state: "unavailable", reason: "eth_getLogs timed out" }),
    );
    await mount();
    expect(container.textContent).toContain("Could not be read");
    expect(container.textContent).toContain("eth_getLogs timed out");
    expect(container.textContent).not.toContain("Approved for nothing yet");
  });

  /**
   * Could-not-check declines to guess rather than refusing the action: with the current bit unknown
   * a toggle could send a redundant write, which the contract refuses with `NoChange()`.
   */
  it("disables the record-type toggles while the current approval state is unknown", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "unavailable", reason: "rpc down" }));
    await mount();
    const vacc = buttonWithText("VACCINATION");
    expect(vacc).toBeDefined();
    expect(vacc!.disabled).toBe(true);

    root.unmount();
    container.remove();
    container = document.createElement("div");
    document.body.appendChild(container);
    root = createRoot(container);
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    expect(buttonWithText("VACCINATION")!.disabled).toBe(false);
  });

  /**
   * A withdrawn approval is a different fact from one that never existed, and only the log can tell
   * them apart - so it is rendered rather than filtered out.
   */
  it("shows a withdrawn approval rather than dropping it", async () => {
    vi.stubGlobal(
      "fetch",
      listReturning({
        state: "resolved",
        entries: [
          { recordTypeKey: `0x${"1".repeat(64)}`, recordType: "GROOMING", allowed: false },
          { recordTypeKey: `0x${"2".repeat(64)}`, recordType: "VACCINATION", allowed: true },
        ],
      }),
    );
    await mount();
    const badges = [...container.querySelectorAll("span")].map((s) => s.textContent ?? "");
    expect(badges.some((t) => t.trim() === "GROOMING")).toBe(true);
    expect(badges.some((t) => t.trim() === "VACCINATION")).toBe(true);
  });
});

describe("Providers - a PENDING provider is visibly inert", () => {
  /**
   * `registerProvider` writes PENDING and `canWriteProvider` admits only ACTIVE, so a provider left
   * here can do nothing at all. On a badge alone that is indistinguishable from ACTIVE.
   */
  it("says in words that a pending provider can do nothing yet", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }, "pending"));
    await mount();
    expect(container.textContent).toContain("INERT");
    expect(buttonWithText("Activate")).toBeDefined();
  });

  it("offers no activate button for a provider that is already active", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }, "active"));
    await mount();
    expect(buttonWithText("Activate")).toBeUndefined();
  });
});

describe("Providers - the registration review", () => {
  async function openDialogAndReview() {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    buttonWithText("Register provider")!.click();
    await settle();
    // The id is pre-generated; the rest is the registrar's assertion.
    type(byId("ctrl"), CONTROLLER);
    type(byId("legalName"), "Seaport Veterinary Clinic Pte Ltd");
    type(byId("jurisdiction"), "Singapore");
    type(byId("verifiedOn"), "2026-08-02");
    await settle();
    buttonWithText("Review")!.click();
    await settle();
  }

  it("only permits a send once the registration has been reviewed", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    buttonWithText("Register provider")!.click();
    await settle();
    // Nothing reviewed yet: the send is unavailable even though the id is already filled in.
    expect(buttonWithText("Register")!.disabled).toBe(true);
  });

  it("permits the send after a review, and shows the digest it committed to", async () => {
    await openDialogAndReview();
    const review = reviewPanel();
    expect(review).not.toBeNull();
    expect(review!.textContent).toContain("dogtag/provider-identity/1");
    expect(review!.textContent).toContain("Seaport Veterinary Clinic Pte Ltd");
    expect(review!.textContent).toMatch(/0x[0-9a-f]{64}/);
    expect(buttonWithText("Register")!.disabled).toBe(false);
  });

  /**
   * THE property this discipline exists for. Editing an input after review must retire the plan's
   * authority to send - here the mistake is unrecoverable, because a providerId cannot be reassigned
   * and the identity anchor's revision only moves forward.
   */
  it("retires the review when an input changes, while keeping it visible and marked superseded", async () => {
    await openDialogAndReview();
    expect(buttonWithText("Register")!.disabled).toBe(false);

    type(byId("legalName"), "A Completely Different Clinic");
    await settle();

    // Authority is gone...
    expect(buttonWithText("Register")!.disabled).toBe(true);
    // ...but the record of what was reviewed is NOT destroyed, and the qualifier sits ON the verdict
    // rather than only in a banner above it.
    const review = reviewPanel();
    expect(review).not.toBeNull();
    expect(review!.textContent).toContain("Superseded");
    expect(review!.textContent).toContain("Seaport Veterinary Clinic Pte Ltd");
    const verdict = [...review!.querySelectorAll("span")].find((s) =>
      (s.textContent ?? "").includes("Reviewed"),
    );
    expect(verdict!.className).toContain("line-through");
  });

  it("re-reviewing after an edit restores the authority to send", async () => {
    await openDialogAndReview();
    type(byId("legalName"), "A Completely Different Clinic");
    await settle();
    expect(buttonWithText("Register")!.disabled).toBe(true);
    buttonWithText("Review")!.click();
    await settle();
    expect(buttonWithText("Register")!.disabled).toBe(false);
    expect(reviewPanel()!.textContent).toContain("A Completely Different Clinic");
  });
});

describe("Providers - the authority banner", () => {
  it("says up front that the hosted key will execute directly", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    const banner = container.querySelector('[data-testid="registrar-authority"]');
    expect(banner!.textContent).toContain("execute directly");
  });

  /** `heldByHosted` is tri-state: null is "could not establish", which is not "no". */
  it("does not claim either way when holdership could not be established", async () => {
    vi.stubGlobal("fetch", async () =>
      new Response(
        JSON.stringify({
          registry: REGISTRY,
          providers: [],
          authority: {
            target: REGISTRY,
            owner: null,
            hostedSigner: null,
            heldByHosted: null,
            capability: "registerProvider",
          },
          identitySchema: { schema: 1, schemaId: "dogtag/provider-identity/1", hashAlgorithm: 27 },
        }),
        { status: 200, headers: { "content-type": "application/json" } },
      ),
    );
    await mount();
    const banner = container.querySelector('[data-testid="registrar-authority"]');
    expect(banner!.textContent).toContain("Could not establish");
    expect(banner!.textContent).not.toContain("execute directly");
  });
});

describe("Providers - a failed read", () => {
  /**
   * An unreadable registry must not fall through to an empty list, which would read as the definite
   * "no providers exist" about a registry we never successfully asked.
   */
  it("says the registry could not be read rather than showing an empty registry", async () => {
    vi.stubGlobal("fetch", async () =>
      new Response(JSON.stringify({ error: "PROVIDER_REGISTRY_ADDR not configured" }), {
        status: 503,
        headers: { "content-type": "application/json" },
      }),
    );
    await mount();
    expect(container.textContent).toContain("could not be read");
    expect(container.textContent).toContain("PROVIDER_REGISTRY_ADDR not configured");
    expect(container.textContent).not.toContain("No providers are registered");
  });
});

describe("Providers - the identity anchor on the row", () => {
  /**
   * The identity statement is never sent to the backend and is stored nowhere, so the on-chain
   * digest is the only thing an admin can check a kept copy against. Reading it costs no extra
   * chain call - the list response already carries it.
   */
  it("renders the on-chain identity digest so a kept statement can be checked against it", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    const anchor = container.querySelector('[data-testid="identity-anchor"]');
    expect(anchor).not.toBeNull();
    expect(anchor!.textContent).toContain("rev 1");
    // Truncated on screen, so it must be offered whole - the elided characters are not in the DOM.
    const copy = [...anchor!.querySelectorAll("button")].find((b) =>
      (b.getAttribute("aria-label") ?? "").includes("identity digest"),
    );
    expect(copy).toBeDefined();
  });

  /** Three states, as everywhere else: an anchor that could not be READ is not a missing one. */
  it("renders an unreadable anchor as its own state rather than as absent", async () => {
    vi.stubGlobal(
      "fetch",
      listWith({ identityAnchor: { unavailable: "publicIdentityAnchor reverted" } }),
    );
    await mount();
    expect(container.textContent).toContain("Identity anchor could not be read");
    expect(container.textContent).toContain("publicIdentityAnchor reverted");
    expect(container.querySelector('[data-testid="identity-anchor"]')).toBeNull();
  });
});

describe("Providers - a dispatched action is a durable record", () => {
  /**
   * THE defect this log exists for. A proposed REGISTRATION broadcasts nothing, so the provider id
   * never reaches `_providerIds`, the list stays empty and the table is not rendered at all. With the
   * payload living in a table row, the unsigned calldata the operator must sign out of band was
   * unreachable - which made a propose-only deployment, the posture a cautious operator would choose,
   * the one configuration in which a registration could not be completed.
   */
  it("renders a proposed registration's calldata even though no provider row exists", async () => {
    vi.stubGlobal("fetch", routing({ providers: [] }));
    await mount();
    await registerOnce();

    // The premise: nothing was broadcast, so there is genuinely no row to hang a payload on.
    expect(container.textContent).toContain("No providers are registered");
    const log = container.querySelector('[data-testid="dispatch-log"]');
    expect(log).not.toBeNull();
    expect(log!.textContent).toContain("Nothing was broadcast");
    // Offered WHOLE: a truncated calldata is not a smaller deliverable, it is no deliverable.
    expect(await copyValueFor(log!, "calldata")).toBe(PROPOSED_CALLDATA);
    expect(await copyValueFor(log!, "target")).toBe(REGISTRY);
  });

  /**
   * Keyed by the ACTION, not by the provider: approving one record type and then another in a
   * propose-only deployment leaves the operator holding TWO payloads to sign, and losing the first
   * is silent - they may never learn it existed.
   */
  it("keeps both payloads when two record types are approved for one provider", async () => {
    vi.stubGlobal("fetch", routing({ providers: [oneProvider()] }));
    await mount();

    buttonWithText("VACCINATION")!.click();
    await settle();
    buttonWithText("GROOMING")!.click();
    await settle();

    const log = container.querySelector('[data-testid="dispatch-log"]')!;
    expect(log.textContent).toContain("approved VACCINATION");
    expect(log.textContent).toContain("approved GROOMING");
    expect(log.querySelectorAll('[data-testid="dispatch-record"]').length).toBe(2);
  });

  /**
   * A reload that fails replaces the table with the read-error panel. The payload must not go with
   * it: the send already happened, and the operator still has to act on what it returned.
   */
  it("survives a failed reload that replaces the provider table", async () => {
    let listCalls = 0;
    vi.stubGlobal(
      "fetch",
      routing({
        providers: [oneProvider()],
        // The reload AFTER the send fails, which is the ordering that used to take the payload.
        listFailsAfter: () => ++listCalls > 1,
      }),
    );
    await mount();

    buttonWithText("VACCINATION")!.click();
    await settle();

    expect(container.textContent).toContain("The provider registry could not be read");
    const log = container.querySelector('[data-testid="dispatch-log"]');
    expect(log).not.toBeNull();
    expect(await copyValueFor(log!, "calldata")).toBe(PROPOSED_CALLDATA);
  });

  /**
   * The success path's half of the file's own rule: what was reviewed stays readable after the send.
   * The statement is never sent to the backend and stored nowhere, and the dialog closes - so the
   * record IS the only remaining copy, and the screen asked the operator to keep theirs on the
   * strength of it being here.
   */
  it("keeps the reviewed identity statement after the dialog closes", async () => {
    vi.stubGlobal("fetch", routing({ providers: [], outcome: "executed" }));
    await mount();
    await registerOnce();

    // The dialog is gone...
    expect(reviewPanel()).toBeNull();
    // ...and the statement it committed to is not.
    const log = container.querySelector('[data-testid="dispatch-log"]')!;
    expect(log.textContent).toContain("Seaport Veterinary Clinic Pte Ltd");
    expect(await copyValueFor(log, "statement")).toContain("Seaport Veterinary Clinic Pte Ltd");
    expect(await copyValueFor(log, "identity digest")).toMatch(/^0x[0-9a-f]{64}$/);
  });
});
