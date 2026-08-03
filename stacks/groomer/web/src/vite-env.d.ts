/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_GROOMER_API_BASE?: string;
  readonly VITE_CENTRAL_API_BASE?: string;
  readonly VITE_REOWN_PROJECT_ID?: string;
  readonly VITE_DEPLOYMENT_URL?: string;
  readonly VITE_GROOMER_API_PROXY?: string;
  readonly VITE_ROAX_RPC?: string;
  readonly VITE_DOGTAG_ISSUER_ADDR?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
