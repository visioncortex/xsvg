# xsvg — Extensibility: Geometry Plugins & Namespaces

A design for making the compiler **extensible by loadable plugins**: WASM modules that act as
custom *geometry generators* ("geometry shaders") — parameterized procedural graphics lowered to
the same plain-SVG subset as the built-ins. The immediate use case is **procedural clipart for
diagrams** (e.g. an outlined thick arrow with configurable head width, shaft thickness, and outline
weight), but the mechanism is general: anything that today would be a hand-written `emit_*` in
`crates/xsvg-core/src/compile/` becomes authorable out-of-tree.

This document is grounded in the shipped architecture ([Plan.md](Plan.md) §1.5 already commits to
*"many WASM modules, JS as orchestrator"* and describes optional engines as *"a plugin a document
pulls only if it needs it"*). Status: **implemented** on every host — core seam, the `clipart` extension, and the
browser, CLI, and Node hosts. Where this document and the code differ, the code wins;
the notes below were reconciled after building it.

> **The spreadsheet framing.** xsvg already has the skeleton of a spreadsheet: `baked_refs`
> (`compile/mod.rs:386`) defines the dependency edges, `dependents_multi_impl` (`:428`) is the
> dirty-cell recalculation fixpoint, and `<x:theme>` tokens are named values. Plugins complete the
> analogy as **user-defined functions**: a parameterized element is a formula cell whose inputs are
> its attributes (and optionally another element's geometry), recomputed exactly when its inputs
> change. A general expression layer (attribute-level `calc()`) is out of scope here but slots into
> the same graph later — see §8.

---

## 1. The three namespace tiers

Extensibility must not blur what is *specified* and what is not. Dispatch is already by **XML
namespace URI** (`serialize`, `compile/mod.rs:623`), so the tiers are namespaces, with conventional
prefixes:

| Tier | Conventional prefix | Namespace URI | Who ships it | Stability contract |
|---|---|---|---|---|
| **Core** | `x:` | `https://xsvg.visioncortex.org` | compiled into `xsvg-core` | Normative — [Specification.md](Specification.md). Never removed. |
| **Official extensions** | `xsvg:` | `https://xsvg.visioncortex.org/ext/<module>` | this repo, published as versioned WASM modules — imported like any plugin | Experimental — may change or graduate. Deprecation window on change. |
| **Third-party** | author's choice | any other URI | anyone, as a WASM module bound by `<x:import>` | Whatever the plugin author promises. |

Rules:

- **The `https://xsvg.visioncortex.org/` URI space is reserved** for the project: only the project
  mints namespaces under it (one per official extension module, e.g. `…/ext/clipart`). Third-party
  plugins MUST declare their own namespace URI (a domain they control, or a `urn:`). This is a
  governance rule, not a mechanical one — the actual trust anchor is `integrity` pinning (§2, §6).
- **Prefixes are convention, not identity** — as in all XML, `xmlns:foo="https://xsvg.visioncortex.org/ext/clipart"`
  makes `foo:arrow` an official-extension element. Docs and samples use `x:` for core and `xsvg:`
  for an official module (module-specific prefixes when a document binds several).
- **Degradation is uniform across tiers** (§3 of the Specification): an element whose namespace has
  no bound generator compiles to a diagnostic comment, never an error; a plain SVG viewer showing
  uncompiled source skips it entirely. Totality is preserved.

### 1.1 The `xsvg:` incubation namespace

The official-extension namespace is where new elements live **before they earn a place in the core
spec** — the experiment can ship, gather usage, and change shape without a normative commitment.

**The lifecycle is identical to a third-party extension's — only the name is blessed.** Official
extensions are implemented through the same plugin seam, bound by the same explicit `<x:import>`
(with `src` and `integrity`), loaded, sandboxed, and cached the same way. There is **no
auto-loading** and no compiled-in privilege: a document that uses `xsvg:arrow` declares its
dependency exactly as it would for `clip:gear`, so every document remains self-describing and its
meaning never depends on which host version happens to bundle what. What the blessing buys is
provenance and governance, not mechanism:

- The namespace URI lives under the reserved space (one per module, e.g. `…/ext/clipart`), the
  module source lives in this repo (e.g. `crates/ext/clipart/`), and its artifacts are published
  and versioned with each release.
- Dogfooding is the point: if the seam can't express our own experiments, it's too weak.
- **Graduation**: when an experiment stabilizes, the element moves to the core namespace (a native
  `emit_*`, specified in Specification.md) and its import becomes unnecessary. The `xsvg:` name
  keeps working for at least one minor release — the published module remains importable and emits
  a deprecation comment alongside its output. Abandoned experiments are removed with a tombstone
  entry in the CHANGELOG.

## 2. Document surface

```xml
<xsvg viewBox="0 0 400 200"
      xmlns:x="https://xsvg.visioncortex.org"
      xmlns:xsvg="https://xsvg.visioncortex.org/ext/clipart"
      xmlns:clip="https://example.com/xsvg-clipart">

  <!-- Every non-core namespace is bound explicitly — official and third-party alike. -->
  <x:import ns="https://xsvg.visioncortex.org/ext/clipart"
            src="https://xsvg.visioncortex.org/pkg/ext/clipart-0.3.0.wasm"
            integrity="sha256-…"/>
  <x:import ns="https://example.com/xsvg-clipart"
            src="./clipart.wasm"
            integrity="sha256-…"/>

  <!-- Official extension (leaf generator) -->
  <xsvg:arrow x="20" y="90" length="200" thickness="24"
              head-width="60" head-length="50" outline="4" fill="var(accent)"/>

  <!-- Official extension (container): children are compiled by the core,
       then handed to the plugin to arrange/wrap -->
  <xsvg:callout x="240" y="20" tail="left" pad="10">
    <x:textbox in="#bubble">Beware of dog</x:textbox>
  </xsvg:callout>

  <!-- Third-party -->
  <clip:gear cx="330" cy="100" teeth="9" r="40" bore="12"/>
</xsvg>
```

- `<x:import>` is a **definition element** like `<x:theme>`: pre-scanned before serialization
  (the `load_theme` pattern, `compile/mod.rs:2513`), emits nothing, forward references work.
  Attributes: `ns` (required), `src` (required; resolved against the document base via the
  existing `Resolver` rules of §4.2 cross-file links), `integrity` (optional but recommended;
  SRI-style hash pinned before instantiation — a mismatch degrades the whole namespace with a
  marker naming the reason).
- One import per namespace; a second import of the same `ns` with a different `src` is a
  degradation (first wins, marker emitted).
- The **native hosts accept local paths only** — a `src` with a scheme degrades rather than
  being fetched, because a CLI or a synchronous Node call reaching the network would be a
  surprise. Vendor the module next to the document, as with any other dependency.
- Plugin elements compose exactly like built-ins: their emitted `d` geometry is harvested by
  `x_output_geometry` (`compile/mod.rs:1887`), so a `<clip:gear>` is a valid `<x:boolean>` operand,
  warp target, or `<x:textbox in=…>` container with **zero additional plumbing**.
- **Reserved attributes stay reserved**: `x:layer`, `x:order`, `x:hidden`, `x:artboard`, `id`,
  `transform` on a plugin element are handled by the core (layering, sourcemap stamping, ref
  identity) and are not passed through to the generator. Everything else on the element is the
  plugin's parameter surface.

## 3. Compiler seam (`xsvg-core`)

The core stays **pure Rust, no WASM runtime, no JS** — it cannot and should not load modules. It
gains one trait, mirroring the existing `Measurer`/`Shaper`/`Resolver` seams:

```rust
/// Host-supplied registry of loaded generator modules.
pub trait GeneratorHost {
    /// Lower one plugin element. `None` = no module bound for this namespace
    /// (or the module doesn't claim this tag) → the caller degrades with a marker.
    fn generate(&self, ns: &str, tag: &str, req: &GenRequest) -> Option<GenResult>;
}

pub struct GenRequest<'a> {
    pub attrs: Vec<(&'a str, &'a str)>,      // non-reserved attributes, document order
    pub input_d: Option<String>,             // resolved `in="#id"` geometry, if declared
    pub children: Vec<ChildFragment>,        // compiled children, if the element declares them
    pub tolerance: f64,                      // QualityProfile::tolerance()
}

/// One child of a container element, already lowered by the core.
pub struct ChildFragment {
    pub svg: String,                // compiled, sanitized-subset SVG fragment
    pub bbox: Option<[f64; 4]>,     // [x, y, w, h] harvested from the fragment's geometry
}

pub enum GenResult {
    Svg(String),          // emitted fragment (sanitized by the core before splicing)
    Pending,              // module known but not yet instantiated → marker now,
                          //   host records the miss for the next fixpoint round
}

pub struct NoGenerators;  // default host: always None (CLI without plugins, tests)
```

Wiring, in order of the dispatch chain in `serialize` (`compile/mod.rs:606`):

1. The core namespace match at `:623` is untouched.
2. Any namespace present in the `ImportTable` — official and third-party alike — calls
   `ctx.generators.generate(...)` **before** the foreign-namespace drop at `:645`. For
   `"container"` elements the core lowers the children first (§4) to build the request.
   Un-imported foreign namespaces (`sodipodi:`, `inkscape:`) still drop as today.
3. `None`/`Pending` → `<!-- xsvg: <ns:tag> no generator bound / loading -->`.
4. **Sanitization is the core's job, not trust in the plugin**: returned SVG is parsed and
   re-serialized through the same static-subset pass as document content — `script`/`animate*`
   dropped (`:679`), `on*` attributes stripped (`:2588`), foreign namespaces dropped. A fragment
   that fails to parse degrades with a marker.
5. `pos_attr` stamps the *plugin element's* source range onto the sanitized output's root nodes,
   so inspector/sourcemap round-tripping works unchanged.
6. **Incrementality**: `baked_refs` (`:386`) learns two edges — a plugin element's `in="#id"`
   creates a dependency edge (recalc when the target edits), and every plugin element depends on
   its namespace's `<x:import>` (recalc all of a namespace when the import edits). That is the
   entire integration with [Incremental.md](Incremental.md).

`Ctx` (`compile/mod.rs:68`) gains `generators: &'a dyn GeneratorHost` and the pre-scanned
`ImportTable`. All `*_impl` entry points thread it through, defaulting to `NoGenerators`.

## 4. Plugin ABI

**String-in / string-out, versioned, boring on purpose.** The compiler has no structured IR — `d`
strings are the geometry currency, re-scraped from emitted markup (`find_path_d_ranges`,
`compile/mod.rs:1268`) — so a string ABI loses nothing versus the built-ins, and stays writable
from Rust, C, Zig, or AssemblyScript with no shared toolchain. WIT / the Component Model is the
principled successor if the surface ever grows past two functions; today it would add toolchain
weight on both sides for no needed capability.

A plugin module exports:

```
memory                                    ; exported linear memory
xsvg_abi_version() -> i32                 ; this document describes ABI 1
xsvg_alloc(len: i32) -> ptr               ; host allocates guest buffers here
xsvg_free(ptr: i32, len: i32)
xsvg_describe() -> ptr                    ; UTF-8 JSON manifest, length-prefixed (u32 LE)
xsvg_generate(req_ptr: i32, req_len: i32) -> ptr   ; request JSON in → response, length-prefixed
```

**Manifest** (`xsvg_describe`) — validated once at instantiation:

```json
{
  "abi": 1,
  "ns": "https://example.com/xsvg-clipart",
  "name": "clipart", "version": "0.3.0",
  "elements": {
    "arrow": {
      "params": {
        "length":     { "type": "number", "default": 100, "min": 0 },
        "thickness":  { "type": "number", "default": 20,  "min": 0 },
        "head-width": { "type": "number", "default": 40 },
        "outline":    { "type": "number", "default": 0 }
      },
      "input": "none",       // "none" | "optional" | "required" — the `in=` geometry
      "children": "none"     // "none" | "container" — see below
    },
    "callout": {
      "params": { "tail": { "type": "enum", "values": ["left", "right", "none"] },
                  "pad":  { "type": "number", "default": 8 } },
      "input": "none",
      "children": "container"
    },
    "gear": { "…": "…" }
  }
}
```

**Containers** (`"children": "container"`). The core **compiles the element's children first** —
each child is lowered exactly as if it stood alone (so nested `x:` elements, themes, and refs all
work), then handed to the plugin as `children: [{ svg, bbox }]` in document order. The plugin
arranges, wraps, decorates, or measures them and emits the composite fragment — typically
splicing the child fragments verbatim inside its own geometry. Consequences:

- Child fragments are **already core-produced and subset-safe**; the plugin's *composite* output
  still goes through the §3 sanitizer like any other.
- Child fragments carry their own `data-xsvg-pos` stamps; a container that splices them verbatim
  preserves inspector/sourcemap round-tripping for its content. Mangling them degrades only
  inspectability, never safety.
- `bbox` is harvested from the fragment's path geometry (the `find_path_d_ranges` +
  `svg_path_bbox` machinery, `compile/mod.rs:1268`/`:1763`). Children that emit **live text**
  contribute their laid-out line boxes from the text pipeline rather than glyph outlines; a child
  with no recoverable geometry gets `bbox: null` and the plugin decides how to degrade.
- A `"children": "none"` element with children ignores them with a marker; a `"container"`
  element with no children receives an empty list (rendering an empty frame is the plugin's call).

The declared `ns` MUST match the `<x:import ns>` that bound the module (mismatch = degradation for
the whole namespace, named in the marker). The manifest is also the **authoring surface**: an
editor host can generate property panels from `params` without instantiating anything else — the
same way a spreadsheet surfaces a UDF's signature.

**The param schema is a shared contract with editors, not a compiler-local detail.** The set of
param types is deliberately small and is kept a **strict subset of the editor-side declared-UI
widget vocabulary** (stylus `DESIGN.md` §3.2): every type + constraint combination legal in a
manifest must map totally onto one native control, so manifest-driven property panels are a pure
function of the manifest — no per-plugin rendering code in any host. Consequences: the manifest
never grows layout (presentation is the editor's; a plugin wanting a richer panel ships a
companion editor extension), and growth is **ordered** — an editor vocabulary gains the control
first, the manifest schema adopts the type second. ABI 1's shipped types are the frozen seed of
the shared set; adding a param type is a cross-spec act, not a manifest patch.

**Generate**: request JSON is `{tag, attrs, input_d, children, tolerance}` — one entry point for
every element the module provides, so a module needs no per-element export. Response is
`{ "svg": "…" }` or `{ "error": "reason" }` (degrades with the reason in the marker). Unknown
attributes are passed through to the plugin (its schema, its call); unknown *elements* in a bound
namespace degrade in the core.

**Determinism is structural, not promised.** Plugins run as WASM on *every* host — browser and
native alike — so the cross-target float-divergence care the built-ins need
(`connector.rs:246-248`) doesn't apply: the same module bytes given the same request produce the
same output string everywhere, by construction. The compare harness (`scripts/compare.mjs`) pins
this per sample.

**Sandbox contract**: no WASI, no imports beyond (optionally) a host `log` for diagnostics. A
plugin cannot see the document, other elements, the DOM, or the network — it is a pure function
from request to fragment. Purity across calls is *enforced* by the host: see §6.

## 5. Host integration

The core asks `GeneratorHost` synchronously; module *acquisition* is async. Unlike the
`Resolver`'s dependency discovery, this needs **no fixpoint round**: `<x:import>` is a
declaration, not something discovered by lowering, so `imports_impl` reports a document's
bindings from a parse alone and the host loads modules **before** the first compile.

`ElementInfo::Pending` remains for hosts that skip the pre-scan (and for a module still in
flight): those elements degrade with a marker and the host recompiles once loading finishes.
Correctness never depends on the pre-scan — only the wasted round does.

*Limitation.* Imports are read from the **entry document** only. A cross-file dependency's own
`<x:import>` is not visible, so plugin elements inside a linked file degrade. Lifting this means
either recursing the pre-scan through the resolver or merging per-file import tables during
`emit_link`; neither is done.

Per platform:

| Host | Runtime for plugin modules | Wiring |
|---|---|---|
| Browser (`xsvg-viewer`) | `WebAssembly.instantiate` in JS | `JsGeneratorHost` trampoline in `crates/xsvg-wasm`, same pattern as `JsMeasurer` (`src/lib.rs:23`); `DepLoader` grows a `loadModule(src)` hook |
| Node (`xsvg-compile`) | Node's own `WebAssembly` | modules read from disk relative to `basePath`; `plugins: false` opts out |
| CLI (`xsvg-cli`) | `wasmi` — a pure-Rust interpreter | `DiskGenerators`; **on by default**, `--no-plugins` opts out |
| Official `xsvg:` modules | same as any imported module — no special path | artifacts published per release (project CDN + npm package); documents import them by URL or a vendored local copy |

Instantiated modules are cached per host by `(src, integrity)` across compiles — recompiling on
every keystroke must not re-instantiate.

## 6. Safety & resource bounds

WASM gives isolation; the host adds **liveness and purity**:

- **Fuel / time**: the native hosts run each `xsvg_generate` call under **wasmi fuel**, so
  exhaustion degrades that element with a marker, extending the existing budget family
  (`MAX_REF_DEPTH`, `REF_FUEL`, `compile/mod.rs:33-44`).
  **The browser host cannot do this**, and does not pretend to: a synchronous WASM call on the
  main thread is not interruptible, so a non-terminating generator hangs the tab. Bounding it
  needs the compile moved to a Worker (terminable) — worth doing before third-party modules are
  loaded from anywhere untrusted, and out of scope here. A module that *traps* is handled: the
  instance is dropped and that element degrades.
- **Memory**: a trap from exhausting memory degrades the element rather than the compile.
- **Purity by construction**: a module keeping mutable globals could return different output for
  the same request. The browser host **drops every instance between compiles** (the compiled
  `WebAssembly.Module` is kept, so this costs an instantiation, not a compilation), making
  retained state unobservable across compiles — which is what makes the incremental canary
  (`fragments_are_verbatim_slices_of_the_full_compile`, [Incremental.md](Incremental.md)) hold
  for plugin content. Statefulness *within* one compile is still possible in principle; memoizing
  by request hash (the `Ctx.ref_memo` discipline) would close that and speed up repeated elements,
  and is not yet done.
- **Output sanitization** in the core (§3, item 4) — the security boundary does not depend on the
  host doing it.
- **Supply chain**: `integrity` pins bytes; hosts SHOULD refuse remote (cross-origin) `src`
  without it. A future registry/signing story is out of scope here.

## 7. What plugins can NOT do (v1)

Explicitly out of scope, to keep the seam small and the contract auditable:

- **No host callbacks** — no text measurement, no font access, no resolving further references
  from inside a plugin. A plugin needing a referenced region declares `input: "optional|required"`
  and receives it pre-resolved as `input_d`. (If a real use case needs measurement, ABI 2 can add
  a restricted import; do not preemptively design it.)
- **No child source introspection** — a container receives its children *compiled* (§4), never
  their XML source, and cannot re-lower a child with different parameters or inject attributes
  into it. Layout influence flows one way: the child compiles standalone, the container places
  the result.
- **No custom attributes on foreign elements** — only elements. The `x:layer`-style attribute
  surface stays core-only.
- **No new paint servers, filters, or text shaping** — output is the same static-SVG subset as
  everything else.

## 8. Relationship to a future formula layer

The xlsx analogy, made precise: plugins are **UDFs**; the missing piece is **formulas** —
attribute-level expressions (`length="ref(#box).width + 20"`). That is a separate subsystem, but
this design is laid out so it composes rather than conflicts:

- Expressions would evaluate in `resolve_var` (`compile/mod.rs:2541`), *before* attributes reach
  `GenRequest` — plugins never see expressions, only resolved values. The ABI does not change.
- Expression references add edges to the same `baked_refs` graph that already drives
  `dependents_multi_impl` — one recalc engine for links, plugin inputs, and formulas.

## 9. Phasing

Steps 1–5 are done: `crates/xsvg-core/src/compile/plugin.rs`, `crates/xsvg-plugin`,
`crates/ext/clipart`, and the three hosts (`packages/xsvg-viewer/src/plugins.ts`,
`crates/xsvg-cli/src/plugins.rs`, `packages/xsvg-compile/src/plugins.ts`).
`dataset/clipart.xsvg` is in the native-vs-browser parity harness and matches
structurally at 0.205% pixel difference — the same font-rendering delta every other
sample shows, since both hosts run identical module bytes.

Step 7 is done too: the interactive viewer's inspector reads a module's manifest and
offers **editable controls** for a pinned generator element
(`packages/xsvg-viewer/src/params.ts`). Reading a schema runs no generator, so a host
can build UI for an element the compiler has never heard of.

Two limits worth knowing:

- **Edits commit on `change`, not `input`.** Applying one recompiles the document and
  rebuilds the panel, which would yank a control out from under a drag in progress.
  Live dragging needs the parameter panel to survive a recompile — i.e. to be owned by
  the host rather than rebuilt with the inspector — which is a worthwhile follow-up.
- **Only plugin elements get generated panels**, because only they have manifests. The
  built-ins' parameters live in Rust source, so `<clip:arrow>` gets controls while
  `<x:pie>` still shows a read-only attribute table. Closing that gap means giving the
  built-ins machine-readable schemas too — a separate piece of work, and the point at
  which the "plugins are UDFs" analogy would extend to the whole language.

Step 6 (a non-Rust plugin example proving the ABI is toolchain-neutral) remains, and is
deliberately skipped for now.

1. **Seam** — `GeneratorHost` + `ImportTable` pre-scan + dispatch arm + output sanitizer +
   `baked_refs` edges. Core-only, `NoGenerators` default, all existing tests green. Unit-test with
   a mock host (the `test_doubles` pattern).
2. **First plugin** — `crates/ext/clipart/` with `arrow` (outlined thick arrow: shaft + head as
   one outlined path, miter geometry from `thickness`/`head-width`/`head-length`/`outline`),
   under `…/ext/clipart`, published as a versioned artifact. Pure `#[no_std]`-leaning Rust +
   kurbo, built with `wasm-pack`.
3. **Browser host** — `JsGeneratorHost` trampoline, `DepLoader.loadModule`, fixpoint round,
   integrity checking. Dataset sample importing the published clipart module + `compare.mjs`
   coverage.
4. **Native hosts** — wasmtime in the CLI (feature-gated), Node host; the compare harness now pins
   browser-vs-CLI plugin output bit-identical.
5. **Containers** — child pre-compilation, `ChildFragment` bbox harvest, and a second official
   plugin exercising it (e.g. `callout`: bubble sized to its children's bboxes plus a tail).
6. **Third-party path** — `<x:import>` of a non-reserved namespace end-to-end, a "writing an xsvg
   plugin" guide with a non-Rust (e.g. AssemblyScript or Zig) example proving the ABI is
   toolchain-neutral.
7. **Editor surface** (stretch) — property panels generated from manifests in the interactive
   viewer.

## 10. Open questions

- **Version pinning syntax** — should `<x:import>` carry a `version` requirement checked against
  the manifest, or is `integrity` (exact bytes) the only pin? Leaning: integrity-only for v1;
  semver ranges invite a resolver we don't want to build yet.
- **Official-extension hosting** — imports need a stable `src`. Options: project CDN URLs
  (convenient, but documents then depend on a remote origin) vs. recommending authors vendor the
  `.wasm` next to the document (self-contained, matches the cross-file-link philosophy). Likely:
  publish both, document vendoring as the default.
- **Container bbox fidelity** — a child whose visible extent isn't captured by harvested path
  geometry or text line boxes (e.g. stroke overshoot, filters) under-reports its bbox. Is
  stroke-width inflation worth doing in v1, or is `bbox: null`-style degradation enough?
- **Params in `<x:theme>`** — should plugin parameters accept `var()` tokens? (Almost certainly
  yes and nearly free, since `resolve_var` runs over attribute values — confirm ordering when
  implementing the pre-scan.) Note this is also a §4 shared-contract question: accepting tokens
  implies every mapped editor widget needs a token-picker mode — a vocabulary capability that
  must land editor-side first, per the ordering rule.
- **Manifest-driven validation strictness** — warn-and-pass vs. clamp-to-schema when an attribute
  violates `min`/`max`. Leaning: clamp, marker comment, keep totality. Clamp also matches native
  editor widgets (a bounded number field clamps at the control), keeping authoring-time and
  compile-time semantics identical — a point for clamp under the §4 shared contract.
