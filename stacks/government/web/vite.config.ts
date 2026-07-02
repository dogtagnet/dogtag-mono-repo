import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Government portal. Dev port 44831; `/api` proxies to the government backend (default
 * http://localhost:44832, override with VITE_GOV_API_PROXY). This is a deliberately lean SPA
 * skeleton (no shared @dogtag/ui / wallet stack) — just enough to demo the issue + verify flows.
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
      },
    },
  };
});
