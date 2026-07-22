import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * Pet-Owner (holder) wallet — the consumer front of DogTag. Dev port 45931.
 *
 * Unlike the issuer/verifier portals this app has NO backend of its own: the owner holds their
 * credentials locally (localStorage) and reads credential validity directly from ROAX. There is no
 * `/api` proxy.
 *
 * `@dogtag/standard` is consumed as a workspace package and excluded from pre-bundling so Vite
 * transpiles it in place. The wallet still derives its local BabyJubjub consent key via the SDK;
 * owner-hidden proof generation itself requires the private tag-profile witness held by mobile.
 */
export default defineConfig({
  plugins: [react()],
  server: {
    port: 45931,
    strictPort: true,
  },
  optimizeDeps: {
    exclude: ["@dogtag/standard"],
  },
});
