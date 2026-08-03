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
  /**
   * The deployed contract set, for the provider self-service page. NO FALLBACK on any of them,
   * deliberately: a baked default would point this deployment at a set nobody chose for it, and
   * would stay plausible after that set was superseded. Unset makes the page report itself
   * unconfigured instead.
   */
  providerRegistryAddr: import.meta.env.VITE_PROVIDER_REGISTRY_ADDR ?? "",
  issuerFactoryAddr: import.meta.env.VITE_DOGTAG_ISSUER_FACTORY_ADDR ?? "",
  serviceDomainResolverAddr: import.meta.env.VITE_SERVICE_DOMAIN_RESOLVER_ADDR ?? "",
  providerDirectoryAddr: import.meta.env.VITE_PROVIDER_DIRECTORY_ADDR ?? "",
  // The S-17 content mirror the profile document and logo are published to and read back from.
  // Blank ships blank, like its four neighbours: a value in the template would opt every deployment
  // that copies it into publishing at a host nobody chose.
  contentMirrorBase: import.meta.env.VITE_CONTENT_MIRROR_BASE ?? "",
  // The bearer the mirror's PUT requires. Blank and fallback-free like its neighbour: the write
  // path refuses up front when it is unset, rather than aborting a publication on its first upload.
  // Reading is unauthenticated, so this gates publishing alone.
  //
  // PUBLIC BY CONSTRUCTION: vite inlines this into the shipped bundle, so every visitor holds it.
  // "Keep it secret" is advice nobody could follow here; what matters is what it GRANTS, which is
  // exactly one capability - publish bytes that hash to their own content address. It reads nothing.
  // It must be the indexer's MIRROR_INGEST_TOKEN and NEVER an oversight scope token, which would
  // put read authority over the event feed into the same public bundle.
  contentMirrorToken: import.meta.env.VITE_CONTENT_MIRROR_TOKEN ?? "",
  /** This operator's own provider id, assigned by DogTag at approval. Opaque; never derived. */
  providerId: import.meta.env.VITE_PROVIDER_ID ?? "",
  demoMode: import.meta.env.VITE_DEMO_MODE === "1" || import.meta.env.VITE_DEMO_MODE === "true",
};
