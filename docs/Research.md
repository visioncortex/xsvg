# xsvg — Prior-Art Research

Grounding research for the [Vision](Vision.md). Findings are from a multi-source,
adversarially-verified survey (24 confirmed claims, 1 refuted). Confidence is **high**
across the board — most claims rest on primary sources (papers, official crate docs/repos,
spec text) with unanimous verification votes. Citations are inline.

The headline: **almost the entire stack can be reused, and it clusters around the
[Linebender](https://linebender.org) ecosystem** (kurbo, parley, skrifa, vello) plus
resvg/usvg. The novel engineering is the *language*, the *lowering passes*, and the
*quality-graded approximation* — not the low-level geometry, shaping, or rasterization.

> **Roadmap update (v1).** The [Vision](Vision.md) has since evolved. **v1 shipped Pillar 1
> (typography), including create-outlines** (text → `<path>`, incl. Google-fonts-by-name and
> stroked outlines). The **old Pillar 2 — variable-width strokes (§1) — is deferred**: it is
> replaced as a headline pillar by a **non-affine, non-destructive geometry-transform pipeline**,
> researched in the new **[§7](#7-non-affine-non-destructive-geometry-transforms-v1-pillar-2)**.
> Pillar 3 (mesh gradients, §3) is refined to **cracks/T-junctions + feathering** and is being
> designed/implemented directly. Sources for §7 are in [Citations.json](Citations.json) under
> `addenda` (single-pass, primary-sourced; not adversarially re-voted like §§1–6).

---

## 1. Variable-width strokes

> **Status: deferred (not a v1 pillar).** The vision replaced this pillar with non-affine geometry
> transforms (**§7**). The research below stays as valid prior art — variable-width stroke may return
> as an optional feature, and its kurbo/stroke-expansion machinery is reused by outlining and by the
> geometry-transform bake (both flatten/subdivide via kurbo).

**Leading approach.** Raph Levien's Euler-spiral-based **stroke expansion** is the state of
the art for both constant- and variable-width stroke-to-fill conversion. The 2024 paper
[*GPU-friendly Stroke Expansion*](https://arxiv.org/abs/2405.00127) (Levien & Uguray, Google;
HPG 2024 Best Paper 3rd) computes approximations to parallel curves **and evolutes** (needed for
the inner offset of high-curvature strokes) and runs as a fully parallel GPU compute shader.
Critically, the fitting technique *"can be adapted to generalized strokes, for example where the
stroke width is variable… requiring only evaluators for area, moment, and distance"* — exactly
xsvg's variable-(bezier)-width-profile requirement.

**Why it's good.** Levien's cubic-Bézier parallel-curve approximation has
[**O(n⁶) error scaling**](https://raphlinus.github.io/curves/2022/09/09/parallel-beziers.html)
(halving a curve cuts error 64×), vs O(n⁴) for his earlier Euler-spiral method and O(n²) for
classic Tiller-Hanson. Practically: tiny tolerances stay cheap, so the **subdivision tolerance is
a clean quality knob** (segment count ↔ fidelity).

**Best Rust building blocks.**
- **[kurbo](https://github.com/linebender/kurbo)** — the geometry core. `offset_cubic` (parallel
  curve of a cubic) and `stroke(path, &Stroke, &StrokeOpts, tolerance) -> BezPath` /`stroke_with`
  ("expand a stroke into a fill") return a fillable `BezPath` ready for SVG `<path>` output.
  Stroking went public/faster in v0.12.0 (2025-09), refined through v0.13.1 (2026-05).
- **[lyon](https://github.com/nical/lyon)** — `StrokeOptions::variable_line_width` modulates width
  per-vertex during *tessellation* (produces GPU-direct triangle meshes, not SVG outlines).
- **[VASEr](https://github.com/tyt2y3/vaserenderer)** — per-vertex color + thickness for polylines
  via per-anchor outsetting; for curves it uses a *linear gradient of color/thickness along the
  curve* (a simpler width-profile model worth borrowing).

**Hard / unsolved parts.**
- kurbo deliberately skips the most rigorous correctness definition (no evolute in the shipped
  stroker) for speed, leaving *"a tail of robustness issues"* (kurbo #279). Treat stroke
  correctness as an **evolving dependency; pin versions.**
- **Joins/caps under *variable* width** are not fully solved by any source — lyon's docs flag it as
  a weak spot, and kurbo's variable path is less battle-tested than its constant-width stroker. This
  is xsvg's main stroke-correctness risk.
- **Trade-off:** kurbo → fill outlines (SVG-direct); lyon/VASEr → triangle meshes (GPU-direct).
  xsvg wants kurbo for the SVG backend and can reuse lyon for the WebGPU backend.

---

## 2. Typography / text layout

**Leading approach (Rust).** A layered stack:
**[Parley](https://github.com/linebender/parley)** (high-level layout: glyph x/y positioning, line
breaking, bidi) → **HarfRust** (a Rust port of HarfBuzz shaping, by Google Fonts) for shaping →
**[Skrifa](https://docs.rs/skrifa)** for glyph-outline extraction → **fontique** for font
enumeration/fallback. As of Parley 0.4 (Oct 2025) the stack switched from swash to **HarfRust +
ICU4X** ([Linebender](https://linebender.org/blog/tmil-22), [font104](https://rsheeter.github.io/font104)).

**Glyph → SVG path.** Skrifa *"converts the raw glyph representations in font files into scaled,
hinted vector paths"*; its `OutlinePen` maps directly to kurbo's `BezPath`, which serializes via
`BezPath::to_svg`. This gives an **end-to-end glyf/CFF → bézier → SVG `<path>` route using the same
kurbo curve type as the stroke engine** — a major architectural simplifier.

**Turnkey alternative.** **[cosmic-text](https://github.com/pop-os/cosmic-text)** bundles shaping
(HarfRust), custom safe-Rust layout, rendering (swash), and font loading (fontdb), with **bidi/RTL**
(via Servo's `unicode-bidi`, UAX #9) and **per-line + per-character + locale-based font fallback**
all marked done, reusing Chromium/Firefox fallback lists. Trade-off: batteries-included and faster
to integrate, but less control and it does *not* share kurbo with the stroke engine. (Caveat:
fallback can render inconsistently in practice — Bevy #16354.)

**Low-level building blocks.** **[rustybuzz](https://github.com/harfbuzz/rustybuzz)** is a complete
pure-Rust HarfBuzz shaping port (matches HarfBuzz v10.1.0, 2221/2252 tests, no C++/system linking)
but does **only shaping** — no layout, line breaking, rendering, or outline extraction (delegates
parsing/outlines to `ttf-parser`). HarfRust (fontations-based) is the newer port Parley/cosmic-text
now use; rustybuzz is older but still current.

**Hard / unsolved parts (the open questions for typography).**
- **Text flowed inside an arbitrary (non-rectangular) polygon** — confirmed sources cover line
  breaking + bidi but are *silent* on polygon-region fitting. xsvg likely needs a **custom
  region-aware line-breaking pass** on top of the chosen stack.
- **Knuth-Plass justification** vs greedy — not exposed/confirmed; greedy first, Knuth-Plass as an
  upgrade.
- **Drop caps / first-letter, per-glyph width scaling** — not confirmed as library features; treat
  as xsvg-layer features.
- **WASM cost** — no bundle-size/latency benchmarks for Parley/HarfRust/Skrifa vs cosmic-text;
  needs a spike. Font-loading strategy in-browser (embed vs subset vs fetch) is unresolved.

> **Plan decision (see [Plan.md](Plan.md) §2 Phase 2a/2b):** v0 sidesteps this entire stack — it
> uses **browser font APIs via `wasm-bindgen`** (`document.fonts.check` to probe a named font +
> canvas `measureText` for metrics — both cross-browser including **Safari**) and emits `<text>`,
> keeping the WASM binary tiny. Note: font *enumeration* (`queryLocalFonts`, Local Font Access API)
> is **Chrome/Edge-only — not Safari or Firefox** — so xsvg relies on probe-by-name + measure, not
> discovery. The Rust stack above is adopted later (Phase 2b) as a swappable `FontProvider` for
> self-contained outline output and headless/CLI use. The core engine stays pure Rust regardless.

---

## 3. Mesh gradients

**Native SVG is dead.** SVG 2 mesh gradients **never shipped in any browser**. Mesh gradients
(and hatching) were dropped from SVG 2 because W3C requires two independent implementations and
these existed only in Inkscape ([LibreArts](https://librearts.org/2018/05/gradient-meshes-and-hatching-to-be-removed-from-svg-2-0/)).
Firefox [bug 1238882](https://bugzilla.mozilla.org/show_bug.cgi?id=1238882) is **WONTFIX**. The
element name even churned `<meshGradient>` → `<mesh>` → `<meshgradient>`
([Inkscape wiki](https://wiki.inkscape.org/wiki/index.php/Mesh_Gradients)). **xsvg cannot
pass-through a mesh gradient — it must compile it to an approximation.** This makes mesh the
prime exhibit for the multi-quality lowering pipeline.

**The math (fixed by spec, no staleness risk).** Per ISO 32000-1 §8.7.4.5 (PDF shading types):
- **Type 6 = Coons patch** — a quadrilateral of 4 cubic Bézier edges = **12 control points**, 4
  corner colors (bilinearly interpolated). Surface: `S = S_C + S_D − S_B` (two linear interpolations
  between opposing edges minus the bilinear corner interpolation).
- **Type 7 = tensor-product patch** — a Coons patch **+ 4 internal control points = 16 total**.

This is the exact data model xsvg's mesh primitive should adopt
([tavmjong](http://tavmjong.free.fr/SVG/MESH/Mesh.html), corroborated by ISO 32000-1).

**Rendering strategies → xsvg quality levels.** Three proven approaches map directly onto graded
lowering:
1. **Recursive subdivision to flat-filled leaves** (Poppler `Gfx.cc fillPatch`: recurse into 4
   sub-patches until color variation < threshold or max depth; `patchMaxDepth=6`,
   `patchColorDelta=1/256`). → many flat `<polygon>`/`<path>` fills, pure scalable SVG.
2. **Gouraud-shaded triangle decomposition** (PDFBox shading 6/7 reused the type 4/5 triangle
   infrastructure: subdivide patch into a grid, emit 2 triangles/quad with interpolated corner
   colors — [pdfbox-1915](https://issues.apache.org/jira/browse/pdfbox-1915)). → GPU-direct;
   in SVG, many small gradient/flat triangles.
3. **Rasterize to an embedded bitmap** (Inkscape: *"replace a mesh by a bitmap"*). → highest
   fidelity / unsupported targets via `<image>`. (Treat as *one option*, not a mandate — the
   wiki wording is tentative.)

Low quality → flat polygons or triangles; high fidelity / hard cases → raster `<image>`; the
**WebGPU reference renderer can render patches natively** (Gouraud triangles or per-pixel eval).

**"Gradient mesh with cracks."** No source defines this term directly. In context it means
**non-conforming / torn patches** — adjacent patches whose shared edge control points or colors
don't match, producing intentional discontinuities (and the rendering "cracks" that naive
subdivision produces at T-junctions). xsvg should model patch adjacency explicitly and decide tear
semantics; this is an **open design question**, not a solved one.

**Refuted (do not assume):** the claim that the two core mesh problems are *point-in-patch
determination* and *color interpolation* was refuted 0-3. The field does **not** use a
scanline/point-in-patch model — it uses **subdivision/triangulation**. Architect accordingly.

---

## 4. SVG subset & compilation target

**Model to copy: usvg ("micro SVG").** [usvg](https://docs.rs/usvg/) *"parses an input SVG into a
strongly-typed tree where all elements, attributes, references and other SVG features are already
resolved and presented in the simplest possible form."* It is the validated template for xsvg's
**low-level IR** and for staged lowering. usvg-tree uses a small fixed set of renderable node types
(Group, Path, Image, Text, plus clip/mask/paint), maintained under Linebender.

**Target the static subset.** [resvg](https://github.com/linebender/resvg) deliberately supports
*"only the static SVG subset; i.e. no `a`, `script`, `view` or `cursor` elements, no events"* — the
natural scope for emitting low-level SVG for cross-browser display. Avoid animation, scripting,
interactivity, and (obviously) `meshgradient`.

**Open gap:** the survey did **not** produce a concrete browser-compatibility allow/deny list
beyond "static subset + no meshgradient." xsvg needs to enumerate its exact emitted-feature set
(which gradient types, filter primitives, clip/mask, blend modes) as an explicit deliverable.

---

## 5. Rust 2D vector graphics + WASM / WebGPU stack

| Crate | Role | Fit for xsvg |
|---|---|---|
| **[kurbo](https://github.com/linebender/kurbo)** | Bézier/geometry, stroke expansion, offset | **Core geometry currency** (shared with skrifa outlines) |
| **[lyon](https://github.com/nical/lyon)** | Path tessellation → triangle meshes | GPU backend; variable-width tessellation; mesh triangulation |
| **[tiny-skia](https://github.com/linebender)** | CPU rasterizer (resvg's backend) | Raster fallback + golden-image testing |
| **[vello](https://github.com/linebender/vello)** | GPU compute 2D renderer on wgpu | **WebGPU reference renderer** candidate (177fps on paris-30k); heavyweight, own scene model |
| **vello_cpu** | CPU variant of Vello | Alternative raster/test path |
| **usvg / usvg-tree** | Normalized SVG IR | **Model for xsvg LIR**; reuse for round-trip testing |

**WASM:** all of the above are pure-Rust and WASM-targetable. The unknowns are **bundle size and
latency**, especially for the text stack — needs a measured spike. Vello/wgpu gives WebGPU in the
browser for the reference renderer, but its scene model differs from SVG so it consumes our geometry
rather than our SVG.

---

## 6. Compiler / IR architecture

The proven pattern (validated by **usvg → resvg**) is a **two-stage shape**: a rich high-level model
parsed from XML, lowered into a **normalized, fully-resolved, strongly-typed low-level tree that is a
strict subset**, which the backend(s) then consume. xsvg generalizes this to **N quality levels at
the lowering boundary** — the single place where strokes are expanded, text is outlined, and mesh
gradients are subdivided/triangulated/rasterized. usvg is a working Rust reference implementation to
study directly.

---

## 7. Non-affine, non-destructive geometry transforms *(v1 Pillar 2)*

**Why it's a pillar.** SVG's `transform` is **affine-only** — `matrix(a,b,c,d,e,f)` carries an implicit
bottom row `[0 0 1]` (6 DOF), so it **cannot express perspective/projective (homography)** or freeform
**warp** on vector geometry
([MDN](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Attribute/transform)). Like mesh
gradients, xsvg therefore can't pass these through — it must **bake** them into plain-SVG geometry.

**Leading approach: a non-destructive effect stack, baked at compile time.** Keep the source geometry
plus an ordered, editable transform stack, and materialize deformed `<path>`s only when compiling —
exactly Illustrator **Envelope Distort** (Make with Warp = 15 presets: Arc/Arch/Bulge/Flag/Wave/Fish/
Rise/Fisheye/Inflate/Squeeze/Twist…; Make with Mesh; Make with Top Object; `Edit Contents` / `Release`
/ `Expand` = edit / revert / bake — [logosbynick](https://logosbynick.com/envelope-distort-in-illustrator/))
and Inkscape **Live Path Effects** (Perspective/Envelope, Bend, Lattice Deformation; stacked effects
"auto-adjust" on source change; `Object to Path` = bake —
[Inkscape manual](https://inkscape-manuals.readthedocs.io/en/latest/live-path-effects.html)).
"Expand" / "Object to Path" *is* xsvg's compiler bake step.

**The deformation models** (all are *space* deformations — they map any point, so they apply uniformly
to sampled path points):
- **Free-Form Deformation** (Sederberg & Parry, SIGGRAPH 1986): embed geometry in a lattice's local
  coordinates and deform via a trivariate tensor-product Bézier over moved control points — the
  canonical **cage / lattice** warp
  ([WPI notes](https://web.cs.wpi.edu/~matt/courses/cs563/talks/smartin/ffdeform.html)).
- **Moving Least Squares** (Schaefer, McPhail & Warren, SIGGRAPH 2006): closed-form **handle-based**
  warps in affine / similarity / rigid classes, weight `w_i = 1/|p_i − v|^(2α)`; per-point and
  real-time ([paper](https://people.engr.tamu.edu/schaefer/research/mls.pdf)).
- **Homography** for perspective (8 DOF).

**Lowering to SVG = subdivide → map → refit.** Béziers are **affine-invariant** (partition-of-unity
Bernstein basis), so affine maps *can* be applied to control points — but **non-affine maps cannot**
(error grows ~ nonlinearity × segment²)
([Bézier](https://en.wikipedia.org/wiki/B%C3%A9zier_curve)). So the fidelity-preserving bake is:
**flatten** each path to a tolerance, **map** the samples through the deformation, **refit** cubics.
kurbo's [`flatten`](https://docs.rs/kurbo/latest/kurbo/fn.flatten.html) bounds the Hausdorff distance
and *"the number of segments tends to scale as the inverse square root of tolerance"* — the clean
**graded quality knob** (halve error ≈ double segments). Reserve the raster path
([`feDisplacementMap`](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Element/feDisplacementMap),
pixel-shift `P'(x,y) = P(x + scale·(XC−½), y + scale·(YC−½))`) as a last resort — it rasterizes,
losing scalability and editability.

**Best Rust building blocks.** `kurbo` (tolerance-bounded flatten/subdivide + cubic refit) +
[`rust_mls`](https://github.com/mpizenberg/rust_mls) (MLS on *arbitrary 2D points*, not just images)
cover a WASM-friendly bake; FFD and homography are simple to evaluate over kurbo samples. This reuses
the **same kurbo geometry currency** as stroking and glyph outlines.

**Hard / open parts.** Correct **cubic refitting** after warp (segment budget vs fidelity); **join /
self-intersection** artifacts under strong deformation; and defining the **non-destructive stack**
semantics in the language — effect order, and how a warp composes with booleans, mesh-fills, and text
outlines.

---

## Caveats & time-sensitivity

- **Text ecosystem moves fast.** Parley switched swash → HarfRust + ICU4X only in Oct 2025 (v0.4);
  any pre-2025 tutorial/dependency list is stale. HarfRust is now preferred over rustybuzz (both
  current).
- **kurbo's stroker is still hardening** (robustness tail through 2026) — pin versions.
- **Mesh:** assume subdivision/triangulation, *not* point-in-patch.
- Bitmap fallback for mesh is *one option among three*, not a mandate.

## Open questions carried into the plan

1. **Non-affine warp bake** (§7): cubic **refitting** after deformation (segment budget vs fidelity),
   **join / self-intersection** artifacts under strong warp, and the **non-destructive effect-stack**
   semantics (order; composition with booleans, mesh-fill, and text outlines).
2. **WASM bundle size & latency** for Parley/HarfRust/Skrifa vs cosmic-text — needs a spike (only if
   a pure-Rust text backend is pursued; v1 ships browser + opentype.js outlining).
3. Concrete **SVG-subset allow/deny list** for the emitted output.
4. **Mesh cracks/T-junctions + feathering** — refined into a concrete design + implementation
   (owner-driven); no longer an open research question.
5. **Variable-width join/cap** geometry (miter/round/bevel) — still unsolved by sources, but
   **deferred** with the pillar (§1).
