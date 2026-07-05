# xsvg — Geometry Transform Capabilities Catalog

A concrete enumeration of the non-affine geometry-transform features xsvg wants, borrowing the
**authoring model from Adobe Illustrator** and the **lowering model from the generic bake pipeline**
of [Specification.md §7](Specification.md). This expands Pillar 2 of the [Vision](Vision.md) and
feeds the roadmap in [Plan.md §2.3](Plan.md); prior art in [Research.md §7](Research.md). Companion
catalog: [Typography.md](Typography.md) (Pillar 1).

**How the two references divide up:**
- **Adobe Illustrator** = the *high-level* capabilities we emulate: **Envelope Distort** (Make with
  Warp — the 15 warp presets; Make with Mesh; Make with Top Object), the **Free Transform** distorts
  (Perspective Distort, Free Distort), the **Effect ▸ Distort & Transform** family, and the *Type on
  a Path* effects. Illustrator keeps these live and *Expand* bakes them — exactly xsvg's model: the
  source stays editable, and **the compiler is the Expand**.
- **The §7 pipeline** = the *low-level* model everything lowers through (the analog of PDF for
  typography): one **bake** — flatten → map → refit ([§7.1](Specification.md)) — over a library of
  pluggable **fields** `D : ℝ² → ℝ²` ([§7.2](Specification.md)), exposed by thin front-ends
  ([§7.3](Specification.md)). An effect is *just a field*; nothing in the pipeline is per-effect.
- **xsvg's differentiator:** SVG `transform` is affine-only, so none of this can ride on vector
  geometry — xsvg bakes to plain `<path>`s that render anywhere (no filters, no script, no raster).
  And because a field maps *points*, it applies to **any geometry**: shapes, paths, and outlined
  text (§6.12) alike — warped type is the same code path as a warped rectangle.

**Tier legend:** **C** = Core (needed for the pillar to feel complete) · **E** = Extended
(professional standard, strongly wanted) · **S** = Stretch (advanced / later).

