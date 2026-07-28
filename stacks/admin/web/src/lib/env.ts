/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "/api",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** deployed IssuerRegistry address — empty until configured */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /**
   * DogTagIssuerFactory used to anchor a credential to its issuing clone. Empty falls back to the
   * SDK's deployed default rather than disabling the anchor - the factory is the trust root, and a
   * verifier that silently stopped asking would be the misconfigure-to-bypass path the pillar's
   * explicit states exist to prevent.
   */
  factoryAddr: import.meta.env.VITE_DOGTAG_ISSUER_FACTORY_ADDR ?? "",
  /**
   * IssuerDomainRegistry, for the bench's issuer-domain check. Deliberately has NO fallback: the
   * contract set is still being revised and this one may yet be folded elsewhere, so an unset address
   * makes the bench report that check UNAVAILABLE rather than read a constant that may have moved.
   */
  issuerDomainRegistryAddr: import.meta.env.VITE_ISSUER_DOMAIN_REGISTRY_ADDR ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
