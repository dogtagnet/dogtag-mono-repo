import { test, expect, type Page, type Route } from "@playwright/test";

/**
 * Oversight console E2E for the GOVERNMENT portal (govarch PR-5) against a MOCKED backend. Proves the
 * UNSCOPED cross-issuer view: the page renders EVERY issuer's on-chain activity, joins the events that
 * are the government's OWN credentials (an "our record" badge) while showing other issuers' events as
 * "other issuer", and falls back to a first-class "indexer not connected" state on a 503. The gov
 * portal has no login gate, so no token seeding is needed.
 */

const GOV_CLONE = "0x1111111111111111111111111111111111111111";
const OTHER_CLONE = "0x2222222222222222222222222222222222222222";
const TX = "0xgovtx1111";

const ACTIVITY = {
  events: [
    {
      id: "0xgovtx1111:0",
      type: "rootIssued",
      clone: GOV_CLONE,
      cloneName: "DogTag Government Authority",
      recordType: "TRAVEL_CLEARANCE",
      root: "0x3333333333333333333333333333333333333333333333333333333333333333",
      txHash: TX,
      txUrl: `https://explorer.roax.net/tx/${TX}`,
      blockNumber: 20,
      blockTimestamp: 1_700_000_000,
      finality: "finalized",
      local: { kind: "issuance", recordType: "TRAVEL_CLEARANCE", dogTagId: "7", receiptId: "RCPT12345", status: "issued" },
    },
    {
      id: "0xother:0",
      type: "rootIssued",
      clone: OTHER_CLONE,
      recordType: "VACCINATION",
      root: "0x9999999999999999999999999999999999999999999999999999999999999999",
      txHash: "0xothertx",
      txUrl: "https://explorer.roax.net/tx/0xothertx",
      blockNumber: 21,
      blockTimestamp: 1_700_000_100,
      finality: "finalized",
      local: null,
    },
  ],
  total: 2,
  matched: 1,
  scope: { label: "government-oversight", unscoped: true },
};

const STATS = { rootIssued: 2, rootRevoked: 0, verifications: 1, clones: 2, local: { credentials: 1, verifications: 0 } };
const HEALTH = { status: "ok", chainId: 135, demo: true, canSign: true, signer: "0x00000000000000000000000000000000000000a1" };

async function mockBackend(page: Page, opts: { activityStatus?: number } = {}) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/api/, "");
    if (path === "/health") {
      return route.fulfill({ json: HEALTH });
    }
    if (path === "/v1/oversight/activity") {
      if (opts.activityStatus === 503) {
        return route.fulfill({
          status: 503,
          json: { error: "oversight indexer not configured (set INDEXER_API_BASE)", indexer: "not-configured" },
        });
      }
      return route.fulfill({ json: ACTIVITY });
    }
    if (path === "/v1/oversight/stats") {
      return route.fulfill({ json: STATS });
    }
    return route.fulfill({ json: {} });
  });
}

test("unscoped feed shows all issuers and highlights the government's own credentials", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/oversight");

  const rows = page.getByTestId("oversight-event-row");
  await expect(rows).toHaveCount(2);

  // the government's OWN issuance is joined + flagged.
  const own = rows.filter({ has: page.getByTestId("oversight-local-own") });
  await expect(own).toHaveCount(1);
  await expect(own).toHaveAttribute("data-own", "true");
  await expect(own.getByTestId("oversight-local")).toContainText("RCPT12345");

  // the OTHER issuer's event is still shown (unscoped) but marked as not ours.
  const other = rows.filter({ has: page.getByTestId("oversight-local-other") });
  await expect(other).toHaveCount(1);
  await expect(other).toHaveAttribute("data-own", "false");
});

test("renders a first-class 'indexer not connected' state on 503", async ({ page }) => {
  await mockBackend(page, { activityStatus: 503 });
  await page.goto("/oversight");
  await expect(page.getByTestId("oversight-unavailable")).toBeVisible();
  await expect(page.getByTestId("oversight-event-row")).toHaveCount(0);
});
