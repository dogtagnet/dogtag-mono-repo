import { test, expect, type Page, type Route } from "@playwright/test";

/**
 * Records-management E2E for the GROOMER portal (shared `Records` page + `@dogtag/ui` client) against a
 * MOCKED backend. Proves the DB-management layer the task adds: the portal lists records from its
 * backend DB with the on-chain proof (tx hash + block + a `https://explorer.roax.net/tx/<hash>` link),
 * edits ONLY off-chain metadata (the request never carries on-chain-derived fields), and revoke is a
 * soft-invalidation that KEEPS the record visible (as `revoked`) with its explorer link intact.
 */

const OP_TOKEN_KEY = "groomer.opToken";
const TX = "0x1111111111111111111111111111111111111111111111111111111111111111";
const REVOKE_TX = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ISSUER = "0x00000000000000000000000000000000000000bb";

/** On-chain-derived keys the edit request must NEVER carry (they are immutable chain state). */
const IMMUTABLE_KEYS = [
  "tx_hash",
  "txHash",
  "block_number",
  "blockNumber",
  "root",
  "merkleRoot",
  "issuer_addr",
  "issuerAddr",
  "contractAddress",
  "explorer_url",
  "explorerUrl",
];

interface MockState {
  patchBodies: Record<string, unknown>[];
}

/** Install a mock vet backend that persists a single issued record and mutates it on patch/revoke. */
async function mockBackend(page: Page, state: MockState) {
  const record: Record<string, unknown> = {
    record_id: "rec-1",
    record_type: "VACCINATION",
    dog_tag_id: "42",
    root: "0xabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc00",
    issuer_addr: ISSUER,
    status: "issued",
    tx_hash: TX,
    block_number: 1024,
    explorer_url: `https://explorer.roax.net/tx/${TX}`,
    created_at: 1_700_000_000,
    updated_at: 1_700_000_000,
    label: null,
    notes: null,
  };

  // Anchor to the origin's `/api/` prefix (NOT a `**/api/**` glob, which would also swallow the
  // `@dogtag/ui` `src/api/*.ts` module scripts Vite serves and break the app mount).
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace(/^\/api/, "");
    const method = req.method();

    if (path === "/settings/signing-mode" && method === "GET") {
      return route.fulfill({ json: { signingMode: "backend" } });
    }
    if (path === "/records" && method === "GET") {
      return route.fulfill({ json: { records: [record] } });
    }
    if (path === "/records/rec-1" && method === "PATCH") {
      const body = (req.postDataJSON() ?? {}) as Record<string, unknown>;
      state.patchBodies.push(body);
      if ("label" in body) record.label = body.label;
      if ("notes" in body) record.notes = body.notes;
      if (body.status === "expired") record.status = "expired";
      return route.fulfill({ json: record });
    }
    if (path === "/records/rec-1/revoke" && method === "POST") {
      record.status = "revoked";
      record.revoked_tx_hash = REVOKE_TX;
      record.revoked_block_number = 1099;
      record.revoke_explorer_url = `https://explorer.roax.net/tx/${REVOKE_TX}`;
      record.invalidated_at = 1_700_000_500;
      return route.fulfill({
        json: { recordId: "rec-1", status: "revoked", txHash: REVOKE_TX, blockNumber: 1099 },
      });
    }
    // any other backend call the app makes on load — succeed emptily.
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  // seed an operator session so the app renders routes instead of the Login screen, and auto-accept
  // the confirm() dialogs the revoke/expire actions raise.
  await page.addInitScript(
    ([k]) => window.localStorage.setItem(k as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
  page.on("dialog", (d) => void d.accept());
});

test("list shows on-chain proof, edit is off-chain only, revoke keeps history", async ({ page }) => {
  const state: MockState = { patchBodies: [] };
  await mockBackend(page, state);

  await page.goto("/records");
  const row = page.getByTestId("record-row");
  await expect(row).toHaveCount(1);
  await expect(row.getByTestId("record-status")).toHaveText("issued");

  // on-chain proof: a working explorer link to https://explorer.roax.net/tx/<hash>.
  const link = row.getByTestId("explorer-link");
  expect(await link.getAttribute("href")).toBe(`https://explorer.roax.net/tx/${TX}`);

  // EDIT off-chain metadata.
  await row.getByTestId("edit-open").click();
  await page.getByTestId("edit-label").fill("Rex — annual booster");
  await page.getByTestId("edit-save").click();
  await expect(page.getByTestId("record-row")).toContainText("Rex — annual booster");

  // the edit request carried ONLY off-chain fields — never an on-chain-derived one.
  expect(state.patchBodies.length).toBeGreaterThan(0);
  for (const body of state.patchBodies) {
    for (const k of IMMUTABLE_KEYS) {
      expect(body, `edit body must not carry on-chain field '${k}'`).not.toHaveProperty(k);
    }
  }
  // still issued, explorer link unchanged after the edit.
  await expect(page.getByTestId("record-status")).toHaveText("issued");
  expect(await page.getByTestId("explorer-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/tx/${TX}`,
  );

  // REVOKE = soft-invalidation: the row STAYS, flips to `revoked`, keeps its original explorer link,
  // and gains a revoke-tx link (still verifiable on-chain).
  await page.getByTestId("revoke").click();
  await expect(page.getByTestId("record-status")).toHaveText("revoked");
  await expect(page.getByTestId("record-row")).toHaveCount(1);
  expect(await page.getByTestId("explorer-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/tx/${TX}`,
  );
  await expect(page.getByTestId("revoke-explorer-link")).toBeVisible();
  expect(await page.getByTestId("revoke-explorer-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/tx/${REVOKE_TX}`,
  );
});

test("expire is an off-chain soft state that keeps the record + its proof", async ({ page }) => {
  const state: MockState = { patchBodies: [] };
  await mockBackend(page, state);

  await page.goto("/records");
  await expect(page.getByTestId("record-status")).toHaveText("issued");
  await page.getByTestId("expire").click();
  await expect(page.getByTestId("record-status")).toHaveText("expired");
  await expect(page.getByTestId("record-row")).toHaveCount(1);
  expect(await page.getByTestId("explorer-link").getAttribute("href")).toBe(
    `https://explorer.roax.net/tx/${TX}`,
  );
  // the expire request is a plain off-chain status transition.
  expect(state.patchBodies.some((b) => b.status === "expired")).toBeTruthy();
});
