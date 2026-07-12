//! Path inset/outset (§7.7): grow or shrink a filled region by a distance — a
//! Minkowski offset, the "Offset Path" (Illustrator) / "Inset & Outset"
//! (Inkscape) operation SVG never got. Built from primitives already here, no new
//! dependency: stroke the region's boundary by `2·|distance|` ([`kurbo`]), then
//! **union** (outset) or **subtract** (inset) that band from the region
//! ([`crate::boolean`], i_overlay). The band reaches `distance` to each side of
//! the boundary, so a union grows the region outward by `distance` and a subtract
//! eats it inward by `distance`. The join controls the corner: `round` is the
//! true disc offset (rounded convex corners), `miter` keeps them sharp (to the
//! limit), `bevel` cuts them flat. i_overlay resolves the offset's
//! self-overlaps at concave corners, so the result is a clean region.

use crate::boolean::{boolean_svg_paths, BoolOp, BoolOperand};
use crate::warp::serialize_compact;
use kurbo::{stroke, BezPath, Join, PathEl, Stroke, StrokeOpts};

/// Ensure every subpath ends closed — offset treats its input as a filled region,
/// so an open subpath is closed rather than stroked with caps into a "worm".
fn closed(path: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let mut open = false;
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if open {
                    out.close_path();
                }
                out.push(*el);
                open = true;
            }
            PathEl::ClosePath => {
                out.push(*el);
                open = false;
            }
            _ => out.push(*el),
        }
    }
    if open {
        out.close_path();
    }
    out
}

/// Offset a filled region (given as one or more SVG path `d` strings, combined
/// under `even_odd`/nonzero) by `distance`: positive **outsets** (grows), negative
/// **insets** (shrinks). Returns compact path data like [`boolean_svg_paths`]:
/// `None` when there is no usable geometry, `Some("")` when the result is
/// legitimately empty (e.g. an inset larger than the region's half-thickness),
/// else `Some(d)`.
pub fn offset_svg_paths(
    paths: &[&str],
    distance: f64,
    join: Join,
    miter_limit: f64,
    even_odd: bool,
    tolerance: f64,
) -> Option<String> {
    // zero (or sub-tolerance) distance is identity — normalize the region's
    // winding through a self-union so the output is still a clean single path
    if distance.abs() <= tolerance.max(1e-3) {
        return boolean_svg_paths(
            &[BoolOperand {
                paths: paths.to_vec(),
                even_odd,
            }],
            BoolOp::Union,
            tolerance,
        );
    }

    // stroke each subpath's (closed) boundary into a band 2·|distance| wide
    let width = 2.0 * distance.abs();
    let style = Stroke::new(width)
        .with_join(join)
        .with_miter_limit(miter_limit.max(1.0));
    let flat_tol = tolerance.max(0.01);
    let mut band_ds: Vec<String> = Vec::new();
    for d in paths {
        let Ok(path) = BezPath::from_svg(d) else {
            continue;
        };
        let band = stroke(closed(&path), &style, &StrokeOpts::default(), flat_tol);
        if band.elements().is_empty() {
            continue;
        }
        if let Some(bd) = serialize_compact(&band, tolerance) {
            if !bd.is_empty() {
                band_ds.push(bd);
            }
        }
    }
    if band_ds.is_empty() {
        return None;
    }

    let band_refs: Vec<&str> = band_ds.iter().map(String::as_str).collect();
    let region = BoolOperand {
        paths: paths.to_vec(),
        even_odd,
    };
    let band = BoolOperand {
        paths: band_refs,
        even_odd: false,
    };
    // outset = region ∪ band; inset = region − band
    let op = if distance > 0.0 {
        BoolOp::Union
    } else {
        BoolOp::Subtract
    };
    boolean_svg_paths(&[region, band], op, tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{PathEl, Shape};

    /// Absolute filled area (shoelace, holes cancelling) of a compact `d`.
    fn area(d: &str) -> f64 {
        let path = BezPath::from_svg(d).unwrap();
        let mut total = 0.0;
        let mut c: Vec<(f64, f64)> = Vec::new();
        let mut flush = |c: &mut Vec<(f64, f64)>| {
            let mut a = 0.0;
            for i in 0..c.len() {
                let (x0, y0) = c[i];
                let (x1, y1) = c[(i + 1) % c.len()];
                a += x0 * y1 - x1 * y0;
            }
            total += a / 2.0;
            c.clear();
        };
        for el in path.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    if !c.is_empty() {
                        flush(&mut c);
                    }
                    c.push((p.x, p.y));
                }
                PathEl::LineTo(p) => c.push((p.x, p.y)),
                PathEl::ClosePath => flush(&mut c),
                _ => {}
            }
        }
        if !c.is_empty() {
            flush(&mut c);
        }
        total.abs()
    }

    const SQ: &str = "M0,0 L100,0 L100,100 L0,100 Z"; // 100×100 = area 10000

    #[test]
    fn outset_grows_inset_shrinks_by_distance() {
        // miter join keeps square corners → exact (100+2d)² and (100−2d)²
        let out = offset_svg_paths(&[SQ], 10.0, Join::Miter, 8.0, false, 0.1).unwrap();
        assert!(
            (area(&out) - 120.0 * 120.0).abs() < 60.0,
            "outset {}",
            area(&out)
        );
        let ins = offset_svg_paths(&[SQ], -10.0, Join::Miter, 8.0, false, 0.1).unwrap();
        assert!(
            (area(&ins) - 80.0 * 80.0).abs() < 60.0,
            "inset {}",
            area(&ins)
        );
    }

    #[test]
    fn outset_bounds_expand_on_every_side() {
        let out = offset_svg_paths(&[SQ], 12.0, Join::Round, 4.0, false, 0.1).unwrap();
        let bb = BezPath::from_svg(&out).unwrap().bounding_box();
        assert!(
            bb.x0 < -8.0 && bb.y0 < -8.0 && bb.x1 > 108.0 && bb.y1 > 108.0,
            "{bb:?}"
        );
    }

    #[test]
    fn round_join_outsets_larger_area_than_bevel_but_less_than_miter() {
        // at a convex corner: miter (full square corner) > round (quarter disc) > bevel (clipped)
        let m = area(&offset_svg_paths(&[SQ], 20.0, Join::Miter, 8.0, false, 0.1).unwrap());
        let r = area(&offset_svg_paths(&[SQ], 20.0, Join::Round, 4.0, false, 0.1).unwrap());
        let b = area(&offset_svg_paths(&[SQ], 20.0, Join::Bevel, 4.0, false, 0.1).unwrap());
        assert!(m > r && r > b, "miter {m} round {r} bevel {b}");
    }

    #[test]
    fn over_inset_collapses_to_empty() {
        // inset a 100×100 square by 60 (>half) → nothing left
        let d = offset_svg_paths(&[SQ], -60.0, Join::Miter, 8.0, false, 0.1).unwrap();
        assert!(d.is_empty(), "expected empty, got {d}");
    }

    #[test]
    fn zero_distance_is_identity() {
        let d = offset_svg_paths(&[SQ], 0.0, Join::Round, 4.0, false, 0.1).unwrap();
        assert!((area(&d) - 10000.0).abs() < 1.0, "{}", area(&d));
    }

    #[test]
    fn degenerate_input_never_panics() {
        assert!(offset_svg_paths(&["nonsense"], 5.0, Join::Round, 4.0, false, 0.1).is_none());
    }
}
