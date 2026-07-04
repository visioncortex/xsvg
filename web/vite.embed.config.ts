import { defineConfig, type Plugin } from "vite";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url)); // the web/ directory

// Inline the wasm-pack `.wasm` as a base64 string via a virtual module, so the
// embed bundle ships as a single self-contained file (no sibling `.wasm` to serve).
// The `transform` hook also strips the dead `new URL('..._bg.wasm', import.meta.url)`
// reference from the wasm-pack glue: the embed inits synchronously via `initSync`
// and never calls the async default `init()`, so leaving that reference in would
// make Vite emit or inline a *second* copy of the wasm. `enforce: "pre"` runs it
// before Vite's own import-meta-url asset handling.
function inlineWasm(): Plugin {
  const id = "virtual:xsvg-wasm";
  const resolved = "\0" + id;
  return {
    name: "xsvg-inline-wasm",
    enforce: "pre",
    resolveId(source) {
      if (source === id) return resolved;
    },
    load(thisId) {
      if (thisId === resolved) {
        const wasm = readFileSync(resolve(here, "pkg/xsvg_wasm_bg.wasm"));
        return `export default ${JSON.stringify(wasm.toString("base64"))};`;
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

// A separate build from the multi-page app (vite.config.ts): library mode, single
// IIFE, everything inlined. Writes into the shared dist/ without wiping it.
export default defineConfig({
  root: here,
  plugins: [inlineWasm()],
  build: {
    outDir: resolve(here, "../dist"),
    emptyOutDir: false,
    target: "es2022",
    lib: {
      entry: resolve(here, "src/embed/index.ts"),
      formats: ["iife"],
      name: "XsvgEmbed",
      fileName: () => "xsvg.js",
    },
    rollupOptions: {
      output: { inlineDynamicImports: true },
    },
  },
});
