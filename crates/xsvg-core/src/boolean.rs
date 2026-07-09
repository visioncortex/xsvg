//! Live path algebra (§7.5): boolean operations over filled regions, backed by
//! [`i_overlay`] — integer-exact and deterministic, matching the reproducible-
//! compile contract. Operands **flatten at the profile tolerance** (the same §7.1
//! graded-approximation step as every bake); the ops themselves are exact on the
//! flattened polygons, and the result serializes compactly.

use crate::warp::serialize_compact;
use i_overlay::core::fill_rule::FillRule as OverlayFillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_shape::base::data::Shapes;
use kurbo::{BezPath, PathEl};

/// The Pathfinder-style operation (§7.5). `union` / `intersect` / `exclude` are
/// symmetric folds over the operands; `subtract` removes every later operand from
/// the first (Illustrator's *Minus Front* — document order is back-to-front).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Intersect,
    Subtract,
    Exclude,
}

impl BoolOp {
    /// `None` for unknown names — the caller degrades behind a marker.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "union" => Some(Self::Union),
            "intersect" => Some(Self::Intersect),
            "subtract" => Some(Self::Subtract),
            "exclude" => Some(Self::Exclude),
            _ => None,
        }
    }
}

/// One operand: the path data of a single child (possibly several `d` strings —
/// e.g. an outlined multi-line textbox), resolved as one region under its own
/// fill rule before the op.
pub struct BoolOperand<'a> {
    pub paths: Vec<&'a str>,
    pub even_odd: bool,
}

type Contour = Vec<[f64; 2]>;

/// Flatten one operand's path data into closed polygon contours at `tolerance`.
/// Open subpaths close implicitly (fill semantics); degenerate contours drop.
fn flatten_operand(paths: &[&str], tolerance: f64) -> Vec<Contour> {
    let tol = tolerance.max(1e-3);
    let mut contours: Vec<Contour> = Vec::new();
    for d in paths {
        let Ok(path) = BezPath::from_svg(d) else {
            continue;
        };
        let mut cur: Contour = Vec::new();
        kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
            PathEl::MoveTo(p) => {
                if cur.len() >= 3 {
                    contours.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                if p.x.is_finite() && p.y.is_finite() {
                    cur.push([p.x, p.y]);
                }
            }
            PathEl::LineTo(p) => {
                if p.x.is_finite() && p.y.is_finite() {
                    cur.push([p.x, p.y]);
                }
            }
            PathEl::ClosePath => {
                if cur.len() >= 3 {
                    contours.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
            }
            _ => {}
        });
        if cur.len() >= 3 {
            contours.push(cur);
        }
    }
    contours
}

