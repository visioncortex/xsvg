# xsvg — Typesetting Capabilities Catalog

A concrete enumeration of the typesetting features xsvg wants, borrowing the **authoring model from
Adobe Illustrator** and the **low-level placement/imaging model from PDF**. This expands Pillar 1 of
the [Vision](Vision.md) and feeds Phase 2 of the [PLAN](Plan.md).

**How the two references divide up:**
- **Adobe Illustrator** = the *high-level* capabilities we emulate (area type, justification engine,
  OpenType controls, the Appearance model where text carries multiple fills/strokes/effects).
- **PDF** (ISO 32000) = the *low-level* text-imaging primitives our lowering should resemble (text
  state parameters, text rendering modes, the text matrix, per-glyph positioning). PDF does **no**
  automatic line breaking — the generator does — so PDF is our *output/placement* model, not our
  layout model.
- **xsvg's differentiator:** because the engine also has variable-width strokes and mesh gradients,
  **text is just vector art** — any glyph run can take a mesh-gradient fill, a variable-width stroke,
  or a warp. Illustrator can do this only after "Create Outlines"; xsvg makes it first-class.

**Tier legend:** **C** = Core (in the Vision / essential to it) · **E** = Extended (professional
standard, strongly wanted) · **S** = Stretch (advanced / later).

**Status column** — where xsvg stands *today* (updated as the compiler grows):
- ✅ **shipped** — implemented in the compiler (or correct via clean passthrough), tested.
- ◑ **partial / passthrough-only** — renders because the attribute is forwarded to the SVG output, but
  the layout engine does not (yet) reason about it, so it may be wrong for wrapped/fitted text.
- ○ **planned** — not built yet, but feasible on the browser-`<text>` v0 path ([PLAN](Plan.md) §2 2a).
- ❌ **needs Phase 2b** — requires outlining ("Create Outlines") or a custom layout pass.

---

## A. Text containers & flow — *where text lives*

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Point type** (unwrapped text at an anchor) | AI | C | ✅ | maps to SVG `<text x y>` (passthrough) |
| **Area type** — flow inside a **rectangle** | AI | C | ✅ | shipped as `<textArea>` / `<x:textbox>`: greedy wrap from `measureText` → positioned `<tspan>`s |
| **Area type in an arbitrary polygon/path** | AI | C | ✅ | **shipped** — `<x:textbox in="#shape">` flows into the outline via a coarse browser raster (align + valign, convex-ideal); the Vision's "fit text in polygon" |
| **Auto-fit / shrink-to-fit-box** (shrink font until paragraph fits) | AI/PPT | C | ✅ | binary-search font size via `measureText`, re-wrap per trial; see [Syntax.md](Syntax.md) `fit` |
| **Type on a path** (baseline follows a curve) | AI | E | ◑ | SVG `<textPath>` exists (passthrough); align-to-path + spacing options need custom |
| Type-on-path options: align (asc/desc/center/baseline), spacing, flip, effect (rainbow/skew/3D-ribbon/stair/gravity) | AI | S | ❌ | own placement after outlining |
| **Multi-column / multi-row area** (gutter, inset) | AI | E | ○ | columns are extra rectangles in the layout pass |
| **Threaded / linked frames** (overflow A→B) | AI/ID | S | ❌ | flow continuation across containers |
| **Text wrap / runaround** (avoid an obstacle shape) | AI | E | ❌ | subtract obstacle from the flow region per line |
| **Vertical writing mode** (CJK top-to-bottom) | AI/PDF | S | ◑ | SVG `writing-mode` (passthrough); real CJK rules are Stretch |
| Inset spacing / first-baseline offset (ascent, cap-height, leading, fixed) | AI | E | ◑ | `<x:textbox>` padding + cap-height baseline shipped; full first-baseline options not exposed |

## B. Paragraph composition — *breaking, spacing, alignment*

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Alignment**: left / right / center | AI | C | ✅ | `text-anchor` per line (`text-align` / `align`) |
| **Justify** (last line left/center/right/full) | AI | C | ✅ | **shipped** — greedy full-justify via `textLength`/`lengthAdjust="spacing"`; paragraph-final lines stay ragged |
| **Justification engine** (every-line vs single-line composer) | AI | E | ❌ | greedy v0 → Knuth-Plass "every-line" in Phase 2b |
| **Justification limits**: word-spacing, letter-spacing, **glyph-scaling** each min/desired/max | AI | E | ❌ | the levers the composer adjusts to set lines |
| **Leading / line spacing**: absolute + auto (% of size) | AI | C | ✅ | line advance in layout (`line-height` / `line-increment`) |
| **Space before / after** paragraph | AI | C | ○ | layout gaps — one paragraph per element today |
| **Indents**: left, right, first-line, last-line | AI | C | ○ | per-line x-origin offsets |
| **Drop cap** (N lines, N characters) | AI | C | ○ | Vision item; oversized run, lines flow around it |
| **Hyphenation** (min word len, after-first/before-last, consecutive limit, zone, hyphenate-caps) | AI | E | ❌ | needs a hyphenation dictionary (e.g. Liang/`hyphenation`) |
| **Tabs**: left/center/right/decimal + leaders | AI | E | ○ | tab-stop resolution in layout |
| **Optical margin alignment** (hang punctuation past margin) | AI | E | ❌ | per-glyph edge adjustment |
| **Roman hanging punctuation** | AI | S | ❌ | as above |
| **Widow/orphan control**, keep-with-next, balance ragged lines | AI/ID | E | ❌ | paragraph composer constraints |
| **Baseline grid alignment** | ID | S | ❌ | snap baselines to a document grid |
| **Bidi / paragraph direction** (LTR/RTL) | AI/PDF | E | ◑ | browser does bidi (passthrough); deterministic in 2b (ICU) |

