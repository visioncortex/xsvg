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
- The compiler **describes its own elements** through the same declarations, exposed as
  `describe()`, so the viewer merges built-ins and plugin manifests into one lookup and a
  property panel treats both tiers identically. **All eleven `x:` elements** are converted, and
  none has a declared parameter its emitter does not read.
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
- **Live editing** — an edit now re-emits only the elements it can have changed and patches
  them into the rendered document, instead of recompiling and replacing the whole drawing
  (docs/Incremental.md §4). Pan/zoom, the source pane, the slide deck, the pinned selection
  and the control under the pointer all survive an edit, so a parameter with a declared
  `min`/`max` gets a slider that **commits as it moves**. When the fast path declines, the
  full recompile now also preserves pan/zoom, so the fallback costs latency and nothing else.
- **Incremental compilation from Node** — `compileFragment`, `fragmentRange`, `dependents` and
  `normalizeSource` in `@visioncortex/xsvg-compile`, and `CompileSession` (via `compileXsvg`'s
  `onSession`) in the browser. A session holds the document's already-compiled generator
  modules and resolved dependencies, so re-emitting a fragment costs one parse plus one
  element's lowering, with no fetching and no module compilation.
- `npm run check:incremental` reconstructs a patch the way the viewer does, headlessly, and
  requires the result to be byte-identical to a full recompile — 326 patches across the whole
  dataset, 214 of them length-changing (so every later offset moves). It found all three of the
  bugs fixed below, and reports its own length-changing count because the first version of it
  did not: nine in ten arithmetic edits keep their width and shift nothing, so it passed
  confidently while barely exercising the mapping it exists to check.
- **Host node keys, wired end to end** (docs/Incremental.md §3.1). `compile_keyed_ext` /
  `compile_fragment_keyed_ext` and `node_spans` in the browser and Node hosts, plus
  `NodeKeyStore` as the reference implementation: the host names every element, the compiler
  stamps the name instead of the byte offset, and an element compiles to the same bytes however
  far an edit elsewhere has moved it. The offset arithmetic moves into the host and stops
  reaching the output — no span re-stamping, and no declining a patch because a generated def id
  would go stale. On the dataset that took the fast path from 326 patches with 23 declines to
  **347 with 2**, and the 2 remaining are elements that compile to nothing rather than anything
  offsets caused. 3251 unedited elements are checked byte-stable across shifting edits.
- **`read_only`** on a parameter declaration, with `font-family` as its first use: selecting a
  face needs it *loaded* before anything can be measured, which is asynchronous and
  document-wide, so an attribute edit alone would lay the document out in a fallback face and
  show the result as if it were real. The panel disables the control and says why.
- **`superseded_by`** on a parameter declaration: the other parameters whose presence makes it
  inert (`<clip:arrow length>` is read only in the polar form; `<x:textbox>` takes its box from
  `in="#shape"`). Carried through the manifest as `supersededBy`, and the panel disables such a
  control and names what is overriding it, instead of offering one that silently does nothing.

### Fixed

- The incremental dependency scan **missed connectors attached by anchor**. A `<x:connector>`
  bakes its endpoint boxes into the routed path, but an endpoint written `from="#box:right"` was
  compared whole against element ids, so `#box:right` never matched `box` and editing the box
  reported no dependents. An editor moving a shape would leave every connector anchored to it
  drawn to where it used to be. Both spellings are now reported.
- **Composite emitters were unaddressable for incremental re-emission.** `<x:table>` and `<x:pie>`
  wrapped their output in a bare `<g>`, tagging only the cells and slices inside it, so no element
  carried the range `fragment_range` reports and a host could not find what to replace — every
  edit to a table or pie silently degraded to a whole-document recompile. Both wrappers now carry
  their element's own span alongside the finer per-cell mapping, and the rule is pinned as a
  normative invariant (docs/Incremental.md §4.1) rather than left to each emitter to remember.
  Affects `sourcemap` output only; compiled output is byte-identical.
- An incremental patch could **strand a generated def id**. Ids the compiler invents (`x-flt-…`,
  `x-mesh-…`, `x-mgc-…`) seed on the node's byte start, so a length-changing edit renames every
  one after it — and a fragment the patch does not re-emit keeps the old name. It still renders
  (the def and its `url(#…)` move together), but a re-emitted dependent can land on an id an
  untouched fragment still holds, and a duplicate id resolves to the wrong element. The viewer
  now declines to patch in that case rather than reimplement the compiler's id scheme; `NodeKeys`
  (docs/Incremental.md §3.1) removes the offset-dependence for good and is the real fix.
- Source ranges were **off by ten bytes per `-x-google-` font marker** in the interactive viewer:
  the compiler is handed the source with those markers stripped, but the inspector indexed the
  original text, so on the affected documents clicking an element highlighted the wrong source and
  vice versa. The viewer now takes the text the compiler actually saw as its source of record.
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
