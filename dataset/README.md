# dataset — sample xsvg diagrams

Hand-authored `.xsvg` samples, each a complete `<svg xmlns:x="…">` document, grouped by the feature
they exercise. Cross-referenced with the normative [Specification.md](../docs/Specification.md).

## Showcases

Realistic composites that combine several features into one artifact.

| File | Shows |
|---|---|
| [architecture.xsvg](architecture.xsvg) | System diagram: uniform service boxes with unequal labels (shrink-to-fit), `<tbreak/>` two-line data nodes, a `glyph-x-scale` banner, and arrow markers |
| [kanban.xsvg](kanban.xsvg) | Sprint board: cards whose bodies wrap and truncate with `text-overflow="ellipsis"`, `<tbreak/>` title/body splits, right-aligned counts |
| [pipeline.xsvg](pipeline.xsvg) | Compile pipeline: stretched heading, five shrink-to-fit stages with wrapping captions, arrow markers |
| [flowchart.xsvg](flowchart.xsvg) | Request flow: branching yes/no decision, shrink-to-fit nodes, terminal states, arrow markers |

## Shape binding & region flow — `<x:textbox in="#id">` (§6.10)

| File | Shows |
|---|---|
| [chat.xsvg](chat.xsvg) | `in="#rect"` binds a label to each rounded bubble — "draw the box once, attach the text" |
| [region-flow.xsvg](region-flow.xsvg) | Text flowed *inside* a triangle, circle, diamond, and a concave hourglass — lines follow each outline; `valign` centers the block |
| [badges.xsvg](badges.xsvg) | Centered labels poured into a hexagon, circle seal, shield (curved path), and pentagon |

## Box models & alignment (§6.3–6.5)

| File | Shows |
|---|---|
| [textarea.xsvg](textarea.xsvg) | `<textArea>` (SVG Tiny 1.2): `text-align`, `display-align`, `line-increment`, auto width/height |
| [textarea-align.xsvg](textarea-align.xsvg) | `<textArea>` `text-align` × `display-align` matrix (all nine) |
| [alignment.xsvg](alignment.xsvg) | `<x:textbox>` `align` × `valign` matrix (all nine placements, cap-height centering) |

## Wrapping, fitting & overflow (§6.1–6.2, 6.6)

| File | Shows |
|---|---|
| [wrap-vs-overflow.xsvg](wrap-vs-overflow.xsvg) | The core win: plain `<text>` overflowing vs `inline-size` wrapping vs `<x:textbox fit="shrink">` |
| [cards.xsvg](cards.xsvg) | Equal-size cards whose variable-length descriptions all shrink to fit |
| [textarea-sizing.xsvg](textarea-sizing.xsvg) | `width=auto` (no wrap), wrapping, height clipping, `line-increment` auto/loose/tight |
| [textarea-ellipsis.xsvg](textarea-ellipsis.xsvg) | `text-overflow`: clip vs ellipsis (block overflow) and inline overflow truncation |

## Paragraph & character typography (§6.7–6.9)

| File | Shows |
|---|---|
| [justify.xsvg](justify.xsvg) | `text-align="justify"`: full lines flush both edges, last line ragged, `<tbreak/>` resets per paragraph |
| [letter-spacing.xsvg](letter-spacing.xsvg) | `letter-spacing` tracking scale, kerning-preserved pairs, layout-aware wrapping |
| [word-spacing.xsvg](word-spacing.xsvg) | `word-spacing` scale + layout-aware wrapping (wider word gaps wrap sooner) |
| [tbreak-and-glyph-scale.xsvg](tbreak-and-glyph-scale.xsvg) | `<tbreak/>` forced breaks + `x:glyph-x-scale` condensed/regular/extended widths |
| [styled-runs.xsvg](styled-runs.xsvg) | `<tspan>` runs: per-run fill / weight / style flowing and wrapping inline, incl. under justify |

## Vector output — create outlines (§6.12)

| File | Shows |
|---|---|
| [outline.xsvg](outline.xsvg) | `font-family="-x-google-Anton"` provisions a Google font by name (live `<text>` via `FontFace`); one box drawn twice at identical geometry — live fill + an `outline="true"` keyline stroke on top — proves the traced `<path>` lands exactly on the live glyphs |

## Geometry transforms — text on a path (§6.13)

| File | Shows |
|---|---|
| [textpath.xsvg](textpath.xsvg) | `<x:textpath in="#wave" effect="skew">` outlines the run and warps it onto the curve via the displacement field — glyphs stay upright and shear, the baseline follows the path |
| [textpath-rainbow.xsvg](textpath-rainbow.xsvg) | `effect="rainbow"` follows the arc — glyphs rotate and deform along the curve; `baseline-shift` offsets runs along the local normal (one run floats above, a second hangs beneath the same path) |
| [textpath-align.xsvg](textpath-align.xsvg) | `align="start\|middle\|end"` + `start` place the run within the path's extent (arc length under rainbow, x-extent under skew); `effect="stair"` steps live, selectable `<text>` glyph-by-glyph along the height profile — no font bytes needed (it is also skew's no-font degradation) |

## Geometry transforms — warp (§7.3)

| File | Shows |
|---|---|
| [warp-presets.xsvg](warp-presets.xsvg) | `<x:warp field="…" bend="…">` bakes an envelope-preset field into plain `<path>`s — eight presets (arch/flag/rise/wave + fisheye/inflate/squeeze/twist); a rect and `outline="true"` text warp together through the same flatten → map pipeline; dashed boxes show the unwarped source |
| [warp-presets-arc.xsvg](warp-presets-arc.xsvg) | The arc & shell families — `arc` wraps the box into an annular sector (midline length preserved; negative bend arcs down), `arc-lower/upper` pin one edge and arc the other, `bulge`/`fish` scale about the midline, `shell-lower/upper` flare the corners. Make-with-Warp parity: **15/15 presets** |
| [warp-perspective.xsvg](warp-perspective.xsvg) | `field="perspective" corners="…"` solves an 8-DOF homography from the envelope corners (straight lines stay straight — no wasted subdivision); `field="free"` blends corners bilinearly; `distort-h`/`distort-v` compose a projective taper after any preset |

## Edge cases & invariants

| File | Shows |
|---|---|
| [degenerate.xsvg](degenerate.xsvg) | Edge cases: empty text, `inline-size=0`, `font-size=0`, shrink, `fit-min>size`, oversized word, degenerate `<x:textpath>` targets (zero-length path → live-text fallback; vertical path under skew; rainbow run outliving its path) |
| [descenders.xsvg](descenders.xsvg) | Proof that descenders (`Gg`) do not shift the baseline vs `Bb` (shared-baseline guide) |

## Viewing them

With the dev server running (`npm run dev`), open any sample by name:

```
http://localhost:5173/view/wrap-vs-overflow.xsvg
http://localhost:5173/?file=pipeline.xsvg
```

The SVG renders full-screen; the source and compiled output are logged to the browser console. The
`?file=` form works on any static host; the `/view/<name>` pretty-URL form relies on the dev
server's SPA fallback.
