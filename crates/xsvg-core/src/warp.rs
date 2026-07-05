//! The geometry-transform pipeline (§7): pure deformation **fields** and the **bake**
//! that lowers them — flatten → map → emit (§7.1; cubic refit is a later quality
//! upgrade). Platform-free and kurbo-backed: the wasm layer hands SVG path data in
//! and gets warped path data out, so the math is natively unit-testable.

use kurbo::{BezPath, PathEl, Point, Rect, Shape};

/// A deformation field: a pure point map `D : ℝ² → ℝ²` (§7.2). Implementations may
/// precompute state (an envelope frame, an arc-length table) but expose only
/// per-point evaluation — the bake is field-agnostic.
pub trait Field {
    fn map(&self, p: Point) -> Point;
}

/// Which axis a warp preset bends along (Illustrator's Horizontal/Vertical radio):
/// `H` sweeps the profile across x and displaces y; `V` the transpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WarpAxis {
    #[default]
    H,
    V,
}

impl WarpAxis {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("v") {
            Self::V
        } else {
            Self::H
        }
    }
}

/// The displacement-family Envelope-Distort presets (§7.2, Transform.md §B): a 1-D
/// profile swept across the bend axis of a normalized envelope frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    /// both edges ride one parabola: `Δ = A·(1 − u²)`
    Arch,
    /// one full sine, uniform through the height: `Δ = A·sin(π·u)`
    Flag,
    /// linear ramp (pure shear profile): `Δ = A·u`
    Rise,
    /// Flag whose phase advances by π/2 from the leading to the trailing edge:
    /// `Δ = A·sin(π·u − (π/4)·(v + 1))`
    Wave,
}

/// An envelope-preset field over a source bounding box. The box normalizes points to
/// `(u, v) ∈ [−1, 1]²` (`u` along the bend axis); the amplitude is `A = bend · L/4`
/// with `L` the bend-axis extent, so a preset scales with the art it warps. Positive
/// bend displaces **up** (−y) for `axis="h"` and **right** (+x) for `axis="v"`.
/// `bend` is clamped to `[−1, 1]` (authored as −100…100 %); non-finite collapses to
/// 0 (the identity), so a degenerate box or bend can never emit NaN.
#[derive(Clone, Copy, Debug)]
pub struct EnvelopePreset {
    profile: Profile,
    bend: f64,
    axis: WarpAxis,
    bbox: Rect,
}

