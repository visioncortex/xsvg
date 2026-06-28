import { defineConfig, type PluginOption } from "vite";

// Dev-server SPA fallback: serve the app for `/view/<name>.xsvg` URLs so the
// client can route on the path. (Pretty URLs are dev-only; `?file=` works on
// any static host.)
const viewRoutes: PluginOption = {
  name: "xsvg-view-routes",
  configureServer(server) {
    server.middlewares.use((req, _res, next) => {
      const r = req as { url?: string };
      if (r.url && r.url.startsWith("/view/")) r.url = "/";
      next();
    });
  },
};

// The SPA lives in web/ and imports the wasm-pack output from web/pkg.
// `--target web` emits `new URL('..._bg.wasm', import.meta.url)`, which Vite
// handles as an asset natively in both dev and build (no extra plugin needed).
export default defineConfig({
  root: "web",
  plugins: [viewRoutes],
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
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
