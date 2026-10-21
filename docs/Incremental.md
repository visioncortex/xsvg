# Incremental Compilation — engine audit & protocol

**Goal.** Live editing: mutate one node of the source, re-emit **just that subtree**, and let the
JS side replace the corresponding DOM node surgically — no full recompile, no full re-render.

This note records the engine audit (2026-07), the invariant that makes incrementality safe, the
wasm API that ships now, and the JS-side protocol for the next slice. The per-flush
cost of the pure entries (every fragment compile re-parses the whole document) and the
in-process API that removes it are specified in [Compile Session.md](Compile%20Session.md).

## 1. Audit findings

The lowering engine was designed around stateless seams, which turns out to be exactly what
incremental compilation needs. Verified properties (each enforced by tests where noted):

| Property | Status |
|---|---|
| **Subtree-pure emission** — `serialize(node)` is a pure function of the node's subtree, the nodes it references, and `Ctx` (quality + seams). No cross-sibling or ordering state. | ✅ enforced by `fragments_are_verbatim_slices_of_the_full_compile` |
| **No shared mutable state** — no maps/cells/statics anywhere in emission; output is deterministic (seeded noise, integer booleans). | ✅ audited |
| **No ancestor inheritance in layout** — `style_from` reads only the node's own attributes. (Browser-side CSS inheritance of *passthrough* presentation attributes still applies live in the DOM, which surgical replacement preserves automatically.) | ✅ audited |
| **Node identity** — every element that corresponds 1:1 to a source element carries `data-xsvg-pos="start-end"` when `sourcemap` is on. | ✅ existing |
| **Cheap parse, expensive lowering** — re-parsing the whole source per fragment call is fine; the cost lives in bake/layout, which the fragment API skips for everything else. | ✅ by design |

**Cross-reference inventory** — the compile-time-baked references are the three `in="#id"`
consumers (`<x:textbox in>`, `<x:textpath in>`, `<x:warp field="bend" in>`) plus the `href` of a
`<use>` that is a **direct child of `<x:boolean>`** (an operand by reference, §7.4 — the one place
`<use>` is baked), plus `<x:connector from/to>` (§7.6 bakes both endpoint boxes into the routed
path). A connector endpoint may carry a `:anchor` suffix — `from="#box:right"` — and the scan
reports **both** spellings, since deciding there whether a suffix is an anchor keyword would
duplicate the router's own parsing: naming an id that does not exist costs a comparison, missing
one leaves a connector drawn to where its box used to be. Everything else that references by id —
gradient/pattern paints via `url(#…)`,
passthrough `<use href>` anywhere else, and the `<textPath href>` that `effect="follow"` emits —
stays a **live reference** the browser re-resolves, so editing those targets needs only the
target's own re-emit. The `baked_ref` helper in the dependency scan is the single point of truth
for this distinction. (Caveat inside the rule: `follow` *also* bakes the path's arc length into
`startOffset`, but its source attribute is `in`, so the dependency scan covers it.)

## 2. The invariant (normative)

> A fragment compile of a top-level element is **byte-identical** to that element's span in the
> full compile output.

This is what makes surgical replacement sound, and it is pinned by the
`fragments_are_verbatim_slices_of_the_full_compile` test across every emitter family (passthrough,
textbox, textpath + referenced path, warp, boolean), with sourcemap both off and on. **Any future
emitter that introduces cross-sibling or accumulated state breaks this test** — treat it as the
architectural canary, not an ordinary regression test.

## 3. The wasm API (shipped)

The **fragment unit** is a direct element child of the root — coarse enough to be simple, fine
enough that one edit re-lowers one feature's worth of work.

- `compile_fragment(input, quality, sourcemap, offset, …seams) → markup` — re-emit the top-level
  element containing byte `offset`. Errors if the offset falls in inter-element whitespace or the
  root tag (caller falls back to a full compile).
- `fragment_range(input, offset) → [start, end]` — the fragment unit's source range, for the
  caller's identity bookkeeping.
