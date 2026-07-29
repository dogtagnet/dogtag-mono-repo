/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  groomerApiBase: import.meta.env.VITE_GROOMER_API_BASE ?? "/api",
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "http://localhost:39742",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** Bundled, chain-guarded RPC default; a browser-local Settings choice may be tried first. */
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /** DogTagIssuer contract address — used to poll isValid(root) in backend mode. */
  dogtagIssuerAddr: import.meta.env.VITE_DOGTAG_ISSUER_ADDR ?? "",
  /** Shared issuer registry used by direct credential verification. */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
