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
| [textpath.xsvg](textpath.xsvg) | `effect="skew"` warps upright glyphs onto a wave (the displacement field); `effect="rainbow"` rotates and bends them along an arc, with `baseline-shift` floating one run above and hanging another beneath the same path |
| [textpath-align.xsvg](textpath-align.xsvg) | `align="start\|middle\|end"` + `start` place the run within the path's extent (arc length under rainbow, x-extent under skew); `effect="stair"` steps live, selectable `<text>` glyph-by-glyph along the height profile — no font bytes needed (it is also skew's no-font degradation) |
| [textpath-effects.xsvg](textpath-effects.xsvg) | `effect="ribbon"` — skew's complement: heights offset along the profile *normal*, so verticals tilt with the curve; `effect="follow"` lowers to SVG's native `<textPath>` — live, selectable, undeformed (align/start → `startOffset`) |

## Geometry transforms — warp (§7.3)

| File | Shows |
|---|---|
| [warp-presets.xsvg](warp-presets.xsvg) | `<x:warp field="…" bend="…">` bakes an envelope-preset field into plain `<path>`s — eight presets (arch/flag/rise/wave + fisheye/inflate/squeeze/twist); a rect and `outline="true"` text warp together through the same flatten → map pipeline; dashed boxes show the unwarped source |
| [warp-presets-arc.xsvg](warp-presets-arc.xsvg) | The arc & shell families — `arc` wraps the box into an annular sector (midline length preserved; negative bend arcs down), `arc-lower/upper` pin one edge and arc the other, `bulge`/`fish` scale about the midline, `shell-lower/upper` flare the corners. Make-with-Warp parity: **15/15 presets** |
| [warp-perspective.xsvg](warp-perspective.xsvg) | `field="perspective" corners="…"` solves an 8-DOF homography from the envelope corners (straight lines stay straight — no wasted subdivision); `field="free"` blends corners bilinearly; `distort-h`/`distort-v` compose a projective taper after any preset |
| [warp-bend.xsvg](warp-bend.xsvg) | `field="bend" in="#spine"` flows a whole group along a path — the envelope midline rides the spine, `align`/`start` place it by arc length (Inkscape's *LPE Bend*); `field="roughen" bend detail` jitters outlines with **deterministic** seeded value noise |

## Path algebra — `<x:boolean>` (§7.4)

| File | Shows |
|---|---|
| [boolean.xsvg](boolean.xsvg) | `op="union"` merges a circle cloud into one outline (single silhouette stroke); `op="subtract"` punches outlined text from a plate (*Minus Front*); `intersect` keeps the lens, `exclude` turns the overlap into a hole; the last card warps a boolean result with `field="flag"` — path algebra and warps compose both ways |
| [boolean-refs.xsvg](boolean-refs.xsvg) | Operands **by reference**: a `<use href>` child borrows geometry without consuming it — the venn lens is derived from circles that keep rendering; motifs stamp by `x`/`y` offset **and by full `transform`** (a rotated-bar rosette); a union's compiled output punches a plate; and a **live textbox's glyphs punch by reference** (auto-outlined, §4) while the text stays selectable |

## Mesh gradients — `<x:mesh>` (§8.2)

| File | Shows |
|---|---|
| [mesh.xsvg](mesh.xsvg) | Indexed mesh gradients: a shared `points` list (SVG's polygon syntax) + `<x:face v fill>` quads/tris with per-corner colors — a seamless two-quad sky, the bilinear twist, a **crack** (shared edge, disagreeing colors → hard split), a barycentric triangle fan, and the **grid sugar** form (`cols`/`rows` + one `fill` of vertex colors, no indices). Lowered by render→refit: each crack region is fitted with a seam-free grid field and serialized as a **texel-aligned tiny PNG** (often 2×2) whose stretch makes the renderer's own bilinear filter reconstruct the gradient, clipped by the exact face-polygon union |

## Pixel adjustments (§8)

| File | Shows |
|---|---|
| [adjust.xsvg](adjust.xsvg) | The standard `filter` attribute with **CSS function syntax** — live in any browser as authored, lowered by the compiler to a portable `<filter>` graph (sRGB interpolation, sized region) for static renderers; `-x-curve[-r/g/b/a]()` adds Photoshop-style **tone curves** (monotone-cubic through control points, sampled into `feComponentTransfer` tables) |

## Composition by reference (§4)

| File | Shows |
|---|---|
| [compose.xsvg](compose.xsvg) | `in="#id"` pointing at an `x:` element resolves its **compiled output**: a textbox flows inside a boolean union's merged silhouette; type rides the arched spine an `x:warp` emitted; and a `path → x:warp bend → x:textpath` chain gives three linked elements one edit point |

## Edge cases & invariants

| File | Shows |
|---|---|
| [degenerate.xsvg](degenerate.xsvg) | Edge cases: empty text, `inline-size=0`, `font-size=0`, shrink, `fit-min>size`, oversized word, degenerate `<x:textpath>` targets (zero-length path → live-text fallback; vertical path under skew; rainbow run outliving its path); **reference cycles** — a self-referential `<use>` operand drops out, a mutual `in=` pair degrades to markers (§4) |
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
