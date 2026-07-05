# xsvg Specification

> **Status: draft, best-effort, evolving.** This is the normative reference for the xsvg language and
> how the compiler lowers it to a static SVG subset. It grows as features land. Companion docs:
> [Syntax.md](Syntax.md) (design narrative & examples), [Plan.md](Plan.md) (architecture & roadmap),
> [Typography.md](Typography.md) (capability catalog), [Research.md](Research.md) (prior art).
>
> Conformance keywords **MUST / SHOULD / MAY** are used in the RFC 2119 sense, loosely. Each section
> is tagged **[implemented]**, **[spec'd]** (rules defined, not yet built), or **[planned]** (sketch
> only). A status table is in [Appendix A](#appendix-a--feature-status).

## 1. Overview

xsvg is an XML interchange format — a **graceful-degradation superset of SVG** — that a compiler
lowers to a **static SVG subset** renderable in any SVG engine. The win should cost one tag-swap or
one attribute, and nothing added can break the file. Design rationale is in [Syntax.md](Syntax.md).

## 2. Namespaces & the prefix policy [implemented]

| Namespace | URI |
|---|---|
| SVG | `http://www.w3.org/2000/svg` |
| xsvg extensions | `https://xsvg.visioncortex.org` (conventional prefix `x:`) |

The document root MAY be `<svg>` (with `xmlns:x` declared) or `<xsvg>` (an alias the compiler treats
as `<svg>`). Naming rules:

| Kind | Rule |
|---|---|
| A name SVG/CSS already defines | use it **unprefixed**, with SVG/CSS semantics (`inline-size`, `text-align`, `<textArea>`, `text-overflow`) |
| A new attribute on a standard element | **`x:`-prefixed** (`x:fit`, `x:width-profile`) |
| A new element | **`x:`-prefixed** (`<x:textbox>`, `<x:vstroke>`, `<x:mesh>`, `<x:boolean>`) |
| Attributes inside an `x:` element | **unprefixed** (the element disambiguates) |

## 3. Degradation contract [implemented]

| Input | Plain SVG viewer | xsvg compiler |
|---|---|---|
| Unknown attribute on a known element | ignores it (element still renders) | applies it |
| Unknown element (`x:` or a revived-but-unsupported SVG element like `<textArea>`) | skips it (content not rendered) | lowers it to the SVG subset |
| Everything else | renders normally | passes through |

A document MUST remain well-formed SVG/XML; the compiler MUST NOT require anything a plain SVG viewer
would reject.

## 4. Processing model [implemented]

```
parse → resolve → lower (quality-parameterized) → emit SVG subset
```

Lowering that needs **font metrics** obtains them through a host-supplied *measurer*: a function
returning, for a string at a size, its advance width and the font's vertical metrics
(`ascent`, `descent`, `cap_height`, `x_height`). In v0 the browser supplies this via canvas
`measureText`; defaults approximate `ascent 0.8em, descent 0.2em, cap_height 0.7em, x_height 0.5em`.
Architecture: [Plan.md §1](Plan.md).

Shape geometry for region flow (§6.10) is obtained through a parallel *shaper* seam: it rasterizes a
filled path into coarse per-row inside-spans. In v0 the browser supplies it (`getBBox` +
`isPointInFill`); a pure-Rust backend can slot in behind the same trait later.

**Robustness (normative).** Compilation is total on well-formed input: degenerate geometry
(zero/negative width, height, padding, or `font-size`), degenerate spacing (negative
`letter-spacing`/`word-spacing`), and pathological measurer output (non-finite or negative advances)
must never panic and must never emit `NaN`/`inf` coordinates — they collapse to empty or zero-sized
output instead. **Element nesting is bounded** at a fixed depth (v0: 512); deeper input is rejected
with an error rather than risking a parser stack overflow. Malformed XML returns an error.

## 5. Graphics elements (reused SVG) [implemented]

`<g>`, `<path>`, basic shapes, gradients, `transform`, `fill`/`stroke` are normalized or passed
through. Defined lowerings:

- **`<rect>`** with no `rx`/`ry` → `<path>`; rounded rects pass through unchanged.

## 6. Text

### 6.1 Common layout primitives [implemented]

- **Measurement.** Words are measured once at the base size; trial sizes scale widths linearly.
- **Wrapping.** Greedy (first-fit): break only at whitespace. A token wider than the available width
  is placed alone (it overflows; see §6.6). No automatic hyphenation in v0.
- **Fitting** (shrink-to-fit, `<x:textbox>` only). Binary-search the font size in
  `[fit-min, font-size]` for the largest whose wrapped block fits the box height; re-wrap each trial.

### 6.2 `<text>` and `inline-size` — Rung 1 [implemented]

Point `<text>` passes through. With **`inline-size="W"`**, the text wraps to width `W` and flows
downward; the **first line's baseline is at `y`** (the SVG `<text>` convention), and subsequent
baselines step by `line-height · font-size` (default `line-height` = 1.2). Horizontal alignment uses
the inherited `text-anchor`.

### 6.3 `<textArea>` — Rung 2, SVG Tiny 1.2 subset [implemented]

Flowed text in a region, per the SVG Tiny 1.2 Recommendation.

| Property / attr | Values | Initial | Effect |
|---|---|---|---|
| `x`, `y` | length | 0 | region corner |
| `width`, `height` | length \| `auto` | `auto` | `width:auto` ⇒ no wrap; `height:auto` ⇒ grow (no clip) |
| `text-align` | `start` \| `end` \| `center` \| `justify` | `start` | inline alignment (`justify` extends Tiny 1.2 — §6.9) |
| `display-align` | `auto` \| `before` \| `center` \| `after` | `auto` (= `before`) | block alignment |
| `line-increment` | `auto` \| `<number>` | `auto` | line-box height; `auto` = 1.1 × font-size |

**Layout.** Wrap to the width (or not, if `auto`). Baselines **step by `line-increment`** (the SVG Tiny
1.2 line-box height). `display-align` then positions the **cap-height ink band** (§6.5) —
`before`=cap-top at the top edge, `center`=band centred, `after`=band bottom at the bottom edge — so
centred text is optically centred, not biased low by the em box. With an explicit `height`, lines whose
ink band falls outside the region are **not rendered** (clipped; see §6.6).

**`<tbreak/>`** [implemented] — a forced line break, per SVG Tiny 1.2. Each `<tbreak/>` child ends the
current line and starts a new one; wrapping resumes independently on each side. Consecutive breaks
produce blank lines. This is the only child element `<textArea>` interprets in v0.

**Not yet implemented:** `xml:space="preserve"`, full UAX #14 line-breaking (we break on
whitespace), `editable`.

