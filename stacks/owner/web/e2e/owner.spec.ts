import { test, expect, type Page } from "@playwright/test";

/**
 * The pet-owner (holder) loop, end to end:
 *
 *   1. RECEIVE  — paste a real wrapped credential → integrity-checked → held in the wallet
 *   2. DISPLAY  — the wallet lists it; the detail view decodes its fields
 *   3. SHARE    — create an integrity-preserving selectively disclosed copy
 *   4. RECEIPT  — render government travel/health receipts with live validity
 *
 * ROAX RPC is mocked at the network layer so the live validity reads are deterministic. Owner-hidden
 * consent proving requires the private tag-profile witness held by the native wallet and is not a
 * browser-wallet surface.
 */

/** Install the sole remote dependency used by this browser wallet: read-only ROAX JSON-RPC. */
async function installMocks(page: Page) {
  // ROAX JSON-RPC (`DogTagIssuer.isValid`) — echo the request id, return true.
  await page.route(/devrpc\.roax\.net/, async (route) => {
    let id: unknown = 1;
    try {
      id = JSON.parse(route.request().postData() || "{}").id ?? 1;
    } catch {
      /* keep default id */
    }
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ jsonrpc: "2.0", id, result: "0x" + "0".repeat(63) + "1" }),
    });
  });

}

test.beforeEach(async ({ page }) => {
  await installMocks(page);
});

test("holder loop: receive → hold → display", async ({ page }) => {
  // Fresh wallet each run.
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // The wallet starts empty and provisions a self-custodial owner address.
  await expect(page.getByTestId("empty-wallet")).toBeVisible();
  await expect(page.getByTestId("owner-address")).toContainText("0x");

  // 1. RECEIVE — fill the sample credential and add it.
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  await expect(page.getByTestId("receive-input")).toHaveValue(/VACCINATION/);
  await page.getByTestId("receive-add").click();

  // 2. DISPLAY — receiving lands on the decoded detail view; integrity is intact.
  await expect(page.getByTestId("detail-name")).toHaveText("Rex");
  await expect(page.getByTestId("detail-integrity")).toContainText("intact");
  await expect(page.getByTestId("detail-fields")).toContainText("Rabies");
  await expect(page.getByTestId("detail-fields")).toContainText("424242");
  await expect(page.getByTestId("detail-receipt")).toHaveCount(0);

  // Unsupported record types fail closed if someone manually opens the receipt route.
  await page.goto("/receipt/0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67");
  await expect(page.getByTestId("receipt-unavailable")).toContainText("VACCINATION");

  // The wallet now holds exactly one credential.
  await page.goto("/wallet");
  await expect(page.getByTestId("cred-count")).toContainText("1 held");
  await expect(page.getByTestId("cred-name")).toHaveText("Rex");

  // The retired browser proof surface is gone; unknown routes return safely to the wallet.
  await page.goto("/present");
  await expect(page.getByTestId("cred-count")).toContainText("1 held");
});

test("holder selective disclosure: withhold a field → redacted copy still verifies", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // Receive the sample credential (lands on its detail view).
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  await page.getByTestId("receive-add").click();
  await expect(page.getByTestId("detail-name")).toHaveText("Rex");

  // Open the Share (redacted copy) flow from the detail view.
  await page.getByTestId("detail-share").click();
  await expect(page.getByTestId("share-fields")).toBeVisible();
  // dogTagId is locked-on (required, non-obfuscatable) and its toggle is disabled.
  await expect(page.getByTestId("share-locked").first()).toBeVisible();
  await expect(page.getByTestId("share-toggle-credentialSubject.dogTagId")).toBeDisabled();

  // Everything revealed by default → the copy is the full, authentic credential.
  await expect(page.getByTestId("share-withheld-count")).toContainText("Every field is revealed");
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  await expect(page.getByTestId("share-preview")).toContainText("Dr. A. Meyer");

  // Withhold the veterinarian field.
  await page.getByTestId("share-toggle-credentialSubject.veterinarian").click();
  await expect(page.getByTestId("share-withheld-count")).toContainText("1 field withheld");
  // The redacted copy STILL verifies authentic (the Merkle root is unchanged)…
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  // …but the recipient no longer sees the withheld value.
  await expect(page.getByTestId("share-preview")).toContainText("withheld by holder");
  await expect(page.getByTestId("share-preview")).not.toContainText("Dr. A. Meyer");

  // Copy the redacted credential; the output JSON drops the cleartext but keeps dogTagId + the root.
  await page.getByTestId("share-copy").click();
  await expect(page.getByTestId("share-copied")).toBeVisible();
  const out = await page.getByTestId("share-output").inputValue();
  expect(out).not.toContain("Dr. A. Meyer");
  expect(out).toContain("424242"); // dogTagId cleartext stays
  const redacted = JSON.parse(out) as { privacy: { obfuscated: string[] }; signature: { merkleRoot: string } };
  expect(redacted.privacy.obfuscated.length).toBe(1); // the withheld leaf's hash is retained
  expect(redacted.signature.merkleRoot).toBe(
    "0x11bd3f84654df12518d490f7e109127b277673641016239863973844ce82dd67",
  ); // unchanged — it is the same on-chain credential
});

