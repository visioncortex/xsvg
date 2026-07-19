/// <reference types="vite/client" />

// opentype.js ships no bundled types; the compiler only narrows `parse` to a local
// shape, so an untyped ambient module suffices.
declare module "opentype.js";

// Provided by the inlineWasm plugin in vite.embed.config.ts (the single-file build):
// the wasm-pack `.wasm` as a base64 string.
declare module "virtual:xsvg-wasm" {
  const wasmBase64: string;
  export default wasmBase64;
}