### 6.4 `<x:textbox>` — Rung 3, xsvg box [implemented]

Box text with diagram ergonomics. Shares `<textArea>`'s cap-height vertical model, differing in line
spacing (`line-height` vs `line-increment`) and keyword (`valign` vs `display-align`); see §6.5.

| Attr | Values | Default | Effect |
|---|---|---|---|
| `x`,`y`,`width`,`height` | length | — | box geometry (ignored when `in` is set) |
| `in` | `#id` | — | bind to a referenced shape (§6.10) |
| `padding` | length | 0 | uniform content inset |
| `align` | `start` \| `center` \| `end` \| `justify` | `start` | horizontal alignment (§6.9) |
| `valign` | `top` \| `middle` \| `bottom` | `top` | vertical alignment |
| `fit` | `none` \| `shrink` | `none` | shrink-to-fit (§6.1) |
| `fit-min` | length | 6 | font-size floor for `shrink` |
| `line-height` | number | 1.2 | line advance multiplier |

### 6.5 Vertical alignment model (normative) [implemented]

Both box elements align on the **cap-height band** — the region from the first line's **cap-top** to
the last line's **baseline + descent** — positioning it optically rather than centring the em box
(which would sit low, since ascent > cap-height). They differ only in **line spacing** and the
keyword: `<textArea>` steps by `line-increment` and takes `display-align` (`before`/`center`/`after`);
`<x:textbox>` steps by `line-height · font-size` and takes `valign` (`top`/`middle`/`bottom`).

**Baseline-stability invariant (both):** alignment reserves the *font's* descent (a constant), not
the per-string ink. Therefore a descender-free and a descender-bearing label in the same box land on
the **same baseline** — descenders fill the reserved descent rather than shifting the text. This MUST
hold for every alignment.

