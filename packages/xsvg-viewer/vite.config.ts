import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));

// Library build. Three entry points, each self-contained (the wasm is inlined into a
// shared chunk they all import):
//   index   — compileXsvg / createPreview / helpers
//   element — the framework-free <xsvg-view> custom element
//   react   — the <XsvgView> React component (react is external / a peer dep)
// Vite handles the two wasm assets natively — the compiler's `new URL('..._bg.wasm',
// import.meta.url)` and the woff2 decoder's `import "./decompress.wasm?url"`.
// opentype.js and react stay external (declared / peer dependencies).
export default defineConfig({
  esbuild: { jsx: "automatic" },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    lib: {
      entry: {
        index: resolve(here, "src/index.ts"),
        element: resolve(here, "src/element.ts"),
        react: resolve(here, "src/react.tsx"),
      },
      formats: ["es"],
    },
    rollupOptions: {
      external: ["opentype.js", "react", "react/jsx-runtime"],
      output: { entryFileNames: "[name].js", chunkFileNames: "[name]-[hash].js" },
    },
  },
});