- `dependents(input, offset) → [start, end, …]` — ranges of the *other* top-level elements that must
  be re-emitted alongside this fragment: the **transitive closure** over baked `in="#id"` references
  (an `x:` element's compiled output can itself be a target — Specification.md §4).

## 3.1 Keyed compiles (editor hosts)

Stock fragment output is **byte-offset-dependent**: `data-xsvg-pos` embeds source
spans and generated def ids seed on byte starts (`x-flt-…`, `x-mesh-…`, `x-mgc-…`),
so an edit that shifts earlier bytes changes later fragments' output even when
nothing about them changed. The `NodeKeys` seam removes that: a host supplies
`key(byte_start) → stable key` and calls the `*_keyed_impl` variants
(`compile_keyed_impl`, `compile_fragment_keyed_impl`, plus `_linked_` forms). With
keys, identity emits as `data-xsvg-node="<key>"`, def ids become `x-flt-<key>` etc.,
a keyed `<x:textbox>` also stamps its authored frame as `data-xsvg-frame="x y w h"`
(what an editor selects, hits and resizes is the frame the text flows in, not the
ink it shows; position-mapped and viewer output are unchanged),
and an unchanged fragment's output is **byte-stable under text shifts** (pinned by
`keyed_fragment_output_is_byte_stable_under_text_shifts`). Hosts MUST key every
element — emitters tag nested elements too (warp children, paragraphs); an unkeyed
element falls back to byte spans. Nothing can *enforce* that contract, so a keyed
full compile that had to fall back appends a trailing comment to its own output —
`<!-- xsvg: N element(s) unkeyed; … -->` — where the sloppy host (and its tests)
will actually see it. Complete hosts pay nothing; fragment compiles never carry it,
so every fragment stays a verbatim slice. Keys are never consulted inside linked
dependencies (their offsets index the dependency file). `dependents_multi_impl`
computes the union closure from several seed fragments in one parse.

**Shipped in the browser and Node hosts.** `compile_ext` / `compile_fragment_ext`
take a `key(byteStart) => string | null` callback as an optional trailing argument
(dedicated `*_keyed_ext` entry points existed briefly and were folded in — one entry
per capability doubles the surface per axis, so new capabilities extend the argument
list instead);
`node_spans(input)` reports every element's range, which is how a host outside Rust
learns what there is to name (the seam demands *every* element be keyed; an unkeyed
one reverts to byte spans and the compile says so in its output). `NodeKeyStore`
(`packages/xsvg-viewer/src/keys.ts`) is the reference host implementation: it mints a
name per element, carries names across an edit through the same pivot the patch uses,
and answers name→range so the inspector can still project rendered elements back to
source once identity stops being a range.

The arithmetic does not disappear — it moves into the host, where it belongs, and
stops reaching the output. Measured on the dataset, adopting keys took the fast path
from 326 patches with 23 declines to **347 patches with 2**, and the 2 that remain are
elements that compile to nothing (a declaration, a hidden layer) rather than anything
offsets caused. xsvg-kit (the editor core) is the other consumer.

## 4. JS-side protocol (shipped)

Implemented by `<xsvg-view-interactive>` (`packages/xsvg-viewer/src/interactive.ts`,
`tryPatch`) over `CompileSession` (`compiler.ts`), and available headless from
`@visioncortex/xsvg-compile` (`compileFragment`, `fragmentRange`, `dependents`).

1. Keep the compiled document in the DOM with `sourcemap` on. The anchors are the
   `data-xsvg-pos` attributes themselves — no side table to fall out of step with the DOM.
2. Describe the edit as **one contiguous replacement** (`singleEdit` in `sourcemap.ts`). Its
   common prefix ends at the same offset in both versions, and that offset is the pivot: every
   source offset at or before it is unmoved, every offset after it shifts by the length
   difference. Any edit is representable this way — a scattered one just yields a span wide
   enough to cover it, which then spans several fragments and falls back.
3. `fragment_range(newSource, pivot)` → the unit to re-emit; `dependents(…)` → the rest. Then
   carry every element's **name** across the pivot (§3.1) and re-sync against the new text. From
   here the arithmetic is finished: a name is what the DOM is anchored on *and* what the compiler
   stamps, and neither moves when text does.
4. Re-emit with `compile_fragment(…)` and parse in an **SVG namespace context** (an `<svg>`
   context element, so `<path>` lands in the SVG namespace rather than becoming inert HTML).
5. **Replace the run, not the node.** A fragment may emit several sibling nodes — a `<filter>`
   def ahead of the shape referencing it, marker comments ahead of both — and only one of them
   carries the fragment's range (§4.1). Replacing just that one strands the others: a second
   `<filter>` with a duplicate id, which the browser resolves to the *stale* one. Recompile the
   fragment as it currently stands to learn the run's length and where the keyed node sits
   inside it, then replace that whole span of siblings.
6. Everything not replaced is left completely alone — no attribute rewriting of any kind. That
   is the payoff of step 3: unedited output is byte-identical, so there is nothing to fix up.
