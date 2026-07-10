# Paint & Pixels — Pillar 3 capability catalog

> Companion to [Typography.md](Typography.md) (Pillar 1) and [Transform.md](Transform.md)
> (Pillar 2): what "paint" capability exists in the design-tool world, what SVG/CSS already give
> us, and what xsvg adds. Pillar 3 covers everything that changes **pixels or fills** rather than
> geometry: image adjustments, tone curves, and (planned) mesh gradients. Normative rules live in
> [Specification.md §8](Specification.md); this file is the map.

## A. Pixel adjustments — the CSS-parity set ✅ shipped

CSS `filter:` shorthand functions are formally sugar over SVG filter primitives — SVG had the
machinery first (SVG 1.0, 2001). xsvg's move: author the **standard `filter` attribute with CSS
function syntax** (already live in every browser, uncompiled — the §3 degradation contract for
free), and let the compiler lower it to the portable primitive form for static renderers.

| Capability | Design-tool analog | CSS | Lowered to | Status |
|---|---|---|---|---|
| Brightness | Exposure/Brightness | `brightness(k)` | `feComponentTransfer` linear, slope k | ✅ |
| Contrast | Contrast | `contrast(k)` | linear, slope k, intercept 0.5(1−k) | ✅ |
| Saturation | Vibrance/Saturation | `saturate(k)` | `feColorMatrix type="saturate"` | ✅ |
| Grayscale | Black & White | `grayscale(k)` | saturate(1−k) | ✅ |
| Sepia | Photo filter | `sepia(k)` | `feColorMatrix` (identity→sepia lerp) | ✅ |
| Invert | Invert | `invert(k)` | transfer table `k, 1−k` | ✅ |
| Hue shift | Hue/Saturation | `hue-rotate(a)` | `feColorMatrix type="hueRotate"` | ✅ |
| Opacity | Layer opacity | `opacity(k)` | alpha transfer table `0, k` | ✅ |

Two correctness details the hand-written form always gets wrong, handled by the lowering: the
filter runs in **sRGB** (`color-interpolation-filters` — SVG's linearRGB default visibly mismatches
CSS), and the region carries a ±10 % margin so strokes outside the fill bbox aren't clipped.

## B. Tone curves — beyond CSS ✅ shipped

Photoshop's *Curves* is a per-channel lookup table; SVG's `feComponentTransfer type="table"` **is**
a lookup table — CSS just never exposed it. xsvg does:

```
filter="-x-curve(0 0, 0.3 0.15, 0.7 0.9, 1 1)"      S-curve, all channels
filter="-x-curve-b(0 0.25, 1 0.9)"                   lift blues (per-channel: -r -g -b -a)
```

Control points in [0, 1]², interpolated **monotone-cubically** (Fritsch–Carlson — no overshoot, a
monotone point set yields a monotone curve), sampled into a 64-entry table. Follows the `-x-`
vocabulary convention (`-x-google-…` fonts): browsers ignore the unknown function and render
unfiltered until compiled — degradation, never breakage.

## C. Deferred / planned

| Capability | Why deferred | Plan |
|---|---|---|
| `blur()` / `drop-shadow()` | need **region inflation** (a blur bleeds past the bbox; the v1 set is pointwise) | lower with radius-derived region growth |
| Levels (`-x-levels(black white gamma)`) | expressible today as a 3-point curve | sugar over `-x-curve` when demanded |
| `backdrop-filter` semantics | needs compositing context, not static-subset-able | out of scope |
| Mesh feathering (per-corner alpha) + smooth-interior T-junctions + `.qmesh` import | v1 meshes are opaque RGB with crack-side T-junctions only | additive on the shipped §8.2 model |

## D. Mesh gradients — `<x:mesh>` ✅ shipped (v1)

The pillar headline ([Specification.md §8.2](Specification.md)): corner colors on an indexed
quad/tri mesh, cracks derived from color disagreement, triangles first-class. Lowered by
**render → refit**: rasterize in linear-light, fit each crack region with a seam-free
shared-vertex grid field (grow until the residual passes the profile tolerance), and serialize
each region as a **texel-aligned tiny PNG** — placed so its texel centers land on the grid
vertices, the renderer's own bilinear image filter reconstructs the field exactly (a single patch
is literally a stretched 2×2). Engine: the workspace `gradient` crate, extracted from vtracer's
quadmesh/gradient work.

**SVG 2 / Inkscape `<meshgradient>` fills compile too**: Coons patches (cubic edges) tessellate
into the same straight-quad mesh — polycurve → points — making Inkscape mesh files renderable
outside Inkscape for the first time.

## Status summary

**Shipped:** the full CSS pixel-adjustment vocabulary (spec-exact primitives, sRGB and region
pitfalls handled), per-channel tone curves CSS never had, and v1 mesh gradients with cracks — the
tagline's last promise. **Next:** mesh feathering (per-corner alpha), smooth-interior
T-junctions, and `.qmesh` import from the vtracer pipeline.
