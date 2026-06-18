import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Admin portal (impl §5.3). Dev port 39741; `/api` proxies to the CENTRAL backend (default
 * http://localhost:39742, override with VITE_CENTRAL_API_PROXY). `@dogtag/ui` and
 * `@dogtag/standard` are consumed as workspace source (no prebuild step).
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_CENTRAL_API_PROXY || "http://localhost:39742";
  return {
    plugins: [react()],
    server: {
      port: 39741,
      strictPort: true,
      proxy: {
        "/api": {
          target: apiTarget,
          changeOrigin: true,
          rewrite: (p) => p.replace(/^\/api/, ""),
        },
      },
    },
    optimizeDeps: {
      // workspace source packages — let Vite transpile them in-place
      exclude: ["@dogtag/ui", "@dogtag/standard"],
    },
  };
});