### 6.6 Overflow & truncation — `text-overflow` [implemented]

Applies to box-bound text (`<textArea>`, `<x:textbox>`); not to point text or `inline-size` flow.

- **`text-overflow`** : `clip` *(initial)* | `ellipsis`. Unprefixed (CSS/SVG 2 name). `clip` is the
  default and reproduces SVG Tiny 1.2 behavior.

**Overflow axes.** *Block:* more wrapped lines than fit the content height. *Inline:* a line wider
than the content width (an unbreakable token).

**Order.** Resolve font size (apply `fit` first) → wrap → compute the fitting line count `C` (lines
whose box lies within the height; `C` = all if height is `auto`) → apply `text-overflow`:

- **`clip`** — render lines `0 … C−1`; drop the rest. Inline overflow renders past the box (a clip
  path MAY be emitted at higher quality).
- **`ellipsis`** — render `0 … C−1`; if lines were dropped, the **last rendered line** is ellipsized;
  any rendered line wider than the content width is ellipsized. If `C = 0`, render nothing.

**Fit vs. truncate (not contradictory — sequential).** `fit` and truncation answer two different
questions: *should the font adapt to the box?* and *what becomes of whatever still overflows?*
`fit="shrink"` reduces *how much* overflows by shrinking the font, but only down to the `fit-min`
legibility floor; `text-overflow` then handles the residual at that floor. So truncation fires
**only when fit bottoms out at `fit-min` and the text still doesn't fit** — with no floor (or a
generous one) fit wins and nothing is truncated; with no `fit`, `text-overflow` acts at the authored
size. `text-overflow` is a single, general overflow control, independent of fitting.

**Ellipsizing a line** (`line`, `max_width`, marker `E` = `…` U+2026):
1. If `width(E) > max_width`, render nothing.
2. Strip trailing whitespace, then trim trailing **characters** until `width(line + E) ≤ max_width`
   (re-stripping exposed whitespace), so the result reads `word…` not `word …`.
3. If `line` is emptied, render just `E`.
4. Emit `line + E`.

Trimming is by character (CSS-like). v0 truncates at the **inline end** only; the marker is a single
`…` glyph. A future `x:ellipsis` MAY allow a custom marker.

| Edge case | Result |
|---|---|
| empty / whitespace-only | nothing |
| box too short for one line (`C=0`) | nothing |
| box narrower than `…` | nothing |
| text fits | no marker (== `clip`) |
| line trims to empty before `…` fits | just `…` |

**Differs from CSS/SVG 2:** one property covers **both** block (multi-line, like `-webkit-line-clamp`)
and inline overflow; implemented by emitting `<tspan>`s, so it renders without SVG-2 support.

### 6.7 Glyph width scaling — `x:glyph-x-scale` [implemented]

A purely **visual** horizontal scale on rendered glyphs. Applies to all three text front-ends:
unprefixed `glyph-x-scale` on `<x:textbox>`; `x:`-prefixed `x:glyph-x-scale` on plain-SVG `<text
inline-size>` and `<textArea>` (the prefix policy of §3 — a new attribute on a reused SVG element is
namespaced).

| Attr | Values | Initial | Effect |
|---|---|---|---|
| `glyph-x-scale` | `<number>` | `1` | multiply each line's advance width by this factor |

**Lowering.** Per emitted line, the compiler measures the natural advance `w`, then emits
`textLength="w · scale" lengthAdjust="spacingAndGlyphs"` on the `<tspan>`. Renderers stretch/compress
the glyphs and inter-glyph spacing to hit that length. **Layout is unchanged** — wrapping, fitting and
overflow all run at the natural width; only the final rendered glyphs are scaled. A value of `1` (the
initial) emits nothing. Empty lines are left untouched. When combined with `letter-spacing` (§6.8),
the `textLength` target is computed from the letter-spaced advance so the two compose.

### 6.8 Letter & word spacing — `letter-spacing`, `word-spacing` [implemented]

CSS/SVG tracking: uniform extra space between grapheme clusters (`letter-spacing`) and at each
inter-word gap (`word-spacing`), on every text element (unprefixed — both are existing SVG/CSS
names, §3).

