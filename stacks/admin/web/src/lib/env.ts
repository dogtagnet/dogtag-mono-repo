/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "/api",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** deployed IssuerRegistry address — empty until configured */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
};
