import { test, expect, type Page, type Route } from "@playwright/test";

/**
 * "Who may sign in your name" — LAYER 2 of the two-layer issuance requirement, against a MOCKED
 * backend.
 *
 * This page exists because layer 1 had a screen and layer 2 had none, so a correctly registered
 * provider with a correctly attached contract could still not issue and nothing in the product said
 * why. What is driven here is everything that does NOT need a wallet: the diagnosis, the three
 * standings, and the two honesty rules the page must never break.
 *
 * The WRITE path is deliberately not driven here. It is a wallet transaction signed by the
 * contract's owner, and a scripted EIP-6963 provider in a Playwright fixture would be testing the
 * shim rather than the product. It is covered by the mounted suite in `packages/ui`
 * (`signerRosterRender.test.tsx`, which drives admit through a stubbed wallet and asserts the
 * pending/settled distinction) and by the live walk recorded in `docs/DEMO_CLICKS.md`.
 */

const OP_TOKEN_KEY = "vet.opToken";
const OWNER = "0x00000000000000000000000000000000000000a1";
const OURS = "0x00000000000000000000000000000000000000b7";
const WITHDRAWN = "0x00000000000000000000000000000000000000c3";
const CLONE = "0x00000000000000000000000000000000000000d4";
const PROFILE_CLONE = "0x00000000000000000000000000000000000000e5";

/** Our signer is on neither list — the state a provider is actually in when issuing fails. */
const NOT_ADMITTED = {
  activeSigner: OURS,
  contracts: [
    {
      recordType: "DOG_PROFILE",
      issuerAddr: PROFILE_CLONE,
      read: {
        state: "resolved",
        owner: OWNER,
        // The backend always gives this shop's own signer a row, on every contract - otherwise the
        // one address the provider needs to act on is invisible in exactly the state they are
        // trying to diagnose. The fixture mirrors that rather than a shape the backend never emits.
        entries: [
          { address: OWNER, allowed: true, everNamed: true },
          { address: OURS, allowed: false, everNamed: false },
        ],
        activeSignerAllowed: false,
      },
    },
    {
      recordType: "VACCINATION",
      issuerAddr: CLONE,
      read: {
        state: "resolved",
        owner: OWNER,
        entries: [
          { address: OWNER, allowed: true, everNamed: true },
          { address: WITHDRAWN, allowed: false, everNamed: true },
          { address: OURS, allowed: false, everNamed: false },
        ],
        activeSignerAllowed: false,
      },
    },
  ],
};

