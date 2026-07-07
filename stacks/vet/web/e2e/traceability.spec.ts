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
const TX = "0xaaaa1111";

/** The scoped, already-joined `/trace/activity` payload the backend would return for this operator. */
const ACTIVITY = {
  events: [
    {
      id: "0xaaaa1111:0",
      type: "rootIssued",
      actor: SIGNER,
      clone: CLONE,
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
      id: "0xbbbb2222:0",
      type: "verified",
      actor: SIGNER,
      txHash: "0xbbbb2222",
      txUrl: "https://explorer.roax.net/tx/0xbbbb2222",
      blockNumber: 11,
      blockTimestamp: 1_700_000_100,
      finality: "pending",
      local: null,
    },
  ],
  total: 2,
  inScope: 2,
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
  await expect(rows).toHaveCount(2);

  // reconciliation counts from the scoped feed.
  await expect(page.getByTestId("trace-inscope")).toHaveText("2");
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

test("renders a first-class 'indexer not connected' state on 503", async ({ page }) => {
  await mockBackend(page, { activityStatus: 503 });
  await page.goto("/traceability");
  await expect(page.getByTestId("trace-unavailable")).toBeVisible();
  await expect(page.getByTestId("trace-event-row")).toHaveCount(0);
});
