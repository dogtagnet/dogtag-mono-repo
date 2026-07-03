import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Government portal (impl §5.x). Dev port 44831. `@dogtag/ui` + `@dogtag/standard` are consumed as
 * workspace source (no prebuild step), so the portal shares the vet/groomer/admin AppShell + tokens.
 *
 * Two proxy prefixes both target the government backend (default http://localhost:44832, override
 * with VITE_GOV_API_PROXY):
 *   - `/api`  the authenticated JSON API (prefix stripped to match the Rust routes).
 *   - `/r`    the PUBLIC, server-rendered PII-free receipt status page (`GET /r/:receiptId`). It is
 *             owned by the backend, NOT the SPA — proxying it here means the receipt-QR URL resolves
 *             on the SAME origin as the portal in dev (nginx does the equivalent in prod).
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_GOV_API_PROXY || "http://localhost:44832";
  return {
    plugins: [react()],
    server: {
      port: 44831,
      strictPort: true,
      proxy: {
        "/api": {
          target: apiTarget,
          changeOrigin: true,
          rewrite: (p) => p.replace(/^\/api/, ""),
        },
        // Public receipt status page — passed through untouched (no prefix strip). The trailing
        // slash matters: a bare "/r" prefix-matches the SPA's own "/receipt/:root" route and would
        // wrongly proxy it to the backend (→ 404). "/r/" matches only the public status path.
        "/r/": {
          target: apiTarget,
          changeOrigin: true,
        },
      },
    },
    optimizeDeps: {
      // workspace source packages — let Vite transpile them in-place
      exclude: ["@dogtag/ui", "@dogtag/standard"],
    },
  };
});