## C. Character & glyph formatting

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Font family / style / size** | AI/PDF | C | ✅ | named font (probe + measure in v0) |
| **Variable-font axes** (wght/wdth/opsz/slnt + custom) | AI | E | ◑ | CSS `font-variation-settings` (passthrough); full control in 2b |
| **Tracking** (uniform range letter-spacing) | AI | C | ✅ | **shipped** — layout-aware, kerning-preserving; PDF `Tc` / SVG `letter-spacing` |
| **Word spacing** | AI | E | ✅ | **shipped** — layout-aware, absolute; PDF `Tw` / SVG `word-spacing` |
| **Kerning**: metrics / **optical** / manual pair | AI | E | ◑ | metrics free in v0 (real `measureText`); optical & manual need shaping (2b) |
| **Horizontal scale** (glyph width) | AI/PDF | C | ✅ | **shipped** as `glyph-x-scale`; PDF `Tz` / SVG `textLength` |
| **Vertical scale** (glyph height) | AI/PDF | E | ○ | non-uniform glyph scaling |
| **Baseline shift** (super/subscript, arbitrary) | AI/PDF | C | ✅ | PDF `Ts` text-rise; SVG `dy`/`baseline-shift` (passthrough) |
| **Per-glyph rotation** | AI/PDF | E | ◑ | SVG `rotate` list (passthrough); needed for vertical text |
| **Case**: All Caps, true Small Caps, synthesized small caps | AI | E | ◑ | OpenType `smcp` (2b) vs CSS approximation |
| **Super/subscript** (OpenType `sups`/`subs` vs synthesized) | AI | E | ◑ | real glyphs need shaping |
| **Underline / strikethrough** (+ weight/offset/type) | AI/ID | E | ◑ | basic via CSS (passthrough); custom decoration in 2b |
| **OpenType — ligatures** (standard, discretionary) | AI | E | ◑ | browser applies `liga`; explicit control in 2b |
| **OpenType — contextual alternates, swashes, stylistic sets/alternates, titling** | AI | E | ◑ | `font-feature-settings` (passthrough); precise in 2b |
| **OpenType — figures** (lining/oldstyle, proportional/tabular), **fractions**, **ordinals** | AI | E | ◑ | feature flags |
| **Glyph substitution by GID / alternates palette** | AI | S | ❌ | direct glyph addressing (2b) |
| **Language / locale tagging** (affects shaping, hyphenation, features) | AI/PDF | E | ◑ | `lang`/`xml:lang` (passthrough) |
| **No-break** range (prevent wrap inside) | AI | E | ○ | layout constraint |

## D. Fill, stroke & graphic appearance — *text as vector art (xsvg's edge)*

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Per-range fill** (solid) | AI/PDF | C | ✅ | **shipped** — `<tspan fill=…>` runs inside flowed text (§6.11) |
| **Per-range stroke** (color, width, dash, join/cap) | AI/PDF | C | ◑ | Vision item; PDF `Tr 1`; whole-element SVG stroke on `<text>` (v0 basic) |
| **Fill + stroke together**, with paint order | AI/PDF | C | ◑ | PDF `Tr 2`; SVG `paint-order` (passthrough) |
| **Gradient fill** on text (linear/radial), block-wide or per-glyph | AI | E | ◑ | gradient `fill` (whole text, passthrough); per-glyph needs outlining |
| **Pattern fill** on text | AI | E | ❌ | needs glyph geometry |
| **★ Mesh-gradient fill** on text | AI+xsvg | E | ❌ | unique to xsvg — glyphs outlined (2b) then mesh-filled (Pillar 3) |
| **★ Variable-width stroke** on glyph outlines | xsvg | E | ❌ | outline glyphs (2b) → variable stroke (Pillar 2) |
| **Multiple stacked fills/strokes** (Appearance panel) | AI | E | ◑ | e.g. fat stroke behind + fill on top; layered `<use>` of the text |
| **Opacity / blend mode** per range or per fill | AI | E | ◑ | SVG opacity (passthrough); blend modes (`mix-blend-mode`) |
| **Effects**: drop shadow, outer/inner glow, blur | AI | E | ◑ | SVG filters (passthrough); some work on `<text>` |
| **Text as clip / mask** (image-through-text) | AI/PDF | E | ◑ | PDF render modes `Tr 4–7` (add to clip); SVG `clipPath`/`<text>` |
| **Knockout / reverse text** | AI | S | ◑ | compositing |
| **Create Outlines** (text → editable vector paths) | AI | C | ❌ | the Phase 2b capability; prerequisite for ★ rows above |
| **Invisible text** (present but not painted, for selection/extraction) | PDF | S | ✅ | PDF `Tr 3`; SVG opacity 0 / aria (passthrough) |

