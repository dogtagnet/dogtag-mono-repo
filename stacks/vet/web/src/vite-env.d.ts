/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_VET_API_BASE?: string;
  readonly VITE_CENTRAL_API_BASE?: string;
  readonly VITE_REOWN_PROJECT_ID?: string;
  readonly VITE_DEPLOYMENT_URL?: string;
  readonly VITE_VET_API_PROXY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
