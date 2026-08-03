import { test, expect, type Page, type Route } from "@playwright/test";

/**
 * Oversight console E2E for the GOVERNMENT portal (govarch PR-5) against a MOCKED backend. Proves the
 * UNSCOPED cross-issuer view: the page renders EVERY issuer's on-chain activity, joins the events that
 * are the government's OWN credentials (an "our record" badge) while showing other issuers' events as
 * "other issuer", and falls back to a first-class "indexer not connected" state on a 503. The gov
 * portal has no login gate, so no token seeding is needed.
 *
 * It also proves the AUDIT-surface contract this console has to meet: chain provenance is stated PER
 * ROW, a real event carries a working explorer link with its time/block/contract populated from the
 * chain data, and a synthetic event (the shape the indexer's `INDEXER_DEMO_MODE` feed actually emits)
 * is visibly marked and offers NO transaction link at all.
 */

const GOV_CLONE = "0x1111111111111111111111111111111111111111";
const OTHER_CLONE = "0x2222222222222222222222222222222222222222";
const REGISTRY = "0x4f5a6b7c8d9e0f1a2b3c4d5e6f70819203040506";
/** A well-formed 32-byte transaction hash, as a real ROAX transaction carries. */
const TX = `0x${"a1".repeat(32)}`;
/** What the demo indexer really emits: too short to be a transaction hash on any chain. */
const DEMO_TX = "0x0800";

const ACTIVITY = {
  events: [
    {
      id: `${TX}:0`,
      type: "rootIssued",
      clone: GOV_CLONE,
      contract: GOV_CLONE,
      cloneName: "DogTag Government Authority",
      // NO event-level recordType: the on-chain RootIssued log carries none, and the indexer sets it
      // only for issuerCreated/whitelisted/delisted. It reaches the row through the local join below,
      // exactly as it does against a real chain - a fixture richer than reality hides defects.
      root: "0x3333333333333333333333333333333333333333333333333333333333333333",
      actor: "0x119f8c7f6d7ec10e7376983739c6f46cf9cc3e96",
      actorName: "DogTag Government Authority",
      txHash: TX,
      txUrl: `https://explorer.roax.net/tx/${TX}`,
      blockHash: `0x${"b2".repeat(32)}`,
      blockNumber: 20,
      logIndex: 0,
      blockTimestamp: 1_700_000_000,
      onchainTs: 1_700_000_000,
      finality: "finalized",
      local: { kind: "issuance", recordType: "TRAVEL_CLEARANCE", dogTagId: "7", receiptId: "RCPT12345", status: "issued" },
    },
    {
      id: `${DEMO_TX}:0`,
      type: "whitelisted",
      clone: OTHER_CLONE,
      // The registry emits `whitelisted`, not the clone — "which contract" is not "which issuer".
      contract: REGISTRY,
      // A hashed record-type key, not a human label: it MUST be rendered with a word saying so.
      recordType: "0x0ea3b61f198af15d1c1f1cd1bd926f52cb69cde62893f72fbb94e628c820321d",
      txHash: DEMO_TX,
      // The indexer composes txUrl unconditionally, so a demo row arrives carrying a live-looking
      // explorer URL for a transaction that cannot exist. The UI must refuse to render it.
      txUrl: `https://explorer.roax.net/tx/${DEMO_TX}`,
      blockNumber: 8,
      blockTimestamp: 1_700_000_100,
      finality: "pending",
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

test("a real event is fully traceable: time, block, contract and a working explorer link", async ({
  page,
}) => {
  await mockBackend(page);
  await page.goto("/oversight");

  const real = page.getByTestId("oversight-event-row").filter({ hasText: "Root issued" });
  await expect(real).toHaveAttribute("data-provenance", "onchain");

  // WHEN — the absolute on-chain time is present, not just a relative hint.
  await expect(real.getByTestId("oversight-when")).toContainText("2023");

  // TX — links to the explorer, using the txUrl the API supplied.
  const txLink = real.getByTestId("oversight-tx-link");
  expect(await txLink.getAttribute("href")).toBe(`https://explorer.roax.net/tx/${TX}`);

  // CONTRACT — the emitting contract, named by role and linked to the explorer.
  const contract = real.getByTestId("oversight-contract");
  await expect(contract).toContainText("issuer clone");
  expect(await contract.getByTestId("oversight-contract-value-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/address/${GOV_CLONE}`,
  );

  // BLOCK + finality.
  await expect(real).toContainText("#20");
  await expect(real).toContainText("finalized");

  // DETAILS — the record type comes from this authority's own root-joined record, since the chain
  // event carries none. Without it the same event reads differently here than in the vet console.
  const details = real.getByTestId("oversight-details");
  await expect(details).toContainText("record type");
  await expect(details).toContainText("TRAVEL_CLEARANCE");

  // The row expands to the full, untruncated provenance — the values are rendered COMPLETE, not
  // merely widened, so the auditor can read them without relying on the clipboard.
  await real.getByTestId("oversight-expand").click();
  const detail = page.getByTestId("oversight-event-detail");
  await expect(detail).toContainText("log 0");
  await expect(detail.getByTestId("oversight-detail-tx")).toHaveText(new RegExp(TX));
  await expect(detail.getByTestId("oversight-detail-contract")).toContainText(GOV_CLONE);
  await expect(detail.getByTestId("oversight-detail-blockhash")).toContainText(`0x${"b2".repeat(32)}`);
  await expect(detail.getByTestId("oversight-synthetic-note")).toHaveCount(0);
});

test("a synthetic event is marked as such and never claims a transaction", async ({ page }) => {
  await mockBackend(page);
  await page.goto("/oversight");

  const demo = page.getByTestId("oversight-event-row").filter({ hasText: "Whitelisted" });
  await expect(demo).toHaveAttribute("data-provenance", "synthetic");
  await expect(demo.getByTestId("provenance-synthetic")).toBeVisible();

  // No explorer link at all — the tx hash renders inert, despite the API supplying a txUrl.
  await expect(demo.getByTestId("oversight-tx-link")).toHaveCount(0);
  await expect(demo.getByTestId("oversight-tx-link-inert")).toBeVisible();

  // The feed-level count reconciles with the per-row marks.
  await expect(page.getByTestId("oversight-synthetic-banner")).toContainText("1 of 2");

  // The hashed record-type key is LABELLED, never a bare 32-byte hex.
  await expect(demo.getByTestId("oversight-details")).toContainText("record type key");

  await demo.getByTestId("oversight-expand").click();
  await expect(page.getByTestId("oversight-synthetic-note")).toBeVisible();
});

test("renders a first-class 'indexer not connected' state on 503", async ({ page }) => {
  await mockBackend(page, { activityStatus: 503 });
  await page.goto("/oversight");
  await expect(page.getByTestId("oversight-unavailable")).toBeVisible();
  await expect(page.getByTestId("oversight-event-row")).toHaveCount(0);
});
