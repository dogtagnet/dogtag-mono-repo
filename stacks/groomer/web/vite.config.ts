import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Groomer portal (impl §5.2). Dev port 43617; `/api` proxies to the groomer backend (default
 * http://localhost:43618, override with VITE_GROOMER_API_PROXY). The groomer backend is
 * STRUCTURALLY IDENTICAL to the vet backend (same routes.rs contracts). `@dogtag/ui` and
 * `@dogtag/standard` are consumed as workspace source (no prebuild step).
 *
 * `/a/` is proxied ALONGSIDE `/api`: it is the PUBLIC per-appointment client handoff (the page a
 * client scans, and the `.ics` their phone opens), owned by the backend and NOT by this SPA. A
 * deployment is free to point `DEPLOYMENT_URL` at either the API directly or at this portal's
 * origin; without this proxy the second choice would send a scanning phone to the SPA's history
 * fallback, which answers 200 with the operator app's `index.html` — a live host serving the wrong
 * thing, which reads as working far more convincingly than a dead link does. nginx does the
 * equivalent in production (see `nginx.conf`).
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_GROOMER_API_PROXY || "http://localhost:43618";
  return {
    plugins: [react()],
    server: {
      port: 43617,
      strictPort: true,
      proxy: {
        "/api": {
          target: apiTarget,
          changeOrigin: true,
          rewrite: (p) => p.replace(/^\/api/, ""),
        },
        // The public client handoff — passed through untouched (no prefix strip), because the
        // backend route IS `/a/:token`. The trailing slash matters: a bare "/a" would prefix-match
        // any future SPA route beginning with that letter and wrongly proxy it away.
        "/a/": {
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
