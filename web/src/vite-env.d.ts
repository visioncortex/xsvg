/// <reference types="vite/client" />

// The embed build (web/vite.embed.config.ts) inlines the compiled WASM as a
// base64 string exposed through this virtual module.
declare module "virtual:xsvg-wasm" {
  const wasmBase64: string;
  export default wasmBase64;
}