7. Fall back to a full compile when: the fragment carries no anchor of its own (it emitted
   nothing — a declaration like `<x:theme>`, or a hidden layer; **not** `<x:doc>`: an edit
   inside the editor's island compiles to nothing *and invalidates nothing*, Specification §4.4,
   so a host skips the render rather than recompiling), the edit touches the root
   element's own tag/attributes, a dependency would have to be fetched, fonts finish loading
   (outline availability changes output globally), or bookkeeping is lost, **or an edit changes
   an `x:layer`/`x:order` value** — layer restacking (§5.1) reorders a parent's children, so
   top-level fragments appear in *sorted* order and their positions shift (each fragment is
   still byte-verbatim; only its placement moves). Editing *within* a layer is a normal in-place
   re-emit.

`font-family` is deliberately not editable at all: selecting a face requires *loading* it before
anything can be measured, which is asynchronous and document-wide, so an attribute edit alone
would lay the document out in a fallback face and present the result as real. The schema says so
(`read_only`) and the panel disables the control rather than leaving that trap live.

The fallback must stay *invisible*: it is a performance path, not a behaviour path. The viewer
therefore carries pan/zoom across a recompile (`PanZoom.view()`/`setView()`), so declining to
patch costs latency and nothing else.

### 4.1 Anchorability (normative)

> A fragment that emits anything renderable carries its own `fragment_range` **exactly once**,
> on one of its top-level nodes.

The verbatim-slice invariant (§2) makes surgical replacement *sound*; this makes it
*addressable*. A host holds the rendered document and knows only source ranges, so an emitter
that wraps its output in an untagged `<g>` is invisible to the lookup and every edit to it
silently degrades to a whole-document recompile — the failure is a performance cliff on one
element type, which is exactly the kind of thing that goes unnoticed. "Exactly once" because the
range is the host's key: a second claimant makes the lookup ambiguous.

Pinned by `every_emitted_fragment_is_anchorable_by_its_own_range`, over both the general fixture
and one covering the composite emitters (`x:table`, `x:pie`, `x:plot`, `x:list`, `x:connector`),
which are the ones that build a wrapper rather than passing one through.

### 4.2 Source normalization

`data-xsvg-pos` and every offset above index the text **the compiler was given**, which is not
always the text on disk: the browser and Node hosts both resolve `-x-google-<Name>` font markers
away first, shifting every later offset by ten bytes per marker. A host that maps output back to
source must work in those coordinates — `CompileSession.source` in the browser, `normalizeSource`
in Node.

## 5. Known limits (v1)

- ~~**Granularity** is the top-level element~~ — **units (§9) go one level down**: a direct child of
  a top-level passthrough container compiles and patches on its own. Deeper nesting still re-emits
  its unit; refine further if profiling demands.
- ~~Dependency scan is one hop~~ — **the scan is now a transitive closure.** Since composition by
  reference shipped (Specification.md §4 *Reference resolution*), an `x:` element can itself be an
  `in`-target: editing `#a` re-emits `<x:warp id="b" in="#a">`, whose changed output re-emits
  `<x:textpath in="#b">`, to a fixpoint. `dependents` computes exactly this closure (document
  order), pinned by `dependents_closure_is_transitive`. Reference **cycles** are a compile-time
  degradation (the cyclic edge resolves to nothing), so the closure always terminates. Within any
  one `compile`/`compile_fragment` call, context-free reference resolutions **memoize** (id →
  geometry), so a fragment that sits on a chain re-derives each upstream target once, not once per
  mention; chain depth itself is capped (Specification.md §4, v0: 32).
- **A fragment can begin with marker comments** (skip/degradation markers precede the element).
  The JS layer should insert all parsed nodes, not just the first element.
- **`<defs>` content** passes through and participates like any top-level unit; editing a gradient
  needs no dependent re-emits (live references).
- ~~Stale generated def ids~~ — **fixed by adopting `NodeKeys` (§3.1).** Worth recording what the
  problem was, because it is the clearest illustration of what keys buy. Ids the compiler invents
  (`x-flt-…`, `x-mesh-…`, `x-mgc-…`) seeded on the node's byte start, so a length-changing edit
  renamed every one after it. A fragment that was not re-emitted kept its old id — which still
  matched its own `url(#…)` and so still rendered — but a re-emitted dependent could land on an id
  an untouched fragment was still using, and a duplicate id resolves to the wrong element.
  Re-stamping host-side would have meant reimplementing `gen_id_seed` outside the compiler, so the
  host declined the patch instead, on documents using filters or meshes. With keys the ids seed on
  the key, nothing renames, and the condition does not arise. (`generated_def_ids_are_all_prefixed_x`
  survives from that era: the `x-` prefix is still the only signal distinguishing a generated id
  from an authored one, and is worth keeping true.)

