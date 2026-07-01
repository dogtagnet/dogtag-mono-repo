import { test, expect, type Page } from "@playwright/test";

/**
 * End-to-end re-test of the two captain-reported government-portal fixes, against a LIVE portal:
 *
 *   1. each credential type renders its OWN Issue-form fields
 *      (EU_HEALTH_CERT: microchip/rabies/vet;  TRAVEL_CLEARANCE: origin/destination/purpose)
 *   2. after a successful issue the wrapped document is displayed with a one-click COPY button
 *
 * The flow: open the portal → issue the credential (its own fields) → COPY the wrapped doc →
 * paste into Verify → assert VALID with all three authenticity pillars (integrity / on-chain /
 * issuer-whitelist) green. Run once per record type.
 */

interface TypeCase {
  recordType: string;
  ownField: string; // a field that ONLY this type has
  foreignField: string; // a field that this type must NOT have (belongs to the other type)
}

const CASES: TypeCase[] = [
  {
    recordType: "EU_HEALTH_CERT",
    ownField: "field-microchipNumber",
    foreignField: "field-destinationCountry",
  },
  {
    recordType: "TRAVEL_CLEARANCE",
    ownField: "field-destinationCountry",
    foreignField: "field-microchipNumber",
  },
];

async function issueAndCopy(page: Page, c: TypeCase, dogTagId: string): Promise<string> {
  await page.goto("/issue");
  await page.getByTestId("record-type").selectOption(c.recordType);

  // Fields are differentiated per record type.
  await expect(page.getByTestId(c.ownField)).toBeVisible();
  await expect(page.getByTestId(c.foreignField)).toHaveCount(0);

  // Dog-tag id is the first (and only bare) input above the per-type field grid.
  await page.locator("input").first().fill(dogTagId);

  await page.getByTestId("issue-submit").click();

  // The wrapped document is displayed with a copy button.
  const wrapped = page.getByTestId("wrapped-doc");
  await expect(wrapped).toBeVisible({ timeout: 45_000 });
  const displayed = (await wrapped.textContent()) || "";
  expect(displayed.length).toBeGreaterThan(50);

  // One-click COPY populates the clipboard with exactly the displayed document.
  await page.getByTestId("copy-wrapped").click();
  await expect(page.getByTestId("copy-wrapped")).toContainText("Copied");
  const clip = await page.evaluate(() => navigator.clipboard.readText());
  expect(clip.trim()).toBe(displayed.trim());
  expect(clip).toContain("merkleRoot");
  return clip;
}

async function verifyValid(page: Page, wrappedJson: string) {
  await page.goto("/verify");
  await page.getByTestId("verify-doc").fill(wrappedJson);

  // Signer is prefilled from /health so the whitelist pillar is exercised; ensure it is non-empty.
  await expect(page.getByTestId("verify-signer")).not.toHaveValue("");

  await page.getByTestId("verify-submit").click();

  await expect(page.getByTestId("verdict")).toContainText("VALID", { timeout: 45_000 });
  await expect(page.getByTestId("verdict")).toHaveClass(/ok/);

  // All three authenticity pillars must be green ("yes").
  await expect(page.getByTestId("pillar-integrity")).toContainText("yes");
  await expect(page.getByTestId("pillar-onchain")).toContainText("yes");
  await expect(page.getByTestId("pillar-whitelist")).toContainText("yes");
}

for (const c of CASES) {
  test(`${c.recordType}: own fields → issue → copy → verify VALID (3 pillars)`, async ({
    page,
  }) => {
    // Unique-ish dogTagId per type so records stay distinguishable (roots are salted-random anyway).
    const dogTagId = c.recordType === "EU_HEALTH_CERT" ? "4242" : "7373";
    const wrapped = await issueAndCopy(page, c, dogTagId);
    await verifyValid(page, wrapped);
  });
}
