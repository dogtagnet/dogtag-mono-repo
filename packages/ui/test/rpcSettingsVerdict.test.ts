// @vitest-environment jsdom
//
// The endpoint settings hook, rendered for real, on React's own scheduler.
//
// Everything else about the RPC preference is pure and lives in rpcEndpoint.test.ts. This file
// exists because one defect in this hook was not reachable from pure code at all: the verdict was
// discarded by React's effect scheduling, so a save that actually changed the endpoint rendered no
// message whatsoever. A settings screen that saves and then says nothing is indistinguishable from
// one that silently failed to save - the same could-not-tell-what-happened defect this repo has
// removed from every other surface.
//
// The hook is mounted in a real DOM against the real `window.localStorage` and the real
// `validateAndSaveRoaxRpcPreference`. Only the network peer is substituted, because a test may not
// depend on a live node. A pure re-implementation of the state machine would agree with itself and
// prove nothing about the wiring that actually broke.
//
// The pin is at the hook because both shipped consumers are pass-throughs that already render each
// tone: `RpcEndpointSettingsCard.tsx` renders `message.text` with `role="alert"`/`text-danger` for
// `danger` and `role="status"`/`text-success` otherwise, and `stacks/owner/web/src/pages/Settings.tsx`
// branches the same way. A consumer that drops `message` would defeat these tests one layer out, so
// keep that rendering when either surface is reworked.
//
// DO NOT rewrite this with `act()`, and do not set `IS_REACT_ACT_ENVIRONMENT`.
// `act()` drains React's work into its own queue, which reorders the save's promise continuation
// against the passive effect that was cancelling it - and that reordering HIDES this defect
// completely: an act()-based version of these tests passes with the regression reintroduced. The
// real ordering is the one the browser uses, so these tests await real macrotasks instead. Verified
// by mutation in both styles; see `settle()` below.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import {
  ROAX_RPC_STORAGE_KEY,
  getRoaxRpcPreference,
  setRoaxRpcPreference,
} from "../src/chain/rpcEndpoint";
import { useRoaxRpcSettings } from "../src/chain/useRoaxRpcSettings";

const DEFAULT_URL = "https://bundled.example/rpc";
const CUSTOM_URL = "https://chosen.example/rpc";
const WRONG_CHAIN_URL = "https://mainnet.example/rpc";
const CHAIN_135 = "0x87";
const CHAIN_1 = "0x1";

/**
 * Answers `eth_chainId` per host, so the peer the guard actually contacted is observable and an
 * unlisted host is refused rather than quietly answering.
 */
