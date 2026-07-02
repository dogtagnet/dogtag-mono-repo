import { defineConfig, devices } from "@playwright/test";

/**
 * E2E config for the VET portal's records-management surface. Like the government/owner e2e it is NOT
 * part of `pnpm test` / CI (needs a served portal + browsers). It drives the REAL Records UI + the
 * shared `@dogtag/ui` API client against a MOCKED backend (network interception), so the full
 * issue → list (with explorer link) → edit (off-chain only) → revoke (soft, keeps history) loop is
 * exercised deterministically without custody/chain. Run against a served dev portal:
 *
 *   pnpm --filter @dogtag/vet-web dev            # in one shell (port 41873)
 *   pnpm --filter @dogtag/vet-web test:e2e       # in another (VET_URL overrides the base)
 */
const BASE_URL = process.env.VET_URL || "http://localhost:41873";

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: { baseURL: BASE_URL, trace: "retain-on-failure" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
