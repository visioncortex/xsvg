//! Minimal reproduction of a direction-dependence in kurbo's `simplify_bezpath`
//! (observed on kurbo 0.13.1) — kept in-repo to upstream, and guarded by the
//! `kurbo_simplify_reversed_run_canary` test so a fixing upgrade surfaces.
//!
//!     cargo run -p xsvg-core --example kurbo_simplify_repro
//!
//! The same polyline — uniform samples of the symmetric parabola
//! `y = −25·(1 − (x/50 − 1)²)`, x ∈ [0, 100] — simplifies to a MoveTo + 2 cubics
//! traversed left-to-right, but barely fuses traversed right-to-left:
//!
//!     n= 2000  default            forward=   3  reversed=  33
//!     n=   16  angle_thresh .25   forward=   3  reversed=  32
//!     n=   16  .25 + Optimize     forward=   3  reversed=   5
//!
//! (At n = 2000 the turning angle per vertex is below the default 1-mrad corner
//! threshold, so the defaults repro with no options at all; the sparse variant
//! needs a raised `angle_thresh` to fuse anything. `SimplifyOptLevel::Optimize`
//! mostly masks the asymmetry but is much slower.)
//!
//! For the upstream issue, replace `xsvg_core::kurbo` with `kurbo`.

use xsvg_core::kurbo::simplify::{simplify_bezpath, SimplifyOptLevel, SimplifyOptions};
use xsvg_core::kurbo::BezPath;

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
    let count = |n: usize, rev: bool, opts: &SimplifyOptions| {
        simplify_bezpath(parabola(n, rev).elements().iter().copied(), 0.25, opts)
            .elements()
            .len()
    };
    for (n, label, opts) in [
        (2000, "default", SimplifyOptions::default()),
        (
            16,
            "angle_thresh .25",
            SimplifyOptions::default().angle_thresh(0.25),
        ),
        (
            16,
            ".25 + Optimize",
            SimplifyOptions::default()
                .angle_thresh(0.25)
                .opt_level(SimplifyOptLevel::Optimize),
        ),
    ] {
        println!(
            "n={n:5}  {label:18} forward={:4}  reversed={:4}",
            count(n, false, &opts),
            count(n, true, &opts)
        );
    }
}
