/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_GOV_API_BASE?: string;
  readonly VITE_GOV_API_PROXY?: string;
  readonly VITE_GOV_API_TOKEN?: string;
  readonly VITE_DEPLOYMENT_URL?: string;
  /** Set = demo/local (enables demo-only affordances like "Fill demo data"); unset = production. */
  readonly VITE_DEMO_MODE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
