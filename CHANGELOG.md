# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
per-package [Semantic Versioning](https://semver.org/).

Versions below track the npm packages — **`@visioncortex/xsvg-viewer`** and
**`@visioncortex/xsvg-compile`** release in version-paired lockstep. The Rust
crates (`xsvg-gradient`, `xsvg-core`, `xsvg-cli`, `xsvg-wasm`) are all at **0.1.0**.

## [Unreleased]

### Added

- **Loadable geometry generators** — `<x:import ns src integrity>` binds an XML namespace to a
  compiled WASM module, so a document can use elements the compiler knows nothing about
  (procedural clipart, custom diagram shapes). Modules are pure functions from parameters to
  markup: no host callbacks, no document access, and their output is re-parsed by the compiler,
  so it cannot escape the static SVG subset. Generator elements compose like built-ins — a
  `<clip:arrow>` is a valid `<x:boolean>` operand or `<x:warp>` target — and participate in
  incremental recompilation. Container elements get their children compiled and measured first,
  so a generator can size itself to what it wraps.
  Design: [docs/Extensibility.md](docs/Extensibility.md); spec: §4.3.
- **`xsvg-plugin`** — the SDK for writing generators: implement one trait, call one macro.
- **`xsvg-schema` + `xsvg-schema-derive`** — declare an element's parameters as a Rust struct
  and derive both its machine-readable description and the code that reads it. A plugin's
  manifest is generated from the declarations, so it cannot drift from the parameters the
  module actually reads: a parameter with no field cannot be read, and a field nothing reads is
  a `dead_code` warning. (The hand-written manifest this replaced already omitted `x` and `y`.)
  The derives are compile-time only — nothing reaches the emitted wasm.
- **`clipart`**, the first official extension (`crates/ext/clipart`) — `<arrow>` (thick shafted
  arrow, one or two heads, optional outline and corner rounding) and `<callout>` (a bubble sized
  to its children). Official extensions load through the same public seam as anyone else's.
- Generators run on **all three hosts** — browser, CLI, and Node — from the same module bytes,
  which is what makes their output identical by construction rather than by matching two
  implementations. `dataset/clipart.xsvg` is in the native-vs-browser parity harness and
  matches. Each host checks `integrity` (one padded-base64 spelling works everywhere) and
  isolates modules from each other and from successive compiles; the native hosts additionally
  bound a runaway generator with wasmi fuel, which the browser cannot do on the main thread.
  Opt out with `xsvg --no-plugins` or `compile(src, { plugins: false })`.
- **Manifest-driven parameter controls** in the interactive viewer's inspector: pinning a
  generator element offers editable fields for the parameters its module declares —
  typed, defaulted, with the manifest's help text as tooltips — and an edit rewrites the
  document and recompiles, keeping the element pinned. Reading a schema runs no
  generator, so this works for elements the compiler has never heard of. Hosts can reach
  the same schemas via `compileXsvg`'s `onPlugins`.

### Fixed

- Connector rails flip to a 4-turn detour when the endpoints don't face each other with room
  (e.g. `a:right → b:left` where `b`'s left edge is at or behind `a`'s right edge). Previously the
  `x-major`/`y-major` rail always drew a 2-turn Z, which doubled the line straight back through its
  own box; it now stubs out of each box and crosses over on a line that clears both — routing
  *around* rather than through (matching Google Docs elbow connectors). The `curve` route gets the
  analogous fix: when the far endpoint is behind the exit, it becomes a **rounded elbow** (the rail
  path with filleted corners) so it routes around and meets each edge along its normal, instead of a
  single cubic cutting flat across the boxes. New `connector-rail-flip` sample compares rail vs curve.

### Added

- `createPreview` accepts the async `loader` option (`DepLoader`), matching
  `compileXsvg` and the interactive element — for embedding the preview surface
  behind an async dependency boundary.
- Cross-file linking on every embed surface: the static `<xsvg-view>` element gains a
  `base-url` attribute plus `baseUrl`/`resolve`/`loader` properties, and both React
  components (`XsvgView`, `XsvgViewInteractive`) gain `baseUrl`/`resolve`/`loader`
  props — so an inline-source embed (e.g. a docs page) can resolve `<use href>` deps.
  The React `XsvgView` also now defaults the base to the fetched `src` URL, matching
  the custom element (deps previously resolved against the page).

## [0.1.6] — 2026-07-23

From this release the two npm packages share one version; `xsvg-compile` jumps
0.1.0 → 0.1.6. Its own changes: fixes the package failing to load under Node
(`exports is not defined` — the wasm pkg is now marked CommonJS), and adds
cross-file `<use href>` linking from disk via the new `basePath` option.
Crates unchanged at 0.1.0.

### Added

- **Bordered text.** `x:border-width` / `x:border-color` (plus optional `x:border-opacity`) on any
  xsvg text element draw an outline that hugs each glyph and sits **behind** the fill, so it never
  thins the letters; `x:border-width` is the width visible outside the glyph. Works on `<x:textbox>`,
  `<x:textpath>`, and outlined text. Raw SVG `stroke*` attributes on live text also pass through now
  (previously dropped). New `bordered-text` sample. Spec §6.17.
- **Cross-file `<use>` links.** A `<use href="logo.xsvg">` (or `#id`) links another file at
  compile time — the dependency is compiled and **baked in** (whole-file as a nested `<svg>`
  viewport, `#id` as that element sized to its **own** extent: declared viewBox/width/height, else
  its drawn geometry's box incl. local `<use>` targets and stroke), so you author a logo once and
  the output stays self-contained. A DAG with cycle/depth guards; degrades gracefully; per-file
  `#id` scoping; no source map inside dependencies (the baked block maps to its `<use>` in the
  entry document). **Discovery is lazy** — the compiler finds links itself; no pre-scan anywhere.
  Sync hosts link in one pass: the CLI / `@visioncortex/xsvg-compile` read from disk on demand
  (new `basePath` option), and a `resolve` option/property links bundled in-memory deps. Async
  hosts converge via a fixpoint driver (cheap probe rounds; deps cached by canonical key, so a
  diamond fetches once): the browser default is **same-origin** `fetch` (cross-origin fails CORS →
  degrades), and a `loader` (`DepLoader`: `key`/`fetch`) option on `compileXsvg` + `loader`/
  `baseUrl` properties on `<xsvg-view-interactive>` let e.g. a VSCode extension host supply deps
  over RPC. Spec §4.2.
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

[Unreleased]: https://github.com/visioncortex/xsvg/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.6
[0.1.5]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.5
[0.1.4]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.4
[0.1.3]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.3
[0.1.2]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.2
[0.1.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.1
[0.1.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.0
