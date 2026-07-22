import { defineConfig, devices } from "@playwright/test";

/**
 * E2E for the PET-OWNER (holder) wallet — receive, hold/display, selective disclosure, and receipts.
 *
 * By default it runs against a LOCAL vite dev server this config starts, with read-only ROAX RPC
 * mocked at the network layer (see e2e/owner.spec.ts). Point it at a live wallet instead with:
 *
 *   OWNER_URL=https://<tunnel> pnpm --filter @dogtag/owner-web test:e2e
 */
const BASE_URL = process.env.OWNER_URL || "http://localhost:45931";
const useOwnServer = !process.env.OWNER_URL;

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
    trace: "retain-on-failure",
    // Grant clipboard write so the "Copy redacted credential" path resolves deterministically - the app
    // now only reports success when the write actually resolves, so the copy assertion must not depend
    // on ambient headless clipboard-permission behaviour.
    permissions: ["clipboard-write"],
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  ...(useOwnServer
    ? {
        webServer: {
          command: "pnpm dev",
          url: BASE_URL,
          reuseExistingServer: true,
          timeout: 120_000,
        },
      }
    : {}),
});
