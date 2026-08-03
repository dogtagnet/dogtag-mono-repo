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

/**
 * The two ORTHOGONAL reads the page now issues on mount, plus the per-provider services read.
 *
 * Every mock below routes through this first. Without it a canned single-response `fetch` answers
 * the providers payload to `GET /v1/admin/resolvers` too, and the page blows up on `kinds.map` -
 * which fails every case in the file for a reason that has nothing to do with what it asserts.
 */
function journeyStub(url: string): Response | null {
  const json = (body: unknown) =>
    new Response(JSON.stringify(body), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  if (url.includes("/verifier-capabilities")) {
    return json({ registry: REGISTRY, purposes: [] });
  }
  if (url.includes("/resolvers")) {
    return json({ registry: REGISTRY, kinds: [] });
  }
  if (url.includes("/services")) {
    return json({ registry: REGISTRY, providerId: PID, services: [] });
  }
  return null;
}

/** `GET /v1/admin/providers` returning one provider with the given approvals shape. */
function listReturning(approvals: unknown, standing = "active") {
  return vi.fn(async (input: RequestInfo | URL) =>
    journeyStub(String(input)) ??
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
  return vi.fn(async (input: RequestInfo | URL) =>
    journeyStub(String(input)) ??
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
    const stub = journeyStub(url);
    if (stub) return stub;
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
    vi.stubGlobal("fetch", async (input: RequestInfo | URL) =>
      journeyStub(String(input)) ??
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

// ---------------------------------------------------------------------------------------------
// The services panel: the rest of the journey, rendered.
//
// The first case here is RE-HOMED from the deleted whitelist console's page test, which was the
// page-level catcher for `AddressRef`'s inert copy affordance. `addressRefCopy.test.tsx` pins the
// component in isolation; this pins that a real page mounting it still gets that treatment.
// ---------------------------------------------------------------------------------------------

/** A `fetch` that answers the list plus one attached service in the given shape. */
function withService(service: Record<string, unknown>) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    const json = (body: unknown) =>
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    if (url.includes("/verifier-capabilities")) return json({ registry: REGISTRY, purposes: [] });
    if (url.includes("/resolvers")) return json({ registry: REGISTRY, kinds: [] });
    if (url.includes("/services")) {
      return json({ registry: REGISTRY, providerId: PID, services: [service] });
    }
    if (url.includes("/issuance-capability") || url.includes("/standing")) {
      return json({
        outcome: "executed",
        executed: true,
        warning: null,
        actions: [
          {
            disposition: "executed",
            txHash: `0x${"e1".repeat(32)}`,
            holder: `0x${"ad".repeat(20)}`,
            summary: "registrar action",
          },
        ],
      });
    }
    return json(listBody([oneProvider()]));
  });
}

const SERVICE_ADDR = `0x${"5a".repeat(20)}`;

function serviceView(over: Record<string, unknown> = {}) {
  return {
    service: {
      serviceAddress: SERVICE_ADDR,
      providerId: PID,
      factoryGeneration: `0x${"df".repeat(32)}`,
      recordTypeKey: `0x${"65".repeat(32)}`,
      recordType: "VACCINATION",
      confirmedOwner: CONTROLLER,
      domainResolver: `0x${"0".repeat(40)}`,
      ownerEpoch: 1,
      standing: "pending",
      attached: true,
    },
    effective: {
      providerStanding: "active",
      serviceStanding: "pending",
      factoryActive: true,
      ownerConfirmed: true,
      hasActiveIssuer: false,
    },
    currentPointer: { state: "resolved", service: `0x${"0".repeat(40)}`, isCurrent: false },
    issuance: { state: "resolved", entries: [] },
    ...over,
  };
}

async function expandServices() {
  const toggle = container.querySelector<HTMLButtonElement>('[data-testid="toggle-services"]');
  toggle!.click();
  await settle();
}

describe("Providers - the services panel", () => {
  /**
   * RE-HOMED: an address the page cannot link must still hand over its FULL value, because
   * `shortAddr` removes the elided characters from the DOM entirely - so on the inert branch a copy
   * affordance is not a nicety, it is the only route to the value.
   */
  it("hands over a controller address it cannot link, without a hover", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    const copy = [...container.querySelectorAll("button")].find((b) =>
      (b.getAttribute("aria-label") ?? "").includes("Copy"),
    );
    expect(copy, "the row must offer a copy affordance").toBeDefined();
    expect(container.textContent).not.toContain(CONTROLLER.slice(2, 30));
  });

  /**
   * Attaching lands the service at PENDING and `canIssue` folds it, so the panel must say what is
   * still missing rather than reading as done.
   */
  it("says what is blocking a freshly attached service rather than reporting it as ready", async () => {
    vi.stubGlobal("fetch", withService(serviceView()));
    await mount();
    await expandServices();
    const row = container.querySelector('[data-testid="service-row"]');
    expect(row, "the service must render").not.toBeNull();
    expect(row!.textContent).toContain("VACCINATION");
    expect(container.querySelector('[data-testid="service-blocker"]')!.textContent).toContain(
      "standing to Active",
    );
    // The five terms are reported APART, so the two that fail are visible individually.
    expect(container.querySelector('[data-testid="service-terms"]')!.textContent).toContain(
      "Service active",
    );
  });

  /**
   * The whole reason the panel exists: an admin who cannot see what a provider has attached will
   * attach a duplicate. A read that FAILED must say so rather than showing an empty list.
   */
  it("renders an unreadable services read as its own state, never as nothing attached", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        const json = (body: unknown, status = 200) =>
          new Response(JSON.stringify(body), {
            status,
            headers: { "content-type": "application/json" },
          });
        if (url.includes("/verifier-capabilities")) return json({ registry: REGISTRY, purposes: [] });
        if (url.includes("/resolvers")) return json({ registry: REGISTRY, kinds: [] });
        if (url.includes("/services")) return json({ error: "eth_call failed" }, 502);
        return json(listBody([oneProvider()]));
      }),
    );
    await mount();
    await expandServices();
    expect(container.textContent).toContain("services could not be read");
    expect(container.textContent).not.toContain("Nothing attached yet");
  });

  /**
   * A capability log that could not be read is its own state - never an empty holder set, which
   * would say nobody may issue on the strength of a read that never happened.
   */
  it("renders an unreadable issuance log as could-not-be-read, not as nobody", async () => {
    vi.stubGlobal(
      "fetch",
      withService(
        serviceView({ issuance: { state: "unavailable", reason: "the log range was capped" } }),
      ),
    );
    await mount();
    await expandServices();
    const row = container.querySelector('[data-testid="service-row"]');
    expect(row!.textContent).toContain("Could not be read");
    expect(row!.textContent).toContain("the log range was capped");
    expect(row!.textContent).not.toContain("Nobody yet");
  });

  /**
   * The current pointer is the PROVIDER's own decision and no registrar route writes it, so the
   * panel reports it and says who has to act - rather than offering a button the registrar cannot
   * honour.
   */
  it("names the provider as the one who publishes the current pointer", async () => {
    vi.stubGlobal("fetch", withService(serviceView()));
    await mount();
    await expandServices();
    const pointer = container.querySelector('[data-testid="service-pointer"]');
    expect(pointer!.textContent).toContain("the provider selects this itself");
  });

  /** The two orthogonal levers are rendered OUTSIDE the provider table, because neither is keyed by
   * a provider: a purpose-keyed verify grant and a fleet-wide resolver approval. */
  it("renders the verify and resolver levers outside the provider rows", async () => {
    vi.stubGlobal("fetch", listReturning({ state: "resolved", entries: [] }));
    await mount();
    const verify = container.querySelector('[data-testid="verifier-capabilities"]');
    const resolvers = container.querySelector('[data-testid="resolvers"]');
    expect(verify, "the verify axis must render").not.toBeNull();
    expect(resolvers, "the resolver allowlist must render").not.toBeNull();
    expect(verify!.textContent).toContain("separate from issuance");
    expect(resolvers!.textContent).toContain("the provider selects it");
    // Neither may be nested inside the provider table, which would misstate what it applies to.
    expect(verify!.closest("table"), "verify must not live inside a provider row").toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------
// The capability dialog.
//
// These three writes previously read their address from `window.prompt` and sent it straight
// through: no review, no direction, and unstubbable in jsdom - which is exactly why they had no
// coverage at all. The worst of the three is `setIssuanceCapability`, which names a key allowed to
// SIGN credentials.
// ---------------------------------------------------------------------------------------------

/**
 * Open the issuance-capability dialog on the one attached service.
 *
 * The dialog renders through a PORTAL, so everything it contains lives under `document.body` rather
 * than inside `container` - the same reason the registration-dialog helpers above are
 * document-scoped.
 */
async function openIssuanceDialog() {
  await expandServices();
  buttonWithText("Issuance capability")!.click();
  await settle();
}

function capabilitySubmit() {
  return document.body.querySelector<HTMLButtonElement>('[data-testid="capability-submit"]');
}

function directionButtons() {
  return [
    ...document.body.querySelectorAll<HTMLButtonElement>(
      '[data-testid="capability-direction"] button',
    ),
  ];
}

function lastPost(fetchMock: { mock: { calls: unknown[][] } }) {
  const post = fetchMock.mock.calls.find(
    ([, init]) => (init as RequestInit | undefined)?.method === "POST",
  );
  if (!post) throw new Error("no write was sent");
  return { url: String(post[0]), body: JSON.parse(String((post[1] as RequestInit).body)) };
}

describe("Providers - the capability dialog", () => {
  it("will not send until a well-formed address has been entered", async () => {
    vi.stubGlobal("fetch", withService(serviceView()));
    await mount();
    await openIssuanceDialog();
    expect(capabilitySubmit(), "the dialog must render").not.toBeNull();
    expect(capabilitySubmit()!.disabled, "nothing typed yet").toBe(true);

    type(byId("cap-addr"), "0xnope");
    await settle();
    expect(capabilitySubmit()!.disabled, "a malformed address must not be sendable").toBe(true);
    expect(document.body.textContent).toContain("Not a 0x-prefixed 20-byte address");
  });

  it("sends the address that was entered, granting by default", async () => {
    const fetchMock = withService(serviceView());
    vi.stubGlobal("fetch", fetchMock);
    await mount();
    await openIssuanceDialog();
    const signer = `0x${"c3".repeat(20)}`;
    type(byId("cap-addr"), signer);
    await settle();
    capabilitySubmit()!.click();
    await settle();

    // The ADDRESS is in the PATH and no service appears anywhere, because `setRights` takes neither
    // a service nor a signer field. Asserted on the URL as well as the body: a page that kept posting
    // to a per-service path would still send a body this shape.
    const { url, body } = lastPost(fetchMock);
    expect(url).toContain(`/v1/admin/rights/${signer}/issue`);
    expect(url).not.toContain("/services/");
    expect(body).toEqual({ allowed: true });
  });

  /**
   * The control says "Grant / withdraw" and every one of these is a two-way lever on the contract -
   * the panels even render a withdrawn entry struck through. A dialog that could only grant would
   * imply a state it gives no way to reach.
   */
  it("can withdraw as well as grant, which is what its label promises", async () => {
    const fetchMock = withService(serviceView());
    vi.stubGlobal("fetch", fetchMock);
    await mount();
    await openIssuanceDialog();
    type(byId("cap-addr"), `0x${"c3".repeat(20)}`);
    await settle();
    const withdraw = directionButtons().find((b) => b.textContent?.trim() === "Withdraw");
    expect(withdraw, "a withdraw direction must be offered").toBeDefined();
    withdraw!.click();
    await settle();
    capabilitySubmit()!.click();
    await settle();
    expect(lastPost(fetchMock).body.allowed).toBe(false);
  });

  /** The resolver lever names its two directions in the registry's own words. */
  it("labels the resolver directions approve and pull", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = String(input);
        const json = (b: unknown) =>
          new Response(JSON.stringify(b), {
            status: 200,
            headers: { "content-type": "application/json" },
          });
        if (url.includes("/verifier-capabilities")) return json({ registry: REGISTRY, purposes: [] });
        if (url.includes("/resolvers")) {
          return json({
            registry: REGISTRY,
            kinds: [
              { kind: "directory", listing: { state: "resolved", resolvers: [] } },
              { kind: "domain", listing: { state: "resolved", resolvers: [] } },
            ],
          });
        }
        if (url.includes("/services")) {
          return json({ registry: REGISTRY, providerId: PID, services: [] });
        }
        return json(listBody([oneProvider()]));
      }),
    );
    await mount();
    buttonWithText("Approve / pull")!.click();
    await settle();
    expect(directionButtons().map((b) => b.textContent?.trim())).toEqual(["Approve", "Pull"]);
  });
});
