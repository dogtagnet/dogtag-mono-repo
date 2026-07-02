import { defineConfig, devices } from "@playwright/test";

/**
 * E2E config for the GOVERNMENT portal. The suite runs against a LIVE deployment (the served,
 * tunnelled portal or a local `demo-up` instance) — set `GOV_URL` to point at it:
 *
 *   GOV_URL=https://<tunnel>.trycloudflare.com pnpm --filter @dogtag/government-web test:e2e
 *
 * It is intentionally NOT part of `pnpm test` / CI (no live portal + browsers there); it is the
 * captain-facing re-test harness for the issue → copy → verify flow.
 */
const BASE_URL = process.env.GOV_URL || "http://localhost:44831";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 20_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: BASE_URL,
    // The copy-button flow needs clipboard access in headless Chromium.
    permissions: ["clipboard-read", "clipboard-write"],
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
