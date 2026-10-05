# Compile Session — one parse per flush

**Status: normative; §2–4, §5's measurement cache and batch seam, and §6
built (2026-10-05); the core's use of the batch seam and the layout cache
are open.** The in-process API that lets an editor compile many fragments of
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
/// Everything the compiler needs from the world it runs in, owned and
/// long-lived. The adapters keep their concrete types; the optional three
/// default to inert values.
pub struct Platform<M, S, O, R = NoResolver, K = NoKeys, G = NoGenerators> { /* private */ }

impl<M: Measurer, S: Shaper, O: GlyphOutliner> Platform<M, S, O> {
    pub fn new(m: M, shaper: S, outliner: O) -> Self;
}
impl<M, S, O, R, K, G> Platform<M, S, O, R, K, G> {
    pub fn with_resolver<R2: Resolver>(self, r: R2) -> Platform<M, S, O, R2, K, G>;
    pub fn with_keys<K2: NodeKeys>(self, k: K2) -> Platform<M, S, O, R, K2, G>;
    pub fn with_generators<G2: GeneratorHost>(self, g: G2) -> Platform<M, S, O, R, K, G2>;
    /// §5: a `MeasureCache` in front of the measurer.
    pub fn with_measure_cache(self) -> Platform<MeasureCache<M>, S, O, R, K, G>;
    /// Parse once, pre-scan once (theme tokens, <x:import> bindings, the id
    /// table). Fails exactly where the pure entries fail (nesting depth, XML
    /// parse error), with the same message.
    pub fn session<'s>(&'s self, input: &'s str, quality: &str, sourcemap: bool, base: &str)
        -> Result<CompileSession<'s, Self>, String>;
}

pub struct CompileSession<'a, P: Host> { /* private */ }

impl<'a, P: Host> CompileSession<'a, P> {
    /// The fragment whose element starts at `offset` — byte-identical to
    /// `compile_fragment_platform_impl(input, quality, sourcemap, offset, base, &platform)`.
    pub fn compile_fragment(&self, offset: usize) -> Result<String, String>;

    /// The whole document — byte-identical to `compile_platform_impl(input, …)`.
    pub fn compile(&self) -> Result<String, String>;