## E. Distortion & path effects

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Type on a path** | AI | E | ◑ | (also in §A) |
| **Envelope distort — warp presets** (arc, arch, bulge, flag, wave, fisheye, inflate, squeeze, twist, rise, shell, fish, stair) | AI | S | ❌ | warp outlined glyphs by a deformation field |
| **Envelope distort — mesh** (free mesh warp) | AI | S | ❌ | ties into the mesh primitive (Pillar 3) |
| **Envelope distort — top object** (fit text to a shape) | AI | S | ❌ | conform geometry to an enclosing path |

## F. The PDF text-imaging model — *our lowering target*

These are the primitives our LIR / "create outlines" path should be able to express; they double as a
checklist that the high-level features above all have a precise low-level representation.

| Primitive | PDF | xsvg note |
|---|---|---|
| **Text matrix `Tm`** (arbitrary affine on text) | §9.4.2 | rotation/skew/scale of a run = SVG transform |
| **Glyph showing `Tj` / `'` / `"`** | §9.4.3 | place a run |
| **`TJ` with per-glyph adjustments** | §9.4.3 | per-glyph kerning/positioning — what shaping produces |
| **Char spacing `Tc`, word spacing `Tw`** | §9.3.2-3 | tracking / space adjust (`Tc` shipped as `letter-spacing`) |
| **Horizontal scale `Tz`** | §9.3.4 | glyph width scaling (shipped as `glyph-x-scale`) |
| **Leading `TL`**, line moves `Td`/`TD`/`T*` | §9.3.5 | line advance |
| **Text rise `Ts`** | §9.3.7 | baseline shift |
| **Text rendering mode `Tr` 0–7** | §9.3.6 | fill / stroke / fill+stroke / invisible / + clip variants |
| **Font programs**: Type1, TrueType, **Type0/CID** (CJK), **Type3** (glyphs as content streams) | §9.6-7 | Type3 ≈ glyphs as arbitrary vector art — conceptually what xsvg outlined glyphs are |
| **Embedding / subsetting** | §9.9 | mirrors xsvg's self-contained outline output (2b) |

## G. Interchange & accessibility

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Unicode round-trip** (text stays real text in source) | PDF `ToUnicode` | C | ✅ | xsvg source is the text; preserved in output |
| **Logical reading order / structure tags** | Tagged PDF | E | ◑ | emit `aria`/`<title>`/`<desc>`; order in DOM |
| **Selectable / copyable text** in output | PDF/SVG | E | ✅ | favors `<text>` output (v0) over outlines |
| **Searchable** (text recoverable from outlined output) | PDF | E | ◑ | keep a hidden `<text>` layer when outlining |

---

## Status summary

**Shipped today (✅):** point, rectangular, **and arbitrary-shape** area type (`<x:textbox in="#shape">`
flows into a triangle/circle/polygon outline), shrink-to-fit, alignment (incl. **full-justify**),
leading, named fonts, **tracking** (`letter-spacing` + `word-spacing`, layout-aware), **horizontal
glyph scale** (`glyph-x-scale`), baseline shift, **styled runs** (per-run fill / weight / style / family
via `<tspan>`), and selectable / Unicode-round-trip text. Alongside these the compiler also ships forced breaks
(`<tbreak/>`), overflow truncation (`text-overflow`), and real browser font metrics — see
[Specification.md](Specification.md) Appendix A.

**Passthrough / partial (◑):** variable-font axes, metric kerning, OpenType features & ligatures,
bidi, gradient / stroke fills, opacity, filters, clip & mask. These render because the attribute is
forwarded to the SVG output, but the layout engine doesn't fully reason about them yet.

**Planned, v0-feasible (○):** indents, drop cap, multi-column, tabs, vertical scale, and no-break
ranges — all reachable on the browser-`<text>` path without outlining.

**Needs Phase 2b (❌ — outlining + custom layout):** Knuth-Plass justification with glyph-scaling
limits, hyphenation, optical margin alignment, the **★ mesh-gradient fill** and **★ variable-width
stroke** on glyphs, pattern fills, envelope/warp distortion, precise/optical kerning, true small caps,
glyph-by-GID access, and deterministic cross-browser layout.

The dividing line is exactly **"Create Outlines"**: everything the browser can render as live
`<text>` is v0; everything that treats a glyph as an editable vector path is Phase 2b — which is also
the gateway to xsvg's signature trick of pouring mesh gradients and variable strokes *into letters*.
