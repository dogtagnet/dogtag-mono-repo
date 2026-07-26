import { expect, test, type Page, type Route } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

/**
 * Dedicated-unlock-route E2E for the GROOMER portal, against a MOCKED backend.
 *
 * It drives the operator experience the change exists for: a backend restart re-locks custody, and
 * the portal must land the operator on a focused `/unlock` page and then put them back where they
 * were headed - instead of making them re-walk the Setup wizard.
 *
 * The mocked payloads are transcribed from the real `vet-api` handlers so the classifier under test
 * sees exactly what the binary emits:
 *   - `GET  /issuer/signers`  -> `{ signers: [] }` when locked (routes.rs `issuer_signers`
 *                               short-circuit), `{ activeSigner, matrix }` when unlocked.
 *   - `POST /admin/login`     -> `{ token, initialized, unlocked }` (routes.rs `admin_login`).
 *   - `POST /admin/unlock`    -> 401 `wrong passphrase` / 409 `not initialized` (routes.rs `unlock`).
 *   - custody-gated actions   -> 409 `not unlocked` (e.g. `export_session_start`).
 *
 * Like the other portal e2e suites this is NOT part of `pnpm test` / CI (it needs a served portal +
 * browsers). Run it against a served dev portal:
 *
 *   pnpm --filter @dogtag/groomer-web dev            # one shell (port 43617)
 *   pnpm --filter @dogtag/groomer-web test:e2e       # another (GROOMER_URL overrides the base)
 */

const OP_TOKEN_KEY = "groomer.opToken";
const ADMIN_TOKEN_KEY = "groomer.adminToken";
const ADMIN_PASSWORD = "admin";
const PASSPHRASE = "demo-pass-0000";
const SIGNER = "0x00000000000000000000000000000000000000a1";
const HOME = "/dashboard";

/** Screenshots land in the evidence dir when one is provided, else beside the Playwright output. */
const SHOTS = process.env.EVIDENCE_DIR ?? "test-results/unlock-shots";
mkdirSync(SHOTS, { recursive: true });
const shot = (page: Page, name: string) =>
  page.screenshot({ path: join(SHOTS, `groomer-${name}.png`), fullPage: true });

interface Custody {
  /** A seal exists (genesis has run). `false` => the instance needs Setup, not a passphrase. */
  initialized: boolean;
  /** The seed is decrypted in the running process. A restart flips this back to `false`. */
  unlocked: boolean;
  /** Overrides `/admin/unlock`'s reply, to exercise the two distinct 401 meanings. */
  unlockError?: { status: number; error: string };
}

/** Install a mock vet backend whose custody state the test mutates like a real restart would. */
async function mockBackend(page: Page, custody: Custody) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    // The on-load probe: the existing read-only operator-gated route, no new endpoint.
    if (path === "/issuer/signers") {
      return custody.unlocked
        ? route.fulfill({ json: { activeSigner: SIGNER, matrix: [] } })
        : route.fulfill({ json: { signers: [] } });
    }
    if (path === "/admin/login" && request.method() === "POST") {
      const pw = (request.postDataJSON() as { password?: string })?.password;
      if (pw !== ADMIN_PASSWORD) {
        return route.fulfill({ status: 401, json: { error: "bad password" } });
      }
      return route.fulfill({
        json: {
          token: "admin-token-e2e",
          initialized: custody.initialized,
          unlocked: custody.unlocked,
        },
      });
    }
    if (path === "/admin/unlock" && request.method() === "POST") {
      if (custody.unlockError) {
        return route.fulfill({
          status: custody.unlockError.status,
          json: { error: custody.unlockError.error },
        });
      }
      if (!custody.initialized) {
        return route.fulfill({ status: 409, json: { error: "not initialized" } });
      }
      const body = request.postDataJSON() as { passphrase?: string };
      if (body?.passphrase !== PASSPHRASE) {
        return route.fulfill({ status: 401, json: { error: "wrong passphrase" } });
      }
      custody.unlocked = true;
      return route.fulfill({ json: { accounts: [{ index: 0, address: SIGNER }] } });
    }
    // Custody-gated actions refuse with the same message the real handlers use while locked.
    if (path === "/verify/session/start" && request.method() === "POST") {
      if (!custody.unlocked) {
        return route.fulfill({ status: 409, json: { error: "not unlocked" } });
      }
      return route.fulfill({ json: { qrUrl: "http://verifier.example/x/t", sessionId: "s1" } });
    }
    if (path === "/settings/signing-mode") return route.fulfill({ json: { signingMode: "backend" } });
    if (path === "/verify/history") return route.fulfill({ json: { verifications: [] } });
    if (path === "/records") return route.fulfill({ json: { records: [] } });
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

