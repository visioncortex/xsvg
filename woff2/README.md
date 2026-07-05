# woff2 → sfnt decoder (built from source)

A Vite-friendly WebAssembly WOFF2 decompressor, built from [google/woff2] +
brotli with emscripten. It replaces the `wawoff2` npm package, whose build is a
322 KB base64-inlined CJS blob with Node-only branches that Vite has to work
around. The decoder is used by the web app to turn Google Fonts' woff2 into sfnt
so opentype.js can trace glyph outlines.

## Why not just use wawoff2?

Same C++ sources and the same approach as [fontello/wawoff2]'s `src/Makefile`,
but linked with modern flags for a clean bundler story:

| | wawoff2 | here |
|---|---|---|
| module format | CJS (`module.exports`) | ESM (`export default`) |
| environments | web **+ node** (`require('fs')`, `__dirname`) | web/worker only |
| wasm delivery | `-s SINGLE_FILE=1` → base64 in JS | separate `.wasm` |
| output | 322 KB `.js` | 35 KB `.mjs` + 213 KB `.wasm` |

The separate `.wasm` lets Vite hash and stream-compile it in the app build; the
single-file embed build inlines it as a data URI (see `web/vite.embed.config.ts`).
We build only the decoder — the app never compresses.

## Rebuilding

Requires only Docker (the pinned `emscripten/emsdk` image supplies `emcc`;
nothing is installed on the host):

```sh
npm run woff2:build      # → ./woff2/build.sh
```

This builds the emscripten toolchain image (`Dockerfile`: clones woff2 at a
pinned commit + its brotli submodule, builds the static libs), links the
`decompress_binding.cc` embind binding via `Makefile`, and installs
`decompress.mjs` + `decompress.wasm` into `web/src/vendor/woff2/` (committed, so
this only needs re-running to bump versions or change flags).

[google/woff2]: https://github.com/google/woff2
[fontello/wawoff2]: https://github.com/fontello/wawoff2
