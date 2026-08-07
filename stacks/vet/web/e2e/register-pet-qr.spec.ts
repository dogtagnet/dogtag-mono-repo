import { expect, test, type Page, type Route } from "@playwright/test";

/**
 * The Register-pet QR card renders the SERVER's truth, never its own clock — against a MOCKED
 * backend.
 *
 * The defect this pins (measured on a live walk, 2026-08-07): the portal held a hardcoded 180s
 * timer and rendered "expired" for every way a session could go nowhere — a phone that never
 * reached this machine (its address had changed), a device that resolved the QR and went quiet,
 * and a genuinely lapsed deadline all read identically, while the server would still have accepted
 * a bind the client's own timer had already declared dead.
 *
 * What is asserted, one state each, all driven purely by mocked server facts:
 *  1. waiting-for-scan shows the SERVER's countdown, not a local one;
 *  2. a resolved pickup is rendered apart from an unclaimed QR;
 *  3. token dead + never resolved says NO DEVICE ARRIVED and points at the network;
 *  4. token dead + resolved says the device went quiet — a different remedy;
 *  5. "minting" renders as anchoring with an honest duration, with no deadline applied;
 *  6. a bind landing in the token's final seconds still flips the card to anchoring — the card
 *     keeps polling after the deadline instead of freezing on "expired";
 *  7. a server-reported error is rendered as the failure it is, never as expiry;
 *  8. a QR whose address this machine no longer answers at is called out with both addresses.
 *
 * Mocked payloads are transcribed from the real vet-api handlers (routes.rs
 * `profile_issue_session_start` / `profile_issue_session_status`).
 *
 * Like the sibling suites this is NOT in `pnpm test` / CI (needs a served portal + browsers); it
 * runs in `make e2e-web`.
 */

const OP_TOKEN_KEY = "vet.opToken";
const SIGNER = "0x00000000000000000000000000000000000000a1";

interface QrAddress {
  host: string;
  check: "selfAddressed" | "notSelfAddressed" | "unknown";
  currentAddress?: string;
  detail?: string;
}

interface StatusResp {
  status: "pending" | "minting" | "bound" | "error";
  dogTagId: string;
  root?: string | null;
  txHash?: string | null;
  resolvedAt?: number | null;
  tokenSecondsLeft?: number;
  qrAddress?: QrAddress;
}

interface Backend {
  startTtlSecs: number;
  qrAddress: QrAddress;
  /** Consumed one per status poll; the LAST entry is sticky. */
  statuses: StatusResp[];
  polls: number;
}

const SELF: QrAddress = { host: "192.168.16.117", check: "selfAddressed" };

function pendingStatus(over: Partial<StatusResp> = {}): StatusResp {
  return {
    status: "pending",
    dogTagId: "7",
    resolvedAt: null,
    tokenSecondsLeft: 540,
    qrAddress: SELF,
    ...over,
  };
}

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
    if (path === "/profiles/issue/session/start" && request.method() === "POST") {
      return route.fulfill({
        json: {
          token: "tok",
          dogTagId: "7",
          sessionId: "sess-1",
          qr: `http://${backend.qrAddress.host}:41874/p/tok`,
          ttlSecs: backend.startTtlSecs,
          qrAddress: backend.qrAddress,
        },
      });
    }
    if (path === "/profiles/issue/session/sess-1") {
      const i = Math.min(backend.polls, backend.statuses.length - 1);
      backend.polls += 1;
      return route.fulfill({ json: backend.statuses[i] });
    }
    if (path === "/issuer/signers") {
      return route.fulfill({ json: { activeSigner: SIGNER, matrix: [] } });
    }
    if (path === "/settings/signing-mode") return route.fulfill({ json: { signingMode: "backend" } });
    return route.fulfill({ json: {} });
  });
}

async function startIssuance(page: Page) {
  await page.goto("/issue-dog-tag");
  // VITE_DEMO_MODE=1 prefills a valid owner + pet, so Start issuance submits directly.
  await page.getByRole("button", { name: "Start issuance" }).click();
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ([key]) => window.localStorage.setItem(key as string, "op-token-e2e"),
    [OP_TOKEN_KEY],
  );
});

test("waiting-for-scan shows the SERVER's countdown, not a local 180s clock", async ({ page }) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [pendingStatus({ tokenSecondsLeft: 540 })],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  await expect(page.getByTestId("qr-waiting")).toBeVisible();
  await expect(page.getByTestId("qr-seconds-left")).toContainText("10m 0s");
  // The first poll replaces the start value with the server's CURRENT remaining life.
  await expect(page.getByTestId("qr-seconds-left")).toContainText("9m 0s", { timeout: 10_000 });
  await expect(page.getByText(/180s/)).toHaveCount(0);
});

