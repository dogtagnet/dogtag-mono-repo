import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Government portal (impl §5.x). Dev port 44831. `@dogtag/ui` + `@dogtag/standard` are consumed as
 * workspace source (no prebuild step), so the portal shares the vet/groomer/admin AppShell + tokens.
 *
 * Three proxy prefixes all target the government backend (default http://localhost:44832, override
 * with VITE_GOV_API_PROXY):
 *   - `/api`  the authenticated JSON API (prefix stripped to match the Rust routes).
 *   - `/r/`   the PUBLIC `/r/` surface: the server-rendered PII-free receipt status page AND the
 *             one-time record-share tokens the owner's phone resolves. Owned by the backend, NOT the
 *             SPA — proxying it here means both resolve on the SAME origin as the portal in dev
 *             (nginx does the equivalent in prod).
 *   - `/x/`   the PUBLIC verify-session token resolver the owner's phone hits while proving.
 *
 * A phone scanning a QR normally reaches the backend DIRECTLY (the QR encodes `DEPLOYMENT_URL`, which
 * in the demo scripts is the API's own host:port). These proxies are what make a portal-origin
 * `DEPLOYMENT_URL` work too, so the deployment is free to point either way.
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
        // Public receipt status page + one-time record-share tokens — passed through untouched (no
        // prefix strip). The trailing slash matters: a bare "/r" prefix-matches the SPA's own
        // "/receipt/:root" route and would wrongly proxy it to the backend (→ 404). "/r/" matches
        // only the public path.
        "/r/": {
          target: apiTarget,
          changeOrigin: true,
        },
        // Public verify-session token resolver (the owner's phone). Same trailing-slash rule.
        "/x/": {
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
