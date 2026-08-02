# Incremental Compilation — engine audit & protocol

**Goal.** Live editing: mutate one node of the source, re-emit **just that subtree**, and let the
JS side replace the corresponding DOM node surgically — no full recompile, no full re-render.

This note records the engine audit (2026-07), the invariant that makes incrementality safe, the
wasm API that ships now, and the JS-side protocol for the next slice.

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
and an unchanged fragment's output is **byte-stable under text shifts** (pinned by
`keyed_fragment_output_is_byte_stable_under_text_shifts`). Hosts MUST key every
element — emitters tag nested elements too (warp children, paragraphs); an unkeyed
element falls back to byte spans. Keys are never consulted inside linked
dependencies (their offsets index the dependency file). `dependents_multi_impl`
computes the union closure from several seed fragments in one parse.

**Shipped in the browser and Node hosts.** `compile_keyed_ext` /
`compile_fragment_keyed_ext` take a `key(byteStart) => string | null` callback;
`node_spans(input)` reports every element's range, which is how a host outside Rust
learns what there is to name (the seam demands *every* element be keyed, and an
unkeyed one silently reverts to byte spans). `NodeKeyStore`
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
   nothing — a declaration like `<x:theme>`, or a hidden layer), the edit touches the root
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

- **Granularity** is the top-level element: editing a node nested in a deep passthrough `<g>`
  re-emits the whole top-level group. Acceptable; refine later if profiling demands.
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
`words`, and the rendered `lines`.

**Caret mapping contract**:

- Every line records `words: [start, end)` into its paragraph's word sequence.
  The range is produced **inside the wrapper** (`fill_line` always consumes
  whole words), not recovered by matching text afterwards — so it stays exact
  when clipping drops leading lines (`valign="bottom"` overflow), pinned by
  `text_layout_valign_bottom_clip_keeps_exact_word_ranges`.
- Caret x within a line = prefix advance of `line.text` measured in the para's
  rendered style **plus** `letter-spacing` per grapheme gap and `word-spacing`
  per space — exactly the renderer's `line_advance` math, interpreted through
  `anchor` (start/middle/end from the line's `x`).
- Word → source offsets is the host's job: the words are its own text content
  split on whitespace, the same walk in both places.
- `text-overflow="ellipsis"` rewrites a line's `text` but not its `words`
  range; `text != words[range].join(" ")` is the truncation test, and carets
  must not be mapped through such a line.

**No drift**: rectangular and paragraph flows share their layout code with
emission (`plan_paragraphs`, `layout_area_measured`), the dispatch mirrors
`emit_textbox` branch-for-branch, and `text_layout_matches_emitted_tspans`
pins the whole surface — every emitted `<tspan x y>` must reappear in the
seam's lines with formatter-identical coordinates across plain, fit-shrink,
valign, indent, and paragraph variants.

**Limits (v1)**: styled-run boundaries inside a line are not broken out per
run (caret x uses whole-line prefixes; correct while runs share the paragraph
size, which §6.16 guarantees); `glyph-x-scale ≠ 1` and warped/textpath text are
not caret targets; curved region flow reports lines and padded bounds but
editors should treat it read-only for now.
