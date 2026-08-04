import { expect, test, type Page, type Route } from "@playwright/test";

const OP_TOKEN_KEY = "groomer.opToken";
const TX_HASH = "0x" + "9".repeat(64);

interface MockState {
  sessionStarts: Record<string, unknown>[];
}

async function mockBackend(page: Page, state: MockState) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    if (path === "/settings/signing-mode") {
      return route.fulfill({ json: { signingMode: "backend" } });
    }
    if (path === "/verify/history") {
      return route.fulfill({ json: { verifications: [] } });
    }
    if (path === "/verify/session/start" && request.method() === "POST") {
      state.sessionStarts.push((request.postDataJSON() ?? {}) as Record<string, unknown>);
      return route.fulfill({
        json: {
          qrUrl: "http://verifier.example/x/owner-hidden-token",
          sessionId: "session-owner-hidden",
        },
      });
    }
    if (path === "/verify/session/session-owner-hidden") {
      return route.fulfill({ json: { status: "recorded", txHash: TX_HASH } });
    }
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

test("starts only the owner-hidden verification flow and sends no mode", async ({ page }) => {
  const state: MockState = { sessionStarts: [] };
  await mockBackend(page, state);

  await page.goto("/verify");
  // The PAGE TITLE, not "any element with this text". "Ad-hoc verification" is also the nav link and
  // the card heading, so a bare getByText resolves to three elements and fails strict mode - which
  // reads like the page not rendering rather than like an ambiguous locator.
  await expect(page.getByRole("heading", { level: 1, name: "Ad-hoc verification" })).toBeVisible();
  // There is ONE owner-hidden flow: the operator is never offered a mode/disclosure choice.
  await expect(page.getByText("Mode", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Normal", { exact: true })).toHaveCount(0);

  await page.getByRole("button", { name: "Start export" }).click();
  await expect(page.getByText("Awaiting owner consent")).toBeVisible();
  // No appointment on the ad-hoc path, so no business context is sent either.
  expect(state.sessionStarts).toEqual([
    { purpose: "grooming_intake", recordType: "VACCINATION" },
  ]);
  expect(state.sessionStarts[0]).not.toHaveProperty("mode");

  await expect(page.getByText("Verified")).toBeVisible({ timeout: 10_000 });
});
