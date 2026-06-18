/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  groomerApiBase: import.meta.env.VITE_GROOMER_API_BASE ?? "/api",
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "http://localhost:39742",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** ROAX RPC used for the on-chain issuance-status poll (DogTagIssuer.isValid). */
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /** DogTagIssuer contract address — used to poll isValid(root) in backend mode. */
  dogtagIssuerAddr: import.meta.env.VITE_DOGTAG_ISSUER_ADDR ?? "",
  /** Deployed ROAX contract addresses (surfaced for parity / status displays). */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  dogTagSbtAddr: import.meta.env.VITE_DOGTAG_SBT_ADDR ?? "",
  verificationRegistryAddr: import.meta.env.VITE_VERIFICATION_REGISTRY_ADDR ?? "",
  poseidon6Addr: import.meta.env.VITE_POSEIDON6_ADDR ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
