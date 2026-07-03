import { expect, test } from "@playwright/test";

/**
 * End-to-end for the PR-2 government RECEIPT UI, against a LIVE portal (demo `GOV_DEMO_MODE`
 * MemChain+MemStore, or a real ROAX+Mongo deploy):
 *
 *   issue a TRAVEL_CLEARANCE → open its printable CDC-modeled receipt (Section A/B/C render from the
 *   authenticated custodial record, with a QR + live status chip) → follow the receipt's public,
 *   PII-free `/r/:receiptId` status page and confirm it shows the on-chain verdict with NO Section-A
 *   person PII.
 *
 * Intentionally NOT in `pnpm test` / CI (needs a served portal + browsers) — the captain-facing
 * harness. Run: `GOV_URL=<portal-url> pnpm --filter @dogtag/government-web test:e2e receipt`.
 */
test("issue → CDC receipt view (sections + QR + live status) → public PII-free status page", async ({
  page,
}) => {
  // 1) Issue a travel clearance with recognizable Section A/B/C values.
  await page.goto("/issue");
  await page.getByTestId("record-type").selectOption("TRAVEL_CLEARANCE");
  await page.locator("input").first().fill("7"); // dogTagId (first input)
  await page.getByTestId("field-importerLastName").fill("Zagara");
  await page.getByTestId("field-animalName").fill("Blaze");
  await page.getByTestId("field-travelType").fill("air");
  await page.getByTestId("issue-submit").click();

  // The success panel offers a link to the printable receipt.
  const viewReceipt = page.getByTestId("view-receipt");
  await expect(viewReceipt).toBeVisible({ timeout: 45_000 });
  await viewReceipt.click();

  // 2) The receipt sheet renders the CDC anatomy: Receipt ID, the three sections, a QR, a status chip.
  const sheet = page.getByTestId("receipt-sheet");
  await expect(sheet).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("receipt-id")).toContainText("Receipt ID:");
  await expect(sheet).toContainText("Section A - Person Importing the Animal");
  await expect(sheet).toContainText("Section B - Animal Information");
  await expect(sheet).toContainText("Section C - Travel Information");
  await expect(sheet).toContainText("Blaze"); // Section B value rendered from the custodial subject
  await expect(sheet).toContainText("Zagara"); // Section A value (custodial, authenticated view)
  // The verification QR (an <svg>) and a live status verdict are present.
  await expect(page.getByTestId("receipt-qr").locator("svg")).toBeVisible();
  const verdict = (await page.getByTestId("receipt-status").textContent()) || "";
  expect(verdict).toMatch(/VALID|EXPIRED|REVOKED/);

  // 3) The QR's public URL (same-origin /r/:receiptId) — read the receiptId off the receipt and open
  //    the PII-free status page a verifier would scan to.
  const receiptIdText = (await page.getByTestId("receipt-id").textContent()) || "";
  const receiptId = receiptIdText.replace("Receipt ID:", "").trim();
  expect(receiptId.length).toBeGreaterThan(6);

  const resp = await page.goto(`/r/${receiptId}`);
  expect(resp?.status()).toBe(200);
  const body = (await page.locator("body").textContent()) || "";
  // Shows the on-chain verdict + validity window...
  expect(body).toMatch(/VALID|EXPIRED|REVOKED/);
  expect(body).toContain(receiptId);
  // ...and NEVER Section-A person PII (the public page is status-only per arch DP-5).
  expect(body).not.toContain("Zagara");
  expect(body).not.toContain("Person Importing the Animal");
});