impl EnvelopePreset {
    /// Build a preset field by name (`arch` | `flag` | `rise` | `wave`) over the
    /// pre-warp union bbox of the geometry it will map. `None` for unknown names.
    pub fn new(name: &str, bend: f64, axis: WarpAxis, bbox: Rect) -> Option<Self> {
        let profile = match name {
            "arch" => Profile::Arch,
            "flag" => Profile::Flag,
            "rise" => Profile::Rise,
            "wave" => Profile::Wave,
            _ => return None,
        };
        let bend = if bend.is_finite() {
            bend.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        Some(Self {
            profile,
            bend,
            axis,
            bbox,
        })
    }
}

impl Field for EnvelopePreset {
    fn map(&self, p: Point) -> Point {
        let (cx, cy) = (self.bbox.center().x, self.bbox.center().y);
        let (hw, hh) = (self.bbox.width() / 2.0, self.bbox.height() / 2.0);
        // normalized coords along the bend axis (u) and across it (v); a collapsed
        // extent normalizes to 0 rather than dividing by it
        let norm = |d: f64, half: f64| if half > 0.0 { d / half } else { 0.0 };
        let (u, v, half_l) = match self.axis {
            WarpAxis::H => (norm(p.x - cx, hw), norm(p.y - cy, hh), hw),
            WarpAxis::V => (norm(p.y - cy, hh), norm(p.x - cx, hw), hh),
        };
        let a = self.bend * half_l / 2.0; // A = bend · L/4
        let d = match self.profile {
            Profile::Arch => a * (1.0 - u * u),
            Profile::Flag => a * (std::f64::consts::PI * u).sin(),
            Profile::Rise => a * u,
            Profile::Wave => {
                a * (std::f64::consts::PI * u - std::f64::consts::FRAC_PI_4 * (v + 1.0)).sin()
            }
        };
        match self.axis {
            WarpAxis::H => Point::new(p.x, p.y - d),
            WarpAxis::V => Point::new(p.x + d, p.y),
        }
    }
}

/// How many times a mapped segment may halve during adaptive subdivision (2^10 ≈
/// 1000 pieces per input segment) — a hard cap so a pathological field terminates.
const MAX_SPLIT_DEPTH: u8 = 10;

/// The bake (§7.1): **flatten** curves to chords within `tolerance` (kurbo), **map**
/// every vertex through `field`, and subdivide each chord adaptively until the
/// mapped midpoint deviates from the mapped chord by at most `tolerance` — so long
/// straight edges curve smoothly under a nonlinear field instead of staying chords.
/// Output is a polyline `BezPath` (`M`/`L`/`Z` only); cubic refit is a later quality
/// tier. Closing edges are subdivided like any other segment before the `Z`.
pub fn bake(path: &BezPath, field: &dyn Field, tolerance: f64) -> BezPath {
    let tol = tolerance.max(1e-3);
    let mut out = BezPath::new();
    let mut cur = Point::ZERO;
    let mut start = Point::ZERO;
    kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            out.move_to(field.map(p));
            cur = p;
            start = p;
        }
        PathEl::LineTo(p) => {
            subdivide(&mut out, field, cur, p, field.map(cur), field.map(p), tol);
            cur = p;
        }
        PathEl::ClosePath => {
            if cur != start {
                subdivide(
                    &mut out,
                    field,
                    cur,
                    start,
                    field.map(cur),
                    field.map(start),
                    tol,
                );
            }
            out.close_path();
            cur = start;
        }
        // flatten only emits Move/Line/Close
        _ => {}
    });
    out
}

fn subdivide(
    out: &mut BezPath,
    field: &dyn Field,
    a: Point,
    b: Point,
    fa: Point,
    fb: Point,
    tol: f64,
) {
    fn rec(
        out: &mut BezPath,
        field: &dyn Field,
        a: Point,
        b: Point,
        fa: Point,
        fb: Point,
        tol: f64,
        depth: u8,
    ) {
        if depth > 0 {
            let m = a.midpoint(b);
            let fm = field.map(m);
            // probe the quarter points as well as the midpoint: an antisymmetric
            // profile (flag's full sine) passes exactly through the chord at the
            // midpoint, so a lone midpoint test would never split it
            let err = fm
                .distance(fa.midpoint(fb))
                .max(field.map(a.lerp(b, 0.25)).distance(fa.lerp(fb, 0.25)))
                .max(field.map(a.lerp(b, 0.75)).distance(fa.lerp(fb, 0.75)));
            if err > tol {
                rec(out, field, a, m, fa, fm, tol, depth - 1);
                rec(out, field, m, b, fm, fb, tol, depth - 1);
                return;
            }
        }
        out.line_to(fb);
    }
    rec(out, field, a, b, fa, fb, tol, MAX_SPLIT_DEPTH);
}

/// Tight bounding box of an SVG path `d` string, or `None` if it doesn't parse or
/// is empty. Used to build the pre-warp envelope frame.
pub fn svg_path_bbox(d: &str) -> Option<Rect> {
    let path = BezPath::from_svg(d).ok()?;
    (!path.elements().is_empty()).then(|| path.bounding_box())
}

/// Bake an SVG path `d` string through `field` and re-serialize it (2-decimal
/// coordinates). `None` if the input doesn't parse, produces nothing, or the field
/// leaks a non-finite coordinate — the caller keeps the original geometry (§4
/// totality: a warp degrades, it never emits NaN).
pub fn warp_svg_path(d: &str, field: &dyn Field, tolerance: f64) -> Option<String> {
    let path = BezPath::from_svg(d).ok()?;
    let baked = bake(&path, field, tolerance);
    let mut s = String::new();
    for el in baked.elements() {
        match el {
            PathEl::MoveTo(p) => push_point(&mut s, 'M', *p)?,
            PathEl::LineTo(p) => push_point(&mut s, 'L', *p)?,
            PathEl::ClosePath => s.push('Z'),
            _ => return None,
        }
    }
    (!s.is_empty()).then_some(s)
}

