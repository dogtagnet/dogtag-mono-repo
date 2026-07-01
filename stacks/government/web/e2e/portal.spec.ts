import { expect, test, type Page } from "@playwright/test";

// End-to-end for the government portal: for each credential type, issue it (with its OWN field set),
// COPY the wrapped document via the copy button, PASTE it into the Verify tab, and assert the
// verdict is VALID with all three authenticity pillars green (integrity + on-chain + issuer whitelist).

const TYPES = [
  {
    recordType: "EU_HEALTH_CERT",
    ownField: "field-rabiesVaccinationDate", // present only for EU health certs
    absentField: "field-originCountry", // a travel-clearance-only field
  },
  {
    recordType: "TRAVEL_CLEARANCE",
    ownField: "field-originCountry",
    absentField: "field-rabiesVaccinationDate",
  },
];

async function issueCopyVerify(page: Page, recordType: string, ownField: string, absentField: string) {
  await page.goto("/issue");

  // Select the credential type and confirm the form shows THIS type's own field set.
  await page.getByTestId("record-type").selectOption(recordType);
  await expect(page.getByTestId(ownField)).toBeVisible();
  await expect(page.getByTestId(absentField)).toHaveCount(0);

  // Issue -> the wrapped document appears.
  await page.getByTestId("issue-btn").click();
  const wrapped = page.getByTestId("wrapped-doc");
  await expect(wrapped).toBeVisible({ timeout: 30_000 });
  await expect(wrapped).not.toHaveValue("");

  // Copy the wrapped document via the copy button, then read it back from the clipboard.
  await page.getByTestId("copy-btn").click();
  await expect(page.getByTestId("copy-btn")).toHaveText(/Copied/);
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied.length).toBeGreaterThan(50);
  const parsed = JSON.parse(copied);
  expect(parsed.issuer.recordType).toBe(recordType);

  // Paste into the Verify tab and verify.
  await page.goto("/verify");
  await page.getByTestId("verify-doc").fill(copied);
  await page.getByTestId("verify-btn").click();

  // Verdict VALID with all three pillars green.
  await expect(page.getByTestId("verdict")).toHaveText(/VALID/, { timeout: 30_000 });
  await expect(page.getByTestId("frag-integrity")).toHaveAttribute("data-value", "yes");
  await expect(page.getByTestId("frag-on-chain")).toHaveAttribute("data-value", "yes");
  await expect(page.getByTestId("frag-issuer-whitelist")).toHaveAttribute("data-value", "yes");
}

for (const t of TYPES) {
  test(`issue ${t.recordType} -> copy -> verify VALID (3 pillars)`, async ({ page }) => {
    await issueCopyVerify(page, t.recordType, t.ownField, t.absentField);
  });
}
