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

- **An empty text box lays one blank line where typing would start.** A textbox with no text —
  its own box, a referenced rect or shape, an empty `<x:p>`, an empty `<text inline-size>` — used
  to emit a bare `<text>` with no position and the layout seam reported no lines, so an editor's
  empty text measured at the origin and a caret had nowhere to sit. Every flow now lays one blank
  line placed by `align`/`valign` as a one-line text would be, emitted as an empty `<tspan x y>`
  (§6.4); paragraph stacking is unchanged, since an empty `<x:p>` already owned its row.

### Added

- **Convert to path for `<x:boolean>`, on the curves** (Incremental.md §11). `to_path` / `to_spline`
  on an `<x:boolean>` write the one `<path>` of its result computed by
  [linesweeper](https://crates.io/crates/linesweeper) over the operands' Béziers — one n-ary sweep,
  each operand under its own fill rule — so every segment the operation does not cut comes back
  as authored and a cut one as exact sub-curves; holes wind opposite to outers, sub-precision
  residue is dropped, and the flattened algebra is the answer if the engine declines or its result
  disagrees with the render on a grid of samples (the engine's known weakness is a boundary within
  epsilon of another). The
  compiled render is unchanged (still the integer-exact flattened algebra). `arcs_to_cubics`
  moved to `xsvg_core::spline`, feature-free. New workspace crate `xsvg-boolfuzz`: a seeded,
  panic-tolerant fuzz of the curve algebra against the flattened one and the sampled truth.

- **`to_path` keeps corners live; `to_spline(offset)` expands** (Incremental.md §11). Convert to
  path now leaves a live corner live, as Illustrator does: a shape's `x:corners` rides onto its
  sharp path, and a `<rect rx>` with circular corners becomes the sharp rect plus `x:corners="rx"`,
  so the corner widget still works on the result; only elliptical corners, circles and ellipses
  bake to arcs. `to_spline` is the expand: corners baked, every arc as cubic Béziers — at most a
  quarter turn per cubic, each the classic `κ = 4/3·tan(θ/4)` cubic, so a circle is four anchors at
  its extremes and a rounded corner one cubic; everything that is not an arc passes through byte
  for byte. Both accept a `<path>`: `to_path` hands it back as authored (corners live), `to_spline`
  bakes its corners and arcs. `arcs_to_cubics(d)` is public for any path data; the CLI has
  `--to-spline OFFSET` beside `--to-path`.

- **Corners: `x:corners` / `x:corner-style` on `<path>`, `<rect>`, `<polygon>`, `<polyline>`.**
  A design tool's live corner as an attribute (§5.4): one radius for every corner or
  `anchor:radius` pairs, a length or a theme token; `round` (an exact circular arc), `chamfer` or
  `inverted`. Illustrator's geometry — the trims `r / tan(θ/2)` along each side, limited to the
  segment's length (a lone corner may take a whole edge, so a square's one corner at its side is a
  quarter circle; two corners sharing an edge scale to fit it), curved segments trimmed by arc length —
  applied in the element's geometry, so references, booleans, offsets, warps and `to_path` see it.
  Anchors are numbered in the closed form compiled contours have, so an editor folding that way
  counts the same. The session's `corners(offset)` reports each corner as drawn for the widget.
  `index:value` is now the stated convention for any `x:` list that keys entries by index, with
  `:` reserved as the pair glue; `indexed_items` is the one parser for it. A corner on a curved
  segment meets the curve at its arc-length trim point (the arc used to end on the tangent's
  projection, leaving a gap); conformance tests check the geometry as area and raster twins
  (quarter circle, chamfer, scallop, winding, a concave corner) against plain SVG constructions.

- **The native CLI shapes text.** Measurement, caret advances and outlines in `xsvg-cli` go
  through rustybuzz — kerning, ligatures, mark placement, variable weight — the shaper resvg
  renders live text with, so what the headless compiler measures, fits and traces is what that
  renderer draws (a kerned word outlined through resvg: 0 px different, from 1.8 %). Boxes fit
  and wrap by kerned widths now, so the dataset's text-bearing samples compile a little
  differently than before.