    /// Baked-ref dependents of the fragments at `seeds`, from the retained
    /// parse — the same answer as `dependents_multi_impl(input, seeds)`.
    pub fn dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)>;

    /// Caret geometry for the textbox at `offset` — the same answer as
    /// `text_layout_impl(input, offset, quality, …)`.
    pub fn text_layout(&self, offset: usize) -> Option<TextLayout>;

    /// The parse-only tables, from the retained parse — the same answers as
    /// `node_spans_impl` and `fragment_range_impl`.
    pub fn node_spans(&self) -> Vec<(usize, usize)>;
    pub fn fragment_range(&self, offset: usize) -> Option<(usize, usize)>;
}
```

- **Two lifetimes, two objects.** The platform is the embedder's for as long
  as it runs: the measurer, shaper, outliner, resolver, keys and generator
  host, and with them everything that should survive a compile (a font
  database, a loaded module, the §5 measurement cache). The session is one
  flush's: it borrows the text and the platform, is created at the top of a
  flush and dropped at the end, and cannot outlive the text it parsed, so
  there is no way to hold a stale parse. Nothing is passed per call.
- **Concrete types, so the auto traits can fall where the adapters put
  them.** The platform keeps its adapters' types instead of erasing them to
  `Box<dyn …>`; the session holds a `&P`. Neither promises `Send` or `Sync`
  (§8), and neither needs `unsafe` or bounds on the seam traits to be
  whatever its adapters make it. `Host` is the sealed trait every `Platform`
  implements; it exists so the session and the `*_platform_impl` entries can
  take any platform.
- **The pure entries become wrappers.** `compile_impl`,
  `compile_fragment_*_impl`, `dependents_multi_impl` and `text_layout_impl`
  each do the parse-once work on a stack-local platform view and call the
  method. Their signatures, outputs and error texts do not change; the CLI,
  the Node host, the viewer and every existing test are untouched. The
  session is additive.
- **Output equality is the contract.** For every `(input, offset)`,
  `platform.session(input, …)?.compile_fragment(offset)` equals
  `compile_fragment_platform_impl(input, …, offset, …, &platform)` byte for
  byte, and likewise for `compile`, `dependents` and `text_layout`. §6 pins it.

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

- **Measurement** (built: `MeasureCache`). Every word measured is a
  `Measurer::measure(text, style, size)` call; across the wasm boundary that
  is a JS hop into canvas `measureText`, natively a `ttf-parser` face walk.
  `MeasureCache<M>` is a `Measurer` that remembers advances per `(family,
  weight, style, size, text)` and vertical metrics per font, bounded, for as
  long as it lives; `Platform::with_measure_cache()` puts one in front of
  the platform's measurer, so the cache lives exactly as long as the
  platform. A second flush over unchanged text reaches the adapter zero
  times (pinned). The key is the whole dependency of `measure` by contract:
  it returns the raw glyph advance, and `line_advance` layers the spacing on.
  `Measurer::measure_batch(&[(text, style, size)]) -> Vec<f64>` is the
  transport seam beside it: a default method that measures each item in
  turn, which an adapter whose single measurement is a round trip overrides
  to make the trip once per batch. `MeasureCache` forwards it hits-first —
  only the misses reach the adapter, in one call — so a cold paragraph costs
  one trip and a warm one none. Still open: the core's `measure_runs` /
  `measure_words` calling it with a paragraph's words at once, and the
  browser adapter's override with its JS-side callback; until then the seam
  is exercised only by its own tests. Embedders may also memoize on their
  side of the seam; the two compose.
- **Paragraph layout** (open). A keystroke relays out every paragraph of its
  textbox. A `LayoutCache` keyed by `(paragraph text, resolved style, area
  spec)` lets the core skip paragraphs whose inputs did not change; the
  caller invalidates nothing (the key is the content) and bounds the size.

Both are opt-in; a platform without them is today's behaviour.

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
- **Caches are transparent** (`tests/session.rs`). One `MeasureCache`
  carried across the whole corpus and a platform without one emit
  byte-identical output for every fragment and every layout; a counting
  measurer shows a second flush over unchanged text reaches the adapter
  zero times, and an edit costs only its new word.

## 7. What the embedder does with it (informative)

The kit's render state, per flush: serialize the tree once, build one
session, run the dirty classifier, `dependents(seeds)`, `compile_fragment`
per dirty fragment, drop the session. Across the wasm boundary a struct with
a lifetime cannot be handed to JS, so the session lives inside one exported
call: a flush entry that takes the text and the dirty offsets, builds the
session, and returns every fragment at once — the unit §8 says a protocol
would wrap, if one were ever needed. The platform can cross: a
`#[wasm_bindgen]` compiler object owning its callbacks (and so its
measurement cache) is a `Platform` over owned `js_sys::Function`s, built
once by the kit instead of six functions handed to every call. The verifier builds its own session
for the from-scratch compile. The caret geometry call uses a session over
the same text when one is live. Measured on the kit's sample deck, this
removes every whole-document parse but one from a keystroke; the remaining
per-flush O(document) costs (the kit's own serialization and scene re-index)
are the kit's to make incremental, and are tracked there.

## 8. Threading: neither `Send` nor `Sync`, by design

A session is created and dropped inside one flush on one thread — wasm is
single-threaded, the native app compiles on the thread that owns the
document, and a CLI compiling many files in parallel wants one session per
file per thread (that needs the *inputs* to be `Send`, not the session). So
the session promises neither `Send` nor `Sync`. Nor does it rule them out:
it holds a `&P` to a `Platform` of concrete adapter types, so by the auto
traits a session over thread-safe adapters is `Send` and one over the wasm
adapters (which wrap `js_sys::Function`) is not — without bounds on the seam
traits, and without a `Box<dyn …>` erasure that would make it never `Send`.

The one case that would want `Session: Sync` — compiling the fragments of
one session in parallel — buys nothing on the editor's hot path (a
keystroke dirties one to three fragments; the cost the session removes is
the parse). If a batch export ever wants it, nothing structural stands in
the way: the per-fragment `Ctx` is already per-call (`Cell`/`RefCell` memo
and budget), and the shared parts — `roxmltree::Document`, the theme, the
imports, the id table, the `Mutex`-guarded measurement cache — are plain
data. Do not pin the auto traits either way (`impl !Send`, or a test): let
them fall where the fields put them. What matters, and what §6 pins, is
that after `THEME` is gone a session's correctness does not depend on
which thread runs it.

## 9. Non-goals

- **No retained document in the core** across flushes, and no mutation API.
  The embedder owns the tree; the core compiles text.
- **No process boundary, no IPC, no watch mode.** If another process ever
  needs the compiler, the session is the unit to put behind that protocol,
  and nothing here precludes it; nothing on the roadmap needs it.
- **No change to output.** Every byte the session emits is a byte the pure
  entries emit today. Faster, not different.
