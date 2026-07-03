# xsvg — Language / Syntax Design

How xsvg extends SVG. Companion to the architecture in [Plan.md](Plan.md) (§2) and the typography
catalog in [Typography.md](Typography.md). Status: **design in progress** — text & box model is
settled; other extensions (variable strokes, mesh, booleans) follow the same rules and are sketched
in [Plan.md §2.3–2.5](Plan.md).

## Design principles

xsvg is a **graceful-degradation superset of SVG**. The win should cost *one tag-swap or one
attribute*, and nothing you add can break the file.

1. **SVG ignores unknown attributes.** An xsvg attribute is a harmless no-op in any plain SVG viewer
   and meaningful to the xsvg compiler — so adding one is always safe.
2. **Reuse SVG's own under-supported vocabulary** instead of inventing names. SVG already specced the
   features browsers fumbled — `inline-size` (SVG2 auto-wrap), `<textArea>` (SVG 1.2 Tiny box text),
   `<meshgradient>` (dropped from SVG2). LLMs already know these; raw files still render their valid
   parts. We invent names only where SVG has no concept.
3. **Keep the `<svg>` root.** An author adds `xmlns:x="https://xsvg.visioncortex.org"` to their existing
   `<svg>` and starts using extensions. No new root; true drop-in.

### Prefix policy

| Kind | Rule | Examples |
|---|---|---|
| Reviving a real SVG/CSS name | use it **exactly, unprefixed** | `inline-size`, `text-wrap`, `line-height`, `<textArea>`, `<meshgradient>` |
| New **attribute** on a standard SVG element | **`x:`-prefixed** (collision-proof, clearly an extension) | `<text x:fit="shrink">`, `<path x:width-profile="…">` |
| New **element** | **`x:`-prefixed** (a dumb viewer skips it; the compiler recognizes it) | `<x:textbox>`, `<x:vstroke>`, `<x:mesh>`, `<x:boolean>` |
| Attributes **inside** an `x:` element | **plain/unprefixed** — we own the element, no ambiguity | `<x:textbox padding="8" align="center">` |

### Degradation contract — what a plain SVG viewer does with raw xsvg

| You wrote | Plain SVG viewer | xsvg compiler |
|---|---|---|
| Unknown attr on a **known** element (`inline-size`, `x:fit`) | **ignores the attr** → still renders the element (e.g. text shows, just unwrapped/unfit) | applies it (wraps / fits) |
| Unknown **element** — `x:`-namespaced (`<x:vstroke>`) *or* a revived-but-unsupported SVG element (`<textArea>`) | **skips it** → its content does **not** render | lowers it to plain SVG |
| Everything else | renders normally | passes through |

**Degradation isn't uniform — it tracks the ladder rung** (see below). Attribute-only extensions
(Rung 1) degrade to *visible-but-plain*; new/revived elements (Rungs 2–3) need the compiler to show
anything. So worst case for attribute extensions, raw xsvg renders like the SVG it already was; for
element extensions, raw xsvg shows nothing for that element until compiled.

---

## The text & box model — a progressive-adoption ladder

Three rungs, increasing power for increasing commitment — and **decreasing graceful degradation**
(Rung 1 still shows in a dumb viewer; Rungs 2–3 need the compiler). Pick the lowest rung that solves
your case.

| Rung | Diff | Box? | Degrades in a plain viewer? |
|---|---|---|---|
| 1 `inline-size` on `<text>` | add 1 attr | width only | ✅ shows (unwrapped) |
| 2 `<textArea>` | swap tag | width × height | ❌ hidden until compiled |
| 3 `<x:textbox>` | new element | width × height (+ shape) | ❌ hidden until compiled |

### Rung 1 — `inline-size` on `<text>` *(add one attribute)*

Smallest possible diff. Wraps at a width; the block grows downward. Revives SVG2's own attribute.

```svg
<text x="90" y="30" text-anchor="middle" inline-size="150" line-height="1.3">
  Some long label that now wraps instead of overflowing
</text>
```
Optional unprefixed companions (real CSS names): `text-wrap="balance"` (even ragged lines — great
for labels), `line-height`. **Lowers to** positioned `<tspan>`s (v0, browser-shaped) or outlined
`<path>` runs (Phase 2b).

### Rung 2 — `<textArea>` *(swap the tag — SVG Tiny 1.2)*

A box with flowed text, implemented to the **SVG Tiny 1.2** spec: `text-align`
(`start|end|center`, plus `justify` from CSS/SVG 2), `display-align`
(`before|center|after`), `line-increment` (`auto` = 1.1·em, or a length), and `auto`
width/height (`width:auto` ⇒ no wrap; `height:auto` ⇒ grow; an explicit height clips
overflow lines). `justify` stretches full lines to the width and needs an explicit width.

```svg
<textArea x="15" y="15" width="150" height="60"
          text-align="center" display-align="center">
  Long label that wraps inside the box
</textArea>
```
For richer control — padding, cap-height vertical centring, shrink-to-fit, binding
to a shape — use the xsvg `<x:textbox>` below.

