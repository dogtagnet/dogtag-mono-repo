import { expect, test, type Page, type Route } from "@playwright/test";

/**
 * Point-of-need custody unlock, GROOMER portal, against a MOCKED backend.
 *
 * THE CUSTODY-GATED ACTION HERE IS STARTING A VERIFICATION, NOT ISSUING.
 *
 * This suite was a copy of the vet's and drove `/issue` and `/records`. A groomer VERIFIES and does
 * not ISSUE - `BUSINESS_TYPE=groomer` mounts no issuance routes at all - and both pages were removed
 * from this portal with the CRM work. The catch-all route silently redirects an unknown path to
 * `/dashboard`, so the suite landed on the dashboard and hunted for an Issue form that cannot exist:
 * seven of its nine tests failed, and nothing noticed because it is not in CI.
 *
 * `POST /verify/session/start` (`routes.rs::export_session_start`) is custody-gated exactly as
 * `/credentials/prepare` is, and it is what this portal's "Start export" button calls - so every
 * property below is the same property, asserted on the surface this role really has.
 *
 * The behaviour under test is the operator complaint this change exists for: a backend restart
 * re-locks custody, and until now the only feedback was "you have to unlock" with no obvious way to
 * do it - the operator had to go hunting through the Setup wizard. Now the refused action raises an
 * unlock prompt IN PLACE and, once the seal opens, the request is replayed - so a half-filled form
 * is never discarded.
 *
 * Mocked payloads are transcribed from the real `vet-api` (the groomer runs the same binary) handlers so the code under test sees
 * exactly what the binary emits:
 *   - `GET  /issuer/signers`  -> `{ signers: [] }` when locked (routes.rs `issuer_signers`
 *                               short-circuit), `{ activeSigner, matrix }` when unlocked.
 *   - `POST /admin/login`     -> `{ token, initialized, unlocked }` (routes.rs `admin_login`).
 *   - `POST /admin/unlock`    -> 401 `wrong passphrase` / 409 `not initialized` (routes.rs `unlock`).
 *   - custody-gated actions   -> 409 `not unlocked`, which every real handler returns from the TOP
 *                               of the handler before any store or chain write - which is precisely
 *                               what makes replaying the request safe rather than a double-submit.
 *
 * Like the other portal e2e suites this is NOT part of `pnpm test` / CI (it needs a served portal +
 * browsers):
 *
 *   VITE_DEMO_MODE=1 pnpm --filter @dogtag/groomer-web dev   # one shell (port 43617)
 *   pnpm --filter @dogtag/groomer-web test:e2e              # another (GROOMER_URL overrides the base)
 *
 * `VITE_DEMO_MODE=1` is required by the sibling specs in this directory, and `scripts/e2e-web.sh`
 * serves the portal with it. This suite does not depend on the demo button: it drives the purpose
 * Select directly, because the demo preset selects `purposes[0]` - which is also the Select's default,
 * so asserting it survived an unlock would pass on a form that had been discarded and remounted.
 */

const OP_TOKEN_KEY = "groomer.opToken";
// Deliberately NOT the demo password "admin": the "neither secret is persisted" scan substring-matches
// each stored entry, and the persisted admin-SESSION key is literally `groomer.adminToken`, so "admin"
// would always self-match the key name and fail on a leak that is not there.
const ADMIN_PASSWORD = "e2e-custody-admin-pw";
const PASSPHRASE = "demo-pass-0000";
const SIGNER = "0x00000000000000000000000000000000000000a1";

interface Custody {
  /** A seal exists (genesis has run). `false` => the instance needs Setup, not a passphrase. */
  initialized: boolean;
  /** The seed is decrypted in the running process. A restart flips this back to `false`. */
  unlocked: boolean;
  /** Call counters, so a test can assert the rate-limiter-safe login behaviour. */
  adminLogins: number;
  /** How many times the custody-gated action actually reached the backend. */
  exports: number;
}

function newCustody(over: Partial<Custody> = {}): Custody {
  return { initialized: true, unlocked: false, adminLogins: 0, exports: 0, ...over };
}

/** Install a mock vet backend whose custody state the test mutates like a real restart would. */
async function mockBackend(page: Page, custody: Custody) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    // The arrival-time probe: an existing read-only operator-gated route, no new endpoint.
    if (path === "/issuer/signers") {
      return custody.unlocked
        ? route.fulfill({ json: { activeSigner: SIGNER, matrix: [] } })
        : route.fulfill({ json: { signers: [] } });
    }
    if (path === "/admin/login" && request.method() === "POST") {
      custody.adminLogins += 1;
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
    // The custody-gated action this role actually has: refuses with the exact message and status the
    // real handler uses while locked, from the TOP of the handler - which is what makes replaying it
    // safe rather than a double-submit.
    if (path === "/verify/session/start" && request.method() === "POST") {
      if (!custody.unlocked) {
        return route.fulfill({ status: 409, json: { error: "not unlocked" } });
      }
      custody.exports += 1;
      return route.fulfill({
        json: { qrUrl: "http://verifier.example/x/e2e-token", sessionId: "session-e2e" },
      });
    }
    // Polled once a session exists. Left PENDING deliberately: this suite is about the unlock, and a
    // session that settled would race the assertions with a success view.
    if (path === "/verify/session/session-e2e") {
      return route.fulfill({ json: { status: "pending" } });
    }
    if (path === "/verifications") return route.fulfill({ json: { verifications: [] } });
    if (path === "/settings/signing-mode") return route.fulfill({ json: { signingMode: "backend" } });
    if (path === "/verify/history") return route.fulfill({ json: { verifications: [] } });
    if (path.startsWith("/trace/")) return route.fulfill({ json: { events: [], stats: {} } });
    return route.fulfill({ json: {} });
  });
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

