import { expect, test, type Page, type Route } from "@playwright/test";

/**
 * The Register pet ANCHOR gate, against a MOCKED backend.
 *
 * The defect this pins (measured on a live walk, 2026-08-07): with `PROFILE_ISSUER_ADDR` unset the
 * portal allocated a dog tag, drew a QR, and had the OWNER scan it — and the refusal the backend
 * had known since boot surfaced as a 503 on the owner's phone. The gate refuses IN PLACE, where
 * the operator can act.
 *
 * Three properties, one test each, plus the not-silently-disabled check:
 *  1. A DEFINITE "cannot anchor" answer replaces the form — no tag allocated, no QR drawn, and the
 *     message names the missing DOG_PROFILE contract and its fixing step.
 *  2. A ready backend renders the form exactly as before.
 *  3. A FAILED probe is could-not-check: it warns and leaves the form usable — an unreachable
 *     backend is never accused of being unconfigured (the backend's own session-start gate refuses
 *     an unanchorable start regardless, allocating nothing).
 *  4. The rest of the portal keeps working while the gate is blocked.
 *
 * Mocked payloads are transcribed from the real vet-api handlers: `GET /health` emits
 * `{ status, dogTagIssuance: { ready, profileIssuerConfigured, sbtConsentConfigured } }` on an
 * issuance-enabled role (routes.rs `health`).
 *
 * Like the sibling suites this is NOT in `pnpm test` / CI (needs a served portal + browsers):
 *   VITE_DEMO_MODE=1 pnpm --filter @dogtag/vet-web dev   # one shell (port 41873)
 *   pnpm --filter @dogtag/vet-web test:e2e               # another (VET_URL overrides the base)
 */

const OP_TOKEN_KEY = "vet.opToken";
const SIGNER = "0x00000000000000000000000000000000000000a1";

interface DogTagIssuance {
  ready: boolean;
  profileIssuerConfigured: boolean;
  sbtConsentConfigured: boolean;
}

/** Counts session-start calls so a test can assert nothing was ever allocated. */
interface Backend {
  health: DogTagIssuance | "unreachable";
  sessionStarts: number;
}

async function mockBackend(page: Page, backend: Backend) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    if (path === "/health") {
      if (backend.health === "unreachable") return route.abort("connectionrefused");
      return route.fulfill({ json: { status: "ok", dogTagIssuance: backend.health } });
    }
    if (path === "/profiles/issue/session/start" && request.method() === "POST") {
      backend.sessionStarts += 1;
      return route.fulfill({
        json: { token: "tok", dogTagId: "1", sessionId: "sess-1", qr: "http://x/p/tok" },
      });
    }
    // The custody arrival probe, in its recognised UNLOCKED shape so no lock banner interferes.
    if (path === "/issuer/signers") {
      return route.fulfill({ json: { activeSigner: SIGNER, matrix: [] } });
    }
    if (path === "/settings/signing-mode") return route.fulfill({ json: { signingMode: "backend" } });
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

test("a backend that cannot anchor refuses Register pet in place — no form, no tag, and the missing contract is named", async ({
  page,
}) => {
  const backend: Backend = {
    health: { ready: false, profileIssuerConfigured: false, sbtConsentConfigured: true },
    sessionStarts: 0,
  };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  const blocked = page.getByTestId("anchor-blocked");
  await expect(blocked).toBeVisible();
  // The operator's vocabulary and the fixing step — not a bare variable name.
  await expect(blocked).toContainText("cannot anchor a dog tag on-chain");
  await expect(blocked).toContainText("DOG_PROFILE issuer contract");
  await expect(blocked).toContainText("Provider page");
  // The configured SBT half is not accused.
  await expect(blocked).not.toContainText("DogTagSBTConsent");

  // The form is GONE, not merely disabled: nothing to fill, nothing to submit, no QR to draw.
  await expect(page.getByRole("button", { name: "Start issuance" })).toHaveCount(0);
  await expect(page.getByText("Owner identity", { exact: true })).toHaveCount(0);
  expect(backend.sessionStarts).toBe(0);
});

test("a ready backend renders the Register pet form exactly as before", async ({ page }) => {
  const backend: Backend = {
    health: { ready: true, profileIssuerConfigured: true, sbtConsentConfigured: true },
    sessionStarts: 0,
  };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  await expect(page.getByText("Owner identity", { exact: true })).toBeVisible();
  const start = page.getByRole("button", { name: "Start issuance" });
  await expect(start).toBeVisible();
  await expect(start).toBeEnabled();
  await expect(page.getByTestId("anchor-blocked")).toHaveCount(0);
  await expect(page.getByTestId("anchor-unknown")).toHaveCount(0);
});

test("a FAILED readiness probe warns and leaves the form usable — could-not-check is never a refusal", async ({
  page,
}) => {
  const backend: Backend = { health: "unreachable", sessionStarts: 0 };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  await expect(page.getByTestId("anchor-unknown")).toBeVisible();
  await expect(page.getByTestId("anchor-unknown")).toContainText(
    "Could not check whether this backend can anchor dog tags",
  );
  // The form stays: an unreachable probe is not evidence the backend is unconfigured.
  await expect(page.getByText("Owner identity", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Start issuance" })).toBeEnabled();
});

test("the rest of the portal keeps working while Register pet is blocked", async ({ page }) => {
  const backend: Backend = {
    health: { ready: false, profileIssuerConfigured: false, sbtConsentConfigured: false },
    sessionStarts: 0,
  };
  await mockBackend(page, backend);
  await page.goto("/records");
  // The Records page renders its own (empty) list — the gate is scoped to the anchor-needing flow.
  await expect(page.getByText(/no records/i).or(page.getByText(/Records/i)).first()).toBeVisible();
  await expect(page.getByTestId("anchor-blocked")).toHaveCount(0);
});
