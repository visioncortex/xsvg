# Compile Session — one parse per flush

**Status: normative; §2–4 and §6's first three tests built (2026-10-04),
§5 open.** The in-process API that lets an editor compile many fragments of
one document without re-parsing it for each. Not a server, not a protocol,
not a daemon: a struct with a lifetime, created and dropped inside one render
flush by whoever already links `xsvg-core` as a library (the kit's wasm module
today, the native app tomorrow). Implementation: `compile/session.rs`. Builds on [Incremental.md](Incremental.md) (fragments, keys, the
verbatim-slice invariant) and [Extensibility.md](Extensibility.md) (seams).

## 1. The problem, measured in parses

The editor's hot path is one flush per keystroke or gesture frame. Per
flush, the kit serializes its tree to text and calls the compiler per dirty
fragment. Each `compile_fragment_*_impl(input, offset, …)` is a pure
function and does, in full, every time:

1. `check_nesting_depth(input)` — a scan of the whole text;
2. `roxmltree::Document::parse(input)` — the whole document;
3. `load_theme(&doc)` into the thread-local `THEME`, `load_imports(&doc)` —
   whole-document pre-scans;
4. compile the one fragment at `offset`.

`dependents_multi_impl(input, seeds)` parses the whole document again to
find baked-ref dependents, and `text_layout_impl(input, offset, …)` parses
it again for caret geometry. A flush that touches three fragments therefore
parses the document four or five times and compiles three subtrees. Steps
1–3 are O(document); step 4 is O(fragment). The purpose of the session is to
pay steps 1–3 once per flush and keep everything else exactly as it is.

What this does **not** attempt: retaining state *across* flushes inside the
core. The long-lived document is the embedder's (the kit's `Dom` and its
render cache); the core stays a pure function of its inputs between calls,
which is what keeps a process boundary and a protocol unnecessary. Caches
that outlive a flush are defined by the core and **owned by the caller**
(§5), never held in core statics.

## 2. The API

```rust
pub struct CompileSession<'a> { /* private */ }

impl<'a> CompileSession<'a> {
    /// Parse once, pre-scan once (theme tokens, <x:import> bindings, the id
    /// table). Fails exactly where the pure entries fail (nesting depth, XML
    /// parse error), with the same message.
    pub fn new(
        input: &'a str,
        quality: &str,
        sourcemap: bool,
        base: &str,
    ) -> Result<Self, String>;

    /// The fragment whose element starts at `offset` — byte-identical to
    /// `compile_fragment_seams_impl(input, quality, sourcemap, offset, base, seams)`.
    pub fn compile_fragment(&self, offset: usize, seams: &Seams) -> Result<String, String>;

    /// The whole document — byte-identical to `compile_seams_impl(input, …)`.
    pub fn compile(&self, seams: &Seams) -> Result<String, String>;

    /// Baked-ref dependents of the fragments at `seeds`, from the retained
    /// parse — the same answer as `dependents_multi_impl(input, seeds)`.
    pub fn dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)>;

    /// Caret geometry for the textbox at `offset` — the same answer as
    /// `text_layout_impl(input, offset, quality, …)`.
    pub fn text_layout(&self, offset: usize, seams: &Seams) -> Option<TextLayout>;

    /// The parse-only tables, from the retained parse — the same answers as
    /// `node_spans_impl` and `fragment_range_impl`.
    pub fn node_spans(&self) -> Vec<(usize, usize)>;
    pub fn fragment_range(&self, offset: usize) -> Option<(usize, usize)>;
}
```

- **Lifetime.** `'a` is the input text's. The session borrows the text. It is
  created at the top of a flush and dropped at the end; it cannot outlive the
  text it parsed, so there is no way to hold a stale parse.
