import { defineConfig, devices } from "@playwright/test";

// E2E for the government portal. By default this boots an ISOLATED demo-mode stack (no live node,
// no gas): government-api in GOV_DEMO_MODE on :44932 (MemChain + MemStore) + the Vite dev server on
// :44931 proxying /api -> :44932. Point at an already-running portal instead with
// GOV_PORTAL_URL=<url> (then the built-in webServer is skipped).
const externalUrl = process.env.GOV_PORTAL_URL;
const WEB_PORT = 44931;
const API_PORT = 44932;
const baseURL = externalUrl || `http://localhost:${WEB_PORT}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL,
    // the copy button uses navigator.clipboard — grant access so the e2e can read it back.
    permissions: ["clipboard-read", "clipboard-write"],
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: externalUrl
    ? undefined
    : [
        {
          // government-api in demo mode. Assumes `cargo build --release -p government-api` has run
          // (CI builds it first); reuses an already-running instance locally.
          command:
            `bash -c 'cd ../../.. && GOV_DEMO_MODE=1 PORT=${API_PORT} ` +
            `TRAVEL_CLEARANCE_ISSUER_ADDR=0x1111111111111111111111111111111111111111 ` +
            `EU_HEALTH_CERT_ISSUER_ADDR=0x2222222222222222222222222222222222222222 ` +
            `exec ./target/release/government-api'`,
          url: `http://localhost:${API_PORT}/health`,
          reuseExistingServer: true,
          timeout: 60_000,
        },
        {
          command: `pnpm exec vite --port ${WEB_PORT} --strictPort`,
          env: { VITE_GOV_API_PROXY: `http://localhost:${API_PORT}` },
          url: `http://localhost:${WEB_PORT}/`,
          reuseExistingServer: true,
          timeout: 60_000,
        },
      ],
});