test("holder receipt: travel clearance renders and respects redacted re-imports", async ({ page }) => {
  await page.goto("/wallet");
  await page.evaluate(() => localStorage.clear());
  await page.reload();

  // Receive the CDC-modeled government travel-clearance sample.
  await page.goto("/receive");
  await page.getByTestId("receive-sample-travel").click();
  await expect(page.getByTestId("receive-input")).toHaveValue(/TRAVEL_CLEARANCE/);
  await page.getByTestId("receive-add").click();

  // Detail now recognizes the nested animal name + exposes the receipt action and derived status.
  await expect(page.getByTestId("detail-name")).toHaveText("Blaze");
  await expect(page.getByTestId("detail-receipt")).toBeVisible();
  await expect(page.getByTestId("detail-onchain")).toContainText("Receipt: Valid");

  // The top-level Receipts nav lists holder-renderable receipt credentials.
  await page.goto("/receipts");
  await expect(page.getByTestId("receipt-count")).toContainText("1 available");
  await expect(page.getByTestId("receipt-row")).toContainText("Blaze");
  await expect(page.getByTestId("receipt-row")).toContainText("9RVBXK8AFQ2C");
  await expect(page.getByTestId("receipt-row-status")).toContainText("Valid");

  await page.getByTestId("receipt-row").click();
  const sheet = page.getByTestId("receipt-sheet");
  await expect(sheet).toBeVisible();
  await expect(page.getByTestId("receipt-status")).toContainText("VALID");
  await expect(page.getByTestId("receipt-id")).toContainText("9RVBXK8AFQ2C");
  await expect(sheet).toContainText("Section A - Person Importing the Animal");
  await expect(sheet).toContainText("Dominic");
  await expect(sheet).toContainText("887524355");
  await expect(sheet).toContainText("Section B - Animal Information");
  await expect(sheet).toContainText("Blaze");
  await expect(sheet).toContainText("Section C - Travel Information");
  await expect(sheet).toContainText("AC 8552");
  await expect(page.getByTestId("receipt-public-url")).toContainText("https://gov.example/r/9RVBXK8AFQ2C");
  await expect(page.getByTestId("receipt-qr").locator("svg")).toBeVisible();
  await expect(page.getByTestId("receipt-live")).toContainText("anchored");
  await expect(page.getByTestId("receipt-root")).toContainText(
    "0x010a607eb1f94fd672622331ae1272c5e08afba9b6d094b52b5b5e3a2bec4a45",
  );

  // Produce a redacted copy that withholds a Section-A identifier, then re-import it. The same root is
  // replaced in localStorage, and the receipt renders only the disclosed leaves.
  await page.getByTestId("receipt-share").click();
  await page.getByTestId("share-toggle-credentialSubject.importer.idNumber").click();
  await expect(page.getByTestId("share-preview-integrity")).toContainText("authentic");
  await page.getByTestId("share-copy").click();
  const redactedJson = await page.getByTestId("share-output").inputValue();
  expect(redactedJson).not.toContain("887524355");
  const redacted = JSON.parse(redactedJson) as { privacy: { obfuscated: string[] }; signature: { merkleRoot: string } };
  expect(redacted.privacy.obfuscated.length).toBe(1);
  expect(redacted.signature.merkleRoot).toBe(
    "0x010a607eb1f94fd672622331ae1272c5e08afba9b6d094b52b5b5e3a2bec4a45",
  );

  await page.goto("/receive");
  await page.getByTestId("receive-input").fill(redactedJson);
  await page.getByTestId("receive-add").click();
  await page.getByTestId("detail-receipt").click();
  await expect(page.getByTestId("receipt-withheld-note")).toContainText("1 field");
  await expect(page.getByTestId("receipt-sheet")).toContainText("Dominic");
  await expect(page.getByTestId("receipt-sheet")).not.toContainText("887524355");
});

test("receive rejects a tampered credential", async ({ page }) => {
  await page.goto("/receive");
  await page.getByTestId("receive-sample").click();
  // Corrupt a disclosed field so the recomputed Merkle root no longer matches.
  const tampered = (await page.getByTestId("receive-input").inputValue()).replace("Rabies", "Sugar");
  await page.getByTestId("receive-input").fill(tampered);
  await page.getByTestId("receive-add").click();
  await expect(page.getByTestId("receive-error")).toContainText("integrity");
});