**Status column** — where xsvg stands *today* (updated as the compiler grows):
- ✅ **shipped** — implemented in the compiler, tested.
- ◑ **partial** — a working slice exists, but pieces are missing (typically: browser-adapter-only,
  or spec'd attributes not yet wired).
- ○ **planned — field-only work** — needs only a field formula + its parameters plumbed through the
  existing bake; no new machinery.
- ❌ **needs pipeline work** — new machinery beyond a field function (cubic refit, arc-length/Frenet
  frames, lattice semantics, anchor-aware mapping, a native kurbo bake).

---

## A. The pipeline core — *machinery every effect shares*

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **The bake**: flatten → map → refit (§7.1) | §7 | C | ◑ | **native kurbo bake shipped** in `xsvg-core` for `<x:warp>` — flatten → map with *adaptive chord subdivision* (mapped-midpoint error ≤ tolerance), natively unit-tested; the §6.13 glyph bake still lives in the browser adapter; **refit is missing** — output is `M/L/Z` polylines |
| **`Field` seam** — `D: ℝ²→ℝ²` trait in `xsvg-core` | Plan §2.3 | C | ✅ | **shipped** — `Field` trait + the `EnvelopePreset` family over a normalized envelope frame; the §6.13 fields stay adapter-side until the glyph bake moves native |
| **Quality knob** — flatten tolerance ← `QualityProfile` | §7.1 | C | ◑ | **wired for `<x:warp>`**: `fast`/`balanced`/`highest` → 1.0/0.25/0.05 user units; the §6.13 adapter still hardcodes `size/12` |
| **`<x:warp>` generic front-end** (§7.3) | AI Envelope | C | ✅ | **shipped** — displacement presets over wrapped children; unknown/absent fields degrade behind a marker, unwarpable children skip with a marker |
| **Warp arbitrary geometry** (basic shapes, `<path>`, `<g>` subtrees) | AI | C | ✅ | **shipped** — shapes convert to path geometry and bake; live text / rounded rects / lines / images are skipped with a marker (never silently unwarped) |
| **Warp outlined text** | AI (after Create Outlines) | C | ✅ | **shipped** — `outline="true"` boxes and `<x:textpath>` output warp like any path inside `<x:warp>` |
| **Field composition** — nested `<x:warp>` | AI effect stack / Inkscape LPE | E | ✅ | **shipped** — nesting bakes innermost-first through the recursive serializer |
| **Non-destructive authoring** (originals stay editable in source) | AI Envelope / Inkscape LPE | C | ✅ | by construction — the xsvg document *is* the live state; every compile re-bakes from it |
| **Raster fallback** (`feDisplacementMap`) | Research §7 | S | ❌ | last resort for pathological path explosion; rasterizes, so off the default path |

## B. Warp presets — *Envelope Distort ▸ Make with Warp*

All 15 Illustrator warp styles, over a shared parameter model. Presets evaluate in the **normalized
envelope frame**: the pre-warp union bbox of the warped children maps to `(u, v) ∈ [−1, 1]²`, the
field maps normalized points, and the result maps back to user units.

**Shared parameters** (on `<x:warp>`, mirroring Illustrator's Warp Options dialog):

| Attr | Values | Initial | Effect |
|---|---|---|---|
| `field` | preset name (below) | — | selects the field |
| `bend` | −100…100 (%) | 0 | primary strength; sign flips direction |
| `axis` | `h` \| `v` | `h` | bend axis (Illustrator's Horizontal/Vertical radio); `v` swaps `u`/`v` |
| `distort-h`, `distort-v` | −100…100 (%) | 0 | Illustrator's Distortion sliders — a perspective-like taper **composed after** the preset (lowers to a homography field, §C) |

```xml
<x:warp field="flag" bend="60">
  <x:textbox x="0" y="0" width="240" height="60" outline="true" font-family="-x-google-Anton">WAVING</x:textbox>
</x:warp>
```

**Field families** (drive implementation order — every family is closed-form per-point, so every
remaining preset is one formula away on the native bake): **displacement** = `(u, v + f(u))`, the
§6.13 skew field with an analytic profile instead of a sampled path — **shipped first (§7.3)**;
**scale** = one axis scaled by a profile of the other; **polar** / **radial** / **rotational** =
true 2-D fields.

| Preset | Family | Tier | Status | xsvg note (field sketch, `axis="h"`, bend `b`) |
|---|---|---|---|---|
| **Arc** | polar | C | ○ | the box bends into an annular sector spanning `b·π`: verticals → radii, horizontals → concentric arcs |
| **Arc Lower** | scale | E | ○ | top edge fixed; height scales by the parabolic profile `1 + b·(1−u²)` so the bottom edge arcs |
| **Arc Upper** | scale | E | ○ | mirror of Arc Lower (bottom fixed, top arcs) |
| **Arch** | displacement | C | ✅ | **shipped** (§7.3) — `Δ = A·(1−u²)`, both edges ride the same parabola |
| **Bulge** | scale | C | ○ | height scales about the midline by `1 + b·(1−u²)` — both edges bow outward symmetrically |
| **Shell Lower** | scale | E | ○ | one-sided bulge: bottom edge bows, opposite curvature to Arc Lower (flared corners) |
| **Shell Upper** | scale | E | ○ | mirror of Shell Lower |
| **Flag** | displacement | C | ✅ | **shipped** (§7.3) — `Δ = A·sin(πu)` uniform in `v`, glyph columns ride the wave rigidly |
| **Wave** | displacement | C | ✅ | **shipped** (§7.3) — Flag with phase advancing π/2 through the height: `Δ = A·sin(πu − (π/4)(v+1))` |
| **Fish** | scale | E | ○ | midline bulge with a pinched tail — asymmetric taper × bulge |
| **Rise** | displacement | C | ✅ | **shipped** (§7.3) — `Δ = A·u`, a linear ramp; the art climbs left→right (pure shear profile) |
| **Fisheye** | radial | E | ✅ | **shipped** (§7.3) — `s = 1 + b·(1−r̂²)` about the frame center (`r̂` = corner-normalized radius); corners pinned; negative bend = pincushion |
| **Inflate** | radial | E | ✅ | **shipped** (§7.3) — per-axis bulge `sx = 1+(b/2)(1−ny²)`, `sy = 1+(b/2)(1−nx²)`; corners pinned |
| **Squeeze** | scale | E | ✅ | **shipped** (§7.3) — `u′ = u·(1−(b/2)(1−v²))`: waist pinch at mid-height, negative = barrel; `axis` transposes |
| **Twist** | rotational | E | ✅ | **shipped** (§7.3) — angle-true swirl `θ = b·90°·(1−r̂)`: center rotates most, corners pinned; same math as Effect ▸ Twist (§F) |

> **Fidelity note.** Illustrator's exact preset curves are unpublished. Each preset's normative
> formula is pinned in [Specification.md §7.2](Specification.md) when it ships, chosen to visually
> match Illustrator at `bend=±50%` on reference art; the dataset sample doubles as the fixture.

## C. Perspective & corner distortion

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Perspective transform** (homography) | AI Free Transform ▸ Perspective Distort | C | ○ | 8-DOF projective `D(x,y) = ((ax+cy+e)/(gx+hy+1), (bx+dy+f)/(gx+hy+1))`; authored as `corners="x0,y0 x1,y1 x2,y2 x3,y3"` (bbox corners → targets), solved by a small 8×8 linear system precomputed as field state. *The headline field for `<x:warp>` v1* |
| **Free Distort** (4-corner, non-projective) | AI Free Transform / Effect ▸ Free Distort | E | ○ | bilinear interpolation of the 4 corner displacements — same `corners` surface, `field="free"`; cheaper, no straight-line preservation |
| **Distortion sliders** (`distort-h` / `distort-v`) | AI Warp Options | C | ○ | one-axis corner taper = a constrained homography composed after any preset (§B) |
| **Axonometric / iso projections** (convenience) | xsvg | S | ○ | *affine* — expressible as SVG `transform` today; a named convenience (`field="iso"`) only |

## D. Envelope & handle warps — *freeform deformation*

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Envelope mesh** (m×n lattice, movable points) | AI Make with Mesh | E | ❌ | bivariate Bézier / FFD lattice (Sederberg–Parry); needs lattice syntax + basis evaluation; shares patch vocabulary with `<x:mesh>` (Pillar 3) |
| **Envelope — top object** (conform to an arbitrary shape) | AI Make with Top Object | S | ❌ | parameterize the target outline into a warped quad domain — the hardest envelope; needs shape parameterization machinery |
| **MLS handle warp** (move-a-few-points) | Schaefer et al. / xsvg | S | ❌ | `handles="x,y→x′,y′ …"`; affine/similarity/rigid classes; `rust_mls` or in-house — new dependency, so not field-only |
| **Bend along a path** (whole group follows a spine) | Inkscape LPE Bend | E | ❌ | the §6.13.2 path-follow field applied to arbitrary geometry — the arc-length + normal machinery now exists in the browser adapter (rainbow); generalizing it rides the `<x:warp>` native bake |

## E. Type on a path — *the text front-end (§6.13)*

The text specialization: the run is outlined (§6.12) and the reference path derives the field.
Catalogued from Illustrator's five *Type on a Path* effects.

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Skew** (vertical displacement by height profile) | AI | C | ✅ | **shipped** — `<x:textpath in="#p" effect="skew">`: glyphs stay upright, verticals stay vertical (§6.13.1) |
| **Rainbow** (arc-length follow + normal offset) | AI | C | ✅ | **shipped** — `effect="rainbow"`: uniform arc-length LUT + normal offset, straight extrapolation past the path's ends (§6.13.2) |
| **Baseline shift** (offset the run from the path) | AI baseline shift / SVG | C | ✅ | **shipped** — `baseline-shift` offsets along the local normal (positive = above the path); applies to skew + rainbow; opposite shifts stack two runs on one path |
| **Stair Step** (per-glyph vertical steps, no deformation) | AI | E | ✅ | **shipped** — authorable `effect="stair"` (§6.13.3): live `<text>` with per-glyph positions on the height profile — selectable, no font bytes needed, honors align/start/shift; also serves as skew's no-font degradation |
| **Gravity** (glyphs rotate toward a center) | AI | S | ○ | per-glyph rotation field about the path's bbox center |
| **3D Ribbon** (horizontals follow, verticals stay) | AI | S | ○ | the complement of skew — horizontal shear from the profile |
| **`align` / `start` placement options** | AI | C | ✅ | **shipped** — `align` distributes slack within the path's extent (x-extent under skew, arc length under rainbow); `start` adds an absolute head-start; both honored by warped runs *and* the stepped fallback |
| **Non-deforming follow** via native `<textPath>` | SVG | E | ○ | live-text lowering for quality `fast` / no-font cases; font-dependent, no shape deformation |

## F. Distort & Transform effects — *Illustrator's Effect menu*

Most of these operate **per anchor / per segment**, not as space fields — they need an anchor-aware
variant of the bake (map anchors + handles, don't flatten). The space-field subset rides §7.1 as-is.

| Capability | From | Tier | Status | xsvg note |
|---|---|---|---|---|
| **Twist** (swirl, rotation ∝ radius) | AI Effect | E | ✅ | **shipped** — `field="twist"` (§B) |
| **Roughen** (jittered edge displacement) | AI Effect | S | ○ | seeded deterministic noise as a displacement field over the flattened polyline (size/detail params); *seeded* so compiles are reproducible |
| **Pucker & Bloat** | AI Effect | S | ❌ | anchors stay, segment midpoints pull toward/away from center — anchor-aware, not a space field |
| **Zig Zag** | AI Effect | S | ❌ | per-segment ridges/waves between anchors — anchor-aware |
| **Tweak** (random anchor/handle jitter) | AI Effect | S | ❌ | anchor-aware |
| **Transform effect** (live repeated copies) | AI Effect | S | ❌ | not a deformation — a repeater; if wanted, it is a separate front-end (`<x:repeat>`), out of this pillar's scope |

## G. Composition & semantics — *cross-cutting rules to pin down*

| Question | Tier | Status | Position |
|---|---|---|---|
| **Warp × affine `transform`** | C | ○ | children's own `transform`s flatten into their geometry *before* the field; a `transform` on `<x:warp>` itself applies *after* the bake (it's affine — free) |
| **Live `<text>` children of `<x:warp>`** | C | ○ | auto-outline when font bytes are registered (as if `outline="true"`); else skip-with-marker — a warp MUST NOT silently emit unwarped text |
| **Stroke under warp** | E | ○ | v1: the baked `<path>` keeps the authored constant-width stroke (stroke paint does not deform — matches AI with Scale-Strokes off). Truly warped strokes = stroke-to-fill first (deferred `<x:vstroke>` machinery) |
| **Paint under warp** | E | ○ | fills ride the baked path; `objectBoundingBox` gradients stretch to the *new* bbox — document as the defined behavior; mesh-fill (Pillar 3) interplay TBD |
| **Envelope frame definition** | C | ○ | normalized frame = pre-warp union bbox of all children (Illustrator's envelope bounds) |
| **Fold-over / self-intersection** | E | ❌ | strong fields can fold geometry (fill-rule artifacts); v1 documents, does not detect; a validation/warning pass is future work |
| **Degradation contract** | C | ✅ | `<x:warp>` is an `x:` element — a plain viewer skips the whole subtree (§3); authors opt into that by using it, same as `<x:textbox>` |

---

## Status summary

**Shipped today (✅):** two front-ends.
**`<x:warp>`** (§7.3) — the generic pipeline: `Field` trait + native kurbo bake in `xsvg-core`
(flatten → map with adaptive chord subdivision, quality-graded tolerance, natively unit-tested), and
**eight presets** — displacement (arch / flag / rise / wave), radial (fisheye / inflate), scale
(squeeze), rotational (twist) — over shapes, paths, and outlined text, with innermost-first nesting
and marker-based degradation ([warp-presets.xsvg](../dataset/warp-presets.xsvg)).
**`<x:textpath>`** (§6.13) — **skew**, **rainbow** (arc-length LUT + normal offset, straight
extrapolation past the ends), authorable **stair**, `baseline-shift`, and `align`/`start` placement,
via the `GlyphOutliner::outline_on_path` browser seam
([textpath.xsvg](../dataset/textpath.xsvg), [textpath-rainbow.xsvg](../dataset/textpath-rainbow.xsvg),
[textpath-align.xsvg](../dataset/textpath-align.xsvg)). Non-destructive authoring holds by
construction.

**Partial (◑):** the bake emits polylines (**no cubic refit** yet — the `balanced`/`highest`
upgrade); the §6.13 glyph bake still lives in the browser adapter with a hardcoded tolerance, so the
text-on-path fields aren't natively tested the way `<x:warp>`'s are.

**Planned, field-only (○):** the remaining **7 warp presets** (§B — the arc/shell/fish scale family,
all closed-form over the shipped envelope frame), **perspective** and **free distort** (§C — a
corner-solved homography / bilinear field), the distortion sliders, and gravity/3D-ribbon type
effects. Each is now one pure function plus its attribute plumbing.

**Needs pipeline work (❌):** **bend-along-path** (the rainbow field generalized to arbitrary
geometry — waits on the `<x:warp>` native bake), **envelope mesh** (FFD lattice), **top-object
envelopes** (shape parameterization), **MLS handles**, the anchor-aware Effect-menu distortions
(Pucker & Bloat, Zig Zag, Tweak), and the raster `feDisplacementMap` fallback.

**Build order** (each slice = spec § + tests + dataset sample):

1. ~~**Skew** — the pipeline's first slice~~ ✅ *(shipped)*
2. ~~**`<x:warp>` + native bake** — `Field` trait in `xsvg-core`, kurbo-backed flatten of arbitrary
   `d`/shapes, `QualityProfile` → tolerance wiring; first fields: the four **displacement presets**
   (arch, flag, rise, wave) on arbitrary geometry + outlined text.~~ ✅ *(shipped)*
3. **Perspective** — homography field + `corners` solver; `distort-h`/`distort-v` sliders; free
   distort rides along.
4. **Remaining analytic presets** — the scale family (arc-lower/upper, bulge, shell ×2, fish) +
   polar (arc). ~~Radial (fisheye, inflate), rotational (twist), squeeze~~ ✅ *(shipped early)*.
   Full Make-with-Warp parity when done.
5. **Refit** — polyline → cubic fitting behind the quality knob (`fast` = polyline, `balanced`/
   `highest` = refit at graded tolerance). *(The `align`/`start` and stair-step items originally
   here shipped early, alongside rainbow.)*
6. ~~**Rainbow** — arc-length + normal machinery~~ ✅ *(shipped early with `baseline-shift`, riding
   the §6.13 adapter seam ahead of the native bake; bend-along-path §D still waits on slice 2)*
7. **Later** — envelope mesh (with Pillar 3's patch vocabulary), top object, MLS, anchor-aware
   effects, raster fallback.

The dividing line mirrors Typography's "Create Outlines" gate: there the gate was *getting glyph
geometry*; here it was **`<x:warp>` + the native bake** (slice 2) — and that gate is now **open**.
Any geometry runs flatten → map in the core, so every remaining ○ row is one pure function away;
the outstanding machinery (❌) is refit, arc-length for arbitrary geometry, and the lattice/handle
warps.
