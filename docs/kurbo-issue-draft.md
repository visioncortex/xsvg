# DRAFT — upstream issue for linebender/kurbo

> Post with `gh issue create -R linebender/kurbo --title "..." --body-file docs/kurbo-issue-draft.md`
> (strip this header first). Repro source: `crates/xsvg-core/examples/kurbo_simplify_repro.rs`
> (replace `xsvg_core::kurbo` with `kurbo` for a standalone reproduction).

**Title:** `simplify_bezpath` is direction-dependent: a reversed polyline barely fuses

## Summary

`simplify_bezpath` (kurbo 0.13.1) produces dramatically different results for the same
geometry depending on traversal direction. A dense uniform polyline sampling of the symmetric
parabola `y = −25·(1 − (x/50 − 1)²)`, `x ∈ [0, 100]`, simplifies to a `MoveTo` + **2 cubics**
traversed left-to-right, but **33 elements** traversed right-to-left — the same curve, mirrored
input order.

| input | options | forward | reversed |
|---|---|---|---|
| n = 2000 samples | defaults | 3 elements | 33 elements |
| n = 16 samples | `angle_thresh = 0.25` | 3 | 32 |
| n = 16 samples | `angle_thresh = 0.25` + `SimplifyOptLevel::Optimize` | 3 | 5 |

At n = 2000 the per-vertex turning angle is below the default 1-mrad corner threshold, so the
defaults reproduce it with no options at all; the sparse variant needs a raised `angle_thresh`
to fuse anything. `Optimize` mostly masks the asymmetry but is substantially slower.

Since the parabola is symmetric, forward and reversed inputs are congruent point sets — an
ideal simplifier should produce mirror-image outputs of equal size.

## Reproduction

```rust
use kurbo::simplify::{simplify_bezpath, SimplifyOptions};
use kurbo::BezPath;

fn parabola(n: usize, reversed: bool) -> BezPath {
    let mut pts: Vec<(f64, f64)> = (0..=n)
        .map(|i| {
            let x = i as f64 * 100.0 / n as f64;
            let u = x / 50.0 - 1.0;
            (x, -25.0 * (1.0 - u * u))
        })
        .collect();
    if reversed {
        pts.reverse();
    }
    let mut path = BezPath::new();
    path.move_to(pts[0]);
    for p in &pts[1..] {
        path.line_to(*p);
    }
    path
}

fn main() {
    let opts = SimplifyOptions::default();
    for reversed in [false, true] {
        let out = simplify_bezpath(parabola(2000, reversed), &opts);
        println!("reversed={reversed}: {} elements", out.elements().len());
    }
}
```

Observed (kurbo 0.13.1): `reversed=false: 3 elements`, `reversed=true: 33 elements`.

## Context

Found while using `simplify_bezpath` to refit flattened glyph outlines in a compiler
(xsvg). Dense, slightly quantized input in "unlucky" traversal directions stayed nearly
unfused, which combined with the cost of `Optimize` led us to disable refitting; a fix would
let us re-enable it. A canary test in our tree (`kurbo_simplify_reversed_run_canary`) watches
for the behavior changing across kurbo upgrades.
