import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv } from "vite";

/**
 * Vet portal (impl §5.1). Dev port 41873; `/api` proxies to the vet backend (default
 * http://localhost:41874, override with VITE_VET_API_PROXY). `@dogtag/ui` and
 * `@dogtag/standard` are consumed as workspace source (no prebuild step).
 */
export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const apiTarget = env.VITE_VET_API_PROXY || "http://localhost:41874";
  return {
    plugins: [react()],
    server: {
      port: 41873,
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