fn push_point(s: &mut String, cmd: char, p: Point) -> Option<()> {
    if !(p.x.is_finite() && p.y.is_finite()) {
        return None;
    }
    s.push(cmd);
    s.push_str(&round2(p.x));
    s.push(',');
    s.push_str(&round2(p.y));
    Some(())
}

/// Format with at most 2 decimals, trimming trailing zeros (`1.50` → `1.5`,
/// `2.00` → `2`).
fn round2(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path() -> BezPath {
        BezPath::from_svg("M0,0 L100,0 L100,40 L0,40 Z").unwrap()
    }

    fn arch(bend: f64, bbox: Rect) -> EnvelopePreset {
        EnvelopePreset::new("arch", bend, WarpAxis::H, bbox).unwrap()
    }

    fn vertices(p: &BezPath) -> Vec<Point> {
        p.elements()
            .iter()
            .filter_map(|el| match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn zero_bend_is_identity() {
        let path = rect_path();
        let baked = bake(&path, &arch(0.0, path.bounding_box()), 0.25);
        // identity map, but the implicit closing edge becomes an explicit LineTo
        // back to the subpath start (so a field could subdivide it)
        let mut expected = vertices(&path);
        expected.push(expected[0]);
        assert_eq!(vertices(&baked), expected);
        assert_eq!(baked.elements().last(), Some(&PathEl::ClosePath));
    }

    #[test]
    fn arch_lifts_the_center_and_pins_the_ends() {
        // a baseline: bbox 100 wide → A = bend·L/4 = 0.8·25 = 20 up at the center
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(0.8, Rect::new(0.0, 0.0, 100.0, 0.0));
        let baked = bake(&line, &f, 0.05);
        let vs = vertices(&baked);
        assert!((vs.first().unwrap().y - 0.0).abs() < 1e-9);
        assert!((vs.last().unwrap().y - 0.0).abs() < 1e-9);
        let min_y = vs.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        assert!((min_y + 20.0).abs() < 0.1, "apex {min_y}, want ≈ −20");
        // the straight source line must have been subdivided to follow the parabola
        assert!(vs.len() > 8, "no adaptive subdivision: {} pts", vs.len());
    }

    #[test]
    fn flag_bows_a_straight_edge_despite_midpoint_symmetry() {
        // flag's sine is zero at both endpoints AND the midpoint of a full-width
        // edge — a midpoint-only error test never splits it (regression: the box
        // stayed a straight rectangle). A = 25: the edge must swing ≈ ±25.
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f =
            EnvelopePreset::new("flag", 1.0, WarpAxis::H, Rect::new(0.0, 0.0, 100.0, 0.0)).unwrap();
        let vs = vertices(&bake(&line, &f, 0.25));
        assert!(vs.len() > 8, "sine edge not subdivided: {} pts", vs.len());
        let min_y = vs.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_y = vs.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        assert!((min_y + 25.0).abs() < 0.5, "trough {min_y}, want ≈ −25");
        assert!((max_y - 25.0).abs() < 0.5, "crest {max_y}, want ≈ +25");
    }

    #[test]
    fn tighter_tolerance_means_more_segments() {
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(1.0, Rect::new(0.0, 0.0, 100.0, 0.0));
        let coarse = vertices(&bake(&line, &f, 1.0)).len();
        let fine = vertices(&bake(&line, &f, 0.05)).len();
        assert!(fine > coarse, "fine {fine} !> coarse {coarse}");
    }

    #[test]
    fn subdivision_meets_the_tolerance() {
        // dense-sample the source line through the field; every mapped sample must
        // sit within ~tol of the baked polyline
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(1.0, Rect::new(0.0, 0.0, 100.0, 0.0));
        let tol = 0.25;
        let vs = vertices(&bake(&line, &f, tol));
        for i in 0..=1000 {
            let p = f.map(Point::new(i as f64 * 0.1, 0.0));
            let dist = vs
                .windows(2)
                .map(|w| dist_to_segment(p, w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(
                dist <= tol * 1.5,
                "sample {p:?} is {dist} from the polyline"
            );
        }
    }

    fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
        let ab = b - a;
        let len2 = ab.hypot2();
        let t = if len2 > 0.0 {
            ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(a + ab * t)
    }

    #[test]
    fn rise_on_axis_v_displaces_x() {
        let line = BezPath::from_svg("M0,0 L0,100").unwrap();
        let f =
            EnvelopePreset::new("rise", 1.0, WarpAxis::V, Rect::new(0.0, 0.0, 0.0, 100.0)).unwrap();
        let baked = bake(&line, &f, 0.25);
        let vs = vertices(&baked);
        // A = 25; rise is linear in u: x goes −25 → +25 while y is untouched
        assert!((vs.first().unwrap().x + 25.0).abs() < 1e-9);
        assert!((vs.last().unwrap().x - 25.0).abs() < 1e-9);
        assert!(vs.iter().all(|p| p.y.is_finite()));
    }

    #[test]
    fn wave_curves_the_closing_edge() {
        // wave depends on v, so the rect's (straight, implicitly closed) left edge
        // must be subdivided — more vertices than the 4 corners, still closed
        let path = rect_path();
        let f = EnvelopePreset::new("wave", 1.0, WarpAxis::H, path.bounding_box()).unwrap();
        let baked = bake(&path, &f, 0.05);
        assert!(vertices(&baked).len() > 8);
        assert_eq!(baked.elements().last(), Some(&PathEl::ClosePath));
    }

    #[test]
    fn degenerate_bbox_and_bend_never_leak_nan() {
        let path = rect_path();
        for bbox in [Rect::ZERO, Rect::new(5.0, 5.0, 5.0, 5.0)] {
            for bend in [0.0, 1.0, -1.0, f64::NAN, f64::INFINITY] {
                let f = EnvelopePreset::new("flag", bend, WarpAxis::H, bbox).unwrap();
                let baked = bake(&path, &f, 0.25);
                assert!(vertices(&baked)
                    .iter()
                    .all(|p| p.x.is_finite() && p.y.is_finite()));
            }
        }
    }

    #[test]
    fn bend_is_clamped() {
        // 500% behaves exactly like 100%
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let bbox = Rect::new(0.0, 0.0, 100.0, 0.0);
        let big = bake(&line, &arch(5.0, bbox), 0.05);
        let one = bake(&line, &arch(1.0, bbox), 0.05);
        assert_eq!(vertices(&big), vertices(&one));
    }

    #[test]
    fn unknown_preset_is_none() {
        assert!(EnvelopePreset::new("twirl", 0.5, WarpAxis::H, Rect::ZERO).is_none());
    }

    #[test]
    fn warp_svg_path_round_trips_and_rejects_garbage() {
        let f = arch(0.5, Rect::new(0.0, 0.0, 100.0, 40.0));
        let d = warp_svg_path("M0,0 L100,0 L100,40 L0,40 Z", &f, 0.25).unwrap();
        assert!(d.starts_with('M') && d.ends_with('Z'), "{d}");
        assert!(!d.contains("NaN") && !d.contains("inf"), "{d}");
        assert!(warp_svg_path("not a path", &f, 0.25).is_none());
        assert!(warp_svg_path("", &f, 0.25).is_none());
    }

    #[test]
    fn svg_path_bbox_parses_and_rejects() {
        let b = svg_path_bbox("M10,20 L110,20 L110,60 Z").unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 20.0, 110.0, 60.0));
        assert!(svg_path_bbox("").is_none());
        assert!(svg_path_bbox("garbage").is_none());
    }
}
