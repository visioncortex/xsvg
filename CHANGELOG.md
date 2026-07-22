# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
per-package [Semantic Versioning](https://semver.org/).

Versions below track the **`@visioncortex/xsvg-viewer`** npm package — the actively
iterating artifact. `@visioncortex/xsvg-compile` and the Rust crates
(`xsvg-gradient`, `xsvg-core`, `xsvg-cli`, `xsvg-wasm`) are all at **0.1.0**.

## [Unreleased]

### Added

- **Bordered text.** `x:border-width` / `x:border-color` (plus optional `x:border-opacity`) on any
  xsvg text element draw an outline that hugs each glyph and sits **behind** the fill, so it never
  thins the letters; `x:border-width` is the width visible outside the glyph. Works on `<x:textbox>`,
  `<x:textpath>`, and outlined text. Raw SVG `stroke*` attributes on live text also pass through now
  (previously dropped). New `bordered-text` sample. Spec §6.17.
- **Cross-file `<use>` links.** A `<use href="logo.svg">` (or `#id`) now links another file at
  compile time — the dependency is compiled and **baked in** (whole-file as a nested `<svg>`
  viewport, `#id` as that element sized to its **own** extent), so you author a logo once and the
  output stays self-contained. Forms a DAG with cycle/depth guards; degrades gracefully. Resolved
  from disk in the CLI / `@visioncortex/xsvg-compile` (new `basePath` option), and **same-origin**
  `fetch` in the browser (`compileXsvg` gains `baseUrl`; cross-origin fails CORS → degrades). The
  `<xsvg-view-interactive>` / `<xsvg-view>` elements gain a `resolve` property to link against
  bundled / in-memory deps instead of fetching. Spec §4.2.
- Connectors: `from`/`to` now accept a **forced anchor** — `#id:<anchor>` where `<anchor>` is an
  edge (`left|right|top|bottom`), a corner (`left-top`…, either order), or `center` — and **raw
  coordinates** via `from-point`/`to-point`, alongside a plain `#id` (the reference wins if both
  are given). New `connector-anchors` sample.

### Changed

- Connectors: `curve` arcs now bow by a **fixed** `bulge` (default 44 px, author-settable) instead
  of an amount that scaled with the endpoint distance, and leave each anchor tilted toward the
  other end — so a same-side pair reads as a leaf rather than ballooning into a half-circle.
- Interactive viewer: the slide deck now pages on ArrowUp/ArrowDown as well as
  ArrowLeft/ArrowRight (and PageUp/PageDown) — Google-Slides-style navigation.

### Fixed

- **Cross-file `#id` collision.** The compiled-output reference memo and cycle stack are keyed by
  bare id but were shared across linked files, so a dependency resolving its own `#a` could hand
  the referrer that geometry (and a shared id could report a false cycle). Each linked file now
  gets its own scope; the work budget stays global.
- No source map inside linked dependencies: a dependency's byte ranges index *its* file, so baked
  content pointed at garbage spans of the entry source. The linked block now resolves to its
  `<use>` in the entry document.
- By-id `<use>` sizing measures a truer box: definition-only subtrees (`<defs>`, `<clipPath>`,
  `<symbol>`, gradients, …) and `display:none` elements no longer inflate it, a nested `<svg>`
  contributes its (clipping) viewport instead of unmapped viewBox content, `<image>` contributes
  its box, and `style="transform:…"` is honoured alongside the attribute.
- Interactive viewer inspector: straight/axis-aligned connectors (and any zero-area shape) now
  get a visible highlight — an SVG rect with a zero dimension isn't drawn, so the highlight band
  is inflated to a small minimum.

## [0.1.5] — 2026-07-19

### Added

- Dark mode: the interactive viewer (`<xsvg-view-interactive>`) and `createPreview`
  chrome follow the viewer's `prefers-color-scheme`. The drawing keeps its own colors.

## [0.1.4] — 2026-07-19

### Added

- `createPreview` gains a `slides` option (default `true`); set `false` to start a
  multi-artboard preview with its slide rail collapsed — the toggle button stays, so
  the viewer can open it.

## [0.1.3] — 2026-07-19

### Added

- Interactive viewer, shipped as a component: `<xsvg-view-interactive>` (`./interactive`)
  and the React `XsvgViewInteractive` — pan/zoom canvas, artboard slide-deck rail, and
  floating controls (zoom capsule, fit/actual-size).
- Opt-in `inspector`: a collapsible Inspector pane over a read-only source pane, a
  resizable gutter, and a bidirectional element↔source link. Its CodeMirror dependency
  is an **optional peer** that lazy-loads only when `inspector` is enabled, so the base
  pan/zoom viewer stays light.

### Changed

- The `/viewer` app is now a thin wrapper over the shipped component (it dogfoods it).

## [0.1.2] — 2026-07-19

### Fixed

- Rebuilt with release WebAssembly. 0.1.1 was accidentally published with a debug build
  (~8 MB unpacked); 0.1.2 is ~3.7 MB.

## [0.1.1] — 2026-07-19 · **deprecated**

Deprecated on npm — published with debug WebAssembly. Use 0.1.2 or later.

### Added

- Embeddable static viewer: the `<xsvg-view>` custom element (`./element`), the React
  `XsvgView` component (`./react`), and a self-contained single-file `dist/xsvg.js`
  (WASM inlined) for zero-build `<script>` / CDN use. Opt-in `droppable` drag-to-load.
- `publishConfig.access: public` on both npm packages.

## [0.1.0] — 2026-07-19

### Added

- Initial release.
  - **`@visioncortex/xsvg-viewer`** (browser): `compileXsvg`, `createPreview`
    (fit-to-contain + multi-artboard slide deck), `findArtboards` / `makeThumb`,
    `downloadSvg`.
  - **`@visioncortex/xsvg-compile`** (Node): browser-free `compile()`.
  - Rust crates on crates.io: `xsvg-gradient`, `xsvg-core`, `xsvg-cli`, `xsvg-wasm`.

[Unreleased]: https://github.com/visioncortex/xsvg/compare/v0.1.5...HEAD
[0.1.5]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.5
[0.1.4]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.4
[0.1.3]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.3
[0.1.2]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.2
[0.1.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.1
[0.1.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.0