- **Convert to path for editors: `to_path(offset)`.** The compiler's own output for a text
  element with outlining forced, or a basic shape as its `<path>`, in the form an editor writes
  back into the source in the node's place: one group of glyph paths and baked backdrop rects,
  every attribute the conversion did not spend kept as authored (`id`, `x:label`, `transform`,
  `opacity`, a `filter` list, `var()` paints). A circle, an ellipse or a rounded rect converts to
  the compiler's own geometry, exact: four quarter arcs from the rightmost point, or edges and
  corner arcs — arcs stay arcs, expanding to Béziers being an editor's operation. `Err` names the font that could not be traced or
  the limit that stopped it (Incremental.md §11). On the way, outlining reaches everywhere text is
  laid out: paragraph-mode boxes (each paragraph and list item in its own style, markers traced),
  and **point text** — a plain `<text>` on `x:outline="true"` or when forced — traced chunk by
  chunk with the anchor over each SVG text chunk, refusing per-glyph lists, `rotate`, `textLength`
  and `baseline-shift` rather than mis-tracing. `npm run check:outline` renders live and outlined
  text through resvg and requires the same pixels.

- **No nested `<svg>` in compiled output.** A nested `<svg>` — a cropped picture, the baked
  whole-file form of a cross-file `<use>` — now lowers to one clipped `<g>` whose transform folds
  the element's own, the frame's position and the `viewBox` fit (`preserveAspectRatio` honoured:
  `none`, `meet`, `slice`, every alignment), with the frame carried as
  `data-xsvg-viewport="x y w h"` in the group's own space for editors to read (§5.3). WebKit
  ignores a `transform` on a nested `<svg>`, so a rotated crop drew unrotated there; a `<g>` draws
  alike everywhere. Percent and missing sizes resolve against the enclosing viewport; a
  `filter`/`mask`/`clip-path` on the viewport wraps it in frame space; a viewport a
  `<use width|height>` sizes, or one whose frame cannot be resolved, is kept as `<svg>` with a
  marker and its transform hoisted. A linked drawing's `<use>` keeps its `transform`, `opacity`
  and `filter`, which the nested-`<svg>` form dropped.

