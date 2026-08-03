import { test, expect, type Page, type Route } from "@playwright/test";

/**
 * Traceability portal E2E for the VET portal (govarch PR-5) against a MOCKED backend. Proves the
 * scoped-view + DB-join UI: the page lists THIS operator's own on-chain activity (as the scoped
 * `/trace/activity` feed returns it), joins each event to its own DB record (a matched "record" badge
 * vs an "on-chain only" event), surfaces the scope/reconciliation counts, and renders a first-class
 * "indexer not connected" state on a 503.
 */

const OP_TOKEN_KEY = "vet.opToken";
const SIGNER = "0x00000000000000000000000000000000516ea001";
const CLONE = "0x000000000000000000000000000000000c10e0a1";
/** A well-formed 32-byte transaction hash — only these are chain-addressable. */
const TX = `0x${"aa".repeat(32)}`;
const TX2 = `0x${"bb".repeat(32)}`;

/** The scoped, already-joined `/trace/activity` payload the backend would return for this operator. */
const ACTIVITY = {
  events: [
    {
      id: `${TX}:0`,
      type: "rootIssued",
      actor: SIGNER,
      clone: CLONE,
      contract: CLONE,
      recordType: "VACCINATION",
      root: "0x1111111111111111111111111111111111111111111111111111111111111111",
      txHash: TX,
      txUrl: `https://explorer.roax.net/tx/${TX}`,
      blockNumber: 10,
      blockTimestamp: 1_700_000_000,
      finality: "finalized",
      local: { kind: "issuance", recordId: "rec-7", dogTagId: "7", status: "issued", label: "Rex annual" },
    },
    {
      id: `${TX2}:0`,
      type: "verified",
      actor: SIGNER,
      txHash: TX2,
      txUrl: `https://explorer.roax.net/tx/${TX2}`,
      blockNumber: 11,
      blockTimestamp: 1_700_000_100,
      finality: "pending",
      local: null,
    },
    {
      // A scripted/demo indexer row: the hash is too short to be a transaction hash on any chain,
      // yet the indexer still composed a live-looking txUrl for it. The UI must refuse to link it.
      id: "0x0800:0",
      type: "whitelisted",
      actor: SIGNER,
      contract: "0x4f5a6b7c8d9e0f1a2b3c4d5e6f70819203040506",
      recordType: "0x0ea3b61f198af15d1c1f1cd1bd926f52cb69cde62893f72fbb94e628c820321d",
      txHash: "0x0800",
      txUrl: "https://explorer.roax.net/tx/0x0800",
      blockNumber: 8,
      blockTimestamp: 1_700_000_200,
      finality: "pending",
      local: null,
    },
  ],
  total: 3,
  inScope: 3,
  matched: 1,
  droppedOutOfScope: 0,
  scope: { label: "Seaport Vet", unscoped: false },
  localScope: { signers: 1, clones: 1 },
};

const STATS = { rootIssued: 1, rootRevoked: 0, verifications: 1, local: { records: 1, verifications: 1 } };

async function mockBackend(page: Page, opts: { activityStatus?: number } = {}) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/api/, "");
    if (path === "/settings/signing-mode") {
      return route.fulfill({ json: { signingMode: "backend" } });
    }
    if (path === "/trace/activity") {
      if (opts.activityStatus === 503) {
        return route.fulfill({
          status: 503,
          json: { error: "oversight indexer not configured (set INDEXER_API_BASE)", indexer: "not-configured" },
        });
      }
      return route.fulfill({ json: ACTIVITY });
    }
    if (path === "/trace/stats") {
      return route.fulfill({ json: STATS });
    }
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([k]) => window.localStorage.setItem(k as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

test("scoped feed joins own record and marks on-chain-only events", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/traceability");

  const rows = page.getByTestId("trace-event-row");
  await expect(rows).toHaveCount(3);

  // reconciliation counts from the scoped feed.
  await expect(page.getByTestId("trace-inscope")).toHaveText("3");
  await expect(page.getByTestId("trace-matched")).toHaveText("1");

  // the rootIssued event is JOINED to this operator's own DB record.
  const issued = rows.filter({ has: page.getByText("Root issued") });
  await expect(issued.getByTestId("trace-local-matched")).toBeVisible();
  await expect(issued.getByTestId("trace-local")).toContainText("rec-7");
  // its explorer link is preserved.
  expect(await issued.getByTestId("trace-tx-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/tx/${TX}`,
  );

  // the verification event has no local record → shown as on-chain only.
  const verified = rows.filter({ has: page.getByText("Verified") });
  await expect(verified.getByTestId("trace-local-none")).toBeVisible();
});

test("a synthetic event is marked and never claims a transaction", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/traceability");

  const rows = page.getByTestId("trace-event-row");
  const real = rows.filter({ hasText: "Root issued" });
  await expect(real).toHaveAttribute("data-provenance", "onchain");
  // The emitting contract is named by role and linked.
  expect(await real.getByTestId("trace-contract-value-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/address/${CLONE}`,
  );

  const demo = rows.filter({ hasText: "Whitelisted" });
  await expect(demo).toHaveAttribute("data-provenance", "synthetic");
  await expect(demo.getByTestId("provenance-synthetic")).toBeVisible();
  // No explorer anchor at all, despite the feed supplying a txUrl.
  await expect(demo.getByTestId("trace-tx-link")).toHaveCount(0);
  await expect(demo.getByTestId("trace-tx-link-inert")).toBeVisible();
  // The hashed record-type key is LABELLED, never bare 32-byte hex.
  await expect(demo.getByTestId("trace-details")).toContainText("record type key");

  await expect(page.getByTestId("trace-synthetic-banner")).toContainText("1 of 3");
});

test("renders a first-class 'indexer not connected' state on 503", async ({ page }) => {
  await mockBackend(page, { activityStatus: 503 });
  await page.goto("/traceability");
  await expect(page.getByTestId("trace-unavailable")).toBeVisible();
  await expect(page.getByTestId("trace-event-row")).toHaveCount(0);
});
