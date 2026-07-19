import { defineConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url)); // packages/xsvg-viewer

// Inline the wasm-pack `.wasm` as a base64 string via a virtual module, so the single-
// file embed ships self-contained (no sibling `.wasm` to serve). The `transform` hook
// also strips the dead `new URL('..._bg.wasm', import.meta.url)` reference from the
// wasm-pack glue: the embed inits synchronously via `initSync` and never calls the
// async default `init()`, so leaving it in would make Vite inline a *second* wasm copy.
//
// The woff2 decoder (src/vendor/woff2) is reached from the compile path (Google fonts →
// outline); its `import "./decompress.wasm?url"` is resolved to an emscripten-style data
// URI so `isDataURI()` decodes it in-process and the embed stays single-file.
function inlineWasm(): Plugin {
  const id = "virtual:xsvg-wasm";
  const resolved = "\0" + id;
  const woff2WasmId = "\0woff2-decompress-wasm-url";
  return {
    name: "xsvg-inline-wasm",
    enforce: "pre",
    resolveId(source) {
      if (source === id) return resolved;
      if (source.endsWith("decompress.wasm?url")) return woff2WasmId;
    },
    load(thisId) {
      if (thisId === resolved) {
        const wasm = readFileSync(resolve(here, "pkg/xsvg_wasm_bg.wasm"));
        return `export default ${JSON.stringify(wasm.toString("base64"))};`;
      }
      if (thisId === woff2WasmId) {
        const wasm = readFileSync(resolve(here, "src/vendor/woff2/decompress.wasm"));
        return `export default "data:application/octet-stream;base64,${wasm.toString("base64")}";`;
      }
    },
    transform(code, thisId) {
      if (thisId.endsWith("pkg/xsvg_wasm.js") && code.includes("xsvg_wasm_bg.wasm")) {
        return {
          code: code.replace(/new URL\('xsvg_wasm_bg\.wasm', import\.meta\.url\)/, '""'),
          map: null,
        };
      }
    },
  };
}

// A separate build from the library entries (vite.config.ts): one self-contained IIFE
// with everything inlined. Writes `xsvg.js` into dist/ without wiping the lib output.
export default defineConfig({
  root: here,
  plugins: [inlineWasm()],
  build: {
    outDir: resolve(here, "dist"),
    emptyOutDir: false,
    target: "es2022",
    lib: {
      entry: resolve(here, "src/cdn.ts"),
      formats: ["iife"],
      name: "XsvgEmbed",
      fileName: () => "xsvg.js",
    },
    rollupOptions: {
      output: { inlineDynamicImports: true },
    },
  },
});
