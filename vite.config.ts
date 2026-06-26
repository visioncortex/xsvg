import { defineConfig } from "vite";

// The SPA lives in web/ and imports the wasm-pack output from web/pkg.
// `--target web` emits `new URL('..._bg.wasm', import.meta.url)`, which Vite
// handles as an asset natively in both dev and build (no extra plugin needed).
export default defineConfig({
  root: "web",
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
  },
  server: {
    port: 5173,
    open: true,
  },
  preview: {
    port: 4173,
    open: true,
  },
});
