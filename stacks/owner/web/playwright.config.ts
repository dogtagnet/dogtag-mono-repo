import { defineConfig, devices } from "@playwright/test";

/**
 * E2E for the PET-OWNER (holder) wallet — the full holder loop: receive → hold/display → generate a
 * ZK proof (real in-browser EdDSA-BabyJubjub consent signing) → present to a verifier → verified.
 *
 * By default it runs against a LOCAL vite dev server this config starts, with the prover + verifier
 * HTTP endpoints MOCKED at the network layer (see e2e/owner.spec.ts). That makes the loop
 * deterministic and self-contained — the REAL client-side crypto still runs; only the two remote
 * services (the owner's prover + the verifier) are stubbed. Point it at a live wallet instead with:
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
