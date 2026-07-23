import { defineConfig, type Plugin } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { createReadStream, existsSync } from "node:fs";

const root = dirname(fileURLToPath(import.meta.url));

// Serve the repo's dataset/ at /dataset/* in dev, so the sample pages resolve
// cross-file <use href> links over real HTTP — the same lazy fetch path an
// embedding site would use (no bundled-resolver special case). Flat names only.
function serveDataset(): Plugin {
  return {
    name: "serve-dataset",
    configureServer(server) {
      server.middlewares.use("/dataset", (req, res, next) => {
        const name = decodeURIComponent((req.url ?? "/").replace(/^\/+/, "").split("?")[0]);
        if (!/^[\w.-]+$/.test(name) || name.includes("..")) return next();
        const file = resolve(root, "dataset", name);
        if (!existsSync(file)) return next();
        res.setHeader("Content-Type", "image/svg+xml");
        createReadStream(file).pipe(res);
      });
    },
  };
}

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
  plugins: [serveDataset()],
  resolve: {
    // The web app dogfoods the published browser package. In the monorepo, resolve it
    // to the package source so dev/build need no separate package build step; the wasm
    // it imports lives at packages/xsvg-viewer/pkg.
    alias: {
      // Subpaths first — a prefix alias would otherwise rewrite them via the bare entry.
      "@visioncortex/xsvg-viewer/element": resolve(root, "packages/xsvg-viewer/src/element.ts"),
      "@visioncortex/xsvg-viewer/interactive": resolve(root, "packages/xsvg-viewer/src/interactive.ts"),
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
