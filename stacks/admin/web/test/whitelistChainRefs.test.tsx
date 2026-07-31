// @vitest-environment jsdom
//
// The Whitelist console's on-chain references, rendered for real.
//
// `packages/ui` already tests `txExplorerHref`/`addressChainRef`, and those tests were green for the
// entire life of the defect this file exists to pin: the page never called them. It wrote
// `href={explorerAddressUrl(e.address)}` and `href={explorerTxUrl(d.txHash)}` directly, so every row
// and every dispatch result offered the operator an explorer link whether or not anything was there to
// look up. A test of the helpers cannot see that. Only rendering the page can.
//
// So the page is mounted for real - the actual `<Whitelist />`, the actual `AppProvider`, the actual
// central client - with ONLY `fetch` substituted, in the same spirit as
// `packages/ui/test/rpcSettingsVerdict.test.ts`. Nothing here re-implements the link rule; each case
// asks the DOM what an operator would actually be shown.
//
// The three states, which must stay VISIBLY distinct and none of which may be a link:
//   on chain      - a well-formed value, an anchor carrying the explorer href.
//   not on chain  - nothing was broadcast (a `proposed` disposition). Stated in words, no anchor.
//   cannot tell   - a value IS present but cannot be looked up. Stated with its reason, no anchor.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { ToastProvider, type GovernanceDisposition } from "@dogtag/ui";
import { AppProvider } from "../src/app/AppContext";
import { DispositionRow, Whitelist } from "../src/pages/Whitelist";

const GOOD_ADDRESS = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9F";
/** Present, and unusable: 39 hex digits, so no explorer page can exist for it. */
const BAD_ADDRESS = "0xB5D6654d8B29096C8fcf71d24bbe6f6de86c5F9";
const GOOD_TX = `0x${"a1".repeat(32)}`;
/** The shape a scripted/demo feed produces - well-formed-looking, addresses nothing. */
const BAD_TX = "0x0800";

let container: HTMLDivElement;
let root: Root;

/** Let React commit and any load effect's promise settle, on the real scheduler (no `act()`). */
async function settle(turns = 6) {
  for (let i = 0; i < turns; i++) await new Promise((r) => setTimeout(r, 0));
}

async function mount(node: Parameters<Root["render"]>[0]) {
  root.render(node);
  await settle();
}

/** The page under its real providers - only the network is a stand-in. */
function page() {
  return createElement(ToastProvider, null, createElement(AppProvider, null, createElement(Whitelist)));
}