## 5.1 End-to-end check

`npm run check:incremental` (`scripts/incremental-check.mjs`) reconstructs a patch exactly as the
viewer does — as strings, headlessly, through the published Node entry points — and demands the
result be **byte-identical to a full recompile of the edited source**. It sweeps every dataset
sample, editing attributes spread across each document.

This exists because the Rust invariants pin the *engine* while the protocol above lives in JS, and
a mistake there renders something the document does not say. It has already earned its keep: the
missed connector-anchor dependency (§1) and the untagged composite wrappers (§4.1) were both found
by it, not by inspection.

Anything the viewer would decline is reported as declined rather than skipped silently, so the
count is a live measure of how much of the corpus the fast path actually covers.

It also reports how many verified patches **changed the source's length**, and that number is the
one to read. A same-length edit (`40` → `47`) shifts no offsets and therefore exercises none of
the mapping this protocol is made of — not the pivot arithmetic, not the re-anchoring, not the
re-stamping. Nine in ten arithmetic tweaks happen to keep their width, so the first version of
this harness was almost entirely made of them and reported a confident pass while barely testing
the interesting path; the stale-def-id limit above went unnoticed until edits were forced to
change length. Generating both widths per attribute is the fix, and printing the count is what
keeps the gap from reopening quietly.

## 6. Text layout for editors (`text_layout_impl`, shipped)

The compiler computes every placed line it renders — `PlacedLine { text, x,
baseline, justify_width, runs, words }` — and used to discard them at emission.
`text_layout_impl(input, offset, quality, measurer, shaper, outliner)` returns
that geometry for the `<x:textbox>` whose source range starts at byte `offset`
(any element depth; the host's span table knows offsets — same convention as
`compile_fragment_impl`), so an editing host can place a caret without becoming
a second text-layout engine. `None` when the offset is not a textbox, the
document doesn't parse, or the box resolves to nothing renderable.

**Shape**: `TextLayout { content, paras }` — one `TextLayoutPara` per flow (the
whole box, or one `<x:p>` in paragraph mode §6.16), carrying its source byte
`span`, the **rendered** style (`size` is post `fit="shrink"`), anchor,
`FontMetrics` at the rendered size (caret height), the flow's whitespace-split
`words`, and the rendered `lines`. An empty flow has one blank line, where typing
would start (§6.4): a caret always has a line to sit on, and an empty `<x:p>`'s
line is on the row it owns.

**Caret mapping contract**:

- Every line records `words: [start, end)` into its paragraph's word sequence.
  The range is produced **inside the wrapper** (`fill_line` always consumes
  whole words), not recovered by matching text afterwards — so it stays exact
  when clipping drops leading lines (`valign="bottom"` overflow), pinned by
  `text_layout_valign_bottom_clip_keeps_exact_word_ranges`.
- Caret x within a line = prefix advance of `line.text` measured in the para's
  rendered style **plus** `letter-spacing` per grapheme gap and the LINE's
  `word_spacing` per space — exactly the renderer's `line_advance` math,
  interpreted through `anchor` (start/middle/end from the line's `x`). A
  justified line's `word_spacing` includes its share of slack (§6.9 lowers
  justification as `word-spacing`), so its caret is exact too; `justify_width`
  is the width it spans.
- Word → source offsets is the host's job: the words are its own text content
  split on whitespace, the same walk in both places.
- `text-overflow="ellipsis"` rewrites a line's `text` but not its `words`
  range; `text != words[range].join(" ")` is the truncation test, and carets
  must not be mapped through such a line.
- Under `overflow="visible"` (Specification §6.6) every line is reported,
  and `overflow: true` marks the ones a cropping box would have dropped —
  the lines the editor output stamps `data-xsvg-overflow`. They map like any
  other line; the flag says how they look, not whether a caret may stand there.

**No drift**: rectangular and paragraph flows share their layout code with
emission (`plan_paragraphs`, `layout_area_measured`), the dispatch mirrors
`emit_textbox` branch-for-branch, and `text_layout_matches_emitted_tspans`
pins the whole surface — every emitted `<tspan x y>` must reappear in the
seam's lines with formatter-identical coordinates across plain, fit-shrink,
valign, indent, and paragraph variants.

