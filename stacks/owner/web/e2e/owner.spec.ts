import { test, expect, type Page } from "@playwright/test";

/**
 * The pet-owner (holder) loop, end to end:
 *
 *   1. RECEIVE  — paste a real wrapped credential → integrity-checked → held in the wallet
 *   2. DISPLAY  — the wallet lists it; the detail view decodes its fields
 *   3. GENERATE — build the §1.10 consent + EdDSA-BabyJubjub sign it IN THE BROWSER (real crypto),
 *                 then obtain a Groth16 proof from the (mocked) trusted prover-service
 *   4. PRESENT  — submit the proof to the (mocked) verifier, which records it and reports VERIFIED
 *
 * The prover + verifier + ROAX RPC are mocked at the network layer so the loop is deterministic; the
 * client-side crypto (consent assembly + EdDSA signature + EIP-712 bind signature) is 100% real.
 */

const VERIFIER = "http://verifier.test";
const VERIFY_LINK = `${VERIFIER}/x/tok_demo_123?a=0x1111111111111111111111111111111111111111`;

const SESSION = {
  sessionId: "sess-abc-001",
  relayer: "0x1111111111111111111111111111111111111111",
  purpose: "grooming_intake",
  recordType: "VACCINATION",
  challenge: "0x" + "ab".repeat(32),
  mode: "zk",
};

const TX_HASH = "0x" + "9".repeat(64);
const NULLIFIER = "0x" + "7".repeat(64);

/** Install the network mocks: verifier session, prover, submit, poll, and the ROAX JSON-RPC. */
async function installMocks(page: Page) {
  // ROAX JSON-RPC (isValid / bindNonce reads) — echo the request id, return a non-zero word.
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

  // Verifier: resolve the scanned verify-session token.
  await page.route(`${VERIFIER}/x/**`, async (route) => {
    await route.fulfill({ contentType: "application/json", body: JSON.stringify(SESSION) });
  });

  // Trusted prover-service: return a well-shaped Groth16 calldata bundle {a,b,c,pub}.
  await page.route("**/prove-verification", async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        a: ["1", "2"],
        b: [["3", "4"], ["5", "6"]],
        c: ["7", "8"],
        pub: ["424242", "9", "0x1111", "0x2222", NULLIFIER, "0x" + "5".repeat(64), SESSION.challenge],
      }),
    });
  });

  // Verifier: accept the submitted proof.
  await page.route(`${VERIFIER}/verify/consent/submit`, async (route) => {
    await route.fulfill({ contentType: "application/json", body: JSON.stringify({ ok: true }) });
  });

  // Verifier: session poll → recorded on-chain.
  await page.route(`${VERIFIER}/verify/session/**`, async (route) => {
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ status: "recorded", txHash: TX_HASH, nullifier: NULLIFIER }),
    });
  });
}

test.beforeEach(async ({ page }) => {
  await installMocks(page);
});

test("holder loop: receive → hold → generate ZK proof → present → verified", async ({ page }) => {
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

  // The wallet now holds exactly one credential.
  await page.goto("/wallet");
  await expect(page.getByTestId("cred-count")).toContainText("1 held");
  await expect(page.getByTestId("cred-name")).toHaveText("Rex");

  // 3 + 4. GENERATE + PRESENT — drive the ZK present flow to a recorded verification.
  await page.goto("/present");
  await page.getByTestId("present-link").fill(VERIFY_LINK);
  await page.getByTestId("present-run").click();

  // The proving step is genuinely reached (real consent + EdDSA signing precede it).
  await expect(page.getByTestId("present-steps")).toBeVisible();

  // Verified — the (mocked) verifier recorded the proof on chain; the tx + nullifier are surfaced.
  await expect(page.getByTestId("present-verified")).toBeVisible({ timeout: 45_000 });
  await expect(page.getByTestId("present-tx")).toContainText(TX_HASH);
  await expect(page.getByTestId("present-nullifier")).toContainText(NULLIFIER);
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