### Rung 3 — `<x:textbox>` *(full diagram ergonomics)*

Box-bound text with the controls diagram tools need: padding, horizontal + vertical centering,
fitting, and the ability to **bind to an existing shape** (draw the box once, attach the label).

```svg
<rect id="node" x="10" y="10" width="160" height="60" rx="6" fill="#eef" stroke="#88a"/>
<x:textbox in="#node" padding="8" align="center" valign="middle" fit="shrink">
  Long node label that wraps, centers, and shrinks to fit the box
</x:textbox>
```
`in="#shape"` binds to a referenced shape ([Specification.md §6.10](Specification.md)): a **rect** uses
its box (with `fit`/`valign`), while a **curved shape** (path/circle/ellipse/polygon) flows text
*inside the actual outline* — the Vision's "fit text in polygon", so lines follow a triangle or circle.
Region flow honors `align` and `valign` (so text centers in a badge/seal) but has no `fit` in v0.
Without `in`, give inline `x`/`y`/`width`/`height`.
Attributes here are unprefixed (we own the element): `padding`, `align` (start|center|end|justify),
`valign` (top|middle|bottom), `fit`, `line-height`, etc.

---

## Fitting text to a box — `fit` *(shrink-to-fit and friends)*

The requested mode: **make the font size smaller until the whole paragraph fits the box.** This is an
xsvg extension on **`<x:textbox>`** (SVG Tiny 1.2's `<textArea>` has no fit — use a textbox for it).

```svg
<x:textbox x="15" y="15" width="150" height="50" fit="shrink" fit-min="9">
  This paragraph shrinks its font size just enough to fit in the box, down to a 9px floor
</x:textbox>
```

**`fit` values:**
| Value | Behavior |
|---|---|
| `none` *(default)* | no autofit; text wraps and may overflow |
| `shrink` | **reduce font size until the wrapped paragraph fits width × height** (never grows) — *the requested mode* |
| `grow-shrink` | scale font size to best fill the box in either direction |

**Companions:** `fit-min` (font-size floor, e.g. `9`), `fit-max` (cap for `grow-shrink`, default =
the authored `font-size`). What to do when it *still* doesn't fit at `fit-min` is governed by
`text-overflow` (`clip` | `ellipsis`) — specified in [Specification.md §6.6](Specification.md). The
pipeline is shrink-to-fit → then truncate.

**Algorithm (engine):** binary-search the font size in `[fit-min, font-size]`; at each trial,
re-wrap at the box width and measure total block height; pick the largest size whose block fits
width × height (no word overflowing the width). Re-wrapping per trial matters — a smaller font
changes the wrap points.

**v0-feasible.** The browser `FontProvider` (`measureText`) supplies the per-trial metrics; the
result is plain `<text>`/`<tspan>` emitted at the solved font size. No outlining required — works in
v0, in Chrome and Safari.

> Font-size shrink is the readable default. Overflow truncation (`text-overflow`) is specified
> separately in [Specification.md §6.6](Specification.md).

---

## Typographic controls *(work on every text element)*

Three shaping controls, usable on `<text inline-size>`, `<textArea>`, and `<x:textbox>` alike. All are
**layout-aware** — wrapping, shrink-to-fit and truncation see their effect — and normative in
[Specification.md §6.7–6.8](Specification.md) / §6.3.

| Control | Where | Does |
|---|---|---|
| `letter-spacing` | unprefixed (real SVG/CSS name) | uniform tracking between letters; an absolute length (doesn't scale with font-size) that **adds on top of kerning**. Widens lines, so text wraps sooner. |
| `word-spacing` | unprefixed (real SVG/CSS name) | same, but added at each inter-word space — spreads words apart; also layout-aware. |
| `glyph-x-scale` | `glyph-x-scale` inside `<x:textbox>`; `x:glyph-x-scale` on reused `<text>`/`<textArea>` | *visual* horizontal stretch/condense of glyphs via `textLength` — layout is unchanged, only the rendered glyphs are scaled. |
| `<tbreak/>` | child of `<textArea>` | a forced line break (SVG Tiny 1.2); wrapping resumes on each side, consecutive breaks make blank lines. |

```svg
<textArea x="10" y="10" width="240" letter-spacing="1.5" x:glyph-x-scale="1.1">
  Tracked, slightly extended<tbreak/>and hand-broken here.
</textArea>
```

---

## Other extensions (same rules)

These are genuinely-new concepts, so they're `x:` elements with unprefixed inner attributes. Syntax
sketches live in [Plan.md](Plan.md):
- **`<x:vstroke>`** — variable-width strokes ([§2.3](Plan.md))
- **`<x:mesh>`** — Coons/tensor mesh gradients with transparency ([§2.4](Plan.md))
- **`<x:boolean op="…">`** — live boolean shape operators ([§2.5](Plan.md))

All obey the degradation contract: a plain viewer skips them; the compiler lowers them to the SVG
subset.
