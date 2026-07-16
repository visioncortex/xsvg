import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));

// Library build. Vite handles the two wasm assets natively — the compiler's
// `new URL('..._bg.wasm', import.meta.url)` (wasm-pack --target web) and the woff2
// decoder's `import "./decompress.wasm?url"` — emitting them as hashed assets the
// consumer's bundler picks up. opentype.js stays external (a declared dependency).
export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    lib: {
      entry: resolve(here, "src/index.ts"),
      formats: ["es"],
      fileName: () => "index.js",
    },
    rollupOptions: {
      external: ["opentype.js"],
    },
  },
});