test.describe("dedicated /unlock route", () => {
  test("a locked backend redirects to /unlock and restores the intended destination", async ({
    page,
  }) => {
    const custody: Custody = { initialized: true, unlocked: false };
    await mockBackend(page, custody);

    // The operator heads for Records; the probe finds a locked seal.
    await page.goto("/records");
    await expect(page).toHaveURL(/\/unlock\?next=%2Frecords$/);
    // Neutral copy until the backend has actually reported the seal state.
    await expect(page.getByText("Groomer Portal")).toBeVisible();
    await expect(page.getByText("Unlock custody", { exact: true })).toBeVisible();
    await expect(page.getByText("Custody is locked")).toHaveCount(0);
    await expect(page.getByText("Custody not set up")).toHaveCount(0);
    await shot(page, "01-redirected-neutral");

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page.getByText("Custody unlocked")).toBeVisible();
    // Back to where the operator was going - not a generic home page.
    await expect(page).toHaveURL(/\/records$/);
    // Same security model: neither secret is persisted anywhere by the new page.
    const leaked = await page.evaluate((secret) => {
      const entries = [localStorage, sessionStorage].flatMap((s) =>
        Object.keys(s).map((k) => `${k}=${s.getItem(k)}`),
      );
      return entries.filter((e) => e.includes(secret as string));
    }, PASSPHRASE);
    expect(leaked, "the unlock passphrase must never be persisted").toEqual([]);
    await shot(page, "03-destination-restored");
  });

  test("claims 'Custody is locked' only after the backend confirms a seal", async ({ page }) => {
    await mockBackend(page, { initialized: true, unlocked: false });

    await page.goto("/unlock");
    // Admin password alone: enough for /admin/login to report the seal, not enough to decrypt it.
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page.getByText("Custody is locked")).toBeVisible();
    await expect(page.getByRole("alert")).toContainText("This instance has a seal");
    await shot(page, "02-locked-confirmed");
  });

  test("an action that trips the lock mid-session redirects, keeping the destination", async ({
    page,
  }) => {
    // Start unlocked, so the portal renders normally and nothing is redirected up front.
    const custody: Custody = { initialized: true, unlocked: true };
    await mockBackend(page, custody);

    await page.goto("/verify");
    await expect(page).toHaveURL(/\/verify$/);

    // The backend restarts: the seal re-locks while the operator session survives (Mongo store).
    custody.unlocked = false;

    await page.getByRole("button", { name: "Start verification" }).click();
    // The refusal itself routes the operator to the fix instead of a dead-end error toast.
    await expect(page).toHaveURL(/\/unlock\?next=%2Fverify$/);
    await shot(page, "04-midsession-lock-redirect");

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();
    await expect(page).toHaveURL(/\/verify$/);
  });

  test("a wrong passphrase shows an inline error, not a bogus expired session", async ({ page }) => {
    await mockBackend(page, { initialized: true, unlocked: false });

    await page.goto("/unlock?next=%2Frecords");
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill("not-the-passphrase");
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page.getByRole("alert")).toContainText("Wrong passphrase");
    // The 401 must NOT be read as a dead admin session by the shared client.
    await expect(page.getByText("Session expired")).toHaveCount(0);
    await expect(page).toHaveURL(/\/unlock\?next=%2Frecords$/);
    expect(await page.evaluate((k) => localStorage.getItem(k), ADMIN_TOKEN_KEY)).toBe(
      "admin-token-e2e",
    );
    await shot(page, "05-wrong-passphrase");

    // Recoverable in place: the correct passphrase still lands the saved destination.
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();
    await expect(page).toHaveURL(/\/records$/);
  });

  test("a genuinely dead admin session on the same route still reports 'Session expired'", async ({
    page,
  }) => {
    await mockBackend(page, {
      initialized: true,
      unlocked: false,
      unlockError: { status: 401, error: "invalid admin session" },
    });

    await page.goto("/unlock");
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();

    // The message-keyed exemption is narrow: this 401 IS a dead session.
    await expect(page.getByText("Session expired")).toBeVisible();
    expect(await page.evaluate((k) => localStorage.getItem(k), ADMIN_TOKEN_KEY)).toBeNull();
  });

  test("a seal-less instance is pointed at Setup, never at a passphrase", async ({ page }) => {
    await mockBackend(page, { initialized: false, unlocked: false });

    await page.goto("/unlock");
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page.getByText("Custody not set up")).toBeVisible();
    await expect(page.getByText("Custody is locked")).toHaveCount(0);
    await expect(page.getByLabel("Unlock passphrase")).toHaveCount(0);
    await shot(page, "06-no-seal-go-to-setup");

    await page.getByRole("link", { name: "Go to Setup" }).click();
    await expect(page).toHaveURL(/\/setup$/);
    await expect(page.getByText("Custody admin login")).toBeVisible();
  });

  test("an unreachable backend is not treated as a locked one", async ({ page }) => {
    await page.route(/^https?:\/\/[^/]+\/api\//, (route: Route) =>
      route.fulfill({ status: 500, json: { error: "backend down" } }),
    );

    await page.goto("/records");
    // `unknown` never redirects - no passphrase can fix a backend that is not answering.
    await expect(page).toHaveURL(/\/records$/);
    await expect(page.getByText("Unlock custody")).toHaveCount(0);
  });

  test("Setup stays reachable while locked and hands the sealed instance to /unlock", async ({
    page,
  }) => {
    const custody: Custody = { initialized: true, unlocked: false };
    await mockBackend(page, custody);

    // Setup is lock-exempt: it owns genesis, the only cure for a seal-less instance.
    await page.goto("/setup");
    await expect(page).toHaveURL(/\/setup$/);
    await expect(page.getByText("Custody admin login")).toBeVisible();

    // The wizard no longer buries unlock: a sealed-but-locked instance is handed to /unlock.
    await page.getByLabel("Admin password").fill(ADMIN_PASSWORD);
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page).toHaveURL(/\/unlock\?next=%2Fsetup$/);
    await shot(page, "07-setup-hands-off-to-unlock");

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();
    await expect(page).toHaveURL(/\/setup$/);

    // Custody is open now, so Setup continues into genesis follow-up rather than an unlock step.
    await page.getByLabel("Admin password").fill(ADMIN_PASSWORD);
    await page.getByRole("button", { name: "Continue" }).click();
    await expect(page.getByText("Derive accounts")).toBeVisible();
    await shot(page, "08-setup-resumed-derive-accounts");
  });

  test("an off-origin ?next= cannot redirect the operator off the portal", async ({ page }) => {
    await mockBackend(page, { initialized: true, unlocked: false });

    await page.goto("/unlock?next=https%3A%2F%2Fevil.example%2Fsteal");
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page).toHaveURL(new RegExp(`${HOME}$`));
  });
});