async function mockBackend(page: Page, body: unknown, status = 200) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace(/^\/api/, "");
    if (path === "/settings/signing-mode") {
      return route.fulfill({ json: { signingMode: "backend" } });
    }
    if (path === "/issuer/issuance-allowed") {
      return route.fulfill({ status, json: body });
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

test("the page diagnoses why this shop cannot issue, per contract", async ({ page }) => {
  await mockBackend(page, NOT_ADMITTED);
  await page.goto("/signers");

  await expect(page.getByTestId("active-signer")).toHaveText(OURS);
  // BOTH anchoring contracts, including the DOG_PROFILE clone the dog-tag bind uses — the half most
  // easily forgotten, because completing a bind needs the phone app.
  const verdicts = page.getByTestId("backend-signer-verdict");
  await expect(verdicts).toHaveCount(2);
  await expect(verdicts.first()).toContainText("does not admit it");
  await expect(verdicts.first()).toContainText(OURS);
  // It must not claim the other half is fine either — layer 1 has its own screen.
  await expect(verdicts.first()).toContainText("issue right");
});

test("withdrawn and never-admitted read differently as PLAIN TEXT", async ({ page }) => {
  await mockBackend(page, NOT_ADMITTED);
  await page.goto("/signers");

  // Scoped to ONE contract: the owner and this shop's signer each have a row on BOTH cards, so an
  // unscoped filter is ambiguous - and an ambiguous locator that happened to resolve would be
  // asserting about whichever card came first.
  const card = page.getByTestId("issuer-contract").filter({ hasText: "VACCINATION" });
  const rows = card.getByTestId("roster-row");
  const withdrawnRow = rows.filter({ hasText: WITHDRAWN });
  const ourRow = rows.filter({ hasText: OURS });
  const ownerRow = rows.filter({ hasText: OWNER });

  // The failure this guards, named in the task: a flattened text dump of the admin page made a
  // withdrawn holder look current, because the distinction was carried by styling alone.
  await expect(withdrawnRow.getByTestId("roster-standing")).toHaveText(/withdrawn/i);
  await expect(ourRow.getByTestId("roster-standing")).toHaveText(/not admitted/i);
  await expect(ownerRow.getByTestId("roster-standing")).toHaveText(/can issue/i);

  // …and reading the whole row as text keeps them apart.
  expect(await withdrawnRow.innerText()).toMatch(/withdrawn/i);
  expect(await ownerRow.innerText()).not.toMatch(/withdrawn/i);
});

test("a list that could not be read never renders as an empty one", async ({ page }) => {
  await mockBackend(page, {
    activeSigner: OURS,
    contracts: [
      {
        recordType: "VACCINATION",
        issuerAddr: CLONE,
        read: { state: "unavailable", reason: "eth_getLogs range too wide" },
      },
    ],
  });
  await page.goto("/signers");

  await expect(page.getByTestId("roster-unavailable")).toBeVisible();
  await expect(page.getByTestId("roster-unavailable")).toContainText(
    "eth_getLogs range too wide",
  );
  // THE assertion. Neither the roster nor the empty-list sentence may appear: a provider deciding
  // who signs medical records in their name must never read "nobody is admitted" from a question
  // that was never answered.
  await expect(page.getByTestId("roster")).toHaveCount(0);
  await expect(page.getByTestId("roster-empty")).toHaveCount(0);
  await expect(page.getByTestId("backend-signer-verdict")).toContainText("not known");
});

test("an unreachable backend says so and claims nothing about who may issue", async ({ page }) => {
  await mockBackend(page, { error: "boom" }, 503);
  await page.goto("/signers");

  await expect(page.getByTestId("signers-load-failed")).toBeVisible();
  await expect(page.getByTestId("signers-load-failed")).toContainText(
    "not a statement that nobody may issue",
  );
  await expect(page.getByTestId("roster")).toHaveCount(0);
});

test("locked custody is reported as locked, not as a refused signer", async ({ page }) => {
  await mockBackend(page, {
    activeSigner: null,
    contracts: [
      {
        recordType: "VACCINATION",
        issuerAddr: CLONE,
        read: {
          state: "resolved",
          owner: OWNER,
          entries: [{ address: OWNER, allowed: true, everNamed: true }],
          activeSignerAllowed: null,
        },
      },
    ],
  });
  await page.goto("/signers");

  await expect(page.getByTestId("no-active-signer")).toBeVisible();
  await expect(page.getByTestId("backend-signer-verdict")).toContainText(/locked/i);
});

test("admitting is refused without a wallet, and says which wallet is needed", async ({ page }) => {
  await mockBackend(page, NOT_ADMITTED);
  await page.goto("/signers");

  await expect(page.getByTestId("admit-submit").first()).toBeDisabled();
  // A disabled control always says why — the rule the provider page had to learn.
  await expect(page.getByTestId("admit-blocked").first()).toContainText(/connect your wallet/i);
});

test("the nav reaches it", async ({ page }) => {
  await mockBackend(page, NOT_ADMITTED);
  await page.goto("/records");
  await page.getByRole("link", { name: "Signing keys" }).click();
  await expect(page).toHaveURL(/\/signers$/);
  await expect(page.getByTestId("active-signer")).toBeVisible();
});
