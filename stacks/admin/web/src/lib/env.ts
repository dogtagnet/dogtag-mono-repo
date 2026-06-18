/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "/api",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** deployed IssuerRegistry address — empty until configured */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /** other deployed ROAX contract addresses (surfaced for parity / displays) */
  dogTagSbtAddr: import.meta.env.VITE_DOGTAG_SBT_ADDR ?? "",
  verificationRegistryAddr: import.meta.env.VITE_VERIFICATION_REGISTRY_ADDR ?? "",
  poseidon6Addr: import.meta.env.VITE_POSEIDON6_ADDR ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
