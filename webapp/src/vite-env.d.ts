/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_MCP_BASE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
