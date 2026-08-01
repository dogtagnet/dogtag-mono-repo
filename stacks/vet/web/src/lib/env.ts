/** Resolved runtime config from Vite env (see .env.example). */
export const env = {
  vetApiBase: import.meta.env.VITE_VET_API_BASE ?? "/api",
  centralApiBase: import.meta.env.VITE_CENTRAL_API_BASE ?? "http://localhost:39742",
  reownProjectId: import.meta.env.VITE_REOWN_PROJECT_ID ?? "REPLACE_WITH_REOWN_PROJECT_ID",
  deploymentUrl: import.meta.env.VITE_DEPLOYMENT_URL ?? window.location.origin,
  /** Bundled, chain-guarded RPC default; a browser-local Settings choice may be tried first. */
  roaxRpc: import.meta.env.VITE_ROAX_RPC ?? "https://devrpc.roax.net",
  /** DogTagIssuer contract address — used to poll isValid(root) in backend mode. */
  dogtagIssuerAddr: import.meta.env.VITE_DOGTAG_ISSUER_ADDR ?? "",
  /** Shared issuer registry used by direct credential verification. */
  issuerRegistryAddr: import.meta.env.VITE_ISSUER_REGISTRY_ADDR ?? "",
  /**
   * The generation-2 registry set, for the provider self-service page. NO FALLBACK on any of them,
   * deliberately: generation 2 is deployed but no client reads it yet (client repointing is
   * C-9/C-10), so a baked default here would repoint this deployment by accident. Unset makes that
   * page report itself unconfigured instead.
   */
  providerRegistryAddr: import.meta.env.VITE_PROVIDER_REGISTRY_ADDR ?? "",
  issuerFactoryV2Addr: import.meta.env.VITE_DOGTAG_ISSUER_FACTORY_V2_ADDR ?? "",
  serviceDomainResolverAddr: import.meta.env.VITE_SERVICE_DOMAIN_RESOLVER_ADDR ?? "",
  providerDirectoryAddr: import.meta.env.VITE_PROVIDER_DIRECTORY_ADDR ?? "",
  // The S-17 content mirror the profile document and logo are published to and read back from.
  // Blank ships blank, like its four neighbours: a value in the template would opt every deployment
  // that copies it into publishing at a host nobody chose.
  contentMirrorBase: import.meta.env.VITE_CONTENT_MIRROR_BASE ?? "",
  /** This operator's own provider id, assigned by DogTag at approval. Opaque; never derived. */
  providerId: import.meta.env.VITE_PROVIDER_ID ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
