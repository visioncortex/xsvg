# xsvg — Implementation Plan

A phased plan to build **xsvg** (eXtensible SVG): an XML interchange format that compiles to a
subset of SVG, written in Rust, compilable to WASM, with graded quality/approximation and an
optional WebGPU reference renderer.

This plan is grounded in [Research.md](Research.md) and realizes the [Vision](Vision.md). Read the
research digest for citations behind every library and algorithm choice referenced here.

**Guiding principle (from the research): reuse, don't reinvent.** Geometry, shaping, rasterization,
and the normalized-IR pattern all exist as mature Rust crates clustered around the
[Linebender](https://linebender.org) ecosystem. xsvg's *original* work is three things:
1. the **language** (an XML surface for typography, variable strokes, mesh gradients),
2. the **lowering passes** that turn those into a plain-SVG subset, and
3. the **quality-graded approximation** knob that all lowering shares.

**Core invariant (non-negotiable): the engine is pure Rust and stays WASM- *and* native-capable.**
The core crates depend only on portable Rust (kurbo, etc.) — **no `web-sys`, no JS, no `std`-only
assumptions** that would break either target. Anything platform-specific — and in particular
**font services** — lives *behind a trait* the core calls, with implementations supplied by thin
adapter crates. This keeps the core headless-testable and portable, and lets the font backend be
swapped without touching the pipeline.

**v0 font strategy: borrow the browser.** Rather than bundle the heavy Rust text stack
(Parley/HarfRust/Skrifa or cosmic-text) and a font loader into the WASM binary on day one, the
first version implements the font-services trait against **browser APIs via `wasm-bindgen`** — the
CSS Font Loading API (`document.fonts.check`) to *probe a named font's availability*, and canvas
`measureText` for *metrics* — and emits SVG `<text>` for the browser to shape and render. This
keeps the binary tiny and defers the bundle-size/latency unknowns.

**Cross-browser note:** xsvg documents *name* their fonts, so v0 needs only **probe-by-name +
measure**, both of which work in Chrome, **Safari**, and Firefox. Font *enumeration*
(`queryLocalFonts`, the Local Font Access API) is **Chrome/Edge-only and permission-gated — not in
Safari or Firefox** (WebKit declined it over fingerprinting), so it is an optional *enhancement* for
authoring/auto-fallback, never part of the baseline.

**v0 deployment is a single static, fully client-side web page.** No application backend, no
server-side logic, no build step at view time — just a `.html` shell + JS glue + the `xsvg` WASM
module, served as static files from **any static HTTP server** (e.g. `python -m http.server`).
(`file://` is *not* a requirement — and is best avoided, since `fetch`/`WebAssembly.instantiateStreaming`
of the `.wasm` want a real origin.) The page takes xsvg input (textarea / file picker / fetched
`.xsvg`), compiles it to an SVG string **entirely in the browser** via WASM, and renders it by
injecting the SVG into the DOM. So **the browser's own SVG
engine is the v0 renderer/viewer** — xsvg ships *no* custom rasterizer for v0 (`tiny-skia` is
test-only; the WebGPU renderer is Phase 4). This is precisely why the browser-font path is coherent:
the emitted `<text>` is shaped and drawn by the same DOM that probed and measured the fonts.

**Embeddable by design — minimal scaffolding around the custom XML.** The viewer is a thin web
component `<xsvg-view>`, registered by a single `xsvg.js` that loads the WASM once. A page is then
just the custom XML wrapped in scaffolding:

```html
<!doctype html>
<script type="module" src="xsvg.js"></script>
<xsvg-view>
  <script type="application/xsvg+xml">
    <xsvg viewBox="0 0 200 120"> … custom xml … </xsvg>
  </script>
</xsvg-view>
```

The element compiles its xsvg via WASM and renders the SVG into its **shadow root** (style-isolated,
sized to the `viewBox`). Embedding options: `<iframe src="…">` for hard isolation, *or* drop the
element straight into a host page (shadow DOM already isolates it), *or* point it at an external doc
with `<xsvg-view src="diagram.xsvg">`. **Parser caveat:** inline xsvg must live inside the
`<script type="application/xsvg+xml">` data island (or load via `src`) — raw `x:`-namespaced XML
placed directly in the HTML body would be mangled by the HTML parser; the data island keeps the
custom XML opaque text until the WASM parses it as XML. The pure-Rust outlining backend
(skrifa → kurbo `<path>`, for self-contained output and headless/CLI use) is a *second*
implementation of the same trait, added later (Phase 2b).

---

## 1. Architecture

### 1.1 Pipeline

The proven [usvg → resvg](Research.md) two-stage shape, generalized to N quality levels at one
lowering boundary:

```
 .xsvg (XML)
     │  parse            (roxmltree)
     ▼
   AST                   faithful, namespaced syntax tree
     │  resolve          (cascade styles, units, defs/use, inheritance)
     ▼
   HIR  ── "rich scene"  text runs, variable-stroke specs, mesh patches are FIRST-CLASS
     │
     │  LOWER  ◄──────── QualityProfile   ← the single approximation boundary
     │   • text     → [FontProvider] measure/shape → <text> (v0, browser) | outline→<path> (skrifa, 2b)
     │   • vstroke  → stroke-to-fill (kurbo)         → <path>
     │   • mesh     → subdivide/triangulate/raster   → <path>+ | <image>
     ▼
   LIR  ── "micro-xsvg"  strict, fully-resolved SVG subset (model: usvg::Tree)
     │
     ├─ emit ─────────────► SVG (the subset)            ← primary backend
     └─ render ───────────► WebGPU reference (vello/wgpu) ← optional, ground truth
```

**Key decisions baked in:**
- **kurbo `BezPath` is the universal geometry currency.** Stroke expansion (kurbo), glyph outlines
  (skrifa → kurbo), and mesh edges all speak the same curve type, and `BezPath::to_svg` is the
  emitter's path serializer. This is the single biggest simplifier the research surfaced.
- **One lowering boundary, one quality knob.** Everything approximate happens in LOWER, parameterized
  by `QualityProfile`. The LIR below it is exact and trivially serializable.
- **LIR is a strict SVG subset** (no variable strokes, no mesh, optionally no text) — so emission is
  near-mechanical and the same tree feeds the GPU renderer.
- **Font services are a trait, not a dependency.** The text-lowering pass calls a `FontProvider`
  (font introspection + glyph/run metrics) it does not own. v0 = a browser-backed impl
  (`wasm-bindgen`/web-sys); later = a pure-Rust impl (skrifa). The core never imports a browser
  crate.

### 1.2 Crate layout (Cargo workspace)

```
xsvg/
├─ crates/                          ── PURE-RUST CORE (WASM + native; no web/JS deps) ──
│  ├─ xsvg-core      // shared types: geometry re-exports (kurbo), Color, Transform,
│  │                 //   QualityProfile, and the FontProvider trait (defined, not impl'd here)
│  ├─ xsvg-syntax    // XML → AST (roxmltree); namespaces; error spans
│  ├─ xsvg-hir       // high-level scene IR + resolve/cascade (defs, use, units, inheritance)
│  ├─ xsvg-lir       // low-level normalized SVG-subset tree (model: usvg-tree)
│  ├─ xsvg-lower     // lowering passes: stroke→fill, text→(text|outline), mesh→approx
│  ├─ xsvg-emit      // LIR → SVG string (xmlwriter)
│  ├─ xsvg-render    // tiny-skia raster (tests/fallback); later vello GPU reference
│  ├─ xsvg-cli       // `xsvg compile …`  (uses the native FontProvider)
│                                   ── ADAPTERS (platform-specific FontProvider impls) ──
│  ├─ xsvg-wasm      // wasm-bindgen entry: compile(input, quality) -> svg
│  │                 //   + BrowserFontProvider (web-sys: document.fonts.check + canvas
│  │                 //   measureText; queryLocalFonts only as a Chrome/Edge enhancement). v0 here.
│  └─ xsvg-text      // (Phase 2b) native FontProvider: skrifa/Parley → kurbo outlines
└─ tests/golden/     // snapshot/golden-image harness
```

Rationale: small crates draw the stage boundaries the pipeline already has and keep WASM builds lean
(the binary pulls `syntax + hir + lir + lower + emit + wasm`, leaving `render`/GPU and the native
text stack out). The **core/adapter split is load-bearing**: `xsvg-core … xsvg-emit` never depend
on `web-sys`, so they compile identically for native and WASM; the `FontProvider` trait is the only
seam where platform fonts enter, and v0 satisfies it from the browser while the CLI gets a native
impl later.

**The `FontProvider` seam** (in `xsvg-core`, pure Rust, sketch):

```rust
pub trait FontProvider {
    /// Introspection: is this family/style available? what should we fall back to?
    fn resolve_family(&self, request: &FontRequest) -> ResolvedFont;
    /// Metrics for layout: advances + ascent/descent for a run, at a size.
    /// Enough for greedy line breaking, polygon fitting, and run positioning.
    fn measure_run(&self, font: &ResolvedFont, text: &str, size: f32) -> RunMetrics;
    /// Optional: glyph outlines as kurbo BezPath. v0 browser impl returns None
    /// (→ emit <text>); the native skrifa impl (2b) returns Some (→ outline to <path>).
    fn outline_run(&self, font: &ResolvedFont, text: &str, size: f32) -> Option<Vec<BezPath>>;
}
```

- **`BrowserFontProvider`** (v0, `xsvg-wasm`): `resolve_family` via `document.fonts.check`,
  `measure_run` via canvas `measureText` (both cross-browser inc. Safari); `outline_run` → `None`,
  so text lowers to `<text>`. (`queryLocalFonts` enumeration, if present, only enriches fallback.)
- **`NativeFontProvider`** (Phase 2b, `xsvg-text`): skrifa/Parley; `outline_run` → `Some`, so text
  lowers to outlined `<path>` for self-contained, font-independent output.

The lowering pass branches only on whether `outline_run` returns `Some` — the rest of the pipeline
is identical across backends.

### 1.3 The QualityProfile

A single struct threaded into every lowering pass. Discrete named profiles + override knobs:

| Profile | Stroke/curve tolerance | Mesh strategy | Text | Raster fallback |
|---|---|---|---|---|
| `highest` | very tight | many flat patches / fine triangles | outlined precisely | only when forced |
| `balanced` *(default)* | moderate | medium subdivision | outlined | mesh only, above threshold |
| `fast` | coarse | coarse subdivision | outlined | mesh eagerly |
| `raster` | — | rasterize to `<image>` | outlined | aggressive |

Orthogonal toggle: **`preserve-text`** — emit `<text>` instead of outlines (smaller, selectable,
but font-dependent). Note: under the **v0 browser `FontProvider` this is effectively forced on**
(it can't produce outlines); the toggle only becomes a real choice once the native skrifa backend
(Phase 2b) can outline. The knob is principally (a) kurbo subdivision **tolerance** (cheap to tighten
thanks to O(n⁶) scaling), (b) mesh recursion depth / color-delta (Poppler-style: depth 6,
delta 1/256), and (c) vector-vs-raster thresholds.

### 1.4 Backends

- **SVG emitter (primary):** LIR → static SVG subset (resvg's scope: no script/animation/events,
  no meshgradient). The exact allow-list is a Phase-5 deliverable; start from usvg's node set.
- **WebGPU reference renderer (optional, Phase 4):** consumes LIR *with mesh patches preserved* for
  ground-truth/preview. Candidate: [vello](https://github.com/linebender/vello) on wgpu (or a
  thin custom wgpu renderer if vello's scene model is too heavy). Doubles as the golden reference
  the approximation pipeline is measured against.

### 1.5 Module topology — many WASM modules, JS as orchestrator

The engine is **not necessarily one WASM binary.** The core pipeline ships as one lean pure-Rust
module (the v0 payload), and heavy or non-Rust capabilities are **separate WASM modules, lazily
loaded and threaded together by JS.** The decision rule:

> **In-core** for light, pure-Rust capabilities; a **separate WASM module** only for *heavy* or
> *non-Rust* engines.

**The trait seams *are* the module boundaries.** A `FontProvider` or boolean-backend implementation
is satisfied either (a) in-process by pure Rust, or (b) by a thin shim that calls **out through JS to
a capability module**. So this is an evolution of the seam design, not a new architecture — and the
core never gains a `web-sys`/JS dependency (the shim lives in the adapter layer).

**This is the relief valve for the pure-Rust-vs-industrial tension.** An industrial C++ engine —
Skia `SkPathOps` for booleans, HarfBuzz for shaping — compiled to *its own* WASM module and wired in
by JS keeps the core pure and the v0 bundle tiny; it's an optional plugin a document pulls only if it
needs it.

| Module | Phase | In/out of core | Engine |
|---|---|---|---|
| **core** (parse→HIR→lower→LIR→emit) | 0 | — | pure Rust |
| boolean (light) | 1 | in-core | `i_overlay`/`flo_curves` (Rust) |
| boolean (industrial, optional) | later | **module** | Skia `SkPathOps` (C++→wasm) |
| text shaping/outlining | 2b | in-core *or* **module** | skrifa/Parley (Rust, in-core) · HarfBuzz (C++→**module**) |
| mesh lowering + raster fallback | 3 | in-core | Rust (subdivision/triangulation) + tiny-skia |
| GPU reference renderer | 4 | **module** | Vello/wgpu (own WebGPU path) |

**Constraints that shape this (don't skip):**
- **Modules don't share memory by default** — data crosses via JS (a copy) or an explicit shared
  `WebAssembly.Memory`. So keep boundaries **coarse**: pass whole scenes / path-sets / text blocks,
  **never per-glyph or per-segment**, or the copy cost dominates.
- **Real multithreading** (WASM threads / `SharedArrayBuffer`) needs **cross-origin isolation
  (COOP/COEP headers)**. Serving over a static HTTP server fixes WASM *loading*, but a **vanilla
  `python -m http.server` does *not* send COOP/COEP** — threads need a server configured to send
  those two headers (trivial, but not the one-liner default), and for iframe embedding the **host
  page must be cross-origin-isolated too** (outside xsvg's control). So **parallelism stays a
  deployment-gated enhancement**; module *orchestration* and **Web-Worker offload (copy-based, no
  shared memory)** work on any plain static server and keep the UI responsive during heavy compiles.
- **Each module needs a stable ABI.** Pragmatic now: `wasm-bindgen` per module + a compact binary
  interchange for the IR/paths. Forward-looking: the **WebAssembly Component Model / WIT** for typed
  composition (today still transpiled to core wasm + JS glue via `jco` — tooling maturity is the
  watch-item). **[Decision D3]**

This keeps v0 a single tiny module while making the heavy/optional/non-Rust capabilities pluggable
without ever compromising the pure-Rust core.

---

## 2. The xsvg language

Namespaced XML: a **familiar SVG-like core** so simple documents are almost plain SVG, plus an
`x:` extension namespace for the three pillars. Authoring stays close to SVG; the compiler does the
hard lowering.

### 2.1 Core (SVG-like, mostly pass-through to LIR)

`<xsvg viewBox quality>`, `<g transform>`, `<path>`, `<rect>/<circle>/<ellipse>/<line>/<polygon>`,
`<defs>`, `<use>`, `<linearGradient>/<radialGradient>`, `fill`/`stroke`/`opacity`/`clip`/`mask`.

```xml
<xsvg xmlns:x="https://xsvg.dev/ext" viewBox="0 0 200 120" quality="balanced">
  <g transform="translate(10,10)">
    <rect x="0" y="0" width="60" height="40" rx="4" fill="#e23"/>
    <path d="M0,0 C40,0 40,80 80,80" fill="none" stroke="#148" stroke-width="3"/>
  </g>
</xsvg>
```

### 2.2 Pillar 1 — Typography

The text surface follows a **progressive-adoption ladder** (full spec in **[Syntax.md](Syntax.md)**):
`inline-size` on `<text>` (add one attribute, wraps) → `<textArea>` (swap tag, box) → `<x:textbox>`
(diagram ergonomics: padding, align/valign, **shrink-to-fit** `fit`, bind to a shape via `in="#…"`).
The richest form is a flow region with paragraphs and styled runs:

```xml
<x:textflow region="#blob" justify="knuth-plass" line-height="1.4">
  <x:p drop-cap="3">Lorem ipsum dolor sit <x:run fill="#c00" stroke="#400" stroke-width="0.3"
       glyph-x-scale="0.9">amet</x:run>, consectetur…</x:p>
</x:textflow>
```
Attributes: `region` (flow into arbitrary polygon), `justify` (`greedy` | `knuth-plass`),
`line-height`, `drop-cap`, per-`<x:run>` `fill`/`stroke`/`font`/`glyph-x-scale`. Lowers to outlined
`<path>` runs (or `<text>` under `preserve-text`).

> The full set of typesetting capabilities — borrowed from Adobe Illustrator (authoring) and PDF
> (placement/imaging), tiered Core/Extended/Stretch with per-feature v0 feasibility — is enumerated
> in **[Typography.md](Typography.md)**. The signature xsvg move it unlocks: **mesh-gradient fills
> and variable-width strokes applied to glyph outlines** ("text as vector art"), available once
> Phase 2b can "create outlines."

### 2.3 Pillar 2 — Variable-width strokes

A skeleton path + a width profile (itself an interpolated/bézier curve along the parameter):

```xml
<x:vstroke d="M0,0 C40,0 40,80 80,80" fill="#222"
           width-profile="0:1  0.5:8  1:2"   <!-- (t : width) control points -->
           cap="round" join="round" taper="0.1"/>
```
Lowers via kurbo stroke-to-fill into a single filled `<path>`. (See the open join/cap risk below.)

### 2.4 Pillar 3 — Mesh gradients (Coons / tensor patches)

A grid of patches with corner colors + edge control points + **per-corner alpha** (transparency).
Adjacency is explicit, enabling intentional "cracks" (torn patches):

```xml
<x:mesh id="sky" x="0" y="0">
  <x:row>
    <x:patch>
      <x:corner color="#faf0c8" alpha="1"  c1="…" c2="…"/>   <!-- 4 corners; edges as cubic bézier -->
      …
    </x:patch>
  </x:row>
</x:mesh>
<rect width="200" height="120" fill="url(#sky)"/>
```
Data model = PDF type 6 (Coons, 12 control points) / type 7 (tensor, 16). Lowers to flat-fill
subdivision, Gouraud triangles, or a raster `<image>` per the quality profile.

### 2.5 Core — live boolean shape operators (path algebra)

A *cross-cutting* capability, not a fourth pillar: **non-destructive boolean composition** modeled on
Illustrator's **Pathfinder / Compound Shapes**. Operands stay editable in the source; the compiler
resolves them to a single flattened `<path>` (SVG has **no native boolean ops**, so this is pure
compile-time value-add).

```xml
<x:boolean op="subtract" fill="url(#sky-mesh)" fill-rule="nonzero">
  <circle cx="50" cy="50" r="40"/>
  <rect x="50" y="50" width="40" height="40"/>
</x:boolean>
```

- `op` ∈ `union` | `intersect` | `subtract` | `exclude` (core shape modes; `subtract` = first child
  − union(rest), `exclude` = XOR). Compound Pathfinder ops (`divide`/`trim`/`merge`/`crop`/
  `outline`/`minus-back`) can follow as extra `op` values.
- **Operands are geometry-only** (like `clipPath`): any shape — basic shapes, `<path>`, **nested
  `<x:boolean>`**, outlined text, or a `<x:vstroke>` outline. Their own paint is ignored.
- The element **carries the result's paint** and is itself just another shape node — so it nests
  (compound shapes) and feeds clips/masks/strokes/mesh-fills. This is what makes the three pillars
  *combinable* (e.g. subtract one glyph outline from another, then mesh-fill the result).
- **Boolean backend — behind a swappable seam (like `FontProvider`), because no pure-Rust option is
  Skia-grade.** The gold standard is **Skia's `SkPathOps`** (battle-tested at billions-of-devices
  scale), but it's C++: **in-core** it would break the pure-Rust invariant, so if industrial-grade
  booleans are ever needed it enters as an **optional out-of-core WASM module** (§1.5), never a core
  dep. (`tiny-skia`, the pure-Rust Skia subset, pointedly didn't port it.) Pure-Rust options for
  in-core, none truly industrial:
    - **`i_overlay`** *(robust default)* — purpose-built for robust polygon booleans
      (integer/fixed-grid stability, self-intersection handling; GIS/CAD-grade). Polygon-domain, so
      we flatten béziers (kurbo, at the profile tolerance) first.
    - **`flo_curves`** *(curve-exact tier, eyes open)* — boolean arithmetic **directly on béziers**
      (no flatten/refit), convenient and higher-fidelity, **but single-maintainer** (written to
      support the author's animation tool), not hardened at scale, no robustness guarantees on
      degenerate cases.
    - **kurbo-native booleans ([#277](https://github.com/linebender/kurbo/issues/277))** — ideal
      long-term home (same `BezPath` type as stroker/glyph outlines, no conversions), but not
      shipped/proven as of early 2026.
  The seam lets us start on `i_overlay`, offer `flo_curves` for curve-exact output, and adopt
  kurbo-native when it lands — without touching the language or LIR.
- **Quality knob = fidelity vs. robustness, not just speed:** robust path → flatten at the profile
  tolerance → `i_overlay` polygon boolean → polyline path; curve-exact path → bézier booleans
  (`flo_curves` / future kurbo), curves preserved but less battle-tested. Result is a kurbo
  `BezPath` → `BezPath::to_svg`.
- *Not* Patrick Walton's "Pathfinder" GPU renderer — a rendering layer (Vello is its successor; see
  Phase 4). This feature is Illustrator-Pathfinder-style **geometry**.

> Surface syntax is provisional and will be refined alongside the engine (the chosen scope is
> *language + engine*, co-designed). The stable parts are the semantic models: flow region, width
> profile, patch mesh, and the boolean operator tree.

---

## 3. Phased roadmap

Sequencing rationale: **core engine first**, then pillars ordered by *dependency surface and
de-risking value* — strokes (smallest surface, exercises the geometry core + quality machinery),
then typography (hardest, highest demand), then mesh (most novel lowering), then the optional GPU
renderer.

### Phase 0 — Foundations *(the first milestone; see §4)*
Workspace, CI, WASM target. Minimal `core → syntax → hir → lir → emit` pipeline for basic shapes +
constant-width stroke-to-fill via kurbo. Golden-image test harness (render emitted SVG with
tiny-skia/resvg, compare). Proves the geometry core, the quality knob, and the WASM path end-to-end.

### Phase 1 — Variable-width strokes (Pillar 2) + path algebra
Width-profile model in language + HIR. kurbo-based variable stroke expansion; caps; taper.
**Confront the open problem:** correct variable-width joins (miter/round/bevel). Quality knobs for
stroke tolerance. *Chosen first* because it has the smallest dependency surface (just kurbo) and
maximizes early visual payoff while hardening the IR + quality plumbing.

**Also lands here: live boolean shape operators** (§2.5) — same pure-geometry-over-kurbo layer,
broadly reused by clips/masks, compound shapes, and (later) glyph booleans. Boolean backend is
**behind a swappable seam** (no pure-Rust option is Skia-grade): **`i_overlay`** as the robust
default (flatten-first), `flo_curves` for curve-exact output (single-maintainer, eyes open),
kurbo-native ([#277](https://github.com/linebender/kurbo/issues/277)) the adopt-when-shipped target;
*not* C++ Skia `SkPathOps` (breaks the pure-Rust core). The quality knob trades fidelity vs.
robustness. Doing it now turns SVG's missing-booleans gap into an xsvg feature before the pillars
that depend on it.

### Phase 2 — Typography (Pillar 1)

Split into a browser-backed first version and a pure-Rust upgrade, behind the one `FontProvider`
seam. The *layout logic* (line breaking, polygon fitting, run positioning, drop caps, per-run
fill/stroke, per-glyph scaling) lives in `xsvg-lower` and is **written once** — it consumes
metrics from whichever provider is active.

**Phase 2a — browser-backed v0** (no Rust text stack, tiny WASM):
- `BrowserFontProvider` via `wasm-bindgen`/web-sys: availability via `document.fonts.check` and
  metrics via canvas `measureText` (→ advances/ascent/descent) — both work in Safari & Firefox.
- Layout pass: paragraph flow, greedy line breaking, line-height, drop caps, per-run fill/stroke,
  per-glyph width scaling (`textLength`/`lengthAdjust` or transform), region-aware fitting into an
  arbitrary polygon (using measured run widths).
- Emit `<text>`/`<tspan>` with explicit positions — the browser shapes & renders. Output is *not*
  self-contained (depends on viewer fonts); that's the accepted v0 trade-off.
- **Baseline = probe-by-name (`document.fonts.check`) + `measureText`**, which works in Chrome,
  Safari, and Firefox. **`queryLocalFonts` (enumeration) is Chrome/Edge-only — not Safari/Firefox**
  — so it's a feature-detected enhancement, never required. If a named font isn't available, fall
  back via the document's font stack (the same `measureText` width-comparison heuristic that probes
  availability), not via enumeration.

**Phase 2b — pure-Rust outlining** (self-contained output + headless/CLI):
- **Spike first:** Parley/HarfRust/Skrifa vs cosmic-text on WASM (bundle size, latency, control) →
  decide. Bias: Parley (shares kurbo, finer control); cosmic-text is the turnkey fallback.
- `NativeFontProvider`: shaping → `outline_run` (skrifa → kurbo → `<path>`), making `preserve-text`
  a real toggle and unlocking self-contained, font-independent SVG.
- Knuth-Plass justification (greedy already shipped in 2a); proper shaping (kerning/ligatures/bidi)
  replacing `measureText` approximations; font-loading strategy for WASM (embed/subset/fetch).

### Phase 3 — Mesh gradients (Pillar 3)
Coons/tensor patch model (PDF type 6/7) in language + HIR. Three quality-graded lowering strategies:
flat-fill subdivision (Poppler-style), Gouraud-triangle decomposition (lyon), raster fallback
(tiny-skia → embedded PNG `<image>`). Transparency handling; define "cracks"/tearing semantics for
non-conforming patches.

### Phase 4 — WebGPU reference renderer *(optional)*
wgpu/vello renderer consuming LIR (mesh patches preserved) for ground-truth comparison and
high-fidelity preview. Becomes the golden reference for the approximation pipeline.

### Phase 5 — Hardening / spec
Finalize the concrete **SVG-subset allow/deny list**; lock the xsvg language spec & docs; optimize
WASM bundle size and perf; broaden the golden-image corpus.

---

## 4. First-milestone spec (Phase 0)

A buildable v0 that proves the whole skeleton end-to-end with one real lowering pass.

### Scope
- **Primary deliverable: a static, fully client-side web page** (HTML + JS glue + WASM) that
  compiles xsvg → SVG in-browser and displays it via the DOM. No server.
- **In:** `<xsvg>`, `<g transform>`, `<rect>`, `<path>`, solid `fill`, constant-width `stroke` →
  expanded to fill via `kurbo::stroke`. The same core also wraps in a thin CLI for headless golden
  tests. `quality` knob affects stroke tolerance (observable as segment count).
- **Out (deferred):** gradients (pass-through stub), text, vstroke, mesh, clip/mask, *any* custom
  renderer (browser renders the SVG; tiny-skia is test-only; WebGPU is Phase 4).

### Minimal input → output
Input:
```xml
<xsvg viewBox="0 0 100 100" quality="balanced">
  <g transform="translate(10,10)">
    <rect x="0" y="0" width="50" height="30" fill="#f00"/>
    <path d="M0,0 C20,0 20,40 40,40" stroke="#00f" stroke-width="4" fill="none"/>
  </g>
</xsvg>
```
Output: normalized SVG where `<rect>` became a `<path>`, the stroked curve became a **filled**
`<path>` outline, transforms are applied/flattened per the LIR rules, nothing but
`<g>`/`<path fill>` remains.

### Component checklist
- `xsvg-core`: `Color`, `Transform`, `QualityProfile`, kurbo re-exports.
- `xsvg-syntax`: roxmltree parse → AST; namespace handling; error spans.
- `xsvg-hir`: resolve transforms/units/defaults → `Scene { nodes: [Group|Shape] }`.
- `xsvg-lir`: `Tree { Group, Path(fill, stroke?) }` — strongly typed, usvg-shaped.
- `xsvg-lower`: shape→path normalization; `kurbo::stroke(skeleton, &Stroke{width}, tol)` → fill path.
- `xsvg-emit`: LIR → SVG via xmlwriter + `BezPath::to_svg`.
- `xsvg-render`: tiny-skia rasterizer for the golden harness.
- `xsvg-wasm`: `compile(input: &str, quality: &str) -> String` (wasm-bindgen).
- **web shell**: `xsvg.js` registering the **`<xsvg-view>` web component** (loads the WASM once;
  reads inline `<script type="application/xsvg+xml">` or a `src` attr; compiles; renders SVG into its
  shadow root) + a minimal `index.html` demo. *The v0 viewer.* Static assets only; served from any
  static HTTP server (e.g. `python -m http.server`) — no `file://` requirement; embeddable via
  `<iframe>` or directly as the custom element.
- `xsvg-cli`: `xsvg compile input.xsvg --quality balanced -o out.svg` (headless test wrapper over
  the same core).

### Acceptance criteria
1. **A page using `<xsvg-view>` with inline xsvg renders fully client-side (served from a static file
   server, e.g. `python -m http.server`; no application backend), and the same
   page embedded via `<iframe src="…">` in a host document renders identically — verified in both
   Chrome and Safari.** The host page needs no xsvg-specific code beyond the iframe/element.
2. The **CLI produces byte-identical SVG to the in-browser WASM path** (proves a shared, pure core).
3. A **golden-image snapshot test** passes (emitted SVG rasterized via tiny-skia, pixel-diff under
   threshold).
4. Changing `quality` from `fast` → `highest` **demonstrably changes the stroked path's segment
   count** (proves the approximation knob is wired through).

---

## 5. Risks & open decisions

| # | Risk / decision | Plan |
|---|---|---|
| R1 | **Variable-width joins/caps** unsolved by prior art | Tackle head-on in Phase 1; lean on kurbo, fall back to per-segment outsetting (VASEr-style) where kurbo is weak; pin kurbo version |
| R2 | **WASM bundle size / latency** of the text stack unknown | **Sidestepped in v0** (browser `FontProvider`, no Rust text stack); spike deferred to Phase 2b before committing Parley vs cosmic-text |
| R3 | **Text-in-arbitrary-polygon + Knuth-Plass + drop caps** not in libraries | Custom layout pass in `xsvg-lower` (Phase 2a), provider-agnostic; consumes metrics from the `FontProvider` |
| R7 | **v0 `<text>` output is not self-contained** (depends on viewer fonts) and typography is browser-only until 2b | Accepted v0 trade-off; native outlining (2b) produces font-independent `<path>` output for headless/portable use |
| R4 | **kurbo stroker robustness tail** (still hardening through 2026) | Pin versions; golden tests catch regressions; treat as evolving dep |
| R5 | **Mesh "cracks" semantics** undefined | Design explicit patch-adjacency + tear model in Phase 3 |
| R6 | **Concrete SVG subset** not yet enumerated | Phase 5 deliverable; start from usvg's node set, validate cross-browser |
| D1 | Parley vs cosmic-text | Deferred to Phase 2b (v0 needs neither); decide post-spike (R2); bias Parley for shared-kurbo + control |
| D2 | vello vs custom wgpu for reference renderer | Defer to Phase 4; vello if its scene model fits, else thin custom |
| D3 | Module interface: `wasm-bindgen` + binary interchange vs WebAssembly Component Model / WIT | §1.5; start pragmatic (`wasm-bindgen`), watch component-model tooling maturity (`jco`); only matters once a 2nd module exists (Phase 2b+) |
| R8 | **Cross-module data-copy cost / threads need COOP-COEP** (vanilla `python -m http.server` doesn't send them; iframe also needs a cross-origin-isolated host) | Keep module boundaries coarse (whole scene/path-set); treat true parallelism as deployment-gated (needs a header-configured server); copy-based Web-Worker offload is the portable default (§1.5) |