**Limits (v1)**: styled-run boundaries inside a line are not broken out per
run (caret x uses whole-line prefixes; correct while runs share the paragraph
size, which §6.16 guarantees); `glyph-x-scale ≠ 1` is not a caret target, and
warped or text-on-path text is not a *layout-tier* target — its outlined form
answers through §7 instead; curved region flow reports lines and padded bounds but
editors should treat it read-only for now.

## 7. Caret stops on outlined text (`caret_stops_impl`, shipped)

Outlined text — an `<x:textbox outline="true">`, inside any number of
`<x:warp>`s or none, and an `<x:textpath>` with a geometric effect (`skew`,
`rainbow`, `ribbon`) — compiles to path data with no glyph mapping, so neither
§6 nor a host's own glyph cells can place a caret on it.
`caret_stops_impl(input, offset, quality, measurer, shaper, outliner)` returns
the caret positions between its outlined characters for the element whose
source range starts at byte `offset` (the §6 convention). `None` exactly when
the element renders live text instead — no `outline`, or the outliner declined
and the compile fell back — or is not text; that is the host's glyph-cell case,
and it keys its tier choice on it.

**Shape**: `CaretStops { runs }`, one `StopRun { text, stops }` per outlined run
in output order (a textbox's lines; a textpath's one run), with
`text.chars().count() + 1` `CaretStop { top, bottom }`s: stop `i` sits before
scalar `i`, the last after the run. `top` is `(x_i, baseline − ascent)` and
`bottom` `(x_i, baseline + descent)` — the paragraph's `FontMetrics`, the same
caret height §6 reports — **mapped through every field the run's path went
through**. Coordinates are the user space of the `d` the compile emitted for
that run, so a host maps them with the CTM of the element carrying the node's
key, the frame it already uses for the other tiers. Stop `0` is the pen origin,
not the first glyph's ink edge; consecutive stops differ by the outline font's
own advances (`GlyphOutliner::advances`: the kerned pen x where each next
scalar begins, one shaping pass per run, so a stop sits exactly at the origin
of the glyph it precedes — a ligature's interior stops sit at the prefix
advances).

