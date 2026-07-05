/// <reference types="vite/client" />

// The embed build (web/vite.embed.config.ts) inlines the compiled WASM as a
// base64 string exposed through this virtual module.
declare module "virtual:xsvg-wasm" {
  const wasmBase64: string;
  export default wasmBase64;
}

// opentype.js ships no bundled types; the compiler only touches `parse`, which it
// re-narrows to a local shape, so an untyped ambient module suffices.
declare module "opentype.js";

// The woff2 decompressor is our own vendored ESM (web/src/vendor/woff2), which is
// fully typed — no ambient declaration needed here.