function chainPeers(
  byUrl: Readonly<Record<string, string>>,
  onProbe?: (url: string) => void,
): typeof fetch {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    const body = JSON.parse(String(init?.body)) as { id: number };
    onProbe?.(url);
    const chainId = byUrl[url];
    if (chainId === undefined) throw new Error("connection refused");
    return new Response(JSON.stringify({ jsonrpc: "2.0", id: body.id, result: chainId }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

/**
 * Let React's real scheduler run.
 *
 * Passive effects are posted through the Scheduler (a MessageChannel macrotask), so awaiting
 * microtasks alone would observe a half-committed render. Several turns are awaited because a
 * settled verdict schedules a further render of its own.
 */
async function settle(turns = 4, ms = 5): Promise<void> {
  for (let turn = 0; turn < turns; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve, ms));
  }
}

/**
 * Poll until a NEW verdict has settled, rather than spending a fixed time budget.
 *
 * A fixed budget would make this suite timing-dependent, and it now gates every change to the repo,
 * so a slow machine would read as this defect returning. Polling removes that: it returns as soon as
 * the verdict lands, and on the regression it simply exhausts the timeout and lets the assertion
 * below report the real symptom (`message: undefined`) instead of an opaque timeout.
 *
 * `previous` is load-bearing. The second save in a test starts with the FIRST save's message still
 * rendered, so waiting only for "some message, not checking" returns instantly on the stale one -
 * which made the rejection case read the earlier success and fail. Each save sets a fresh object, so
 * identity is what distinguishes this verdict from the last.
 */
async function waitForNewVerdict(
  harness: Harness,
  previous: Settings["message"],
  timeoutMs = 2_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (harness.current.message !== previous && !harness.current.checking) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

type Settings = ReturnType<typeof useRoaxRpcSettings>;

interface Harness {
  /** The most recently rendered value - what the operator would be looking at. */
  readonly current: Settings;
  unmount(): void;
}

async function mountSettings(): Promise<Harness> {
  const container = document.createElement("div");
  document.body.appendChild(container);
  let latest: Settings | undefined;

  function Probe() {
    latest = useRoaxRpcSettings(DEFAULT_URL);
    return null;
  }

  const root: Root = createRoot(container);
  root.render(createElement(Probe));
  await settle();

  const harness: Harness = {
    get current(): Settings {
      if (!latest) throw new Error("the settings hook never rendered");
      return latest;
    },
    unmount() {
      root.unmount();
      container.remove();
    },
  };
  mounted.push(harness);
  return harness;
}

/**
 * Type a value into the field and submit it, then let every scheduled effect land.
 *
 * `expectVerdict: false` is for the superseded case, where the claim is an ABSENCE and so cannot be
 * polled for - that one spends a deliberately generous fixed budget instead, long enough that a
 * verdict which was going to appear would have.
 */
async function saveDraft(
  harness: Harness,
  value: string,
  { expectVerdict = true }: { expectVerdict?: boolean } = {},
): Promise<void> {
  harness.current.setDraft(value);
  await settle();
  const previousVerdict = harness.current.message;
  await harness.current.save();
  if (expectVerdict) {
    await waitForNewVerdict(harness, previousVerdict);
  } else {
    await settle(20, 10);
  }
}

const mounted: Harness[] = [];
let originalFetch: typeof fetch;

beforeEach(() => {
  originalFetch = globalThis.fetch;
  window.localStorage.clear();
});

afterEach(() => {
  while (mounted.length > 0) mounted.pop()?.unmount();
  globalThis.fetch = originalFetch;
  window.localStorage.clear();
});

describe("endpoint settings verdicts", () => {
  it("reports success for a save that actually changes the endpoint", async () => {
    globalThis.fetch = chainPeers({ [CUSTOM_URL]: CHAIN_135, [DEFAULT_URL]: CHAIN_135 });
    const harness = await mountSettings();

    // The precondition is the whole point: this save CHANGES the stored preference. A no-op save
    // leaves the preference alone, never re-runs the effect, and so always reported correctly -
    // which is exactly why the defect survived review.
    expect(getRoaxRpcPreference(DEFAULT_URL).isCustom).toBe(false);

    await saveDraft(harness, CUSTOM_URL);

    expect(harness.current.message).toEqual({
      tone: "success",
      text: "Custom endpoint saved and confirmed on ROAX chain 135.",
    });
    expect(harness.current.checking).toBe(false);
    // The verdict describes a change that really happened, not a message rendered over nothing.
    expect(window.localStorage.getItem(ROAX_RPC_STORAGE_KEY)).toBe(CUSTOM_URL);
    expect(harness.current.rpcUrl).toBe(CUSTOM_URL);
  });

  it("reports why a rejected save cleared the custom peer, instead of clearing it in silence", async () => {
    // Start from a working custom peer so the rejection has something real to take away. This is
    // the worst case of the defect: the endpoint the operator was relying on is removed and the
    // screen says nothing at all about it.
    globalThis.fetch = chainPeers({ [CUSTOM_URL]: CHAIN_135, [DEFAULT_URL]: CHAIN_135 });
    const harness = await mountSettings();
    await saveDraft(harness, CUSTOM_URL);
    expect(getRoaxRpcPreference(DEFAULT_URL).isCustom).toBe(true);

    globalThis.fetch = chainPeers({ [WRONG_CHAIN_URL]: CHAIN_1, [DEFAULT_URL]: CHAIN_135 });
    await saveDraft(harness, WRONG_CHAIN_URL);

    const message = harness.current.message;
    expect(message?.tone).toBe("danger");
    // It must name the chain it actually found and say the override is gone. "Something went wrong"
    // is not a verdict an operator can act on.
    expect(message?.text).toContain("chain 1");
    expect(message?.text).toContain("The custom endpoint was removed");
    expect(harness.current.checking).toBe(false);

    // The stated consequence is the one that really happened.
    expect(window.localStorage.getItem(ROAX_RPC_STORAGE_KEY)).toBeNull();
    expect(harness.current.rpcUrl).toBe(DEFAULT_URL);
    expect(harness.current.draft).toBe(DEFAULT_URL);
  });

  it("still lets a genuinely foreign change supersede an in-flight save", async () => {
    // Removing the self-cancel must not have removed the protection against a real competing
    // choice: a stale probe may never overwrite what another tab just committed, and the superseded
    // save must report no verdict, because its answer is no longer about the current preference.
    const otherTabChoice = "https://other-tab.example/rpc";
    let intervened = false;
    globalThis.fetch = chainPeers(
      { [CUSTOM_URL]: CHAIN_135, [DEFAULT_URL]: CHAIN_135, [otherTabChoice]: CHAIN_135 },
      (url) => {
        if (url !== CUSTOM_URL || intervened) return;
        intervened = true;
        // Another tab commits a different endpoint while this probe is still in flight.
        setRoaxRpcPreference(otherTabChoice, DEFAULT_URL, window.localStorage);
      },
    );
    const harness = await mountSettings();

    await saveDraft(harness, CUSTOM_URL, { expectVerdict: false });

    expect(intervened).toBe(true);
    expect(harness.current.message).toBeUndefined();
    // Superseded is not an excuse to strand the spinner: a save that abandons its verdict must still
    // put the control back, or the card sits on "Checking..." forever with no way out.
    expect(harness.current.checking).toBe(false);
    // The other tab's choice stands; the superseded save did not write over it.
    expect(window.localStorage.getItem(ROAX_RPC_STORAGE_KEY)).toBe(otherTabChoice);
    expect(harness.current.rpcUrl).toBe(otherTabChoice);
  });
});
