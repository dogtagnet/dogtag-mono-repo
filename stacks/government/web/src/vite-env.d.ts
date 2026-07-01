/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_GOV_API_BASE?: string;
  readonly VITE_GOV_API_PROXY?: string;
  readonly VITE_DEPLOYMENT_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
