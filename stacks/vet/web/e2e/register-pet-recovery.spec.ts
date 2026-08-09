import { expect, test, type Page, type Route } from "@playwright/test";

/**
 * The Register pet RECOVERY card, against a MOCKED backend.
 *
 * The defect this pins (measured on the captain's stack, 2026-08-09): a failed dog-tag issuance
 * could only be retried from the portal page that started it — the Retry card read the session
 * from in-page React state. A page reload, and above all a backend restart (every tunnel rotation
 * forces one), erased the route back while the stranded root stayed anchored on-chain. The backend
 * now journals sessions and serves `GET /profiles/issue/sessions`; this suite pins the portal half:
 *
 *  1. A failed issuance the page never held is LISTED, with what failed, and Retry re-arms it —
 *     straight into the QR card for the SAME dog tag.
 *  2. No failed issuances → no card (and no warning): silence may only claim a checked absence.
 *  3. A list read that FAILS renders could-not-check — never nothing, which would claim there is
 *     nothing to recover; the form stays usable.
 *
 * Mocked payloads are transcribed from the real vet-api handlers (`profile_issue_sessions_list`,
 * `profile_issue_session_retry` in routes.rs).
 *
 * Like the sibling suites this is NOT in `pnpm test` / CI (needs a served portal + browsers):
 *   VITE_DEMO_MODE=1 pnpm --filter @dogtag/vet-web dev   # one shell (port 41873)
 *   pnpm --filter @dogtag/vet-web test:e2e               # another (VET_URL overrides the base)
 */

const OP_TOKEN_KEY = "vet.opToken";
const SIGNER = "0x00000000000000000000000000000000000000a1";

interface SessionRow {
  sessionId: string;
  dogTagId: string;
  status: string;
  createdAt: number;
  petName: string;
  ownerName: string;
  error?: string | null;
  errorStage?: string | null;
}

interface Backend {
  sessions: SessionRow[] | "unreachable";
  retries: string[];
}

const FAILED_ROW: SessionRow = {
  sessionId: "sess-stranded",
  dogTagId: "6",
  status: "error",
  createdAt: 1_754_500_000,
  petName: "Rex",
  ownerName: "Alex Doe",
  error:
    "mintCustodial failed on the SBT: signer lacks ISSUER_ROLE. The root IS anchored on-chain " +
    "(issue(R) landed), so this dog tag can only be completed by fixing the cause and using " +
    "Retry issuance on THIS session",
  errorStage: "mint",
};

async function mockBackend(page: Page, backend: Backend) {
  await page.route(/^https?:\/\/[^/]+\/api\//, async (route: Route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api/, "");

    if (path === "/health") {
      return route.fulfill({
        json: {
          status: "ok",
          dogTagIssuance: { ready: true, profileIssuerConfigured: true, sbtConsentConfigured: true },
        },
      });
    }
    if (path === "/profiles/issue/sessions") {
      if (backend.sessions === "unreachable") return route.abort("connectionrefused");
      return route.fulfill({ json: { sessions: backend.sessions } });
    }
    const retry = path.match(/^\/profiles\/issue\/session\/([^/]+)\/retry$/);
    if (retry && request.method() === "POST") {
      backend.retries.push(retry[1]);
      return route.fulfill({
        json: {
          token: "fresh-tok",
          dogTagId: FAILED_ROW.dogTagId,
          sessionId: retry[1],
          qr: "http://vet.local/p/fresh-tok",
          ttlSecs: 180,
        },
      });
    }
    if (path.startsWith("/profiles/issue/session/")) {
      // The status poll after a retry adopts the session: still pending, waiting for the phone.
      return route.fulfill({
        json: { status: "pending", dogTagId: FAILED_ROW.dogTagId, tokenSecondsLeft: 170 },
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

test("a failed issuance the page never held is listed and Retry re-arms it into the QR card", async ({
  page,
}) => {
  const backend: Backend = { sessions: [FAILED_ROW], retries: [] };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  const card = page.getByTestId("issuance-recovery");
  await expect(card).toBeVisible();
  const row = page.getByTestId("issuance-recovery-row");
  await expect(row).toHaveCount(1);
  await expect(row).toContainText("Rex");
  await expect(row).toContainText("Dog tag 6");
  await expect(row).toContainText("failed at: mint");
  await expect(row).toContainText("can only be completed");

  await page.getByTestId("issuance-recovery-retry").click();
  // The retry targeted THE listed session — not a fresh registration.
  await expect.poll(() => backend.retries).toEqual(["sess-stranded"]);
  // The page adopts the re-armed session: the QR card for the SAME dog tag, waiting for the phone.
  await expect(page.getByText("Owner scans to receive their dog tag")).toBeVisible();
  await expect(page.getByTestId("qr-waiting")).toBeVisible();
});

test("no failed issuances → no recovery card and no warning — silence claims a checked absence", async ({
  page,
}) => {
  const backend: Backend = { sessions: [], retries: [] };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  // The form is the answer; the card and the could-not-check warning are both absent.
  await expect(page.getByText("Owner identity", { exact: true })).toBeVisible();
  await expect(page.getByTestId("issuance-recovery")).toHaveCount(0);
  await expect(page.getByTestId("issuance-recovery-unavailable")).toHaveCount(0);
});

test("a session list that cannot be read renders could-not-check, never silence", async ({
  page,
}) => {
  const backend: Backend = { sessions: "unreachable", retries: [] };
  await mockBackend(page, backend);
  await page.goto("/issue-dog-tag");

  await expect(page.getByTestId("issuance-recovery-unavailable")).toBeVisible();
  await expect(page.getByTestId("issuance-recovery-unavailable")).toContainText(
    "Could not check for unfinished dog-tag issuances",
  );
  // ...and the form stays usable: an unreadable list must not block ordinary registration.
  await expect(page.getByText("Owner identity", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Start issuance" })).toBeEnabled();
});
