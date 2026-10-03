# Changelog

All notable changes are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
per-package [Semantic Versioning](https://semver.org/).

One version covers the whole project: the two npm packages
(**`@visioncortex/xsvg-viewer`**, **`@visioncortex/xsvg-compile`**) and every
published crate (`xsvg-gradient`, `xsvg-schema-derive`, `xsvg-schema`, `xsvg-core`,
`xsvg-plugin`, `xsvg-cli`, `xsvg-wasm`) release together, via `scripts/publish.sh`.
A release is all three of its phases — crates.io, npm, and the GitHub release
carrying the raw wasm; shipping one without the others is a broken release, not a
partial one.

## [Unreleased]

### Fixed

- **Mesh gradients render exactly in resvg.** A mesh lowers to tiny texel-aligned PNGs
  placed so that *bilinear* upsampling reproduces each region's fitted colour field, but
  the images left `image-rendering` at `auto`, and resvg's `auto` is bicubic. Measured,
  that drifted up to ~9/255 off the field. Too little to see, but no longer the field. The
  images now ask for bilinear by name and land within 0.5/255 (8-bit rounding) at 1× and
  3×, curved Coons patches included. Both spellings are emitted: resvg honours only
  `style="image-rendering:smooth"` and silently drops the attribute, while the attribute
  survives tools that strip `style`. Chromium renders the new output pixel-identically
  to the old, and Safari, which doesn't know `smooth`, keeps `auto` and looks unchanged.
- `scripts/compare.mjs` reads its dev server from `XSVG_SERVER`, and refuses a page that
  isn't xsvg's. With another project's Vite server on the default port 5173 it had been
  slicing out a 24×24 UI icon and reporting every sample as a structural mismatch.

### Added

- `npm run check:mesh` renders every region of `dataset/mesh.xsvg` through resvg and
  requires each pixel inside its clip to match the bilinear field within 1/255. Removing
  the bilinear request fails all 14 measurements. Skips cleanly without `resvg` on PATH.

## [0.2.1] — 2026-08-12

A security patch, so it reaches `^0.2.0` without anyone opting in. **Generator output
could reach the network** — the static subset stopped it executing and never stopped
it fetching, so a module could emit `<image href="https://…">`, a `javascript:` link,
a `<foreignObject>`, or a remote `url(…)` paint. That matters more than each vector
alone suggests: a container generator is handed its children *already compiled*, which
was safe only because a module has no way to transmit, and any outbound request
composes with it into exfiltration of document content with no script involved.

**A plugin that emitted a remote URL will now draw without it**, and say so in a
marker. Authored markup is deliberately untouched — an author writing a remote
`<image href>` chose it. Nothing else moves: all 51 dataset samples compile
**byte-identically** to 0.2.0, the generator-using one included, since only output
that actually reaches for a remote URL, a `<foreignObject>`, or an unscoped id is
affected.

Also fixes an inspector trap: an element edited into invisibility (`thickness="0"` on
an arrow is legitimate — it has no area) left its controls standing but inert, so the
one edit you needed to reverse was the one that removed every way of reversing it.

**The Rust crates rejoin the release here**, going 0.1.0 → 0.2.1 to match everything
else. They were last published on 2026-07-19, before the schema and plugin work
existed — so `xsvg-schema`, `xsvg-schema-derive` and `xsvg-plugin` reach crates.io
for the first time, and `xsvg-core` on crates.io stops being a version that predates
most of the compiler. 0.2.0 shipped to npm only; that was a mistake, not a policy.

### Added

- **[docs/Sandbox.md](docs/Sandbox.md) — the plugin sandbox model written down**: who is trusted
  with what, which component enforces each guarantee, and what is deliberately *not* guaranteed
  (the browser cannot bound a generator's execution time; a same-origin module without
  `integrity` is trust-on-first-use). Worth stating plainly because the guarantees were real but
  scattered across three hosts and a serializer.

### Security

- **`npm run check:sandbox` covers the layers that decide whether hostile code runs at all.**
  The output rules were well tested and the two layers underneath them were not: the browser's
  loader — the only thing standing between a document and a silently swapped remote module — had
  **no tests whatsoever**, and output hardening does nothing for a module that gets *loaded*. Now
  pinned against a real module: a matching hash loads, a wrong one is refused, cross-origin
  without a hash is refused, an unparseable location fails closed, and a host loader's own bytes
  are verified like any other. Every guard is verified load-bearing by reverting it.
- **A module `src` is refused by scheme, not by `://`.** Found by writing the test above:
  `data:` and `blob:` carry no authority, so they slipped the native hosts' check and became a
  file read reporting "no such file" for something that was never a file. Not exploitable — there
  is nothing to fetch with — but the check was narrower than the rule it documented.
- **Generator output can no longer reach the network.** The static subset stopped output from
  *executing* — `<script>`, `on*` handlers — but never from *fetching*: `<a href="javascript:…">`,
  `<image href="https://…">`, `<foreignObject><iframe>` and `fill="url(https://…)"` all passed
  through. Inside generator output, `href`/`src`/`xlink:href` and every `url(…)` are now held to
  a same-document `#fragment` or a `data:image/…` payload, `<foreignObject>` is dropped, and what
  was removed is counted in a marker rather than silently swallowed.

  This mattered more than each vector suggests alone. A **container** generator is handed its
  children already compiled — real document content entering third-party code — which was safe
  only because a module has no way to transmit. Any outbound request composes with it into
  exfiltration of document content with no script involved: read the children, encode them into a
  URL, let the renderer fetch it.
- **Generator-emitted ids are namespaced** to the element that produced them (`x-plg-<seed>-`),
  with the module's own `url(#…)` references rewritten to match. An unscoped id could shadow an
  authored one and capture the document's own paint references or `<use href="#…">`.
- Authored markup is deliberately **unchanged** — an author writing a remote `<image href>` chose
  it; someone who wrote `<clip:arrow thickness="8"/>` chose nothing of the kind.

### Fixed

- **An element edited into invisibility could not be edited back.** Dragging
  `<clip:arrow thickness="0">` to zero is legitimate — the arrow has no area, so the
  generator degrades it to a marker and it leaves the drawing — but the inspector then
  dropped its pin, leaving the controls standing and inert. The canvas offered nothing
  left to click, so the one edit the author needed to reverse was the one that removed
  every way of reversing it. The panel now pins a **source range** and treats the
  rendered element as optional: it says the element renders nothing, keeps its
  attributes editable, and re-pins the same way after a full recompile (resolving
  through the key store, which is built from source spans rather than the DOM).
- **Long attribute values are elided in the inspector.** A compiled `d` is routinely
  thousands of characters — baked outlines, a warped path — and one of them buried
  every other row. Values over 96 characters keep their head, which is what identifies
  them, and report how much was hidden; the full value is on the row's tooltip.

## [0.2.0] — 2026-08-02

A minor rather than a patch, because **the same document can compile to different
bytes**: with `sourcemap` on, `<x:table>` and `<x:pie>` wrappers now carry their own
source range (they carried none, so every edit to one silently fell back to a whole
document recompile), and connectors attached by anchor route differently. No exported
API changed shape — the viewer's exports and `<xsvg-view-interactive>`'s attributes are
identical to 0.1.6, and older 12-argument `compile_ext` calls still work — but rendered
output moving under a patch bump is not something to do to a downstream host.

The panel also offers a **different set of controls**: geometry is gone from it, and the
`x:border-*` block is new. A host building its own panel from `describe()` should note
that those border attributes carry an `ns` and must be written with the prefix the
document binds — writing them unprefixed compiles to nothing at all. Crates unchanged
at 0.1.0.

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
- **Host node keys, wired end to end** (docs/Incremental.md §3.1). An optional trailing
  `key` callback on `compile_ext` / `compile_fragment_ext` (briefly separate `*_keyed_ext`
  entry points, folded in before release) and `node_spans` in the browser and Node hosts, plus
  `NodeKeyStore` as the reference implementation: the host names every element, the compiler
  stamps the name instead of the byte offset, and an element compiles to the same bytes however
  far an edit elsewhere has moved it. The offset arithmetic moves into the host and stops
  reaching the output — no span re-stamping, and no declining a patch because a generated def id
  would go stale. On the dataset that took the fast path from 326 patches with 23 declines to
  **347 with 2**, and the 2 remaining are elements that compile to nothing rather than anything
  offsets caused. 3251 unedited elements are checked byte-stable across shifting edits.
- A keyed full compile that had to fall back to byte spans **says so in its own output** — a
  trailing `<!-- xsvg: N element(s) unkeyed … -->` comment. "Key every element" is a host
  contract nothing can enforce, so the violation is reported where the sloppy host (and its
  tests) will see it; complete hosts pay nothing, and fragment output never carries it, so
  every fragment stays a verbatim slice.
- `npm run check:dom` drives the real `patchDocument` (now its own module,
  `packages/xsvg-viewer/src/patch.ts`, with type-only imports so it runs anywhere) over a jsdom
  tree and demands the patched DOM serialize identically to a full recompile parsed fresh —
  the sibling-run walking the string harness deliberately does not share, whose failures
  otherwise only ever degrade to silent full recompiles. The incremental check also refuses
  any new call site that reads `data-xsvg-pos`/`data-xsvg-node` outside the sanctioned
  funnels, so dual-spelling handling cannot leak per call site.
- **`geometry`** on a parameter declaration, and the inspector stops offering it. Position and
  extent — `x`/`y`, `width`/`height`, `cx`/`cy`/`r`, endpoints, and polar substitutes for an
  endpoint — want handles on the canvas, not a number field beside a colour picker that fights
  the arrangement the document already expresses; the attribute table above shows the values
  regardless. What is left is the styling half, so the panel reads as a style block and moving
  something stays the source pane's job. Declared on every element we ship, built-in and
  `clipart` alike. It is a **fact, not a policy**: an editor with handles reads the same flag to
  know what its handles own, undeclared means editable, and dimensions that shape the drawing
  without placing it (`stroke-width`, `inner-radius`, `bulge`, `padding`) stay editable.
- **`x:border-width` / `x:border-color` / `x:border-opacity` are declared** (§6.17) and appear in
  the panel for `<x:textbox>` and `<x:textpath>`. The compiler had always read them straight off
  the node — and reading an attribute is invisible to a host, so the convenience worked while no
  property panel could know it existed. Anything an emitter reads but nothing declares is in the
  same position; `in` still is.
- **`ns` on a parameter declaration**, for attributes that live in another namespace. `x:border-width`
  is `border-width` *in the xsvg namespace* (the plain spellings stay reserved for raw SVG stroke
  passthrough), so a host resolves the namespace to the prefix **that document** binds and skips
  the control when it binds none — hard-coding `x:` would write an unbound prefix, which is not
  merely wrong but malformed XML.
- The panel's heading is now **attributes** rather than "parameters", matching what they are in the
  document. `ParamSpec` / `#[param]` / the manifest's `params` are unchanged: that vocabulary is
  the plugin ABI and describes the declaration, not the XML.
- **`read_only`** on a parameter declaration, with `font-family` as its first use: selecting a
  face needs it *loaded* before anything can be measured, which is asynchronous and
  document-wide, so an attribute edit alone would lay the document out in a fallback face and
  show the result as if it were real. The panel disables the control and says why.
- **`superseded_by`** on a parameter declaration: the other parameters whose presence makes it
  inert (`<clip:arrow length>` is read only in the polar form; `<x:textbox>` takes its box from
  `in="#shape"`). Carried through the manifest as `supersededBy`, and the panel disables such a
  control and names what is overriding it, instead of offering one that silently does nothing.

### Fixed

- **`<x:import>` modules could not be routed through a host loader**, which made external-WASM
  plugins impossible in any sandboxed embedder. `compileXsvg` accepted a `PluginLoader` from the
  day generators landed — its docstring even named "an embedder with its own module store" as
  the case — but no host surface forwarded it, so imports always fell back to a same-origin
  fetch of the module `src`. A VS Code webview under a strict CSP reaches neither `file://` nor
  an arbitrary origin, so the sibling `.wasm` was simply unreachable, with nothing to intercept
  it. `plugins` is now a property on `<xsvg-view>` and `<xsvg-view-interactive>` and an option on
  `createPreview`, mirroring the `loader` hook for `<use href>` deps; `PluginLoader` is exported
  from the package root so a TypeScript host can name it. Unset, behaviour is unchanged.
  `npm run check:hosts` pins every host hook to every surface — a hook that reaches no surface
  is indistinguishable from no hook at all, and fails silently rather than loudly.
- **Regression: `route="curve"` sent same-side anchors on a detour** instead of arcing.
  `#a:left-top → #b:top` — the "both ends leave the top and arc over" idiom — routed out past
  the far box and doubled back, drawing two brackets where the sample asks for two Ls. The
  elbow flip added for back-to-back endpoints tested whether an exit points *away at all*, but
  a corner anchor is only 45° off the chord and a same-side edge is 90°, and both bow round
  perfectly. Now only an exit facing nearly straight back down the chord takes the detour, with
  margin either side of a corner's −0.707 so the choice never hinges on float noise. The rail
  flip's own cases are unchanged.
- **A forced anchor now decides which axis its rail leaves along.** `:top` on an `x-major` rail
  is the more specific instruction and leaves vertically; before, only the horizontal component
  was read, and a `:top` normal's zero component has `signum` +1 — so the rail confidently left
  rightward along an axis the anchor never faced. Perpendicular exits (one end horizontal, one
  vertical) route through the corner where their exit lines meet, stubbing clear first when that
  corner would land inside a box.
- **A diagonal corner anchor contributes a sign to a rail, not its 0.707 component**, which had
  been scaling one end's 18px stub to 12.7 and leaving a stair-step where the two stubs meet.
- **Rounded elbows are no longer sharp when two stubs coincide.** `round_rail` shares one radius
  across every corner, capped by the shortest segment a corner eats into — so the zero-length
  segment such a rail emitted pinned that radius to 0 and left the whole path sharp. Repeated and
  straight-through waypoints are dropped before the fillets are laid.
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

[Unreleased]: https://github.com/visioncortex/xsvg/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.2.1
[0.2.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.2.0
[0.1.6]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.6
[0.1.5]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.5
[0.1.4]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.4
[0.1.3]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.3
[0.1.2]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.2
[0.1.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.1
[0.1.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.0