- **Seams are per call, and the session is `Send`.** The measurer, shaper,
  outliner, resolver, keys and generator host are passed to each compiling
  method rather than held, because a session that borrowed `&dyn Measurer`
  could be `Send` only if that trait object were `Sync`, and a host whose
  seams wrap single-threaded handles (the browser's `measureText` callback)
  can never provide one. The seams are the same for every call of a flush
  and cost nothing to pass. The session holds no thread-bound state and is
  `Send` (pinned by `tests/session.rs`); it is not `Sync`, and need not be.
  The parse-only methods take no seams.
- **The pure entries become wrappers.** `compile_impl`,
  `compile_fragment_*_impl`, `dependents_multi_impl` and `text_layout_impl`
  each construct a one-shot session and call the method. Their signatures,
  outputs and error texts do not change; the CLI, the Node host, the viewer
  and every existing test are untouched. The session is additive.
- **Output equality is the contract.** For every `(input, offset)`,
  `CompileSession::new(input, …)?.compile_fragment(offset)` equals
  `compile_fragment_seams_impl(input, …, offset, …)` byte for byte, and
  likewise for `compile`, `dependents` and `text_layout`. §6 pins it.

## 3. What the session retains

Everything the pure entries recompute per call and nothing more:

| Retained | Today | In the session |
|---|---|---|
| Nesting-depth check | per call | once in `new` |
| `roxmltree::Document` | per call | once; fragment compiles take `doc.get_node(...)` by byte offset exactly as `compile_fragment_seams_impl` does after its own parse |
| Theme tokens (`<x:theme>` colours and fonts) | loaded into the thread-local `THEME` per call | a `Theme` value owned by the session, threaded through `Ctx` (§4) |
| `<x:import>` bindings | `load_imports` per call | once, already a `Ctx` field |
| Id table (what `resolve_ref` scanned for, first declaration winning) | a document-order scan per reference | built once; `dependents` and `#id` resolution in the entry document read it (a linked dependency's nodes still scan their own document) |
| `QualityProfile` | parsed per call | once |

Per-fragment state — the `resolved` memo, the `resolving` cycle stack, the
`fuel`/`cuts` budget, the link `files` stack and `base` — stays per compile
call exactly as today: it is a whole-compile work budget and a cycle guard
whose scope is one compile, and sharing it across fragments would change
degradation behaviour. A session's `compile_fragment` constructs a fresh
`Ctx` over the shared retained parts.

## 4. The theme stops being a thread-local

`THEME` is a `thread_local!` reloaded at the top of every compile. It exists
because the token lookups (`theme_color`, `font_token`) are called from deep
inside emitters that do not carry a context. It is global mutable state in
disguise, the reason every fragment compile reloads the theme, and the
source of the stale-theme hazard the kit once hit (a fragment compiled after
a different document's full compile on the same thread saw the wrong tokens).

The session makes the theme a `Ctx` field like `imports`: emitters that
resolve tokens receive the `Ctx` they already take for seams and keys, and
`resolve_var` / `paint` / `font_props` are methods on it. `THEME` is deleted;
no compile path touches process-wide state. This is a mechanical change with
one test-visible consequence: the stale-theme test becomes impossible to fail
rather than a guarded case.

## 5. Caller-owned caches (companion seams, not session state)

Two costs are inside step 4 and survive a session unchanged: text
measurement and paragraph layout. Both are worth caching across flushes,
and both caches belong to the **caller**, who knows when the document
changed. The core defines the types and the invalidation key; the caller
passes `&mut` storage and keeps it as long as it likes.

- **Measurement.** Every word measured is a `Measurer::measure(text, style,
  size)` call; across the wasm boundary that is a JS hop into canvas
  `measureText`. Two additive seams: a `measure_batch(&[(text, style, size)])
  -> Vec<f64>` default method on `Measurer` (the core calls it with a
  fragment's words at once; the default loops over `measure`, so existing
  adapters need nothing), and an `AdvanceCache` the core consults before
  the seam, keyed by `(family, weight, style, size, text)`. Embedders may
  also memoize on their side of the seam; the two compose.
- **Paragraph layout.** A keystroke relays out every paragraph of its
  textbox. A `LayoutCache` keyed by `(paragraph text, resolved style, area
  spec)` lets the core skip paragraphs whose inputs did not change; the
  caller invalidates nothing (the key is the content) and bounds the size.

Both are optional parameters with `None` meaning today's behaviour.

## 6. Tests

- **Equality** (`tests/session.rs`). For every document in the dataset and
  every fragment offset, in plain, source-mapped and keyed modes: session
  `compile_fragment` equals the pure entry byte for byte; session `compile`
  equals `compile_seams_impl`; `dependents` (per seed and all seeds),
  `text_layout` (every textbox), `node_spans` and `fragment_range` agree.
  Errors match too: parse, nesting-depth and bad-offset messages.
- **One parse** (`compile/tests.rs`). Every whole-document parse goes
  through one `parse_document` helper with a parse counter behind
  `cfg(test)`; a session with N fragment compiles, dependents, layouts and a
  whole compile parsed once, where the pure entries parse N + 1 times. This
  is the property the session exists for; without the test it erodes.
- **Theme isolation** (`compile/tests.rs`). Two sessions over two documents
  with different `<x:theme>` colour and font tokens, compiled interleaved on
  one thread, each see their own tokens — the former stale-theme hazard, now
  structurally impossible.
- **Caches are transparent** (§5, unbuilt). With and without `AdvanceCache` /
  `LayoutCache`, outputs are byte-identical.

## 7. What the embedder does with it (informative)

The kit's render state, per flush: serialize the tree once, build one
session, run the dirty classifier, `dependents(seeds)`, `compile_fragment`
per dirty fragment, drop the session. Across the wasm boundary a struct with
a lifetime cannot be handed to JS, so the session lives inside one exported
call: a flush entry that takes the text and the dirty offsets, builds the
session, and returns every fragment at once — the unit §8 says a protocol
would wrap, if one were ever needed. The verifier builds its own session
for the from-scratch compile. The caret geometry call uses a session over
the same text when one is live. Measured on the kit's sample deck, this
removes every whole-document parse but one from a keystroke; the remaining
per-flush O(document) costs (the kit's own serialization and scene re-index)
are the kit's to make incremental, and are tracked there.

## 8. Non-goals

- **No retained document in the core** across flushes, and no mutation API.
  The embedder owns the tree; the core compiles text.
- **No process boundary, no IPC, no watch mode.** If another process ever
  needs the compiler, the session is the unit to put behind that protocol,
  and nothing here precludes it; nothing on the roadmap needs it.
- **No change to output.** Every byte the session emits is a byte the pure
  entries emit today. Faster, not different.