/** One application, so the table renders exactly the (recordType, address) rows we asked for. */
function applicationsReturning(addresses: string[]) {
  return vi.fn(async () =>
    new Response(
      JSON.stringify({
        applications: [
          {
            applicationId: "app-1",
            issuerEntityId: "ent-1",
            addresses,
            recordTypes: ["VACCINATION"],
            domain: "vet.example",
            status: "approved",
          },
        ],
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    ),
  );
}

beforeEach(() => {
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  root.unmount();
  container.remove();
  vi.unstubAllGlobals();
});

describe("Whitelist — the address column", () => {
  it("links an address that can be looked up", async () => {
    vi.stubGlobal("fetch", applicationsReturning([GOOD_ADDRESS]));
    await mount(page());

    const link = container.querySelector<HTMLAnchorElement>('[data-testid="whitelist-address"]');
    expect(link).not.toBeNull();
    expect(link!.tagName).toBe("A");
    expect(link!.getAttribute("href")).toContain(GOOD_ADDRESS);
    // The inert form must be absent, not merely unlinked: two elements for one value would mean the
    // row is making both claims at once.
    expect(container.querySelector('[data-testid="whitelist-address-inert"]')).toBeNull();
  });

  it("does not link an address it cannot look up, and says why", async () => {
    vi.stubGlobal("fetch", applicationsReturning([BAD_ADDRESS]));
    await mount(page());

    // The load-bearing assertion: NO anchor anywhere carries this value. A link to nowhere reads to
    // the operator as evidence the address exists on chain.
    const anchors = [...container.querySelectorAll("a")].map((a) => a.getAttribute("href") ?? "");
    expect(anchors.some((h) => h.includes(BAD_ADDRESS))).toBe(false);

    const inert = container.querySelector('[data-testid="whitelist-address-inert"]');
    expect(inert).not.toBeNull();
    expect(inert!.tagName).not.toBe("A");
    // Withholding a link is itself a claim, so it has to carry its reason - otherwise it is
    // indistinguishable from a page that simply forgot to link it.
    expect(inert!.getAttribute("data-reason")).toBe(
      "not a well-formed 20-byte address, so it has no explorer page",
    );
    // The value is still shown: it is what the backend reported, and hiding it costs the operator the
    // one fact they need to go and investigate.
    expect(inert!.textContent).toBeTruthy();
    // ...and shown TRUNCATED, with no `href` to copy the whole thing out of, so the row has to offer
    // the affordance itself or the address is reachable only by hovering. Scoped to this row rather
    // than the page, so it cannot be satisfied by a copy button belonging to some other cell - the
    // wrong-column mistake `ActorCell`'s split testIds exist to prevent.
    // `addressRefCopy.test.tsx` pins that the button writes the FULL value; this one pins that the
    // page actually renders it.
    expect(inert!.parentElement!.querySelector('[data-testid="copy-button"]')).not.toBeNull();
  });

  it("keeps the two states distinguishable in one table", async () => {
    vi.stubGlobal("fetch", applicationsReturning([GOOD_ADDRESS, BAD_ADDRESS]));
    await mount(page());

    // A per-row rule, not a per-page one: one unusable address among good ones must be visible as
    // such, which a page-level banner could never do.
    expect(container.querySelectorAll('[data-testid="whitelist-address"]')).toHaveLength(1);
    expect(container.querySelectorAll('[data-testid="whitelist-address-inert"]')).toHaveLength(1);
  });
});

describe("Whitelist — a dispatched governance action", () => {
  const executed = (txHash: string): GovernanceDisposition =>
    ({ disposition: "executed", txHash, holder: GOOD_ADDRESS }) as GovernanceDisposition;
  const proposed = (): GovernanceDisposition =>
    ({
      disposition: "proposed",
      holder: GOOD_ADDRESS,
      hostedSigner: GOOD_ADDRESS,
      target: GOOD_ADDRESS,
      calldata: "0xdeadbeef",
      authority: "role",
    }) as GovernanceDisposition;

  it("ON CHAIN: an executed action links its transaction", async () => {
    await mount(createElement(DispositionRow, { label: "whitelistFor", d: executed(GOOD_TX) }));

    const link = container.querySelector<HTMLAnchorElement>('[data-testid="disposition-tx"]');
    expect(link).not.toBeNull();
    expect(link!.tagName).toBe("A");
    expect(link!.getAttribute("href")).toContain(GOOD_TX);
    expect(container.querySelector('[data-testid="disposition-tx-unverifiable"]')).toBeNull();
  });

  it("NOT ON CHAIN: a proposed action offers no link and says nothing was broadcast", async () => {
    await mount(createElement(DispositionRow, { label: "whitelistFor", d: proposed() }));

    // `proposed` means the hosted key lacked the authority, so calldata came back for the governance
    // signer and NOTHING reached the chain. There is no transaction, so there must be no anchor.
    expect(container.querySelectorAll("a")).toHaveLength(0);
    const stated = container.querySelector('[data-testid="disposition-not-broadcast"]');
    expect(stated).not.toBeNull();
    expect(stated!.textContent).toMatch(/not broadcast/i);
    // Silence is the failure mode being closed here: an operator returning to this trail must be able
    // to tell "nothing was sent" from "sent, link missing".
    expect(stated!.textContent).toMatch(/no transaction exists to link/i);
  });

  it("CANNOT TELL: an executed action whose hash cannot be looked up states the contradiction", async () => {
    await mount(createElement(DispositionRow, { label: "whitelistFor", d: executed(BAD_TX) }));

    const anchors = [...container.querySelectorAll("a")].map((a) => a.getAttribute("href") ?? "");
    expect(anchors.some((h) => h.includes(BAD_TX))).toBe(false);

    // Neither neighbour is correct here: we may not claim it landed (nothing can be checked) nor that
    // it did not (the backend says it broadcast). The row says exactly that.
    const warning = container.querySelector('[data-testid="disposition-tx-unverifiable"]');
    expect(warning).not.toBeNull();
    expect(warning!.textContent).toMatch(/reported as executed/i);
    expect(warning!.textContent).toMatch(/no usable transaction hash/i);

    // The hash is still rendered, inert, so the operator can copy it into an incident report.
    const inert = container.querySelector('[data-testid="disposition-tx-inert"]');
    expect(inert).not.toBeNull();
    expect(inert!.tagName).not.toBe("A");
  });

  it("an executed action that came back with no hash at all is not silently blank", async () => {
    await mount(createElement(DispositionRow, { label: "whitelistFor", d: executed("") }));

    expect(container.querySelectorAll("a")).toHaveLength(0);
    // Empty and malformed are the same situation for the reader - nothing to look up - so they get the
    // same statement rather than one of them rendering as an unexplained gap.
    expect(
      container.querySelector('[data-testid="disposition-tx-unverifiable"]')!.textContent,
    ).toMatch(/no usable transaction hash/i);
  });
});
