# The plugin sandbox model

How much a `<x:import>`ed generator module is trusted, and what enforces it.

Companion to [Extensibility.md](Extensibility.md), which covers the *design* of the
plugin seam. This covers the *security* of it: what an author is trusting when they
reference a module, which component holds each guarantee, and — as importantly — what
is **not** guaranteed, so nobody infers a promise that isn't there.

## 1. Who is trusted

| Party | Trusted with | Why |
|---|---|---|
| The **document author** | Everything their document says | It is their document. Authored markup is their own words. |
| The **host** (viewer, CLI, extension) | Module bytes, resolution, network policy | It is the embedder; it already runs the compiler. |
| A **generator module** | **Nothing** | Referenced by URL, often written by someone else, and updatable without the author noticing. |

The last row is the whole point. An author who writes `<image href="https://…">`
themselves has decided to make that request. An author who writes
`<clip:arrow thickness="8"/>` has not decided anything of the kind — so plugin output
is held to a **stricter contract than authored markup**, and that asymmetry is
deliberate rather than an oversight.

## 2. Layer 1 — acquisition

Getting the bytes at all.

- **Native hosts (CLI, Node) accept local paths only.** A `src` carrying a scheme
  degrades with a marker instead of being fetched: a command-line compile reaching the
  network would be a surprise. Vendor the module beside the document.
- **The browser fetches same-origin**, and **refuses a cross-origin module that has no
  `integrity`** — that is precisely the case where a silently swapped module would be
  undetectable.
- **Every host verifies `integrity` when it is declared** (`sha256-<base64>`, the HTML
  spelling). The check is in each host, not in the core, because each host is what
  holds the bytes.
- **Origin comparison fails closed**: a `src` that will not parse as a URL is treated
  as cross-origin, so it needs a hash.
- A host may supply its own [`PluginLoader`](Extensibility.md#2-authoring-surface) —
  a sandboxed embedder reaches its module store over its own channel. `integrity` is
  still verified. Beyond that the host is trusting itself, which is the correct place
  for that decision to sit.

**Not guaranteed:** a same-origin module with no `integrity` is trust-on-first-use.
Nothing forces an author to pin, and nothing detects the file changing underneath
them. There is no registry and no signing.

## 3. Layer 2 — execution

What a module can do while it runs. This is the strongest layer.

- **No imports at all.** The module is instantiated with an **empty import object** —
  no WASI, no host callbacks, no clock, no randomness, no network, no filesystem. A
  generator is a pure function from a request buffer to a reply buffer, and this is
  enforced by construction rather than by policy: there is nothing to call.
- **No document access.** A generator receives its own element's declared parameters,
  and nothing else about the document — not its source, not its other elements, not
  the file it came from.
- **Except** a *container* generator, which receives its children **already compiled**
  (§4 of Extensibility.md). This is real document content flowing into third-party
  code and is the one deliberate hole in "no document access". It is contained by the
  fact that the module cannot transmit anything — see §4, where that becomes load
  bearing.
- **Resource bounds**: native hosts meter each call with wasmi fuel, so a runaway
  generator degrades that one element. A trap (including memory exhaustion) degrades
  the element rather than failing the compile.
- **Purity across compiles**: the browser drops every instance between compiles, so
  retained state cannot be observed from one compile to the next. That is what lets
  the incremental byte-stability invariant hold for plugin content.

**Not guaranteed:**
- **The browser cannot bound execution time.** A synchronous WASM call on the main
  thread is not interruptible, so a non-terminating generator hangs the tab. Fixing it
  means moving the compile to a Worker. Do that before loading modules from anywhere
  untrusted.
- **Statefulness *within* one compile** is still possible: a module may return
  different output for the same request at different points in a single pass.

## 4. Layer 3 — output

What the module is allowed to have drawn. Generator markup is re-parsed and pushed
through the **same serializer as authored content**, so this boundary does not depend
on the host implementing anything.

Applied to everyone, author and plugin alike:

- `<script>`, `<animate>`, `<animateMotion>`, `<animateTransform>`, `<set>` and
  `<discard>` are dropped with a marker — they cannot exist in the static output
  contract.
- `on*` event handlers are stripped.
- Foreign-namespace elements are dropped.

Applied **only to plugin output**, because only plugin output is untrusted:

- **`<foreignObject>` is dropped.** It is an escape hatch into HTML parsing, and a
  geometry generator has no business emitting one.
- **URL-valued attributes are allowlisted.** `href`, `xlink:href` and `src` may name
  a same-document `#fragment` or a `data:image/…` payload. Everything else — a remote
  URL, `javascript:`, any other scheme — is dropped.
- **`url(…)` in any attribute value** (`fill`, `stroke`, `filter`, `style`, …) is held
  to the same rule, and a reference that fails it becomes `none`.
- **Emitted `id`s are namespaced** to the element that produced them, and
  same-output references are rewritten to match — so a generator cannot shadow an
  authored `id` and capture a document's `url(#…)` paint or `<use href="#…">`.
- **No re-entry**: generator output cannot contain generator elements.

Anything dropped is counted, and the wrapper carries a marker naming the count, so a
degraded document says so instead of quietly rendering less.

### Why these five, specifically

Each closes a way for untrusted output to *act* rather than merely *draw*:

| Without it | What it buys an attacker |
|---|---|
| `javascript:` in `href` | Script execution on click |
| Remote `href`/`src` | An outbound request on render: IP and user-agent disclosure, confirmation a document was opened |
| Remote `url()` | The same request, through paint instead |
| `<foreignObject>` | Arbitrary embedded content, HTML parsing rules |
| Unscoped `id` | Hijacking an authored reference — the document's own `url(#logo)` resolving to plugin geometry |

The combination is what matters most. A container generator can read the markup it
wraps (§3), and on its own that is harmless because it cannot transmit. Give it *any*
outbound request and the two compose into **exfiltration of document content with no
script involved**: read the children, encode them into a URL, emit an `<image>`, let
the renderer make the request. Closing egress is what keeps §3's one deliberate hole
from becoming a channel.

## 5. What this model does not claim

- It does not make an untrusted module **safe to run in a browser tab** — see the
  execution-time gap in §3.
- It does not protect an author from a module that simply **draws the wrong thing**.
  Output is constrained in kind, not in meaning.
- It does not verify that the module you fetched is the module its author published.
  `integrity` pins bytes to a hash *the document itself carries*; a document that
  never pinned, or that pinned a hash from a compromised source, gets no help.
- It says nothing about the **document** being safe to render. Authored markup may
  still carry remote references, by design — the author chose them.

## 6. Where it is enforced

| Guarantee | Enforced in |
|---|---|
| Local-only `src` (native) | `crates/xsvg-cli/src/plugins.rs`, `packages/xsvg-compile/src/plugins.ts` |
| Cross-origin needs `integrity` | `packages/xsvg-viewer/src/plugins.ts` |
| `integrity` verification | all three hosts |
| Empty import object | all three hosts |
| Fuel metering | native hosts (wasmi) |
| Output filtering | `crates/xsvg-core/src/compile/plugin.rs` + `mod.rs`, host-independent |

Pinned by `plugin_output_cannot_reach_the_network`,
`plugin_ids_cannot_shadow_the_document`, and the static-subset tests in
`crates/xsvg-core/src/compile/tests.rs`.