| Attr | Values | Initial | Effect |
|---|---|---|---|
| `letter-spacing` | `normal` \| `<length>` | `normal` (= 0) | extra advance added per inter-grapheme gap |
| `word-spacing` | `normal` \| `<length>` | `normal` (= 0) | extra advance added per inter-word space |

**Model (normative).** Both are **absolute lengths** in user units — they do *not* scale with
`font-size` (so under shrink-to-fit the spacing stays put while glyphs shrink), matching CSS/SVG. They
are **additive on top of kerning**, not a replacement: the font's pair kerning stays in the glyph
advances (whatever the measurer models — real for canvas, none for the test fixtures) and the spacing
is layered over it. The rendered advance of a run of *n* grapheme clusters containing *s* inter-word
spaces is:

```
advance = kerned_advance(run) + (n − 1) × letter-spacing + s × word-spacing
```

**Layout-aware.** This spaced advance — not the raw glyph advance — drives wrapping, shrink-to-fit,
ellipsis truncation and alignment, so tracked text breaks and fits correctly. The attributes are then
emitted on the output `<text>` (or forwarded on a passed-through `<text inline-size>`); the renderer
reproduces exactly the width layout assumed. `normal` and `0` emit nothing. Counts use code points in
v0 (grapheme-cluster segmentation is a future refinement; wrapped lines join words with a single
space, so the inter-word count is exact); `font-kerning` is left at the renderer default (`normal`).

### 6.9 Justification — `text-align="justify"` / `align="justify"` [implemented]

Full-justify on the box front-ends (`<textArea>` via `text-align`, `<x:textbox>` via `align`). The
value `justify` extends the SVG Tiny 1.2 `text-align` vocabulary with the CSS/SVG 2 value (§3).

**Model (normative).** After wrapping, a line is **justified** — stretched to the content width — iff
**all** of:
- alignment is `justify` and the box has a **positive, known content width** (auto-width `<textArea>`
  has no target, so it degrades to `start`);
- it is **not** the last line of its paragraph (the last line, and the last line before each
  `<tbreak/>`, stay ragged — normal typographic convention);
- it contains **more than one word** (a lone word has nothing to stretch between).

An ellipsized line (§6.6) is never justified. Justified lines anchor at the **start** edge.

**Lowering.** A justified line emits `textLength="<content-width>" lengthAdjust="spacing"` on its
`<tspan>`; the renderer distributes the slack into the inter-glyph/word spacing (glyph shapes are not
scaled — contrast `glyph-x-scale`, §6.7, which uses `spacingAndGlyphs`). On a line that would carry
both, justification wins. v0 distributes slack uniformly across all gaps (not word-gaps-only); a
word-spacing-only composer is a future refinement.

### 6.10 Shape binding & region flow — `<x:textbox in="#id">` [implemented]

Binds a textbox to a referenced shape instead of its own `x/y/width/height`.

- **`in="#id"`** resolves to any element in the document. A missing target renders nothing (a comment
  marker is emitted).
- If the target is a **`rect`**, the textbox uses its box and the rectangular model of §6.4/§6.5 —
  `fit`, `valign`, `padding`, `align` all apply. This is the common "label a box" case.
- **Any other fillable shape** (`path`, `circle`, `ellipse`, `polygon`, `polyline`) → text is flowed
  **inside the actual filled outline** (the Vision's *fit-text-in-polygon*): each line is wrapped to
  the shape's inside width *at that height*, so a triangle's lines shorten toward its apex and a
  circle's bulge across the middle.

**Geometry seam (normative architecture).** Region geometry is obtained through a host-supplied
*shaper* — the geometric analog of the *measurer* (§4). The shaper rasterizes a filled path into a
coarse table of per-row inside-spans `[left, right]`; the pure layout then flows text into that table
(intersecting spans across each line box so glyphs never cross the outline). In v0 the browser is the
shaper — curve flattening, bounds, and inside-testing are deferred to `getBBox` + `isPointInFill` — and
native tests replay browser-generated raster fixtures. A pure-Rust shaper (flatten + scanline) is a
later backend behind the same seam.

**Alignment.** `align` (`start`/`center`/`end`) positions each line within its own span. `valign`
(`top`/`middle`/`bottom`) positions the flowed block within the region's vertical extent: a first pass
sizes the block, then the flow re-runs from a shifted start (so each line still gets the span at its
final height — a middle-aligned block in a diamond straddles the widest band). The shift clamps to 0
when the block is taller than the region, so it never drops words top-alignment wouldn't.

**v0 scope.** No **shrink-to-fit** in region mode (rect fast-path only). `text-overflow` clips at the
region's bottom and can ellipsize the last line. Vertical resolution is coarse (row height ≈
font-size ⁄ 3). Any outline whose every horizontal slice is a **single run** flows correctly —
including vertical-pinch concavities like an hourglass (lines just narrow at the waist). A row that
splits into **two runs** (a donut, a horizontal bowtie, a `U`) collapses to its outer
`[leftmost, rightmost]` span, so text bridges the gap — v0 has no multi-span rows. A word wider than a
line's span still overflows the outline (the usual lone-word rule) unless `text-overflow="ellipsis"`
trims it.

