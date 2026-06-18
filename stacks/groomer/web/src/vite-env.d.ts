/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_GROOMER_API_BASE?: string;
  readonly VITE_CENTRAL_API_BASE?: string;
  readonly VITE_REOWN_PROJECT_ID?: string;
  readonly VITE_DEPLOYMENT_URL?: string;
  readonly VITE_GROOMER_API_PROXY?: string;
  readonly VITE_ROAX_RPC?: string;
  readonly VITE_DOGTAG_ISSUER_ADDR?: string;
  readonly VITE_ISSUER_REGISTRY_ADDR?: string;
  readonly VITE_DOGTAG_SBT_ADDR?: string;
  readonly VITE_VERIFICATION_REGISTRY_ADDR?: string;
  readonly VITE_POSEIDON6_ADDR?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