/**
 * The verification form's purpose selector - the operator's one input on this page, and so the state
 * that must survive an unlock. `VerifyFlow` renders it as a Radix `Select`, whose trigger carries the
 * chosen label, so reading the trigger reads the selection.
 */
// A CSS locator, NOT `getByRole("combobox")`, and that is the whole point of this helper: the
// preserved-state assertion below is taken WHILE THE UNLOCK DIALOG IS OPEN, and a modal correctly
// hides the rest of the page from the accessibility tree - so a role query finds nothing and the
// failure reads as "the form was discarded" when the form is right there. The vet's twin escapes this
// only because `getByPlaceholder` is not a role query either.
const purposeField = (page: Page) => page.locator('[role="combobox"]').first();

/**
 * Put the form in the state the operator would, returning the selection that must survive.
 *
 * IT MUST NOT BE THE DEFAULT, and using the demo button here made the whole assertion VACUOUS.
 * `VerifyFlow` initialises `useState(purposes[0]?.value)` and its demo handler picks `purposes[0]` -
 * the same value - so "the selection survived" and "the form was discarded and remounted at its
 * default" render identically, and the one property this suite exists for could not fail. The vet
 * twin escapes this only because it types a dog-tag id no default can produce.
 *
 * So this drives the Select to a purpose that is deliberately NOT `purposes[0]`. A remount now shows
 * "Grooming intake" where the test demands "Daycare access", and the assertion bites.
 */
async function fillVerifyForm(page: Page): Promise<string> {
  await purposeField(page).click();
  // Both labels are read BEFORE anything is clicked - selecting closes the listbox, so a check made
  // afterwards would find no options and quietly pass.
  const labels = (await page.getByRole("option").allTextContents()).map((t) => t.trim());
  // Guard the guard: if the portal ever ships a single purpose, the choice IS the default and this
  // helper goes back to proving nothing. Fail loudly rather than pass vacuously.
  expect(
    labels.length,
    "the purpose list must offer a NON-DEFAULT option or the preserved-state assertion is vacuous",
  ).toBeGreaterThan(1);
  const chosen = labels[labels.length - 1];
  await page.getByRole("option", { name: chosen, exact: true }).click();
  await expect(purposeField(page)).toHaveText(chosen);
  return chosen;
}

const submitVerify = (page: Page) =>
  page.getByRole("button", { name: /Start export/i }).first().click();