**Recorded, not recomputed.** The compile is the one source of these numbers:
the outline branch of `write_area_text` knows each traced line's text, pen
origin and baseline; `emit_warp` holds the field it bakes the `d`s through, built
from the union ink bbox of *every* path child, siblings included; the text-on-path
effects pick theirs in `path_effect_field`. A `caret_stops` query therefore
compiles the element once more into a scratch buffer with a recorder on the
`Ctx` (`compile/stops.rs`): the outline branch records a run per traced line,
every field maps the runs recorded beneath it (nested warps compose by
construction), and the query keeps the runs whose span is the element's. The
scratch compile starts at the element's outermost ancestor outside plain SVG (an
`x:warp`, an `x:boolean`, a generator, a `<use>`), or at the element itself when
it has none — not at its top-level fragment. A plain SVG ancestor neither moves
the element's paths nor records anything, so the answer is the same, and an
editor asking on every keystroke no longer recompiles a whole artboard (its
offsets, warps and tables) to place one caret. An
ordinary compile has no recorder — nothing is recorded, nothing is emitted, and
the output is byte-identical to before the seam existed. Scratch serializations
that harvest geometry rather than render the element (a referenced `x:` target,
a boolean operand, a plugin's children, a linked file) suspend the recorder.

**Pinned**: stop `0` of each line is the `M` of its emitted path; a `bend="0"`
warp and an identity outer warp leave the stops equal to the flat box's; a bend
moves them, keeps x monotone and every coordinate finite though the caret heads
lie outside the ink bbox; a flat text-on-path run's stops advance by the
outliner's widths from `run_offset`'s placement with the metrics about the
path's y; two boxes in one warp answer separately; a boolean operand answers
`None`.

**Limits (v1)**: paragraph mode has no outline branch, so a box with `<x:p>`
children never outlines and never has stops; stops are per scalar, so a caret
inside a ligature is honest only at cluster edges.

## 8. Sizing a textbox to its content (`measure_textbox_impl`, shipped)

An editor that pastes a block of formatted text — paragraphs, a list — knows the
width it may use (the canvas bounds it) and needs the height that just holds the
content. `measure_textbox_impl(input, offset, quality, measurer, shaper,
outliner)` answers for the `<x:textbox>` whose source range starts at byte
`offset` (the §6 convention), laid out through the real engine with the real
seams: `TextboxSize { height, width }`.

**Semantics.** The box is laid out at its authored `width` (its own, or its
`in="#rect"`'s) with height unbounded, `valign` forced to top, `fit="shrink"`
and `text-overflow` ignored — the question is how tall the box must be, not how
the content squeezes into the box it has. Everything else is the real stack:
`<x:p>` gaps, `<x:list>` blocks with their markers and gutters, `text-indent`,
mixed run sizes, the inherited `font-family` and theme tokens, which is why the
box is measured **in place** in the document rather than on a scratch copy.
`height` is the cap band of the stacked blocks — the first baseline at cap
height below the top, the last line's descent at the bottom — plus padding
above and below; compiled at that height the box renders every line with its
descenders inside the box (§6.5 clips by baseline, so a box up to one descent
shorter still shows every line, hanging the last descenders out). An empty box
is one line tall. `width` is the widest line plus padding, at most the authored
width, so a short paste can take a narrower box; lines are measured in their
paragraph's style at the line's size, so a line whose runs change weight or
family is approximate while the height is exact. `None` when the offset is not
a rectangular textbox: curved region flow has no height to grow into.

**The editor's flow**, one transaction and one undo step: insert
`<x:textbox x y width="{max}" height="1">…</x:textbox>`, call `measureTextbox`
mid-transaction (legal, like `textLayout`), then set `height` — and `width`, if
a narrower box is wanted — on the same node.

**Pinned**: the single-flow height is cap + (n − 1) advances + descent plus
padding and one descent less loses a line; a paragraph-and-list box compiled at
the measured height renders every block and the layout seam puts the last
descent exactly on the padded bottom edge; a referenced rect gives the same
answer as the box's own geometry; a short line reports its own width and an
empty box one line; a circle-bound box answers `None`.

## 9. Units: incremental compilation one level down (shipped)

A deck is a list of artboards, each a top-level `<g>` holding its slide: text, warps, tables,
offsets. §2's unit was the top-level element, so a keystroke in one textbox re-emitted the whole
artboard — every warp and boolean on the slide baked again to place one word. A **unit** is now a
direct child of a top-level element whose parent is a **passthrough container**: a plain SVG
container (`g`, `a`, `defs`, `symbol`, `marker`, `pattern`, `mask`, `clipPath`, `switch`) that is
not hidden. The serializer copies such a parent through — attributes, an artboard backdrop, then its
children, restacked by `x:layer`/`x:order` when any child asks — and nothing of the parent's reaches
a child's output, so a child compiled alone is byte-identical to its span in the parent's fragment.
Everything a unit *does* depend on is read from the whole parsed document: inherited `font-family`,
theme tokens, `in="#id"` targets anywhere, imports.

**One predicate.** `is_passthrough_container` is read by three things that must agree: `serialize`
routes those elements to its copy-through branch by it; `unit_at(offset)` partitions a top-level
element by it; the caret-stop scope climbs by it. A top-level element that is not such a container
(an `<x:warp>`, a `<text>` with its tspans, a hidden group) is its own unit, as before.

**API** (`CompileSession` and the `*_impl` entries):

- `unit_range(offset) → (start, end)`: the unit containing a byte — the child, or the top-level
  element when the offset lies in its own tag, when it is not a container, or when it is hidden.
- `compile_unit(offset) → String`: the unit alone — a **run of output nodes**, since marker
  comments and a `filter` def precede the element inside its own output; a hidden child is `""`.
- `compile_fragment_spans(offset) → (fragment, [UnitSpan { source, output }])`: the fragment plus
  where each unit landed in it, **in output order**, from the serializer's own child loop (the
  recorder is `None` on every other compile). Empty when the top-level element is its own unit. The
  bytes between spans are the container's own — its tag, backdrop, inter-child whitespace, closing
  tag — and a host leaves them alone.
- `unit_dependents(seeds) → [unit ranges]`: `dependents` at unit granularity. The top-level scan
  only looks *between* top-level elements, so it could not say that `<x:offset in="#w">` beside the
  `<x:warp id="w">` it traces must re-emit when the warp's word changes; the unit scan does. Ids
  inside an included unit go live, the `<x:import>` namespace edge holds, seeds are excluded.

**Host protocol.** An edit inside a unit: `compile_unit` it and splice its span, then `unit_dependents`
and splice those. An edit to the container's own tag or attributes — `x:layer`/`x:order` on a child,
`x:hidden`, `x:fill`, `font-family`, `x:font`, `transform` — re-assembles the parent: compile the
fragment with spans (it changes the backdrop, the order, or what every child inherits). A unit
added with an `id` forces a full render, as a top-level element with one does. The §2 fallbacks
apply unchanged.

**Invariants, pinned over every dataset sample.** The structural one
(`units_are_verbatim_slices_of_their_fragment_over_the_dataset`, sourcemap off and on): each unit's
span in its fragment equals the unit compiled alone, spans are in output order and disjoint,
`unit_range` names the same unit, and each fragment is a slice of the full compile — unit ⊂
fragment ⊂ full, in-process. The **edit oracle**
(`unit_patches_equal_a_full_recompile_over_the_dataset`, keyed, sourcemap on): for every unit, a
small edit inside it — a character appended to its text, or a geometry attribute bumped — then the
document rebuilt as the host protocol says, the edited unit and its `unit_dependents` recompiled
alone and spliced over their old spans with everything else kept verbatim, must equal a full
compile of the edited source byte for byte. A missing dependency edge or a unit that leans on a
neighbour fails there. Plus the hidden, restacked, filtered, `<text>`-with-tspans and own-tag
cases, and the offset→warp dependency the top-level scan misses.

**Known limit.** `Ctx` carries a per-call reference budget (`fuel`); a unit compiled alone starts
with a full one and could differ from its in-fragment output only when a pathological reference
graph exhausts it, which the host's verify oracle would catch.

## 10. Document queries (`CompileSession`, shipped)

An editor's chrome asks the document questions on every render — the font at the caret, a
paragraph's size, which run controls do anything on this element — and the answers are the
compiler's to give: inheritance, theme tokens, the sealed `x:doc`, a box's block structure and
which run attributes an emitter draws are all rules it applies while compiling. Copies of those
rules in the editor drift (xsvg-kit `docs/Queries.md` §2.2 lists three that already had). So the
session answers them from the parse it compiles from, by the same functions the emitters call:

| `CompileSession::` | Answer |
|---|---|
| `resolved(offset, &[property])` | each property's effective value on the element at `offset`, as authored and with theme colours substituted (`computed`), and its `Source`: `Own`, `Token(name)`, `Inherited { from, token }` (an enclosing node's source offset), `Implied` (a header cell's bold) or `Initial` (the element's declared default, else the compiler's) |
| `theme()` | the `<x:color>` and `<x:font>` tokens in document order, a redeclared name at its last place, nothing inside `x:doc` |
| `text_style(offset)` | the element's text as pieces, each in the style it renders in, offsets in UTF-16 code units of the text content (each `<tbreak/>` one) |
| `blocks(offset)` | a box's paragraphs and list items (level, list, marker kind), or a list's items, in reading order |
| `run_support(offset)` | which run attributes the element's text draws: typography, fill, border, highlight, decoration, baseline-shift, links |
| `corners(offset)` | each corner a shape's `x:corners` draws (§5.4): anchor, the sharp point, requested and effective radius, `clamped`, the trim points, the arc's centre — for the corner widget |

**One cascade.** Style properties — the eight `<x:font>` token properties and `fill` — come from
an ordered list of **levels** (`query.rs`): the element's own attributes, then its token, then each
ancestor's `font-family` (§4.1); a paragraph or list in front of its box's, an item in front of its
list's, a cell in front of its row's, its column's, a header cell's implied bold and the table's.
`style_from`, the paragraph planner, the list and table emitters and `resolved` all compute from
the same levels, so the style a query reports is the style that was laid out
(`resolved_agrees_with_the_emitters_over_the_dataset` pins it on every element of every sample).
A value counts at a level only when it is valid for the property — a positive size, `normal` or a
number for spacing, a known `text-transform` — else the cascade falls through, as CSS ignores an
invalid declaration. A block's layout properties follow the paragraph planner (§6.16): `align` and `direction`
are the paragraph's own, else its box's; `text-indent` the paragraph's own, else none (a box's
indent is for its own single flow); an item in a list is always start-aligned and unindented
(`Implied`). Any other attribute (`x:` naming the xsvg namespace, whatever prefix the document
binds) is the element's own, else the default its element declares in `describe()`.

