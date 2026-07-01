/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_OWNER_PROVER_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