test.describe("point-of-need unlock", () => {
  test("a locked action prompts in place, preserves the form, and continues after unlocking", async ({
    page,
  }) => {
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/verify");
    const purpose = await fillVerifyForm(page);

    // The backend re-locked while the operator was typing; submitting trips the lock.
    await submitVerify(page);

    // The prompt is raised OVER the page - no navigation, no teardown.
    await expect(page.getByRole("dialog")).toBeVisible();
    await expect(page.getByText("Custody is locked")).toBeVisible();
    await expect(page).toHaveURL(/\/verify$/);
    // The captain's actual complaint: the typed record must still be there.
    await expect(purposeField(page)).toHaveText(purpose);

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock and continue" }).click();

    // The refused request is replayed, so the action the operator started actually COMPLETES - the
    // chosen purpose is carried into the export rather than handed back to be re-picked. The form is
    // legitimately replaced by the awaiting-consent view here precisely BECAUSE the replay succeeded;
    // the "nothing you chose is lost" property is the assertion above, taken while the prompt is up.
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByText("Awaiting owner consent").first()).toBeVisible();
    await expect(page).toHaveURL(/\/verify$/);
    // Exactly once: the client replays a refused request a single time, so unlocking can never
    // double-submit the operator's issuance.
    await expect.poll(() => custody.exports).toBe(1);
  });

  test("a wrong passphrase shows an inline error and never reports a dead session", async ({
    page,
  }) => {
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/verify");
    await fillVerifyForm(page);
    await submitVerify(page);
    await expect(page.getByRole("dialog")).toBeVisible();

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill("not-the-passphrase");
    await page.getByRole("button", { name: "Unlock and continue" }).click();

    await expect(page.getByRole("alert")).toContainText("Wrong passphrase");
    // /admin/unlock answers a wrong passphrase with 401, the same status its admin gate uses for a
    // dead session. Only the message separates them, so this must NOT clear the session.
    await expect(page.getByText("Session expired")).toHaveCount(0);
    await expect(page.getByRole("dialog")).toBeVisible();
  });

  test("retrying a wrong passphrase does NOT re-issue an admin login", async ({ page }) => {
    // Regression guard. `/admin/login` ends in `record_success(&ip)`, which REMOVES the IP from the
    // rate limiter that also guards `/admin/unlock`. Logging in before each attempt would wipe the
    // failure tally between guesses, so the per-IP lockout could never trip.
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/verify");
    await fillVerifyForm(page);
    await submitVerify(page);
    await expect(page.getByRole("dialog")).toBeVisible();

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill("wrong-one");
    await page.getByRole("button", { name: "Unlock and continue" }).click();
    await expect(page.getByRole("alert")).toContainText("Wrong passphrase");
    const afterFirst = custody.adminLogins;

    await page.getByLabel("Unlock passphrase").fill("wrong-two");
    await page.getByRole("button", { name: "Unlock and continue" }).click();
    await expect(page.getByRole("alert")).toContainText("Wrong passphrase");

    expect(custody.adminLogins).toBe(afterFirst);
  });

  test("dismissing the prompt surfaces the original refusal and replays nothing", async ({
    page,
  }) => {
    // The other half of the replay contract: `onCustodyLocked` resolving FALSE must rethrow the
    // original refusal rather than retry, so a dismissed prompt cannot loop the operator or fire the
    // action behind their back. The client replays at most once, and only on a successful unlock.
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/verify");
    const purpose = await fillVerifyForm(page);
    await submitVerify(page);
    await expect(page.getByRole("dialog")).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    // The refusal reaches the page it came from, and the form is still there to retry from.
    await expect(page.getByText(/not unlocked/i).first()).toBeVisible();
    await expect(page).toHaveURL(/\/verify$/);
    // The CHOSEN purpose, not merely "something is selected": the Select's default is `purposes[0]`,
    // so a non-empty assertion passes on a form that was discarded and remounted - the exact state
    // this test rules out.
    await expect(purposeField(page)).toHaveText(purpose);
    expect(custody.exports).toBe(0);
    // Still locked, so the banner takes over as the standing way back in.
    await expect(page.getByText(/Custody is locked/)).toBeVisible();
  });

  test("a locked backend never blocks read-only pages; it shows a banner instead", async ({
    page,
  }) => {
    // Operator login and custody-admin are SEPARATE credentials, so front-desk staff who hold the
    // former but not the latter must keep the read-only work they are entitled to do.
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/verifications");
    await expect(page).toHaveURL(/\/verifications$/);
    await expect(page.getByText(/Custody is locked/)).toBeVisible();

    await page.goto("/dashboard");
    await expect(page).toHaveURL(/\/dashboard$/);
  });

  test("an unreachable backend is never announced as locked", async ({ page }) => {
    // `unknown` is not `locked`: no passphrase can fix a backend that is not answering.
    await page.route(/^https?:\/\/[^/]+\/api\//, (route: Route) => route.abort());
    await page.goto("/verifications");
    await expect(page).toHaveURL(/\/verifications$/);
    await expect(page.getByText(/Custody is locked/)).toHaveCount(0);
  });
});

test.describe("dedicated /unlock route (fallback surface)", () => {
  test("is reachable directly and restores ?next=", async ({ page }) => {
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/unlock?next=%2Fverifications");
    await expect(page.getByText("Unlock custody", { exact: true })).toBeVisible();

    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page).toHaveURL(/\/verifications$/);
  });

  test("an instance with NO seal points at Setup instead of asking for a passphrase", async ({
    page,
  }) => {
    const custody = newCustody({ initialized: false });
    await mockBackend(page, custody);

    await page.goto("/unlock");
    // The admin password ALONE reveals the state - the operator is never asked to invent a
    // passphrase for a seal that does not exist.
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByRole("button", { name: "Unlock" }).click();

    await expect(page.getByText(/no seal yet/i)).toBeVisible();
    await expect(page.getByRole("link", { name: "Go to Setup" })).toBeVisible();
    await expect(page.getByLabel("Unlock passphrase")).toHaveCount(0);
  });

  test("neither secret is persisted anywhere", async ({ page }) => {
    const custody = newCustody();
    await mockBackend(page, custody);

    await page.goto("/unlock");
    await page.getByLabel("Custody admin password").fill(ADMIN_PASSWORD);
    await page.getByLabel("Unlock passphrase").fill(PASSPHRASE);
    await page.getByRole("button", { name: "Unlock" }).click();
    await expect(page.getByText("Custody unlocked").first()).toBeVisible();

    for (const secret of [PASSPHRASE, ADMIN_PASSWORD]) {
      const leaked = await page.evaluate((s) => {
        const entries = [localStorage, sessionStorage].flatMap((store) =>
          Object.keys(store).map((k) => `${k}=${store.getItem(k)}`),
        );
        return entries.filter((e) => e.includes(s as string));
      }, secret);
      expect(leaked).toEqual([]);
    }
  });
});
