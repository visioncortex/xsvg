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

**Cross-reference inventory** — the only compile-time-baked references are the three `in="#id"`
consumers (`<x:textbox in>`, `<x:textpath in>`, `<x:warp field="bend" in>`). Everything else that
references by id — gradient/pattern paints via `url(#…)`, `<use href>`, and the `<textPath href>`
that `effect="follow"` emits — stays a **live reference** the browser re-resolves, so editing those
targets needs only the target's own re-emit. (Caveat inside the rule: `follow` *also* bakes the
path's arc length into `startOffset`, but its source attribute is `in`, so the dependency scan
covers it.)

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
- `dependents(input, offset) → [start, end, …]` — ranges of the *other* top-level elements whose
  baked `in="#id"` references point into this fragment; they must be re-emitted alongside it.

## 4. JS-side protocol (next slice)

1. Keep the compiled document in the DOM with `sourcemap` on; maintain a map
   `sourceRange → DOM node` from the `data-xsvg-pos` attributes.
2. On an edit at `[a, b)` with byte delta `d`: shift every bookkept range ≥ `b` by `d` (single
   contiguous edits only; anything fancier → full recompile).
3. `fragment_range(newSource, editOffset)` → the unit to re-emit; `dependents(…)` → additional
   units. For each: `compile_fragment(…)`, parse the markup in an **SVG namespace context**
   (`<template>`/`createContextualFragment` inside an `<svg>`), and replace the mapped DOM node.
4. Fall back to a full compile when: the fragment previously emitted no element (marker-only
   output has no DOM anchor), the edit touches the root element's own tag/attributes, fonts
   finish loading (outline availability changes output globally), or bookkeeping is lost.

## 5. Known limits (v1)

- **Granularity** is the top-level element: editing a node nested in a deep passthrough `<g>`
  re-emits the whole top-level group. Acceptable; refine later if profiling demands.
- **Dependency scan is direct, not transitive** — `in` targets are plain shapes in practice, and
  `x:` elements cannot themselves be `in`-targets (they lower to something else), so there are no
  meaningful chains today. Revisit if a referenceable lowered element ever ships.
- **A fragment can begin with marker comments** (skip/degradation markers precede the element).
  The JS layer should insert all parsed nodes, not just the first element.
- **`<defs>` content** passes through and participates like any top-level unit; editing a gradient
  needs no dependent re-emits (live references).