### 6.11 Styled runs — `<tspan>` [implemented]

Inline styling inside `<textArea>` and `<x:textbox>`: a child `<tspan>` is a **run** whose text
shares the surrounding paragraph but overrides its paint/style. This reuses SVG's own inline-span
element (§3, reuse-unprefixed) — the same `<tspan>` the compiler emits, now also read on input.

| Overridable on a run | Notes |
|---|---|
| `fill` | per-run colour |
| `font-weight`, `font-style` | e.g. bold / italic words |
| `font-family` | per-run typeface |

**Model (normative).** Runs affect a word's **advance** (weight/style/family change glyph widths), so
each word is measured in its own run style and wrapping stays correct; a run boundary may fall
mid-word (two pieces, no break between them). **`font-size` is not overridable in v0** — mixed sizes
would perturb line-height and baseline; runs share the paragraph size, `letter-spacing`, and
`word-spacing`. Nested `<tspan>`s compose (inner wins). `<tbreak/>` still breaks across runs.

**Lowering.** Each output line is one positioning `<tspan x y>` (which also carries any justify /
`glyph-x-scale` `textLength`); within it, base-styled text is emitted bare and each run becomes a
nested `<tspan>` carrying only the attributes that differ from the base `<text>`. So a plain paragraph
emits exactly as before (no nested spans). Ellipsis truncates from the last run inward.

**v0 limits.** Styled runs apply to the rectangular box front-ends (`<textArea>`, `<x:textbox>` — incl.
`in="#rect"`); curved-shape region flow (§6.10) and `<text inline-size>` flatten runs to the base
style. Per-run `stroke` and `font-size` are future work.

### 6.12 Create outlines — `outline="true"` [implemented]

Turn a text element's glyphs into vector `<path>` geometry at compile time ("Create Outlines" in
Illustrator terms). Set `outline="true"` on `<x:textbox>`, or the prefixed `x:outline="true"` on the
reused `<textArea>` (§2). Layout is unchanged — outlining is a purely **emit-time** transform applied
after wrapping, fitting, and alignment have run.

