/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "/api",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /**
   * DogTagIssuerFactory used to anchor a credential to its issuing clone. THERE IS NO FALLBACK any
   * more - the shared default it used to fall back to was a literal, and this whole variable exists
   * so an address is chosen by an operator rather than by a source file.
   *
   * Unset does not silently disable the anchor either: the verifier REFUSES by name, so the bench
   * renders every on-chain row as could-not-run with the reason, rather than as a verdict. That
   * keeps the misconfigure-to-bypass path closed, which is what the old fallback was protecting.
   */
  factoryAddr: import.meta.env.VITE_DOGTAG_ISSUER_FACTORY_ADDR ?? "",
  /**
   * The provider authority. Advisory on the bench's verify path - the mandatory pillar sources its
   * authority from the resolved clone's own `registry()`, never from a client-configured address -
   * but the registry-governs-issuer row reports whether THIS client's pair is coherent, and that is
   * a fault worth telling an operator about.
   */
  providerRegistryAddr: import.meta.env.VITE_PROVIDER_REGISTRY_ADDR ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
