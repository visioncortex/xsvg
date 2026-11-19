# Changelog

All notable changes, one sentence each with the commit that introduced it. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). One version covers the whole project —
the npm packages and every published crate — and releases together via `scripts/publish.sh`.

## [Unreleased]

## [0.3.0] — 2026-11-19

Meshes as any paint, live corners, outlined text that matches live text, richer text blocks, and editor queries with convert-to-path; the breaking API changes are listed under Changed.

### Added

- **Mesh fit cache.** `Platform::with_mesh_cache` lets the compiler keep fitted meshes between compiles, so only a changed mesh is fitted again. ([37a7d85](https://github.com/visioncortex/xsvg/commit/37a7d85))
- **Rounded viewports.** A nested `<svg>` takes `x:corners` and `x:corner-style` to round its clip as a `<rect>` does. ([b7bdca1](https://github.com/visioncortex/xsvg/commit/b7bdca1))
- **Mesh swatches.** `CompileSession::mesh_definitions()` lists a document's meshes and `mesh_swatch()` compiles one on its own as a standalone SVG for a palette. ([37a7d85](https://github.com/visioncortex/xsvg/commit/37a7d85))
- **`<x:mesh>` as a fill.** A shape can name an `<x:mesh>` as its fill, as it already could a `<meshgradient>`. ([d5aac40](https://github.com/visioncortex/xsvg/commit/d5aac40))
- **A mesh as any paint.** A mesh can also paint a stroke, use bounding-box units, and paint booleans, offsets and outlined text. ([81823d5](https://github.com/visioncortex/xsvg/commit/81823d5))
- **Mesh textures at display resolution.** An option emits a mesh's texture at a chosen pixel density instead of its compact grid, for renderers that cannot rely on bilinear filtering. ([0ec57a2](https://github.com/visioncortex/xsvg/commit/0ec57a2))
- **Live corners.** `x:corners` and `x:corner-style` on `<path>`, `<rect>`, `<polygon>` and `<polyline>` round, chamfer or scallop any corner like a design tool's live corner, compiled to exact arcs. ([5375f03](https://github.com/visioncortex/xsvg/commit/5375f03))
- **A corner style per anchor.** `x:corner-style` takes per-anchor styles in the same form as `x:corners`. ([44e88bc](https://github.com/visioncortex/xsvg/commit/44e88bc))
- **Spacing on text blocks.** `letter-spacing` and `word-spacing` work on paragraphs, list items, table cells, rows and columns. ([ce7399d](https://github.com/visioncortex/xsvg/commit/ce7399d))
- **List items take the whole type block.** An `<x:li>` can override any font, spacing, case or fill property of its list. ([2f895c0](https://github.com/visioncortex/xsvg/commit/2f895c0))
- **`text-transform`.** `uppercase` and `lowercase` render text in capitals or small letters whatever case it was typed in, on any text element, run or font token. ([8fbda9d](https://github.com/visioncortex/xsvg/commit/8fbda9d))
- **`overflow="visible"` on text boxes.** A text box can show every line instead of cropping, and editor output marks the lines a cropping box would hide. ([a2358d0](https://github.com/visioncortex/xsvg/commit/a2358d0))
- **Lists inside a textbox.** An `<x:list>` can sit among a textbox's paragraphs, so one box holds a heading, bullets and a closing paragraph. ([f6e9831](https://github.com/visioncortex/xsvg/commit/f6e9831))
- **Table column and row styles.** `<x:col>` children and `<x:tr>` attributes set cell defaults such as alignment, fill and font for a whole column or row. ([5d11adc](https://github.com/visioncortex/xsvg/commit/5d11adc))
- **`transform` on tables and lists.** A transform on `<x:table>` or `<x:list>` is now kept, so tables and lists can be moved, rotated and scaled. ([41e03e4](https://github.com/visioncortex/xsvg/commit/41e03e4))
- **Styled runs in curved region flow and `<text inline-size>`.** Their `<tspan>` runs keep their own fill, border, weight, size and family. ([e0fc317](https://github.com/visioncortex/xsvg/commit/e0fc317))
- **Run borders and outlined run paint.** A `<tspan>` in a text box can carry its own border, and outlined boxes now honour each run's fill and border. ([a0cb297](https://github.com/visioncortex/xsvg/commit/a0cb297))
- **Paint runs on text on a path.** A `<tspan>` inside `<x:textpath>` repaints its stretch with its own fill and border on every effect. ([5a69a7d](https://github.com/visioncortex/xsvg/commit/5a69a7d))
- **Connector anchors turn with the shape.** An anchor on a rotated or skewed shape attaches to the shape's own edge rather than its axis-aligned bounds. ([63374e9](https://github.com/visioncortex/xsvg/commit/63374e9))
- **Connector anchors on ellipses.** An ellipse's or circle's eight anchors lie on its circumference. ([a2dd9bc](https://github.com/visioncortex/xsvg/commit/a2dd9bc))
- **`<x:doc>` for document settings.** An editor's document-level settings live in one element the compiler drops whole and never reads. ([cd533e4](https://github.com/visioncortex/xsvg/commit/cd533e4))
- **Artboard backdrops.** `x:fill` on an artboard paints its frame, replacing a hand-written backdrop `<rect>`. ([f36aa04](https://github.com/visioncortex/xsvg/commit/f36aa04))
- **The CLI shapes text.** `xsvg-cli` measures and outlines text with rustybuzz (kerning, ligatures, marks), so its output matches what resvg draws. ([f518b62](https://github.com/visioncortex/xsvg/commit/f518b62))

*For editing hosts* (`xsvg-core`, [docs/Incremental.md](docs/Incremental.md)):

- **Convert to path.** `to_path(offset)` returns a node as source paths an editor can write back in its place, converting text to outlined glyphs and shapes to `<path>` with corners left live, and keeping authored attributes. ([9b58fd7](https://github.com/visioncortex/xsvg/commit/9b58fd7))
- **CLI `--to-path`.** The CLI converts a node to paths from the command line. ([d3bd410](https://github.com/visioncortex/xsvg/commit/d3bd410))
- **Expand to spline.** `to_spline(offset)` (and CLI `--to-spline`) is `to_path` with corners baked and every arc written as cubic Béziers. ([7ce4595](https://github.com/visioncortex/xsvg/commit/7ce4595))
- **`xsvg-path-ops`.** A new crate performs boolean operations on paths keeping the original curves, fed by the core's `boolean_operands` query. ([0fe6860](https://github.com/visioncortex/xsvg/commit/0fe6860))
- **Convert offsets, warps, connectors and groups.** `to_path` also converts `<x:offset>`, `<x:warp>`, `<x:connector>` and every convertible child of a `<g>`. ([3152dd7](https://github.com/visioncortex/xsvg/commit/3152dd7))
- **Outline lists, tables and wrapped text.** `outline="true"` traces `<x:list>`, `<x:table>` and `<text inline-size>`, and `to_path` converts all three. ([8cf9b53](https://github.com/visioncortex/xsvg/commit/8cf9b53))
- **Document queries.** `CompileSession` reports what the compiler does with a node — `resolved`, `theme`, `text_style`, `blocks`, `run_support` and `corners` — from the same data the output is computed from. ([29c10f9](https://github.com/visioncortex/xsvg/commit/29c10f9))
- **`ParsedDocument`.** A host can keep a parse between edits, query it, and compile from it without parsing again. ([0786a16](https://github.com/visioncortex/xsvg/commit/0786a16))
- **The typography matrix in the manifest.** The `describe` manifest now lists the text blocks and plain SVG text elements with which typography properties each takes. ([2f895c0](https://github.com/visioncortex/xsvg/commit/2f895c0))
- **Runs, corners and `x:` attributes in the manifest.** The manifest also declares `<tspan>` runs, the corner attributes on shapes, the `x:` attributes on SVG elements, and where inherited block defaults come from. ([688642a](https://github.com/visioncortex/xsvg/commit/688642a))
- **Units one level down.** Each child of an artboard group compiles as its own unit, so an edit in one slide element no longer recompiles its neighbours. ([d5d0748](https://github.com/visioncortex/xsvg/commit/d5d0748))
- **Sizing a textbox to its content.** `measure_textbox` returns the size a textbox needs to hold its content at its authored width. ([99a5525](https://github.com/visioncortex/xsvg/commit/99a5525))
- **Caret stops on outlined text.** `caret_stops` returns caret positions between the outlined glyphs of an outlined text box or text-path effect, in the emitted geometry. ([723b713](https://github.com/visioncortex/xsvg/commit/723b713))

### Changed

- **Mesh region clips are outlines.** A mesh region is now clipped by the outline of its faces instead of one polygon per face, so output is far smaller and overlapping faces of opposite winding stay filled instead of cancelling to a hole. ([36702f2](https://github.com/visioncortex/xsvg/commit/36702f2))
- **`<meshgradient>` definitions are not emitted.** The definition is consumed at compile and only its lowered paint reaches the output. ([5a105bf](https://github.com/visioncortex/xsvg/commit/5a105bf))
- **Compiled text inherits `font-family`.** Textboxes, lists, tables, text on a path and `<textArea>` without their own `font-family` now take the nearest ancestor's instead of always using `sans-serif`. ([81bddcc](https://github.com/visioncortex/xsvg/commit/81bddcc))
- **No nested `<svg>` in compiled output.** A nested `<svg>` now lowers to a clipped `<g>`, so rotated crops draw correctly in WebKit and a linked drawing's `<use>` keeps its `transform`, `opacity` and `filter`. ([407062d](https://github.com/visioncortex/xsvg/commit/407062d))
- **An invalid text style value falls through.** An unreadable `letter-spacing`, `word-spacing` or `text-transform` now gives way to the `<x:font>` token's value instead of hiding it. ([29c10f9](https://github.com/visioncortex/xsvg/commit/29c10f9))
- **`GlyphOutliner::advances` replaces `advance_width`.** Custom outliners and raw wasm `compile_ext` callers must now return a run's cumulative advances in one call instead of a single width. ([143517b](https://github.com/visioncortex/xsvg/commit/143517b))
- **`xsvg-schema`: `ElementSpec::ns`.** A hand-written `ElementSpec` impl must now supply a namespace, since an element may differ from its manifest's. ([2f895c0](https://github.com/visioncortex/xsvg/commit/2f895c0))
- **`xsvg-schema`: `ParamEnum::DEFAULT`.** A hand-written `ParamEnum` impl must now name its default variant. ([29c10f9](https://github.com/visioncortex/xsvg/commit/29c10f9))
- **`Options` and `Backend` gain public fields.** Code that builds either with a struct literal must now end it with `..Default::default()`. ([0ec57a2](https://github.com/visioncortex/xsvg/commit/0ec57a2))

### Fixed

- **A mesh-filled shape keeps what was not paint.** A mesh-filled shape no longer loses its `id`, `opacity`, `clip-path`, `mask` or `display`, and now honours `fill-rule`, `fill-opacity` and `paint-order`. ([81823d5](https://github.com/visioncortex/xsvg/commit/81823d5))
- **A mesh no longer darkens at its rim.** Texels the mesh does not reach now continue the nearest edge's colour instead of being black. ([352ba6a](https://github.com/visioncortex/xsvg/commit/352ba6a))
- **A sliver of a mesh no longer costs unbounded memory.** Very elongated meshes are now fitted squeezed and stretched back instead of asking for a huge raster. ([81823d5](https://github.com/visioncortex/xsvg/commit/81823d5))
- **Mesh-painted children of `<x:boolean>` and `<x:warp>`.** Such a child now contributes its geometry instead of its lowered paint's clips and textures. ([81823d5](https://github.com/visioncortex/xsvg/commit/81823d5))
- **A fractional viewport fit keeps its precision.** Crops under a fractional `viewBox` no longer drift off their frame from rounding. ([a015a62](https://github.com/visioncortex/xsvg/commit/a015a62))
- **Boolean operands keep their transforms.** A transformed group or nested boolean used as an operand is no longer combined at the origin. ([ad77746](https://github.com/visioncortex/xsvg/commit/ad77746))
- **An empty text box lays one blank line where typing would start.** Empty text boxes now emit a positioned blank line so an editor's caret has somewhere to sit. ([ba14352](https://github.com/visioncortex/xsvg/commit/ba14352))
- **Outlines honour `letter-spacing`, `word-spacing`, justification and `glyph-x-scale`.** Outlined text now matches the live text's spacing instead of using the font's bare advances. ([dffe2bc](https://github.com/visioncortex/xsvg/commit/dffe2bc))
- **Outlined runs keep their type.** Bold, italic, resized or differently faced `<tspan>`s in an outlined box are now traced in their own style instead of the box's. ([5ee6735](https://github.com/visioncortex/xsvg/commit/5ee6735))
- **Outlined glyphs are closed contours whichever outliner traced them.** Every outlined contour now ends in `Z`, so editors no longer see open shapes from opentype.js. ([6fa6a12](https://github.com/visioncortex/xsvg/commit/6fa6a12))
- **No repeated node at a glyph contour's seam.** A closing line back to the contour's start is now dropped so no node sits on top of the first. ([e42345e](https://github.com/visioncortex/xsvg/commit/e42345e))
- **Warped paths close cleanly.** The warp bake no longer leaves a duplicate node at the seam of a closed contour, for glyphs and shapes alike. ([5c0b6b7](https://github.com/visioncortex/xsvg/commit/5c0b6b7))
- **Outlined text kept its highlights and decorations.** `outline="true"` now keeps a run's highlight, underline and strike-through instead of dropping them. ([9b58fd7](https://github.com/visioncortex/xsvg/commit/9b58fd7))
- **`direction` on standalone lists and tables.** `direction` now works on standalone list items, lists and tables, not only inside a text box. ([da78706](https://github.com/visioncortex/xsvg/commit/da78706))
- **An empty list item and an empty table cell lay one blank line.** Empty list items and table cells now give a caret a place to sit, like every other empty flow. ([da78706](https://github.com/visioncortex/xsvg/commit/da78706))
- **A spacing with a relative unit or a `var()` is invalid, not its leading number.** Values like `letter-spacing="50%"` are now rejected instead of being misread as user units. ([e855028](https://github.com/visioncortex/xsvg/commit/e855028))
- **`describe()` declares enum parameters' defaults.** Enum parameters such as `align` and `fit` now report their default in `describe()`. ([29c10f9](https://github.com/visioncortex/xsvg/commit/29c10f9))
- **`x:border-*` on plain `<text>`, `<tspan>` and `<textPath>`.** Text borders now render on plain SVG text instead of being dropped. ([129a3f5](https://github.com/visioncortex/xsvg/commit/129a3f5))
- **Drop shadows (and every other `filter` function) on text.** Filters such as `drop-shadow()` now render correctly on every text element instead of being dropped, passed through or clipped. ([2be2720](https://github.com/visioncortex/xsvg/commit/2be2720))
- **Underline and strike-through rendered dashed under `letter-spacing` in Safari.** Decorations are now baked as rects so they draw whole in every renderer. ([f2163fb](https://github.com/visioncortex/xsvg/commit/f2163fb))
- **Underline and strike-through on justified lines rendered dashed.** Justified lines now stretch through `word-spacing`, so decorations draw whole and carets land exactly. ([b459a33](https://github.com/visioncortex/xsvg/commit/b459a33))
- **Paragraph mode ignored `text-overflow="ellipsis"`.** A truncated textbox with `<x:p>` paragraphs now ends its last visible line in `…`. ([ef978b8](https://github.com/visioncortex/xsvg/commit/ef978b8))

### Known limitations

- **`xsvg-path-ops` can panic** on rare ordinary geometry, which aborts the module in wasm unless the crate runs in its own Worker.
- **Point text that `to_path` cannot place faithfully is refused** when it uses per-glyph positions, `rotate`, `textLength`, `baseline-shift` or `<textPath>`.

### Internal

- **`xsvg-path-ops` in CI.** The crate is now built for wasm and gated by a fuzz regression range in CI. ([4a03053](https://github.com/visioncortex/xsvg/commit/4a03053))
- **Path data is found by reading tags.** `to_spline` and the CLI's curve swap now read `d` as an attribute instead of finding it by substring. ([4a03053](https://github.com/visioncortex/xsvg/commit/4a03053))
- **One clip-and-truncate pass and one fit bisection.** Every text flow now shares the same truncation and shrink-to-fit code, with no output change. ([6972374](https://github.com/visioncortex/xsvg/commit/6972374))
- **Paragraph stacking is a shared engine.** Paragraph stacking moved out of the emitter into an engine the textbox and the editor's layout seam share. ([59ef552](https://github.com/visioncortex/xsvg/commit/59ef552))
- **Standalone lists stack through the paragraph engine.** The standalone list now uses the same paragraph engine, with no output change. ([961f7fc](https://github.com/visioncortex/xsvg/commit/961f7fc))
- **`arcs_to_cubics` moved.** It now lives in `xsvg_core::spline`, outside the `compile` feature. ([ed15d50](https://github.com/visioncortex/xsvg/commit/ed15d50))
- **The CLI links `xsvg-path-ops`.** The CLI uses it for `--to-path` on an `<x:boolean>`, while the core never links it. ([0fe6860](https://github.com/visioncortex/xsvg/commit/0fe6860))

## [0.2.2] — 2026-10-08

A security fix for script injection through authored attribute values, plus a round of text typesetting; the npm packages, wasm module and CLI keep their interfaces, and only the `xsvg-core` Rust API changed.

### Security

- **Authored values could break out of their attribute.** A `"` in an authored value could close its attribute and add a live event handler such as `onload`, so opening an untrusted `.xsvg` in the viewer or VS Code extension could run script; every value is now escaped where it is written. ([3d75ba9](https://github.com/visioncortex/xsvg/commit/3d75ba9))
- **Generator output could slip past the sandbox by emitting xsvg text.** A module emitting an `<x:textbox>` could carry a `javascript:` link or remote `url()` through the text emitters, so the output rules now run over generator markup after it is lowered, also dropping `<style>` and blocking `<use href="file">` reads. ([3d75ba9](https://github.com/visioncortex/xsvg/commit/3d75ba9))

### Added

- **Mixed sizes in one paragraph.** A `<tspan>` takes `font-size`, and each line steps by its largest run. ([85fa78c](https://github.com/visioncortex/xsvg/commit/85fa78c))
- **Superscript and subscript.** `baseline-shift` on a run takes `super`, `sub`, a length or a percentage. ([85fa78c](https://github.com/visioncortex/xsvg/commit/85fa78c))
- **Underline, strike-through and overline.** `text-decoration` on a run draws them, continuing across a space only between two decorated words. ([da10202](https://github.com/visioncortex/xsvg/commit/da10202))
- **Links in text.** `href` on a `<tspan>`, or an `<a>` around a run, becomes a clickable SVG link. ([cd9eefc](https://github.com/visioncortex/xsvg/commit/cd9eefc))
- **The highlighter.** `x:highlight` on a run draws a background behind it on every line it spans. ([a007e88](https://github.com/visioncortex/xsvg/commit/a007e88))
- **Right-to-left paragraphs.** `direction="rtl"` on a textbox or `<x:p>` makes the right edge the start edge for alignment and indent. ([a751890](https://github.com/visioncortex/xsvg/commit/a751890))
- **Styled runs in lists and tables.** Bold, colour, size and other run styling inside bullets and table cells now render instead of flattening to plain text, and `<tbreak/>` breaks a line there. ([c7a2b97](https://github.com/visioncortex/xsvg/commit/c7a2b97))
- **Fitting a linked drawing to its frame.** `preserveAspectRatio` on a cross-file `<use href="file.xsvg">` is forwarded to the viewport it becomes. ([1fdd320](https://github.com/visioncortex/xsvg/commit/1fdd320))
- **Editor output marks a textbox's frame.** Keyed (editor) output stamps each textbox with `data-xsvg-frame` so an editor can select the box rather than the ink. ([8e5853f](https://github.com/visioncortex/xsvg/commit/8e5853f))

### Fixed

- **A rotated or scaled textbox rendered upright.** `transform` on an `<x:textbox>` is now carried to the element it lowers to. ([544ecdf](https://github.com/visioncortex/xsvg/commit/544ecdf))
- **A list item opening with a large run overlapped the item above.** Every line now advances by its own size. ([f5f1fcc](https://github.com/visioncortex/xsvg/commit/f5f1fcc))
- **A highlighted superscript left its highlight on the baseline.** The highlight now moves with the run's `baseline-shift`. ([f5f1fcc](https://github.com/visioncortex/xsvg/commit/f5f1fcc))
- **A line of large type could render wider than its box.** Spaces inside a larger run are now measured at the run's size when wrapping. ([f5f1fcc](https://github.com/visioncortex/xsvg/commit/f5f1fcc))
- **Mesh gradients render exactly in resvg.** Mesh images now request bilinear smoothing by name, which resvg needs to reproduce the colour field. ([12f46ba](https://github.com/visioncortex/xsvg/commit/12f46ba))
- **A text path could crash the compiler.** A coordinate large enough to overflow the path's length no longer panics. ([f37a002](https://github.com/visioncortex/xsvg/commit/f37a002))

### Known limitations

- Highlights under `direction="rtl"` land on the wrong glyphs when a left-to-right word or number is embedded.
- Lists and tables stay left-to-right.

### Internal

- **`CompileSession`.** `xsvg-core` can parse a document once and compile any number of its fragments from that parse, and the theme is no longer a thread-local. ([67da20b](https://github.com/visioncortex/xsvg/commit/67da20b))
- **`Seams` became an owned `Platform`.** The `*_seams_impl` entries are renamed `*_platform_impl`, and a `MeasureCache` avoids re-measuring text across compiles. ([90438b7](https://github.com/visioncortex/xsvg/commit/90438b7))
- **`Measurer::measure_batch`.** A new seam lets the measurer take a whole batch, and the cache forwards only its misses. ([c923b20](https://github.com/visioncortex/xsvg/commit/c923b20))
- **One compile entry.** `compile_with(input, &Options, &platform)` is the single entry the other compile functions wrap. ([daffcf1](https://github.com/visioncortex/xsvg/commit/daffcf1))
- **CI on every push.** Formatting, clippy, tests, both wasm targets, the web build and every `check:*` script run on each push. ([48ce6fd](https://github.com/visioncortex/xsvg/commit/48ce6fd))
- **Fuzzing.** A fuzz target drives every compile entry over arbitrary input, and CI runs it for a minute per push. ([f37a002](https://github.com/visioncortex/xsvg/commit/f37a002))
- **`npm run check:mesh`.** Checks mesh output against resvg pixel by pixel. ([12f46ba](https://github.com/visioncortex/xsvg/commit/12f46ba))
- **`tests/attribute_injection.rs`.** Probes every attribute of every sample for the escaping bug above. ([3d75ba9](https://github.com/visioncortex/xsvg/commit/3d75ba9))
- **`scripts/compare.mjs`.** Reads its server from `XSVG_SERVER` and refuses a foreign page. ([2b119df](https://github.com/visioncortex/xsvg/commit/2b119df))
- **`scripts/bump.mjs`.** Now covers every workspace crate. ([216396b](https://github.com/visioncortex/xsvg/commit/216396b))
- **Release notes from the changelog.** `scripts/publish.sh` takes the GitHub release notes from this changelog. ([ad921cc](https://github.com/visioncortex/xsvg/commit/ad921cc))
- **No false dry-run label.** `scripts/publish.sh` no longer labels a real publish as a dry run. ([defb889](https://github.com/visioncortex/xsvg/commit/defb889))
- **Sandbox docs corrected.** `docs/Sandbox.md` no longer claims the Node compiler bounds a generator's run time, since only the CLI does. ([216396b](https://github.com/visioncortex/xsvg/commit/216396b))

## [0.2.1] — 2026-08-12

A security patch that stops generator output from reaching the network, released as a patch so it reaches `^0.2.0`, and the release where the Rust crates rejoin at 0.2.1 (`xsvg-schema`, `xsvg-schema-derive` and `xsvg-plugin` reach crates.io for the first time).

### Added

- **[docs/Sandbox.md](docs/Sandbox.md).** The plugin sandbox model is written down: who is trusted with what, which component enforces each guarantee, and what is not guaranteed. ([3668a7b](https://github.com/visioncortex/xsvg/commit/3668a7b))

### Security

- **Generator output could reach the network.** A module could emit a `javascript:` link, a remote `<image href>`, a `<foreignObject>` or a remote `url()` paint, which combined with container generators could exfiltrate document content, so generator output (not authored markup) is now limited to same-document `#fragment` and `data:image/…` references, drops `<foreignObject>`, and reports removals in a marker. ([3668a7b](https://github.com/visioncortex/xsvg/commit/3668a7b))
- **Generator-emitted ids could shadow authored ones.** Ids from a generator are now namespaced to the element that produced them, so they can no longer capture the document's own paint references or `<use href="#…">`. ([3668a7b](https://github.com/visioncortex/xsvg/commit/3668a7b))
- **The plugin loaders had no tests.** `npm run check:sandbox` now verifies against a real module that a wrong hash, a cross-origin module without a hash and an unparseable location are all refused. ([928ae60](https://github.com/visioncortex/xsvg/commit/928ae60))
- **`data:` and `blob:` module sources slipped the native hosts' check.** A module `src` is now refused by scheme rather than by looking for `://` (not exploitable, but narrower than the documented rule). ([928ae60](https://github.com/visioncortex/xsvg/commit/928ae60))

### Fixed

- **An element edited into invisibility could not be edited back.** The inspector now keeps a pinned element's attributes editable when it renders nothing, such as an arrow dragged to `thickness="0"`. ([69fa0e1](https://github.com/visioncortex/xsvg/commit/69fa0e1))
- **Long attribute values are elided in the inspector.** Values over 96 characters show their head, with the full value in the row's tooltip. ([69fa0e1](https://github.com/visioncortex/xsvg/commit/69fa0e1))

## [0.2.0] — 2026-08-02

A minor release because the same document can compile to different bytes (anchored connectors route differently, and `sourcemap` output gains spans on tables and pies), and the inspector panel drops geometry controls and gains `x:border-*` attributes, which hosts must write with the document's own prefix.

### Added

- **Loadable geometry generators.** `<x:import ns src integrity>` binds a namespace to a WASM module, so a document can use elements the compiler does not know, and they compose like built-ins. ([210acc8](https://github.com/visioncortex/xsvg/commit/210acc8))
- **`xsvg-plugin`.** An SDK for writing generators by implementing one trait and calling one macro. ([4fe74d9](https://github.com/visioncortex/xsvg/commit/4fe74d9))
- **`xsvg-schema` and `xsvg-schema-derive`.** Declare an element's parameters as a Rust struct and derive both its manifest and the code that reads it, so the two cannot drift. ([b59e948](https://github.com/visioncortex/xsvg/commit/b59e948))
- **The compiler describes its own elements.** Built-in `x:` elements expose their parameters through `describe()` in the same shape as a plugin manifest, so a property panel treats both alike. ([63826b5](https://github.com/visioncortex/xsvg/commit/63826b5))
- **`clipart`, the first official extension.** It provides `<arrow>` and `<callout>` and loads through the same public seam as any other plugin. ([4fe74d9](https://github.com/visioncortex/xsvg/commit/4fe74d9))
- **Generators in the browser.** The viewer fetches, integrity-checks and runs a document's `<x:import>` modules, isolated from each other and between compiles. ([dad4178](https://github.com/visioncortex/xsvg/commit/dad4178))
- **Generators in the CLI and Node.** Both native hosts run the same module bytes as the browser, bound runaway generators with fuel, and can opt out with `xsvg --no-plugins` or `compile(src, { plugins: false })`. ([df83991](https://github.com/visioncortex/xsvg/commit/df83991))
- **Parameter controls from a module's manifest.** Pinning a generator element in the interactive viewer's inspector offers typed, editable fields for the parameters its module declares. ([eab0adf](https://github.com/visioncortex/xsvg/commit/eab0adf))
- **Live editing.** An edit now patches only the affected elements into the rendered drawing, keeping pan/zoom, selection and the control under the pointer, so sliders commit as they move. ([a6e2576](https://github.com/visioncortex/xsvg/commit/a6e2576))
- **Incremental compilation from Node and the browser.** `@visioncortex/xsvg-compile` gains `compileFragment`, `fragmentRange`, `dependents` and `normalizeSource`, and the browser gains a `CompileSession` via `onSession`. ([a6e2576](https://github.com/visioncortex/xsvg/commit/a6e2576))
- **`npm run check:incremental`.** Requires every patch the viewer would make across the dataset to be byte-identical to a full recompile. ([a6e2576](https://github.com/visioncortex/xsvg/commit/a6e2576))
- **Host node keys.** A host can name every element through an optional `key` callback, so an element compiles to the same bytes however far an edit elsewhere has moved it. ([9f8cef6](https://github.com/visioncortex/xsvg/commit/9f8cef6))
- **Unkeyed elements are reported.** A keyed compile that had to fall back to byte spans says so in a trailing comment in its output. ([9ee6220](https://github.com/visioncortex/xsvg/commit/9ee6220))
- **`npm run check:dom`.** Runs the real `patchDocument` over a jsdom tree and requires the result to match a fresh full recompile. ([c31f04c](https://github.com/visioncortex/xsvg/commit/c31f04c))
- **`geometry` on a parameter declaration.** Position and extent parameters are flagged as geometry, and the inspector no longer offers them as fields. ([dff91ec](https://github.com/visioncortex/xsvg/commit/dff91ec))
- **`x:border-width`, `x:border-color` and `x:border-opacity` are declared.** They now appear in the panel for `<x:textbox>` and `<x:textpath>`. ([dff91ec](https://github.com/visioncortex/xsvg/commit/dff91ec))
- **`ns` on a parameter declaration.** Attributes in another namespace are written with the prefix the document binds, and skipped when it binds none. ([dff91ec](https://github.com/visioncortex/xsvg/commit/dff91ec))
- **The panel heading reads "attributes".** It replaces "parameters" to match the document, while the plugin ABI's names are unchanged. ([dff91ec](https://github.com/visioncortex/xsvg/commit/dff91ec))
- **`read_only` on a parameter declaration.** `font-family` is the first use, and the panel disables its control and says why. ([9f8cef6](https://github.com/visioncortex/xsvg/commit/9f8cef6))
- **`superseded_by` on a parameter declaration.** The panel disables a parameter made inert by another and names what is overriding it. ([8026108](https://github.com/visioncortex/xsvg/commit/8026108))
- **`createPreview` accepts an async `loader`.** It matches `compileXsvg` and the interactive element for embedding behind an async dependency boundary. ([9f32103](https://github.com/visioncortex/xsvg/commit/9f32103))
- **Cross-file linking on every embed surface.** `<xsvg-view>` gains `base-url` plus `baseUrl`/`resolve`/`loader`, and both React components gain the same props, so inline-source embeds can resolve `<use href>` deps. ([bb8b83f](https://github.com/visioncortex/xsvg/commit/bb8b83f))
- **React `XsvgView` resolves deps against its `src`.** It now defaults the base to the fetched `src` URL, as the custom element does, instead of the page. ([bb8b83f](https://github.com/visioncortex/xsvg/commit/bb8b83f))

### Fixed

- **`<x:import>` modules could not go through a host loader.** `plugins` is now a property on both viewer elements and an option on `createPreview`, so sandboxed embedders such as a VS Code webview can supply modules. ([4f48a32](https://github.com/visioncortex/xsvg/commit/4f48a32))
- **Regression: `route="curve"` sent same-side anchors on a detour.** Same-side and corner anchors arc again, and only an exit facing nearly straight back takes the detour. ([5af27a2](https://github.com/visioncortex/xsvg/commit/5af27a2))
- **A forced anchor now decides which axis its rail leaves along.** A `:top` anchor on an `x-major` rail leaves vertically instead of rightward. ([5af27a2](https://github.com/visioncortex/xsvg/commit/5af27a2))
- **A diagonal corner anchor left a stair-step in a rail.** Its two stubs now come out the same length. ([5af27a2](https://github.com/visioncortex/xsvg/commit/5af27a2))
- **Rounded elbows went sharp when two stubs coincided.** Degenerate waypoints are dropped before the corners are rounded. ([5af27a2](https://github.com/visioncortex/xsvg/commit/5af27a2))
- **Editing a box did not update connectors anchored to it.** The incremental dependency scan now matches `from="#box:right"` to `box`. ([bce1f9b](https://github.com/visioncortex/xsvg/commit/bce1f9b))
- **Edits to a table or pie always fell back to a full recompile.** `<x:table>` and `<x:pie>` wrappers now carry their own source range in `sourcemap` output. ([bce1f9b](https://github.com/visioncortex/xsvg/commit/bce1f9b))
- **An incremental patch could strand a generated def id.** The viewer now declines to patch when that would happen. ([31bf8a0](https://github.com/visioncortex/xsvg/commit/31bf8a0))
- **Source ranges were off by ten bytes per `-x-google-` font marker.** Clicking an element in the interactive viewer now highlights the right source. ([a6e2576](https://github.com/visioncortex/xsvg/commit/a6e2576))
- **Rail connectors doubled back through their own box.** When the endpoints don't face each other with room, the rail now detours around both boxes. ([51fc5cd](https://github.com/visioncortex/xsvg/commit/51fc5cd))
- **Curve connectors cut across the boxes when the far end sat behind.** The curve becomes a rounded elbow that routes around and meets each edge along its normal. ([be501e4](https://github.com/visioncortex/xsvg/commit/be501e4))

## [0.1.6] — 2026-07-23

From this release the two npm packages share one version, so `xsvg-compile` jumps 0.1.0 → 0.1.6 while the crates stay at 0.1.0.

### Added

- **Bordered text.** `x:border-width`, `x:border-color` and `x:border-opacity` on any xsvg text element draw an outline behind the fill that never thins the letters. ([43c805c](https://github.com/visioncortex/xsvg/commit/43c805c))
- **Cross-file `<use>` links.** A `<use href="logo.xsvg">` (or `#id`) compiles the other file and bakes it in, so the output stays self-contained. ([6940127](https://github.com/visioncortex/xsvg/commit/6940127))
- **Linked files from disk.** The CLI and `@visioncortex/xsvg-compile` read linked files on demand, Node via the new `basePath` option. ([014a563](https://github.com/visioncortex/xsvg/commit/014a563))
- **Linked files in the browser.** `compileXsvg` fetches linked files same-origin, and cross-origin failures degrade gracefully. ([2f4165f](https://github.com/visioncortex/xsvg/commit/2f4165f))
- **A `resolve` property on the interactive viewer.** It links bundled in-memory deps. ([c580798](https://github.com/visioncortex/xsvg/commit/c580798))
- **An async `loader` for linked files.** `compileXsvg` takes a `DepLoader` and `<xsvg-view-interactive>` gains `loader`/`baseUrl`, so a host such as a VS Code extension can supply deps. ([466ba9b](https://github.com/visioncortex/xsvg/commit/466ba9b))
- **By-id links size to their own extent.** `<use href="file#id">` sizes the element by its declared or drawn box rather than the whole file. ([43c805c](https://github.com/visioncortex/xsvg/commit/43c805c))
- **Forced connector anchors and raw points.** `from`/`to` accept `#id:<edge>` to force where a connector attaches, and `from-point`/`to-point` aim at raw coordinates. ([7c76933](https://github.com/visioncortex/xsvg/commit/7c76933))
- **Corner and center anchors.** Forced anchors extend to corners (`left-top`…) and `center`. ([6f1945b](https://github.com/visioncortex/xsvg/commit/6f1945b))

### Changed

- **Curve connectors bow by a fixed `bulge`.** Curves bow by an author-settable amount instead of scaling with distance, so a same-side pair reads as a leaf. ([6f1945b](https://github.com/visioncortex/xsvg/commit/6f1945b))
- **The slide deck pages on ArrowUp/ArrowDown.** It joins ArrowLeft/ArrowRight and PageUp/PageDown in the interactive viewer. ([4cbafe1](https://github.com/visioncortex/xsvg/commit/4cbafe1))

### Fixed

- **`@visioncortex/xsvg-compile` failed to load under Node.** The wasm package is now marked CommonJS, fixing `exports is not defined`. ([9870b72](https://github.com/visioncortex/xsvg/commit/9870b72))
- **Raw SVG `stroke*` on live text was dropped.** It now passes through. ([43c805c](https://github.com/visioncortex/xsvg/commit/43c805c))
- **Zero-area shapes had no inspector highlight.** Straight connectors and other zero-area shapes now get a visible highlight. ([6f1945b](https://github.com/visioncortex/xsvg/commit/6f1945b))

## [0.1.5] — 2026-07-19

### Added

- **Dark mode.** The interactive viewer and `createPreview` chrome follow `prefers-color-scheme`, while the drawing keeps its own colors. ([11fd5ad](https://github.com/visioncortex/xsvg/commit/11fd5ad))

## [0.1.4] — 2026-07-19

### Added

- **`createPreview` `slides` option.** Set it to `false` to start a multi-artboard preview with its slide rail collapsed. ([27c2feb](https://github.com/visioncortex/xsvg/commit/27c2feb))

## [0.1.3] — 2026-07-19

### Added

- **Interactive viewer component.** `<xsvg-view-interactive>` and the React `XsvgViewInteractive` ship a pan/zoom canvas with a slide-deck rail and floating zoom controls. ([2fabe37](https://github.com/visioncortex/xsvg/commit/2fabe37))
- **Opt-in inspector.** `inspector` adds a source pane linked both ways to the drawing, with CodeMirror as an optional peer that loads only when enabled. ([2fabe37](https://github.com/visioncortex/xsvg/commit/2fabe37))

### Changed

- **`/viewer` dogfoods the component.** The app is now a thin wrapper over the shipped interactive viewer. ([2fabe37](https://github.com/visioncortex/xsvg/commit/2fabe37))

## [0.1.2] — 2026-07-19

### Fixed

- **Rebuilt with release WebAssembly.** 0.1.1 was accidentally published with a debug build. ([24c296a](https://github.com/visioncortex/xsvg/commit/24c296a))

## [0.1.1] — 2026-07-19 · **deprecated**

Deprecated on npm because it was published with debug WebAssembly, so use 0.1.2 or later.

### Added

- **Embeddable static viewer.** The `<xsvg-view>` custom element, the React `XsvgView` and a single-file `dist/xsvg.js` for zero-build `<script>` use ship, with opt-in `droppable` drag-to-load. ([16c4688](https://github.com/visioncortex/xsvg/commit/16c4688))
- **Public npm access.** Both npm packages set `publishConfig.access: public`. ([7202c05](https://github.com/visioncortex/xsvg/commit/7202c05))

## [0.1.0] — 2026-07-19

### Added

- **`@visioncortex/xsvg-viewer`.** The browser package provides `compileXsvg`, `createPreview`, `findArtboards`/`makeThumb` and `downloadSvg`. ([a644109](https://github.com/visioncortex/xsvg/commit/a644109))
- **`@visioncortex/xsvg-compile`.** The Node package provides a browser-free `compile()`. ([49d6146](https://github.com/visioncortex/xsvg/commit/49d6146))
- **Rust crates on crates.io.** `xsvg-gradient`, `xsvg-core`, `xsvg-cli` and `xsvg-wasm` are published. ([53464cb](https://github.com/visioncortex/xsvg/commit/53464cb))

[Unreleased]: https://github.com/visioncortex/xsvg/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.3.0
[0.2.2]: https://github.com/visioncortex/xsvg/releases/tag/v0.2.2
[0.2.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.2.1
[0.2.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.2.0
[0.1.6]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.6
[0.1.5]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.5
[0.1.4]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.4
[0.1.3]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.3
[0.1.2]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.2
[0.1.1]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.1
[0.1.0]: https://github.com/visioncortex/xsvg/releases/tag/v0.1.0
