import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * Pet-Owner (holder) wallet — the consumer front of DogTag. Dev port 45931.
 *
 * Unlike the issuer/verifier portals this app has NO backend of its own: the owner holds their
 * credentials locally (localStorage) and, to present a proof, talks DIRECTLY to two hosts it is
 * given at runtime — a verifier's portal (the `/x/<token>` verify-session it scans) and a trusted
 * prover-service (`POST /prove-verification`, the owner's own or a service they trust). Those hosts
 * are absolute URLs (from the scanned QR + `VITE_OWNER_PROVER_URL`), so there is no `/api` proxy.
 *
 * `@dogtag/standard` is consumed as a workspace package and excluded from pre-bundling so Vite
 * transpiles it in place (it pulls in circomlibjs for the in-browser EdDSA-BabyJubjub consent
 * signature — the genuine "phone ZK" client-side crypto).
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