**Runs.** A run's properties do not follow the element cascade: they compose from the enclosing
`<tspan>`s inward over the element's style, a run's `font-size` is taken against its paragraph's,
and its border completes from the nearest bordered container (§6.11, §6.17). So runs are answered
by `text_style`, through the run walker the emitters use (`collect_runs`), never by `resolved`.
What a piece reports is filtered by `run_support`, which answers for what *draws*, by the compiler
or the renderer: text on a path reports paint only; a plain `<text>` passes through, where SVG
composes a `<tspan>`'s typography, fill, decoration, baseline shift and an `<a>`'s link natively
and the compiler lowers its border, so everything but a highlight is reported.

**Host protocol: one parse per change.** The queries live on `ParsedDocument<'a>`, a parse that
*borrows* its text (the core owns no text and keeps no state between calls; roxmltree is
zero-copy), and `CompileSession::from_parsed(platform, &doc, …)` compiles from one without
parsing — byte-identical to a session over the same text, pinned over the dataset with the parse
counter at zero. A host therefore keeps **one `ParsedDocument` per document, paired with the text
it borrows, stamped with the mutation count it was built at**: every query reads it; a flush
compiles from it; when the count has moved, the host re-serializes, parses once and replaces it.
The pairing is a self-referential value (the tree borrows the text), which is the host's to hold
(`self_cell` or the like — `ParsedDocument` is covariant in its lifetime, which such a cell
requires; `parsed_document_is_covariant` fails to build if that ever changes). Offsets are source
byte starts, as for units (§9). The kit wraps the queries as its `queries` (xsvg-kit
`docs/Queries.md`), whose JS memo then only saves the bridge hop.

