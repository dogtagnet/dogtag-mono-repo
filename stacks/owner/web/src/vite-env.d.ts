/// <reference types="vite/client" />

interface ImportMetaEnv {
  /**
   * The owner-hidden VerificationRegistryConsent the consent-history scan reads. Blank and
   * fallback-free: see src/lib/config.ts for why a wrong value here is silent in the worst way.
   */
  readonly VITE_VERIFICATION_REGISTRY_CONSENT_ADDR?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
