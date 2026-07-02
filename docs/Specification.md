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
| xsvg extensions | `https://xsvg.dev/ns` (conventional prefix `x:`) |

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
| `text-align` | `start` \| `end` \| `center` | `start` | inline alignment |
| `display-align` | `auto` \| `before` \| `center` \| `after` | `auto` (= `before`) | block alignment |
| `line-increment` | `auto` \| `<number>` | `auto` | line-box height; `auto` = 1.1 × font-size |

**Layout (line-box model).** Wrap to the width (or not, if `auto`). Each line occupies a line box of
height `line-increment`; line boxes stack from the region's before-edge; a line's baseline sits at
the font ascent for an assumed font-size = `line-increment`. `display-align` then positions the block
(`before`=top, `center`=middle, `after`=bottom). With an explicit `height`, lines whose box falls
outside the region are **not rendered** (clipped; see §6.6).

**`<tbreak/>`** [implemented] — a forced line break, per SVG Tiny 1.2. Each `<tbreak/>` child ends the
current line and starts a new one; wrapping resumes independently on each side. Consecutive breaks
produce blank lines. This is the only child element `<textArea>` interprets in v0.

**Not yet implemented:** `xml:space="preserve"`, full UAX #14 line-breaking (we break on
whitespace), `editable`.

### 6.4 `<x:textbox>` — Rung 3, xsvg box [implemented]

Box text with diagram ergonomics. Distinct from `<textArea>` in vertical model (see §6.5).

| Attr | Values | Default | Effect |
|---|---|---|---|
| `x`,`y`,`width`,`height` | length | — | box geometry |
| `in` | `#id` | — | bind to a referenced shape's geometry *(planned)* |
| `padding` | length | 0 | uniform content inset |
| `align` | `start` \| `center` \| `end` | `start` | horizontal alignment |
| `valign` | `top` \| `middle` \| `bottom` | `top` | vertical alignment |
| `fit` | `none` \| `shrink` | `none` | shrink-to-fit (§6.1) |
| `fit-min` | length | 6 | font-size floor for `shrink` |
| `line-height` | number | 1.2 | line advance multiplier |

### 6.5 Vertical alignment models (normative) [implemented]

The two box elements deliberately differ:

- **`<textArea>`** uses the **line-box** model (§6.3): `display-align` over boxes of height
  `line-increment`.
- **`<x:textbox>`** uses **cap-height band** centering: the band runs from the first line's
  **cap-top** to the last line's **baseline + descent**; `valign` positions that band. This centers
  letterforms optically rather than the em box.

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

## 7. Other pillars [planned]

Sketched in [Plan.md §2](Plan.md); not yet implemented.

- **`<x:vstroke>`** — variable-width strokes (skeleton path + width profile), lowered to a filled `<path>`.
- **`<x:mesh>`** — Coons/tensor mesh gradients with transparency, lowered to flat patches / triangles / raster.
- **`<x:boolean op="…">`** — live boolean shape operators (union/intersect/subtract/exclude), lowered to one resolved `<path>`.

## 8. Lowering target [implemented]

Output is the **static SVG subset** (resvg's scope): no script, animation, events, or `meshgradient`.
Text lowers to `<text>`/`<tspan>` in v0 (browser-shaped) or outlined `<path>` later. The concrete
allow/deny feature list is a pending deliverable ([Plan.md](Plan.md) R6).

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
| `<x:textbox in="#shape">` binding | planned |
| `xml:space=preserve`, UAX #14, `editable` | not implemented |
| `<x:vstroke>`, `<x:mesh>`, `<x:boolean>` | planned |
| Outlined-text backend; concrete SVG-subset list; WebGPU renderer | planned |
