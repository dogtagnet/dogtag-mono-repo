import { test, expect, type Locator, type Page } from "@playwright/test";

/**
 * End-to-end re-test of the per-role DB / records-management layer, against a LIVE government portal
 * backed by its own store (demo `GOV_DEMO_MODE` = MemChain + MemStore, or a real ROAX + Mongo deploy):
 *
 *   issue → the credential is persisted with its on-chain proof (tx hash + block + a working
 *   `https://explorer.roax.net/tx/<hash>` link) → EDIT updates off-chain metadata while the on-chain
 *   proof is untouched → REVOKE is a soft-invalidation: the record STAYS listed as `revoked`, keeps
 *   its original explorer link, gains a revoke-tx link, and is still verifiable on-chain.
 *
 * Intentionally NOT in `pnpm test` / CI (needs a live portal + browsers) — the captain-facing harness.
 */

/** Issue a TRAVEL_CLEARANCE with a unique dogTagId so its record row is findable, return the id. */
async function issue(page: Page, dogTagId: string) {
  await page.goto("/issue");
  await page.getByTestId("record-type").selectOption("TRAVEL_CLEARANCE");
  await page.locator("input").first().fill(dogTagId);
  await page.getByTestId("issue-submit").click();
  await expect(page.getByTestId("wrapped-doc")).toBeVisible({ timeout: 45_000 });
}

/** The records row whose dog-tag cell equals `dogTagId`. */
function rowFor(page: Page, dogTagId: string): Locator {
  return page
    .getByTestId("record-row")
    .filter({ has: page.locator("td.mono", { hasText: new RegExp(`^${dogTagId}$`) }) });
}

test("issue → persisted with on-chain proof → edit (off-chain only) → revoke keeps history", async ({
  page,
}) => {
  const dogTagId = `9${Date.now() % 100000}`;
  await issue(page, dogTagId);

  await page.goto("/records");
  await page.getByTestId("records-refresh").click();
  const row = rowFor(page, dogTagId);
  await expect(row).toHaveCount(1, { timeout: 20_000 });

  // 1) issued + on-chain proof: a working explorer link to https://explorer.roax.net/tx/<hash>.
  await expect(row.getByTestId("record-status")).toHaveText("issued");
  const link = row.getByTestId("explorer-link");
  const href = await link.getAttribute("href");
  expect(href).toMatch(/^https:\/\/explorer\.roax\.net\/tx\/0x[0-9a-fA-F]+$/);

  // 2) edit OFF-CHAIN metadata — the label persists; the on-chain explorer link is untouched.
  await row.getByTestId("edit-open").click();
  await page.getByTestId("edit-label").fill("case-A / priority");
  await page.getByTestId("edit-save").click();
  await expect(rowFor(page, dogTagId)).toContainText("case-A / priority", { timeout: 20_000 });
  // still issued, same explorer link (on-chain proof immutable across the edit).
  const rowAfter = rowFor(page, dogTagId);
  await expect(rowAfter.getByTestId("record-status")).toHaveText("issued");
  expect(await rowAfter.getByTestId("explorer-link").getAttribute("href")).toBe(href);

  // 3) revoke = soft-invalidation: row STAYS, flips to `revoked`, keeps the original explorer link
  //    AND gains a revoke-tx link (still verifiable on-chain).
  page.once("dialog", (d) => void d.accept());
  await rowAfter.getByTestId("revoke").click();
  const revoked = rowFor(page, dogTagId);
  await expect(revoked.getByTestId("record-status")).toHaveText("revoked", { timeout: 20_000 });
  await expect(revoked).toHaveCount(1); // never deleted
  expect(await revoked.getByTestId("explorer-link").getAttribute("href")).toBe(href);
  await expect(revoked.getByTestId("revoke-explorer-link")).toBeVisible();
  const revokeHref = await revoked.getByTestId("revoke-explorer-link").getAttribute("href");
  expect(revokeHref).toMatch(/^https:\/\/explorer\.roax\.net\/tx\/0x[0-9a-fA-F]+$/);
});

test("expire is an off-chain soft state that keeps the record + its proof", async ({ page }) => {
  const dogTagId = `8${Date.now() % 100000}`;
  await issue(page, dogTagId);

  await page.goto("/records");
  await page.getByTestId("records-refresh").click();
  const row = rowFor(page, dogTagId);
  await expect(row).toHaveCount(1, { timeout: 20_000 });
  const href = await row.getByTestId("explorer-link").getAttribute("href");

  page.once("dialog", (d) => void d.accept());
  await row.getByTestId("expire").click();

  const expired = rowFor(page, dogTagId);
  await expect(expired.getByTestId("record-status")).toHaveText("expired", { timeout: 20_000 });
  // record retained, on-chain proof link unchanged.
  await expect(expired).toHaveCount(1);
  expect(await expired.getByTestId("explorer-link").getAttribute("href")).toBe(href);
});