## 11. Convert to path (`CompileSession::to_path` / `to_spline`, shipped)

An editor's *Create outlines* / *Convert to path* replaces a node by the markup the compiler
would draw for it, so the rule is: **`to_path(offset)` returns the compiler's own output for the
element at `offset` with outlining forced, in the form that goes back into the source in the
node's place.** One group (one `<path>` for a basic shape), its glyph paths and baked backdrop
rects inside, and **every attribute the conversion did not spend kept as authored**: `id`,
`class`, `x:label` and the editor's other `x:` attributes, `transform`, `opacity`, a `filter`
list (lowered again at render), `clip-path`, `mask`, `style`, and every paint with its `var()`
token, so the theme, the filter and the editor's tools see the converted element as they saw the
text. Spent: the text and what laid it out — `x y width height`, `font-*` and `x:font`, the
alignment, fit, overflow and spacing attributes, `in`, `outline`, and for point text the anchor
and tspan geometry; `x:border-*` lowers to `stroke`/`stroke-width`/`paint-order`. No source map,
no key: the host re-keys the markup it inserts.

A basic shape converts to the `<path>` of its geometry, exact. **Corners stay live**, as
Illustrator's convert-to-path leaves live corners live: a `<path>` carries `x:corners` natively, so
there is nothing to spend — the shape's `x:corners` and `x:corner-style` ride onto its sharp
geometry (a polyline opened), and a `<rect rx>` with circular corners becomes the sharp rect plus
`x:corners="rx"` (a token stays a token), so the corner widget still works on the result and the
render is the same (§5.4). What has no live form is the compiler's arcs: a circle or an ellipse as
four quarter arcs (`A`) from its rightmost point, clockwise, anchors at the four extremes; a rect
with elliptical corners (`rx ≠ ry`) as its edges and one arc per corner, an edge the corners consume
left out. A `<path>` converts to itself — its `d` as authored, its corners live — so a mixed
selection's Convert to path need not skip it. Arcs stay arcs in `to_path`. For an editor whose
path model is Béziers only,
**`to_spline(offset)`** is *expand*: the corners baked, then every `A` as cubics — at most a quarter
turn per cubic, each the classic `κ = 4/3·tan(θ/4)` cubic (0.5523 for a quarter circle), so a
circle is four anchors at its extremes and a rounded corner one cubic, never a flattening;
everything that is not an arc passes through byte for byte, a degenerate arc is dropped and a
zero-radius one is the line SVG draws. The converter is public as `arcs_to_cubics(d)` for any
path data (a compiled corner path, say). `Err(ToPathError)` is the
answer when nothing half-converts: `family`/`weight`/`style` name the
font the outliner had no bytes for ("Inter 700 italic has no font file to trace"), else `reason`
names the limit (`follow` and `stair` on a path, a per-glyph list or `rotate` on point text, an
element that is not text, a basic shape or a path, a text with no characters). The compiled output of
the document is byte-identical with or without this call.

Coordinates: the node compiles alone, so the markup is in its parent's user space, exactly what
replacing it in place needs; a text inside an `<x:warp>` converts to paths the warp still
bends. The round trip is pinned over the dataset (`to_path_markup_compiles_to_what_the_outlined_
text_rendered`): every traced path of a conversion is in the compile of the document with the
node replaced; and `npm run check:outline` renders live and outlined text to the same pixels.
