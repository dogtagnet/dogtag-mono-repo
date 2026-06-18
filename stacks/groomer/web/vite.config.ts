import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Groomer portal (impl §5.2). Dev port 43617; `/api` proxies to the groomer backend (default
 * http://localhost:43618, override with VITE_GROOMER_API_PROXY). The groomer backend is
 * STRUCTURALLY IDENTICAL to the vet backend (same routes.rs contracts). `@dogtag/ui` and
 * `@dogtag/standard` are consumed as workspace source (no prebuild step).
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
      },
    },
    optimizeDeps: {
      // workspace source packages — let Vite transpile them in-place
      exclude: ["@dogtag/ui", "@dogtag/standard"],
    },
  };
});