**Model (normative).** Each laid-out line is traced from its text at the element's **base style** and
emitted as a `<path>` inside a single `<g>` that carries the element's paint — `fill` plus any
`stroke*` / `paint-order` (so an outline can be filled, stroked as a keyline, or both). Outlining is
**all-or-nothing per element with graceful fallback**: if the backend cannot outline *every* line
(e.g. the font's bytes are unavailable), the element falls back to live `<text>` unchanged — so an
outline request never breaks a drawing, it only upgrades it when the font is present. Lines are
anchored by their measured width (`text-anchor` start/middle/end resolved to an `x` origin), matching
the live-text placement.

**The seam.** Outlines come from a platform `GlyphOutliner` (parallel to the `Measurer`/`Shaper`
seams of §4): given a run, its style, size, and baseline origin, it returns a path `d` or `None`. The
browser adapter backs it with [opentype.js](https://opentype.js.org) and therefore needs the font's
**bytes** — the host application registers fonts by family (a family with no registered font ⇒ live
`<text>`). Arbitrary installed system fonts are not outlinable this way.

**Fonts by name (reference app).** So a source needn't ship font bytes, the web app resolves a
`font-family="-x-google-<Name>"` marker by fetching that family from Google Fonts once (css2 →
woff2 → decompressed to sfnt by a vendored decoder), then (a) registering it with the outliner and
(b) adding it to the document via `FontFace` so **live `<text>` and canvas metrics** use the real face
too — not only the outlined `<path>`. The marker is stripped to the bare family before compile, so a
resolution failure degrades to the normal live-`<text>` fallback. This is app-level font provisioning,
not part of the interchange format.

**Why.** The drawing then carries the **true display face as geometry** — it renders identically
anywhere, with no font install or `@font-face` embed, at the cost of selectable text (a hidden `<text>`
layer for searchability is future work, §G). This is also the prerequisite for the *text-as-vector-art*
effects of the remaining pillars (§7): geometry warp/envelope and mesh-fill applied to glyph outlines.

**v0 limits.** Outline mode uses the base style per line: per-run styling (§6.11), `justify` (§6.9),
`glyph-x-scale` (§6.7), and `letter-spacing`/`word-spacing` (§6.8) do not apply to the traced path, and
curved-shape region flow (§6.10) keeps its own per-line placement. Per-run outlining is future work.

## 7. Roadmap — remaining pillars [planned]

Pillar 1 (typography, §6) **shipped in v1**, through create outlines (§6.12). The remaining pillars,
sketched in [Plan.md](Plan.md) and grounded in [Research.md](Research.md):

- **Non-affine, non-destructive geometry transforms** *(Pillar 2)* — perspective / warp / envelope on
  vector geometry, which SVG's **affine-only** `transform` cannot express. A non-destructive effect
  stack (source geometry + editable transforms) is **baked at compile time** by **flatten → map →
  refit** (kurbo flatten tolerance = the quality knob); deformation models are FFD (lattice/cage),
  moving-least-squares (handles), and homography (perspective). See [Research.md §7](Research.md).
- **`<x:mesh>`** *(Pillar 3)* — Coons/tensor mesh gradients with **cracks / T-junctions** and
  **transparency (feathering / fade)**, lowered to flat patches / gradient triangles / raster `<image>`.
- **Deferred** (valuable, but no longer headline pillars): **`<x:vstroke>`** variable-width strokes
  (research retained in [Research.md §1](Research.md)) and **`<x:boolean>`** live path algebra.

## 8. Lowering target [implemented]

Output is the **static SVG subset** (resvg's scope): no script, animation, events, or `meshgradient`.
Text lowers to `<text>`/`<tspan>` in v0 (browser-shaped), or to outlined `<path>` on demand (§6.12).
The concrete allow/deny feature list is a pending deliverable ([Plan.md](Plan.md) R6).

## Appendix A — Feature status

| Feature | Status |
|---|---|
| Namespaces, prefix policy, degradation contract | implemented |
| `<rect>` → `<path>` | implemented |
| `<text inline-size>` wrap flow | implemented |
| `<textArea>` (text-align, display-align, line-increment, auto sizing, clip) | implemented |
| `<tbreak/>` forced line break | implemented |
| `<x:textbox>` (align, valign, padding, fit) + cap-height centering | implemented |
| Real browser font metrics (ascent, descent, cap-height, x-height) | implemented |
| `text-overflow` (clip default; **ellipsis**) | implemented |
| `glyph-x-scale` (visual glyph width scaling via `textLength`) | implemented |
| `letter-spacing` / `word-spacing` (layout-aware tracking, kerning-preserving) | implemented |
| `text-align="justify"` / `align="justify"` (greedy full-justify via `textLength`) | implemented |
| `<x:textbox in="#shape">` binding (rect box + region flow into curved outlines) | implemented |
| Styled runs (`<tspan>` per-run fill / weight / style / family in flowed text) | implemented |
| Create outlines (`outline="true"` → glyphs as `<path>` via the `GlyphOutliner` seam, live-text fallback) | implemented |
| `xml:space=preserve`, UAX #14, `editable` | not implemented |
| `<x:vstroke>`, `<x:mesh>`, `<x:boolean>` | planned |
| Per-run outlines; hidden selectable-text layer; concrete SVG-subset list; WebGPU renderer | planned |
