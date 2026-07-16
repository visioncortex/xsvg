import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const root = dirname(fileURLToPath(import.meta.url));

// The multi-page app lives in web/ and imports the wasm-pack output from web/pkg.
// `--target web` emits `new URL('..._bg.wasm', import.meta.url)`, which Vite handles
// as an asset natively in both dev and build (no extra plugin needed).
//
// One entry per page, as directory-style index.html files so URLs drop the
// `.html` and work on any static host (e.g. `/viewer/` → `web/viewer/index.html`,
// no server rewrites needed). The landing hub is `/`. fixtures/ and embed-demo/
// are dev-only (served by the dev server, not part of the production build). The
// barebone embed is built separately by web/vite.embed.config.ts into a single
// self-contained dist/xsvg.js.
export default defineConfig({
  root: "web",
  resolve: {
    // The web app dogfoods the published browser package. In the monorepo, resolve it
    // to the package source so dev/build need no separate package build step; the wasm
    // it imports lives at packages/xsvg-viewer/pkg.
    alias: {
      "@visioncortex/xsvg-viewer": resolve(root, "packages/xsvg-viewer/src/index.ts"),
    },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: {
      input: {
        index: resolve(root, "web/index.html"),
        viewer: resolve(root, "web/viewer/index.html"),
        playground: resolve(root, "web/playground/index.html"),
        preview: resolve(root, "web/preview/index.html"),
      },
    },
  },
  server: {
    port: 5173,
    open: true,
    // allow reading dataset/ (sibling of the web/ root) for the sample glob
    fs: { allow: [".."] },
  },
  preview: {
    port: 4173,
    open: true,
  },
});
