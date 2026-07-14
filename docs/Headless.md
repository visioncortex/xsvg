# Headless compiler & browser-parity tests

xsvg compiles in two places over **one shared core** (`crates/xsvg-core`):

- **Browser** — `crates/xsvg-wasm` backs the core's platform seams (`Measurer`,
  `Shaper`, `GlyphOutliner`) with JS callbacks (canvas `measureText`, an offscreen
  path rasterizer, opentype.js). Used by the web viewer/playground/preview.
- **Native** — `crates/xsvg-cli` backs the same seams with Rust libraries and needs
  no browser: `ttf-parser` for metrics + glyph outlines, `kurbo` for the shape-flow
  rasterizer (which mirrors the browser rasterizer's row/step formulas so spans line
  up). Fonts are loaded at runtime from `--font-directory` (see below).

```
xsvg [--quality fast|balanced|highest] [--font-directory DIR] [--sourcemap] [-o OUT] INPUT
      # INPUT is a path or - for stdin; OUT defaults to stdout
cargo run -p xsvg-cli -- --font-directory assets/fonts dataset/pie.xsvg -o pie.svg
```

## Fonts

The native build can't see the browser's system/Google fonts, so it loads its own from a
directory passed with `--font-directory` (the repo ships a matching set in `assets/fonts`,
see its README). Each `.ttf`/`.otf` is classified by family name: **Anton** is matched by
name (samples bake it to outlines, so it must match the browser glyph-for-glyph);
everything else falls back to **Arimo**, which is metric-compatible with Arial/Helvetica so
line-wrapping decisions agree. Without `--font-directory` the compile still succeeds —
text just uses default metrics and stays live `<text>` (no outline baking).

## Parity test suite

`npm run compare` (dev server + Edge must be available) builds the CLI and runs
`scripts/compare.mjs`, which for each dataset sample:

1. **browser SVG** — extracted from `--dump-dom` of the preview page (the wasm
   compiler with real browser fonts): the reference.
2. **native SVG** — the `xsvg` CLI.
3. **structural diff** — the element-tag sequence with `<text>` subtrees collapsed to
   one token. Font-independent: it catches geometry/structure divergence while
   ignoring how many `<tspan>` lines the wrapping produced. Any mismatch fails.
4. **pixel diff** — both SVGs are rendered in Edge at the viewBox size and compared
   with `pixelmatch`. Reported as % differing pixels: `✓` < 1%, `~` warn, `✗` > 5%.

Geometry/gradient/outline samples (plot, pie, boolean, offset, …) are pixel-identical
(0.000%); text-heavy samples differ by a few percent — sub-pixel per-glyph jitter from
the native vs. browser font renderer, not a layout difference (wrapping still agrees).
Diff images and renders are written to a temp dir printed at the end.