- **Document queries for editors.** `CompileSession` answers what the compiler does with a node,
  from the parse it compiles from: `resolved(offset, properties)` (the effective value of each
  property and where it came from — own, token, inherited from the box, list, row, column, table
  or an ancestor, implied, initial), `theme()` (the tokens in document order, `x:doc` excluded),
  `text_style(offset)` (an element's text as pieces in the style each renders in, UTF-16 offsets),
  `blocks(offset)` (a box's paragraphs and list items) and `run_support(offset)` (which run
  attributes the element's text draws — by the compiler, or natively by the renderer for a plain
  `<text>`). A block's `align`, `direction` and `text-indent` cascade from its box as the
  paragraph planner reads them, the planner now reading the same rule. The text cascade is now data the emitters compute from
  and the queries report, so the two cannot disagree; pinned over the dataset
  (Incremental.md §10). The queries live on `ParsedDocument`, a borrowing parse a host keeps
  between flushes, and `CompileSession::from_parsed` compiles from one without parsing again:
  one parse per change, shared by every query and the flush.

- **Runs in curved region flow and `<text inline-size>`.** Both flattened their `<tspan>`s to the
  base style, losing fill and border along with weight, size and family. They now flow styled runs
  like a box — highlights and decorations baked behind `<text inline-size>`, an outlined region flow
  traced run by run — and text without runs compiles byte-identically (§6.11).

- **A run's border, and outlined runs' paint.** `x:border-*` on a `<tspan>` in `<x:textbox>`,
  `<textArea>`, a paragraph, list item or table cell now borders that run — each attribute it leaves
  out inherited from the enclosing run or the box, so a run naming only a colour keeps the box's
  width (§6.11). An outlined box (`outline="true"`, warped or not) honours its runs' fill and border:
  a painted line is traced run by run where the whole line puts each piece, all borders behind all
  fills (§6.12). Typography still stays the box's; unpainted boxes compile byte-identically.

- **Paint runs on text on a path.** A `<tspan>` inside `<x:textpath>` now repaints its stretch —
  its `fill` and its border (`x:border-*`, inherited attribute by attribute from the element) — on
  every effect; the run's typography stays the element's, so nothing moves. Outlined effects emit one
  path per stretch, traced where the whole string puts it, all borders behind all fills. Also: the
  element's own border now reaches live `follow` / `stair` text, where it was dropped (§6.13).

- **`overflow="visible"` on `<x:textbox>` and `<textArea>`** — nothing cropped: every line
  renders where layout placed it (`valign` placement kept, so middle-aligned overflow spills both
  ways), `text-overflow` has nothing to act on, and curved region flow continues past the shape at
  its bounding-box width. In editor output the lines a cropping box would drop are stamped
  `data-xsvg-overflow="1"` (tspans, outline paths, highlight and decoration rects, list markers),
  and `text_layout` flags them `overflow: true`, so an editor can show a box's hidden text as a
  ghost while it is edited — through a render override, without writing to the document (§6.6).

- **`text-transform` — render text in capitals (or small letters) whatever case it was typed in.**
  `uppercase` / `lowercase` / `none` on any text element the compiler emits, on `<tspan>` runs, and
  in an `<x:font>` token; it cascades within a text element (box › paragraph › run) like the font
  properties. The compiler applies it before layout, so wrapping, fitting, outlines and caret
  stops measure the capitals that render, and bakes the mapped characters into the output (SVG has
  no such attribute). The mapping is one character to one, so a caret offset in the source is the
  same offset in the output; `ß` becomes `ẞ` (§6.18).

- **Units: incremental compilation one level down** for editing hosts. A direct child of a
  top-level passthrough container (an artboard's `<g>`) is now a unit of its own: `compile_unit`
  compiles it alone, byte-identical to its span in the parent's fragment; `compile_fragment_spans`
  returns the fragment with every unit's span in output order; `unit_dependents` finds dependents
  inside the same container, which the top-level scan could not — a keystroke in a slide's textbox
  no longer re-bakes the slide's warps ([Incremental.md §9](docs/Incremental.md)).

- **Sizing a textbox to its content** for editing hosts. `measure_textbox_impl` (and
  `CompileSession::measure_textbox`) returns the height a textbox needs to hold its paragraphs and
  lists at its authored width — the cap band of the stacked blocks plus padding — and the width its
  content actually uses, through the same engine that renders it, so a pasted block can be given a
  box that just fits ([Incremental.md §8](docs/Incremental.md)).

- **Caret stops on outlined text** for editing hosts. `caret_stops_impl` (and
  `CompileSession::caret_stops`) returns the caret positions between the outlined glyphs of an
  `<x:textbox outline="true">` — through any `<x:warp>` it sits in — or an `<x:textpath>` effect,
  in the coordinates of the emitted paths. The compile records them as it traces and warps, so
  they agree with the rendered geometry by construction; an ordinary compile is unchanged. A
  query compiles only the element (or the warp it sits in), not its whole top-level fragment, so
  asking on every keystroke costs the element, not its artboard
  ([Incremental.md §7](docs/Incremental.md)).

- **`GlyphOutliner::advances`** replaces `advance_width`: the outline seam now answers the
  cumulative advance after each character of a run in one call (the last entry is the run's
  width), so caret stops and text-on-path placement shape a run once. The browser callback
  `advances(text, family, weight, style, size)` returns `number[]` (`[]` to decline) where
  `advanceWidth` returned a number; both JS adapters make one `forEachGlyph` pass.

- **Lists inside a textbox.** A `<x:list>` stands among a textbox's paragraphs as a block of the
  same stack: its items flow as paragraphs with the list's gutter and markers, under the box's
  shrink-to-fit, `valign` and padding — one box holds a heading, bullets and a closing paragraph
  (§6.16). The standalone list remains.

- **`transform` on `<x:table>` and `<x:list>`** passes through to the `<g>` each lowers to, as a
  textbox's does: rotating or scaling a table or a list is its transform (§6.14, §6.15). Found by
  the editor: a transform on a table was consumed and never re-emitted, so the table could not move.

- **Table column and row styles.** `<x:col>` children (one per column, HTML's `<col>`) and
  attributes on `<x:tr>` carry cell defaults — `align`, `valign`, `fill`, `bg`, `font-*`,
  `line-height` — resolving cell › row › column › header default › table, so a bold first
  column or a tinted header row is one declaration and an inserted row keeps the look (§6.15).

- **Connector anchors sit on the shape.** `to="#id:top"` on a rotated or skewed plain
  shape attaches to the shape's own top-edge midpoint, mapped through its transform,
  instead of the top of its axis-aligned bounds, where no edge of the shape lies; on an
  ellipse or circle the eight anchors lie on the circumference — the four axis points and
  the four diagonals at 45° — where a box's corners would float off the curve (§7.6).

- **`<x:doc>` — document settings.** The one element for an editor's document-level settings
  (artboard defaults and arrangement, a print size, …), dropped whole at compile with no marker
  and no validation; its children are the tool's, in the `x:` namespace by convention (§4.4).

- **`x:fill` on an artboard paints its frame.** A slide's backdrop used to be a hand-written
  `<rect>` repeating the frame's size; `<g x:artboard="…" x:frame="x y w h" x:fill="#0b1020">`
  now lowers that rect itself, as the group's first child, mapped through the group's
  transform so a placed slide gets it at its own origin (§5.2). The deck sample uses it.

### Changed

- **An invalid text style value falls through.** A `letter-spacing`, `word-spacing` or
  `text-transform` the compiler cannot read on an element now gives way to its `<x:font>` token's
  value, as an invalid `font-size` or `line-height` already did and as CSS ignores an invalid
  declaration (§4.1). Previously such a value read as the initial one and hid the token.

- **`<x:doc>` is a sealed island.** The compiler already dropped the editor's settings element
  from its output; now it never reads inside it at all: an `id` there cannot be referenced or
  shadow a real element's, an `<x:theme>` or `<x:import>` there binds nothing, a textbox there has
  no text layout, and an edit there invalidates nothing — a host can skip the render (§4.4).

- **Compiled text inherits `font-family`.** A textbox, list, table, text on a path or `<textArea>`
  with no `font-family` of its own now takes the nearest ancestor's — a `<g font-family>`, a
  `<g x:font>` token, or the root `<svg>` — the way plain `<text>` always has in the browser, and
  measures in it (§4.1). Before, it was measured and rendered in `sans-serif` whatever the document
  said, so a deck's root `font-family` reached its plain text but none of its boxes. Only the
  family inherits; `font-size` and the other font properties stay the element's own, so nothing
  above a box can reflow it. Documents that set a root `font-family` will see their compiled text
  switch to it.

### Fixed

- **Outlines honour `letter-spacing`, `word-spacing`, justification and `glyph-x-scale`.**
  A spaced or justified line was traced at the font's bare advances. A run with spacing is now
  traced glyph by glyph at the pens the live text is laid by — its kerned advance, the tracking
  after every glyph (the next run starting past the last one's), the word gap and the justify
  slack after each space — and a `glyph-x-scale` stretches the traced line from its left edge
  as the live `textLength` does. Pinned by `check:outline`: 0 px against live text on all of
  tracking, justification and glyph scale.

- **Outlined runs keep their type.** A bold, italic, differently sized or differently faced
  `<tspan>` in an outlined box was traced in the box's own face, its neighbours placed by the
  box's widths. A run is now traced in its own measuring style, the pen advanced by that style's
  width, from the line's anchored left edge (the edge the backdrops use; right-to-left lines lay
  runs leftward). `to_path` names the run's face when it is the one the outliner lacks.

- **Outlined glyphs are closed contours whichever outliner traced them.** opentype.js 2 emits
  no `Z`, drawing a line back to each contour's start instead; a fill closed it implicitly, but an
  editor's path tools saw an open shape. The compiler now closes every contour of every outline
  (`close_contours`): an open contour ends in `Z`, and a line back to the contour's start —
  which a font parser's builder draws before its own `Z`, and opentype.js draws instead of one —
  is dropped, so no node sits on the first at the seam. A well-formed outline passes through
  byte for byte. The viewer's regex workaround is gone. The warp bake had the same seam — the
  subdivided closing edge's last leg landed on the start before its `z` — and keeps `z` as that
  leg now, for warped glyphs and warped shapes alike. The contract is pinned over the `to_path`
  of every text element of every dataset sample, through the test outliner and the native CLI.

- **Outlined text kept its highlights and decorations.** `outline="true"` dropped a run's
  `x:highlight`, underline and strike-through; they bake as rects beside the paths now, as beside
  live text.

- **`describe()` declares enum parameters' defaults.** An enum parameter with no explicit default
  (`align`, `valign`, `fit`, `text-overflow`, a list's `list`, …) reads as its type's `#[default]`
  variant, but its spec said it had none; `ParamEnum` now carries a `DEFAULT` and the derive
  declares it.

- **`x:border-*` on plain `<text>`, `<tspan>` and `<textPath>`** lowers to the bordered-text stroke
  as on a textbox; it was dropped with the other `x:` attributes of passthrough text, so white text
  meant to sit on a black border rendered as plain white. With a border, the element's own
  `stroke*` attributes give way to it (§6.17).

- **Drop shadows (and every other `filter` function) on text.** `filter="drop-shadow(…)"` was
  dropped from `<x:textbox>`, copied through unlowered on `<text inline-size>`, and clipped on a
  live `<text>` (the ±50 % fallback region is shorter than a shadow on a single line). Every text
  element now takes the §8 lowering: the def before the element, the filter on the `<text>` or on
  one `<g>` around everything glyph-like (paragraphs, markers, baked decorations, outlines), the
  region computed from where the glyphs land, so each glyph casts its own shadow as in a slide
  tool. Highlights stay outside the filter. Text on a path included, every effect.

- **Underline and strike-through rendered dashed under `letter-spacing` in Safari.** WebKit draws
  SVG text decorations per glyph once letter-spacing is set, leaving every gap bare. A run's
  decoration is now baked, like `x:highlight`: a `<rect>` per run per line in the run's own colour,
  behind the text, as wide as the measured advance — letter-spacing and justified spacing included —
  so it draws the same in every renderer. Position and thickness come from the font's own tables
  where a measurer has them (the CLI, the Node package), else from where the font's underscore sits
  (browser hosts probe it), else from proportions (§6.11). The `text-decoration` attribute is no
  longer written on the output.

- **Underline and strike-through on justified lines rendered dashed.** Justification lowered to
  `textLength` + `lengthAdjust="spacing"`, and renderers draw a stretched line's decorations per
  glyph, leaving the stretched gaps bare. A justified line now carries its slack as `word-spacing`
  on the line — spaces widen, glyph gaps and shapes stay — so decorations draw whole, highlight
  rects land on the widened spaces, and a caret on a justified line is exact (§6.9). Output of
  justified text changes accordingly.

- **Paragraph mode ignored `text-overflow="ellipsis"`.** A textbox with `<x:p>` children
  dropped the lines that fell past its bottom but never marked the cut, so a truncated
  card read as complete. The block's last rendered line now ends in `…`, whichever
  paragraph it falls in, and a line wider than its paragraph is cut to fit, as §6.6 says
  for every other flow. The editor's text-layout seam reports the same rewritten line.

### Internal

- Text layout consolidated, with no output change: one clip-and-truncate pass
  (`truncate::clip_lines`) for the area, textArea and paragraph flows, one shrink-to-fit
  bisection (`fit::bisect_fit`), and paragraph stacking moved out of the emitter into a
  `text::paragraphs` engine that the textbox, the standalone list and the editor's
  text-layout seam all share.

## [0.2.2] — 2026-10-08

**A security fix and a round of text typesetting.** Opening an untrusted `.xsvg` in the
viewer or the VS Code extension could run script: an authored attribute value containing
a quote broke out of its attribute and added an event handler. Every value is now escaped
where it is written. Upgrading is recommended for anyone who opens documents they did not
write.

Text runs gain sizes, super/subscripts, decoration, links and highlights, paragraphs gain
right-to-left direction, and list items and table cells finally render their styled runs.
Existing documents compile the same, apart from the fixes below: of the 51 dataset samples,
44 are byte-identical to 0.2.1's output and the other 7 render the same or better.

The npm packages, the wasm module and the CLI keep their interfaces. The `xsvg-core` Rust
API changed (see *Internal*), which matters only to code that embeds the compiler directly.

### Security

- **Authored values could break out of their attribute and add event handlers.** About
  three dozen emitters formatted values straight into their output, so a `"` in a value
  closed the attribute and opened another: `fill='#000" onload="…'` compiled to a live
  `onload` on text, connectors, charts, tables, lists, textpaths, highlights, theme
  tokens, and the viewport a cross-file link bakes. Every value is now escaped where it
  is written (theme tokens after they resolve, since a token's value is authored too),
  and `preserveAspectRatio` is forwarded only when it is a value SVG defines.
  Legitimate documents compile byte-identically.
- **Generator output could bypass the sandbox by emitting xsvg text.** The output rules
  covered the attributes a generator wrote directly, but a module emitting an
  `<x:textbox>` could carry a run's `javascript:` link, a remote `url()` highlight or a
  remote paint through the text emitters. The rules now run as one pass over the
  generator's markup *after* it is lowered, whatever it lowered through. The pass also
  drops `<style>` (it restyles the whole document and can `@import`), and stops a
  generator's `<use href="file">` from making the host read another file.

### Added

- **Mixed sizes in one paragraph.** A `<tspan>` takes `font-size`, as a length or as a
  percentage or `em` of the paragraph's size. A line steps by the largest run on it, as a
  word processor does, and fit and wrapping account for it.
- **Superscript and subscript.** `baseline-shift` on a run takes `super`, `sub`, a length
  or a percentage. Pair it with a smaller `font-size` for footnote marks and formulas.
- **Underline, strike-through and overline.** `text-decoration` on a run takes
  `underline`, `line-through`, `overline`, or a space-separated list. An underline stops at
  the end of a word and continues across the space only between two underlined words.
- **Links in text.** `href` on a `<tspan>`, or an authored `<a>` around a run, becomes a
  clickable SVG link in the output.
- **The highlighter.** `x:highlight="#fde047"` on a run draws a background behind it, one
  rectangle per line the run spans. It follows the run's size, a justified line's
  stretch, and its `baseline-shift`.
- **Right-to-left paragraphs.** `direction="rtl"` on a textbox, or on one `<x:p>`, makes the
  start edge the right one: `align="start"` right-aligns, `end` left-aligns, and the
  first-line indent insets from the right. Glyph order within a line is left to the
  renderer's bidi algorithm.
- **Styled runs in lists and tables.** Bold, colour, size and the rest inside a bullet or a
  table cell now render; before, they were flattened to plain text. `<tbreak/>` breaks a
  line inside an item or a cell, and a row grows to fit.
- **Fitting a linked drawing to its frame.** `preserveAspectRatio` on a cross-file
  `<use href="file.xsvg">` is forwarded to the viewport it becomes, so `none` stretches
  the drawing to the frame instead of letterboxing it.
- **Editor output marks a textbox's frame.** Keyed output (the editor's mode) stamps each
  textbox with `data-xsvg-frame="x y w h"`, the box the text flows in, so an editor can
  select and resize the box rather than the ink. Viewer output is unchanged.

### Fixed

- **A rotated or scaled textbox rendered upright and unscaled.** `transform` on an
  `<x:textbox>` was consumed with the rest of its attributes and never re-emitted. It is
  now carried to the element the box lowers to, as on a plain `<text>`. A document that
  carried a stray transform on a textbox will now show it.
- **A list item opening with a large run overlapped the item above.** The step into an
  item used the item's base size. Every line now advances by its own size.
- **A highlighted superscript kept its highlight on the baseline below it.** The
  highlight now moves with the run's `baseline-shift`.
- **A line of large type could render wider than its box.** A space inside a larger run
  was measured at the paragraph's size when wrapping, though it renders at the run's.
- **Mesh gradients render exactly in resvg.** Mesh images now ask for bilinear smoothing
  by name, which resvg needs; its default is bicubic, which drifted up to ~9/255 off the
  colour field. Browsers render the new output as before.
- **A text path could crash the compiler.** A coordinate large enough to overflow the
  path's length panicked instead of compiling. Found by the new fuzz target.

### Known limitations

- Highlights under `direction="rtl"` do not handle mixed bidi: an embedded left-to-right
  word or number is reordered by the renderer and its highlight lands on the wrong glyphs.
- Lists and tables stay left-to-right.

### Internal

- `xsvg-core`: a `CompileSession` parses a document once and compiles any number of its
  fragments from that parse, for editors. `Seams` became an owned `Platform`;
  `compile_seams_impl` and `compile_fragment_seams_impl` are now `compile_platform_impl`
  and `compile_fragment_platform_impl`; `compile_with(input, &Options, &platform)` is the
  single entry the others wrap. A `MeasureCache` and a `Measurer::measure_batch` seam
  avoid re-measuring text across compiles. The theme is no longer a thread-local.
- CI runs on every push: formatting, clippy, tests, both wasm targets, the web build,
  every `check:*` script, and a minute of fuzzing. `npm run check:mesh` checks mesh output
  against resvg pixel by pixel. `tests/attribute_injection.rs` probes every attribute of
  every sample for the escaping bug above.
- `scripts/compare.mjs` reads its server from `XSVG_SERVER` and refuses a foreign page.
  `scripts/bump.mjs` covers every crate. `scripts/publish.sh` takes the GitHub release
  notes from this changelog and no longer labels a real publish as a dry run. `docs/Sandbox.md` no longer claims the Node
  compiler bounds a generator's run time; only the CLI does.

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