/// Combine `operands` under `op` at the profile `tolerance`. Returns:
/// - `None` — no usable geometry among the operands (the caller marks it);
/// - `Some("")` — a legitimately **empty** result (e.g. a disjoint `intersect`);
/// - `Some(d)` — the combined region as compact path data with deterministic
///   windings (outer/hole contours wind oppositely, so it renders identically
///   under either fill rule).
pub fn boolean_svg_paths(operands: &[BoolOperand], op: BoolOp, tolerance: f64) -> Option<String> {
    // resolve each operand to clean shapes under its own fill rule (this also
    // resolves self-intersections — e.g. fold-over from an earlier warp)
    let resolved: Vec<Shapes<[f64; 2]>> = operands
        .iter()
        .filter_map(|operand| {
            let contours = flatten_operand(&operand.paths, tolerance);
            if contours.is_empty() {
                return None;
            }
            let rule = if operand.even_odd {
                OverlayFillRule::EvenOdd
            } else {
                OverlayFillRule::NonZero
            };
            Some(contours.simplify_shape(rule))
        })
        .collect();
    if resolved.is_empty() {
        return None;
    }

    let rule = match op {
        BoolOp::Union => OverlayRule::Union,
        BoolOp::Intersect => OverlayRule::Intersect,
        BoolOp::Subtract => OverlayRule::Difference,
        BoolOp::Exclude => OverlayRule::Xor,
    };
    let mut acc = resolved[0].clone();
    for next in &resolved[1..] {
        acc = acc.overlay(next, rule, OverlayFillRule::NonZero);
    }

    let mut path = BezPath::new();
    for shape in &acc {
        for contour in shape {
            let mut pts = contour.iter();
            let Some(first) = pts.next() else { continue };
            path.move_to((first[0], first[1]));
            for p in pts {
                path.line_to((p[0], p[1]));
            }
            path.close_path();
        }
    }
    if path.elements().is_empty() {
        return Some(String::new()); // legitimately empty result
    }
    serialize_compact(&path, tolerance).or(Some(String::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curved_operands_flatten_into_the_algebra() {
        // quadratic and cubic segments in operand paths (the flatten arms)
        let ops = [
            BoolOperand {
                paths: vec!["M0,0 Q20,30 40,0 Z"],
                even_odd: false,
            },
            BoolOperand {
                paths: vec!["M10,0 C15,-15 25,-15 30,0 Z"],
                even_odd: false,
            },
        ];
        let d = boolean_svg_paths(&ops, BoolOp::Union, 0.1).unwrap();
        assert!(!d.is_empty());
        let area = net_area(&d);
        assert!(
            area.abs() > 100.0,
            "curved union should have real area: {area}"
        );
    }

    fn operand(paths: Vec<&str>) -> BoolOperand<'_> {
        BoolOperand {
            paths,
            even_odd: false,
        }
    }

    /// Signed (shoelace) area of the polygons in a compact `d` string, holes
    /// cancelling — the net filled area under nonzero winding.
    fn net_area(d: &str) -> f64 {
        let path = BezPath::from_svg(d).unwrap();
        let mut total = 0.0;
        let mut contour: Vec<(f64, f64)> = Vec::new();
        let mut flush = |contour: &mut Vec<(f64, f64)>| {
            let mut a = 0.0;
            for i in 0..contour.len() {
                let (x0, y0) = contour[i];
                let (x1, y1) = contour[(i + 1) % contour.len()];
                a += x0 * y1 - x1 * y0;
            }
            total += a / 2.0;
            contour.clear();
        };
        for el in path.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    if !contour.is_empty() {
                        flush(&mut contour);
                    }
                    contour.push((p.x, p.y));
                }
                PathEl::LineTo(p) => contour.push((p.x, p.y)),
                PathEl::ClosePath => flush(&mut contour),
                _ => {}
            }
        }
        if !contour.is_empty() {
            flush(&mut contour);
        }
        total.abs()
    }

    const A: &str = "M0,0 L10,0 L10,10 L0,10 Z"; // 10×10 at origin
    const B: &str = "M5,0 L15,0 L15,10 L5,10 Z"; // 10×10 shifted +5 (overlap 50)
    const FAR: &str = "M100,0 L110,0 L110,10 L100,10 Z";

    #[test]
    fn ops_produce_the_right_areas() {
        let a = || operand(vec![A]);
        let b = || operand(vec![B]);
        let cases = [
            (BoolOp::Union, 150.0),
            (BoolOp::Intersect, 50.0),
            (BoolOp::Subtract, 50.0),
            (BoolOp::Exclude, 100.0),
        ];
        for (op, want) in cases {
            let d = boolean_svg_paths(&[a(), b()], op, 0.25).unwrap();
            assert!(
                (net_area(&d) - want).abs() < 0.5,
                "{op:?}: area {} want {want}: {d}",
                net_area(&d)
            );
        }
    }

    #[test]
    fn disjoint_intersect_is_legitimately_empty() {
        let d = boolean_svg_paths(
            &[operand(vec![A]), operand(vec![FAR])],
            BoolOp::Intersect,
            0.25,
        )
        .unwrap();
        assert!(d.is_empty());
    }

    #[test]
    fn subtract_takes_the_first_child_as_subject() {
        // A − B leaves A's left half: bbox x ∈ [0, 5]
        let d = boolean_svg_paths(
            &[operand(vec![A]), operand(vec![B])],
            BoolOp::Subtract,
            0.25,
        )
        .unwrap();
        use kurbo::Shape;
        let bb = BezPath::from_svg(&d).unwrap().bounding_box();
        assert!(bb.x0.abs() < 1e-6 && (bb.x1 - 5.0).abs() < 1e-6, "{d}");
    }

    #[test]
    fn multi_operand_folds() {
        // union of three: 10×10 + two disjoint neighbours → 300
        let d = boolean_svg_paths(
            &[
                operand(vec![A]),
                operand(vec![FAR]),
                operand(vec!["M200,0 L210,0 L210,10 L200,10 Z"]),
            ],
            BoolOp::Union,
            0.25,
        )
        .unwrap();
        assert!((net_area(&d) - 300.0).abs() < 0.5, "{d}");
        // subtract two clips from a subject: 100 − 25 − 25 (disjoint bites)
        let d = boolean_svg_paths(
            &[
                operand(vec![A]),
                operand(vec!["M0,0 L5,0 L5,5 L0,5 Z"]),
                operand(vec!["M5,5 L10,5 L10,10 L5,10 Z"]),
            ],
            BoolOp::Subtract,
            0.25,
        )
        .unwrap();
        assert!((net_area(&d) - 50.0).abs() < 0.5, "{d}");
    }

    #[test]
    fn holes_wind_opposite_so_nonzero_renders_them() {
        // frame = outer exclude inner → net area 100 − 36 = 64, two contours
        let inner = "M2,2 L8,2 L8,8 L2,8 Z";
        let d = boolean_svg_paths(
            &[operand(vec![A]), operand(vec![inner])],
            BoolOp::Exclude,
            0.25,
        )
        .unwrap();
        assert!((net_area(&d) - 64.0).abs() < 0.5, "{d}");
        assert!(d.matches('M').count() + d.matches('m').count() == 2, "{d}");
    }

    #[test]
    fn operand_fill_rule_resolves_its_region() {
        // one operand containing nested same-winding squares: nonzero fills the
        // whole outer (area 100); evenodd leaves a hole (area 64)
        let nested = vec![A, "M2,2 L8,2 L8,8 L2,8 Z"];
        let nz = boolean_svg_paths(
            &[BoolOperand {
                paths: nested.clone(),
                even_odd: false,
            }],
            BoolOp::Union,
            0.25,
        )
        .unwrap();
        let eo = boolean_svg_paths(
            &[BoolOperand {
                paths: nested,
                even_odd: true,
            }],
            BoolOp::Union,
            0.25,
        )
        .unwrap();
        assert!((net_area(&nz) - 100.0).abs() < 0.5, "{nz}");
        assert!((net_area(&eo) - 64.0).abs() < 0.5, "{eo}");
    }

    #[test]
    fn curves_flatten_at_the_tolerance() {
        // two overlapping circles (as cubic approximations) union into one blob
        let circle = |cx: f64| {
            format!(
                "M{},50 C{},77.6 {},100 {},100 C{},100 {},77.6 {},50 C{},22.4 {},0 {},0 C{},0 {},22.4 {},50 Z",
                cx - 50.0, cx - 50.0, cx - 27.6, cx, cx + 27.6, cx + 50.0, cx + 50.0,
                cx + 50.0, cx + 27.6, cx, cx - 27.6, cx - 50.0, cx - 50.0
            )
        };
        let (c1, c2) = (circle(50.0), circle(90.0));
        let d = boolean_svg_paths(
            &[operand(vec![&c1]), operand(vec![&c2])],
            BoolOp::Union,
            0.25,
        )
        .unwrap();
        let one = net_area(&boolean_svg_paths(&[operand(vec![&c1])], BoolOp::Union, 0.25).unwrap());
        let both = net_area(&d);
        assert!(
            both > one * 1.2 && both < one * 2.0,
            "union {both} vs one {one}"
        );
    }

    #[test]
    fn degenerate_input_never_panics() {
        // garbage paths contribute nothing; all-garbage → None (caller marks)
        assert!(boolean_svg_paths(
            &[operand(vec!["nonsense"]), operand(vec![""])],
            BoolOp::Union,
            0.25
        )
        .is_none());
        let d = boolean_svg_paths(
            &[operand(vec![A, "garbage"]), operand(vec!["M5,5 L5,5 Z"])],
            BoolOp::Union,
            0.25,
        )
        .unwrap();
        assert!((net_area(&d) - 100.0).abs() < 0.5, "{d}");
        assert!(!d.contains("NaN"), "{d}");
        assert!(BoolOp::parse("divide").is_none());
    }
}