test("a device pickup is rendered apart from an unclaimed QR", async ({ page }) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [pendingStatus({ resolvedAt: 1_770_000_000, tokenSecondsLeft: 280 })],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  await expect(page.getByTestId("qr-picked-up")).toBeVisible({ timeout: 10_000 });
  await expect(page.getByTestId("qr-picked-up")).toContainText("picked this up");
});

test("token dead and never resolved says NO DEVICE ARRIVED and points at the network — not 'expired'", async ({
  page,
}) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [pendingStatus({ tokenSecondsLeft: 0 })],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  const dead = page.getByTestId("qr-dead-unclaimed");
  await expect(dead).toBeVisible({ timeout: 20_000 });
  await expect(dead).toContainText("No device ever picked this QR up");
  await expect(dead).toContainText("could not reach this machine");
  await expect(dead).toContainText("192.168.16.117");
  await expect(dead).toContainText("same network");
  await expect(page.getByRole("button", { name: "Start over" })).toBeVisible();
});

test("token dead after a pickup says the DEVICE WENT QUIET — a different fault, a different remedy", async ({
  page,
}) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [pendingStatus({ resolvedAt: 1_770_000_000, tokenSecondsLeft: 0 })],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  const dead = page.getByTestId("qr-dead-after-pickup");
  await expect(dead).toBeVisible({ timeout: 20_000 });
  await expect(dead).toContainText("picked this QR up but never sent");
  await expect(dead).toContainText("lost its connection");
  await expect(page.getByTestId("qr-dead-unclaimed")).toHaveCount(0);
});

test("an accepted bind renders as anchoring with an honest duration, and no deadline applies", async ({
  page,
}) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [
      {
        status: "minting",
        dogTagId: "7",
        resolvedAt: 1_770_000_000,
        tokenSecondsLeft: 0,
        qrAddress: SELF,
      },
    ],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  const anchoring = page.getByTestId("qr-anchoring");
  await expect(anchoring).toBeVisible({ timeout: 10_000 });
  await expect(anchoring).toContainText("anchored on-chain");
  await expect(anchoring).toContainText("a minute or two");
  // No expiry treatment while the chain writes run — the phone waits ~3 minutes on this phase.
  await expect(page.getByTestId("qr-dead-after-pickup")).toHaveCount(0);
});

test("a bind landing in the token's final seconds still flips the card to anchoring — polling survives the deadline", async ({
  page,
}) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [
      // Two dead polls (below the declare threshold), then the bind's "minting" arrives.
      pendingStatus({ resolvedAt: 1_770_000_000, tokenSecondsLeft: 0 }),
      pendingStatus({ resolvedAt: 1_770_000_000, tokenSecondsLeft: 0 }),
      {
        status: "minting",
        dogTagId: "7",
        resolvedAt: 1_770_000_000,
        tokenSecondsLeft: 0,
        qrAddress: SELF,
      },
    ],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  await expect(page.getByTestId("qr-anchoring")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("qr-dead-after-pickup")).toHaveCount(0);
});

test("a server-reported failure renders as the failure it is, never as expiry", async ({ page }) => {
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: SELF,
    statuses: [
      {
        status: "error",
        dogTagId: "7",
        txHash: "identity attestation integrity failed: bad opening — start a FRESH session",
        resolvedAt: 1_770_000_000,
        tokenSecondsLeft: 100,
        qrAddress: SELF,
      },
    ],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  const errorCard = page.getByTestId("qr-error");
  await expect(errorCard).toBeVisible({ timeout: 10_000 });
  await expect(errorCard).toContainText("identity attestation integrity failed");
  await expect(page.getByTestId("qr-dead-after-pickup")).toHaveCount(0);
  await expect(page.getByTestId("qr-dead-unclaimed")).toHaveCount(0);
});

test("a QR this machine no longer answers at is called out with both addresses, while the QR is on screen", async ({
  page,
}) => {
  const moved: QrAddress = {
    host: "192.168.1.71",
    check: "notSelfAddressed",
    currentAddress: "192.168.16.117",
  };
  const backend: Backend = {
    startTtlSecs: 600,
    qrAddress: moved,
    statuses: [pendingStatus({ qrAddress: moved })],
    polls: 0,
  };
  await mockBackend(page, backend);
  await startIssuance(page);

  const notice = page.getByTestId("qr-address-mismatch");
  await expect(notice).toBeVisible();
  await expect(notice).toContainText("Phones cannot reach this QR");
  await expect(notice).toContainText("192.168.1.71");
  await expect(notice).toContainText("192.168.16.117");
  await expect(notice).toContainText("Restart the vet stack");
  // The warning sits BESIDE the QR — the operator can still choose to proceed if they know better.
  await expect(page.getByTestId("qr-waiting")).toBeVisible();
});
