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
use kurbo::{BezPath, PathEl, Rect};

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
            PathEl::LineTo(p) if p.x.is_finite() && p.y.is_finite() => cur.push([p.x, p.y]),
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

/// The curve-preserving algebra behind convert-to-path (Incremental.md §11;
/// the §7.4 seam): the same operands and op as [`boolean_svg_paths`], run on
/// the curves themselves through [`linesweeper`] instead of on flattened
/// polygons. Every segment the operation does not cut comes back exactly as
/// authored (a cubic's handles to the bit); a cut segment comes back as exact
/// sub-curves split at the intersection. Arcs enter as their κ cubics
/// ([`crate::spline::arcs_to_cubics`]); quadratics come back as cubics. The
/// result winds outer contours and holes oppositely; zero-area slivers (an
/// exact tangency's residue) are dropped.
///
/// `Ok(None)` is no usable geometry, `Ok(Some(""))` a legitimately empty
/// result, `Err` the engine declining the input — the caller falls back to the
/// flattened algebra, so a conversion never fails outright.
pub fn boolean_svg_paths_curved(
    operands: &[BoolOperand],
    op: BoolOp,
) -> Result<Option<String>, String> {
    let paths: Vec<(BezPath, bool)> = operands
        .iter()
        .filter_map(|o| curved_operand(&o.paths).map(|p| (p, o.even_odd)))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let inputs: Vec<kurbo::PathSeg> = paths.iter().flat_map(|(p, _)| p.segments()).collect();
    let result = sweep_normalized(&paths, op)?;
    let snapped = prune_thin(&snap_to_inputs(&result, &inputs));
    Ok(Some(serialize_curves(&snapped)))
}

/// The sweep in a canonical frame: the operands' joint extent moved to the
/// origin and scaled to about a thousand units, the result mapped back. The
/// engine's accuracy is absolute (its epsilon floors at 1e-6), so a tiny
/// drawing would be swept coarsely and a far-off one at the mercy of its
/// magnitude; in this frame every drawing is swept alike.
fn sweep_normalized(paths: &[(BezPath, bool)], op: BoolOp) -> Result<BezPath, String> {
    use kurbo::{Affine, Shape};
    let mut bb: Option<Rect> = None;
    for (p, _) in paths {
        let b = p.bounding_box();
        bb = Some(bb.map_or(b, |a| a.union(b)));
    }
    let bb = bb.unwrap_or(Rect::ZERO);
    let span = bb.width().max(bb.height());
    if !span.is_finite() || span <= 0.0 {
        return sweep(paths, op);
    }
    let to_frame = Affine::scale(1024.0 / span) * Affine::translate((-bb.x0, -bb.y0));
    let framed: Vec<(BezPath, bool)> = paths
        .iter()
        .map(|(p, eo)| (to_frame * p.clone(), *eo))
        .collect();
    let result = sweep(&framed, op)?;
    Ok(to_frame.inverse() * result)
}

/// The curve-preserving result when it agrees with the flattened one, else the
/// flattened one: convert-to-path's answer. The two algebras are compared on a
/// jittered grid of samples over the result's extent; a disagreement beyond
/// what the flattening tolerance explains along the boundary means the curve
/// engine mis-resolved the topology (its known weakness is a boundary within
/// epsilon of another), and the conversion is then no worse than the render.
/// `None` is no usable geometry, `Some("")` a legitimately empty result.
pub fn boolean_svg_paths_checked(
    operands: &[BoolOperand],
    op: BoolOp,
    tolerance: f64,
) -> Option<String> {
    let flat = boolean_svg_paths(operands, op, tolerance);
    // an engine panic is a declined input too, on hosts that unwind (a wasm
    // host aborts on any panic; the fuzz keeps the rate down there)
    let curved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        boolean_svg_paths_curved(operands, op)
    }));
    let Ok(Ok(Some(curved))) = curved else {
        return flat;
    };
    match &flat {
        Some(f) if agree(&curved, f) => Some(curved),
        _ => flat,
    }
}

/// Whether two path-data results draw the same region, to a boundary band:
/// the inside sets sampled at 48×48 jittered points of their joint extent
/// disagree at fewer than 1.5% of them (and fewer than 8).
fn agree(a: &str, b: &str) -> bool {
    use kurbo::Shape;
    let (Ok(pa), Ok(pb)) = (BezPath::from_svg(a), BezPath::from_svg(b)) else {
        return false;
    };
    if pa.elements().is_empty() != pb.elements().is_empty() {
        return false;
    }
    if pa.elements().is_empty() {
        return true;
    }
    let bb = pa.bounding_box().union(pb.bounding_box());
    if !(bb.width().is_finite() && bb.height().is_finite()) {
        return false;
    }
    let n = 48;
    let mut differ = 0;
    for j in 0..n {
        for i in 0..n {
            // a fixed jitter keeps a grid off any axis-aligned or 45° edge
            let (jx, jy) = (
                ((i * 7 + j * 3) % 11) as f64 / 11.0 * 0.6 + 0.2,
                ((i * 5 + j * 11) % 13) as f64 / 13.0 * 0.6 + 0.2,
            );
            let p = kurbo::Point::new(
                bb.x0 + (i as f64 + jx) * bb.width() / n as f64,
                bb.y0 + (j as f64 + jy) * bb.height() / n as f64,
            );
            if (pa.winding(p) != 0) != (pb.winding(p) != 0) {
                differ += 1;
            }
        }
    }
    differ < 8 || (differ as f64) < 0.015 * (n * n) as f64
}

/// The stages of [`boolean_svg_paths_curved`] — the engine's oriented
/// contours, after the snap-to-input pass, after pruning — as path data, for
/// the fuzz harness to see which stage a disagreement enters at.
#[doc(hidden)]
pub fn boolean_svg_paths_curved_stages(
    operands: &[BoolOperand],
    op: BoolOp,
) -> Result<Option<[String; 3]>, String> {
    let paths: Vec<(BezPath, bool)> = operands
        .iter()
        .filter_map(|o| curved_operand(&o.paths).map(|p| (p, o.even_odd)))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    let inputs: Vec<kurbo::PathSeg> = paths.iter().flat_map(|(p, _)| p.segments()).collect();
    let raw = sweep_normalized(&paths, op)?;
    let snapped = snap_to_inputs(&raw, &inputs);
    let pruned = prune_thin(&snapped);
    Ok(Some([
        serialize_curves(&raw),
        serialize_curves(&snapped),
        serialize_curves(&pruned),
    ]))
}

/// [`boolean_svg_paths_curved`] without the snap-to-input and pruning passes:
/// the engine's own contours, oriented. A diagnostic for the fuzz harness, to
/// tell an engine disagreement from one the passes introduce.
#[doc(hidden)]
pub fn boolean_svg_paths_curved_raw(
    operands: &[BoolOperand],
    op: BoolOp,
) -> Result<Option<String>, String> {
    let paths: Vec<(BezPath, bool)> = operands
        .iter()
        .filter_map(|o| curved_operand(&o.paths).map(|p| (p, o.even_odd)))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    Ok(Some(serialize_curves(&sweep_normalized(&paths, op)?)))
}

/// A contour's length. A degenerate quadratic (its control point on the
/// chord) can make kurbo's closed-form arc length non-finite; such a segment
/// counts its control polygon instead, which is never shorter than the curve.
fn perimeter(path: &BezPath) -> f64 {
    path.segments()
        .map(|s| {
            let l = kurbo::ParamCurveArclen::arclen(&s, 1e-6);
            if l.is_finite() {
                l
            } else {
                match s {
                    kurbo::PathSeg::Line(l) => (l.p1 - l.p0).hypot(),
                    kurbo::PathSeg::Quad(q) => (q.p1 - q.p0).hypot() + (q.p2 - q.p1).hypot(),
                    kurbo::PathSeg::Cubic(c) => {
                        (c.p1 - c.p0).hypot() + (c.p2 - c.p1).hypot() + (c.p3 - c.p2).hypot()
                    }
                }
            }
        })
        .sum()
}

/// Drop contours thinner on average than the output precision resolves (an
/// exact out-and-back along one curve has no area at all): the residue of
/// near-coincident edges, and what snapping can leave of it.
fn prune_thin(path: &BezPath) -> BezPath {
    use kurbo::Shape;
    let mut out = BezPath::new();
    let mut sub = BezPath::new();
    let keep = |sub: &mut BezPath, out: &mut BezPath| {
        if sub.elements().is_empty() {
            return;
        }
        let area = sub.area().abs();
        let extent = sub.bounding_box();
        let visible = extent.width().max(extent.height()) >= 5e-3;
        if visible && area.is_finite() && area >= 1e-9 && area >= 2e-3 * perimeter(sub) {
            out.extend(sub.elements().iter().copied());
        }
        *sub = BezPath::new();
    };
    for el in path.elements() {
        if let PathEl::MoveTo(_) = el {
            keep(&mut sub, &mut out);
        }
        sub.push(*el);
    }
    keep(&mut sub, &mut out);
    out
}

/// How many operands one sweep tracks winding numbers for; more fold in by
/// sweeping again with the result as the first operand.
const SWEEP_OPERANDS: usize = 16;

/// Per-operand winding numbers for the n-ary sweep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Windings([i32; SWEEP_OPERANDS]);

impl std::ops::Add for Windings {
    type Output = Self;
    fn add(mut self, rhs: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(rhs.0) {
            *a += b;
        }
        self
    }
}

impl std::ops::AddAssign for Windings {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl linesweeper::topology::WindingNumber for Windings {
    type Tag = u8;
    fn single(tag: u8, positive: bool) -> Self {
        let mut w = Self::default();
        w.0[tag as usize] = if positive { 1 } else { -1 };
        w
    }
    fn of_tag(&self, tag: u8) -> Self {
        let mut w = Self::default();
        w.0[tag as usize] = self.0[tag as usize];
        w
    }
}

/// One sweep over every operand at once, each read under its own fill rule
/// and the op decided on the per-operand windings — the raw inputs go in, so
/// nothing the engine wrote is fed back to it (re-feeding its output, whose
/// pieces lie within epsilon of the inputs, is where coincident-boundary
/// decisions go wrong).
fn sweep(paths: &[(BezPath, bool)], op: BoolOp) -> Result<BezPath, String> {
    use kurbo::Shape;
    use linesweeper::topology::Topology;
    let (head, rest) = paths.split_at(paths.len().min(SWEEP_OPERANDS));
    let mut bb: Option<Rect> = None;
    for (p, _) in head {
        let b = p.bounding_box();
        bb = Some(bb.map_or(b, |a| a.union(b)));
    }
    let bb = bb.unwrap_or(Rect::ZERO);
    // the engine's own accuracy choice for a binary op, over our extent
    let m2 = bb
        .min_x()
        .min(bb.min_y())
        .abs()
        .max(bb.max_x().max(bb.max_y()).abs());
    let eps = (m2 * (f64::EPSILON * 64.0)).max(1e-6);
    let top = Topology::<Windings>::from_paths(
        head.iter().enumerate().map(|(i, (p, _))| (p, i as u8)),
        eps,
    )
    .map_err(|e| format!("{e:?}"))?;
    let n = head.len();
    let rules: Vec<bool> = head.iter().map(|(_, eo)| *eo).collect();
    let inside = |w: &Windings| {
        let ins = |i: usize| {
            if rules[i] {
                w.0[i] % 2 != 0
            } else {
                w.0[i] != 0
            }
        };
        match op {
            BoolOp::Union => (0..n).any(ins),
            BoolOp::Intersect => (0..n).all(ins),
            BoolOp::Subtract => ins(0) && !(1..n).any(ins),
            BoolOp::Exclude => (0..n).filter(|&i| ins(i)).count() % 2 == 1,
        }
    };
    let result = contours_to_path(top.contours(inside));
    if rest.is_empty() {
        return Ok(result);
    }
    // more operands than one sweep tracks: the result leads the next sweep
    let mut next: Vec<(BezPath, bool)> = vec![(result, false)];
    next.extend(rest.iter().cloned());
    sweep(&next, op)
}

/// Where the result runs along an input segment, write that segment: the
/// engine returns such stretches as pieces (split at sweep extrema, with
/// epsilon-sized slivers around them and handles moved by its tolerance); this
/// pass finds each run of result pieces lying on one input segment and
/// replaces it with the exact sub-curve of the input between the run's ends —
/// the input itself, to the bit, when the run covers it whole. Pieces on no
/// input (none are expected) pass through.
fn snap_to_inputs(path: &BezPath, inputs: &[kurbo::PathSeg]) -> BezPath {
    use kurbo::{ParamCurve, ParamCurveNearest, PathSeg, Shape};
    if inputs.is_empty() {
        return path.clone();
    }
    let mut bb: Option<Rect> = None;
    for seg in inputs {
        let b = seg.bounding_box();
        bb = Some(bb.map_or(b, |a| a.union(b)));
    }
    let span = bb.map_or(1.0, |b| b.width().max(b.height())).max(1.0);
    let eps = 1e-6 * span;
    let boxes: Vec<Rect> = inputs
        .iter()
        .map(|s| s.bounding_box().inflate(eps, eps))
        .collect();
    // which input a result piece lies on, with the input's parameters at the
    // piece's ends
    let locate = |seg: &PathSeg| -> Option<(usize, f64, f64)> {
        let (a, m, z) = (seg.start(), seg.eval(0.5), seg.end());
        let sb = seg.bounding_box();
        let mut best: Option<(usize, f64, f64, f64)> = None;
        for (i, input) in inputs.iter().enumerate() {
            let ib = &boxes[i];
            if sb.x0 < ib.x0 || sb.x1 > ib.x1 || sb.y0 < ib.y0 || sb.y1 > ib.y1 {
                continue;
            }
            let na = input.nearest(a, 1e-9);
            let nm = input.nearest(m, 1e-9);
            let nz = input.nearest(z, 1e-9);
            let worst = na
                .distance_sq
                .max(nm.distance_sq)
                .max(nz.distance_sq)
                .sqrt();
            if worst < eps && best.is_none_or(|b| worst < b.3) {
                best = Some((i, na.t, nz.t, worst));
            }
        }
        best.map(|(i, t0, t1, _)| (i, t0, t1))
    };
    // runs of consecutive pieces on one input, parameters contiguous
    let runs_of = |located: &[Option<(usize, f64, f64)>]| -> Vec<(usize, usize)> {
        let n = located.len();
        let mut runs = Vec::new();
        let mut i = 0;
        while i < n {
            let mut j = i + 1;
            while j < n {
                let same = match (located[i], located[j - 1], located[j]) {
                    (Some((a, _, _)), Some((_, _, t_prev)), Some((b, t0, _))) => {
                        a == b && (t0 - t_prev).abs() < 1e-6
                    }
                    _ => false,
                };
                if !same {
                    break;
                }
                j += 1;
            }
            runs.push((i, j));
            i = j;
        }
        runs
    };
    let mut out = BezPath::new();
    let mut sub = BezPath::new();
    let flush = |sub: &mut BezPath, out: &mut BezPath| {
        if sub.elements().is_empty() {
            return;
        }
        let mut segs: Vec<PathSeg> = sub.segments().collect();
        let mut located: Vec<Option<(usize, f64, f64)>> = segs.iter().map(locate).collect();
        let n = segs.len();
        // a run may wrap across the contour's start: rotate so it does not
        let runs = runs_of(&located);
        if runs.len() > 1 {
            let (f0, f1) = runs[0];
            let (l0, l1) = runs[runs.len() - 1];
            if let (Some((a, t0, _)), Some((b, _, t1))) = (located[f0], located[l1 - 1]) {
                if a == b && (t0 - t1).abs() < 1e-6 && f1 - f0 + (l1 - l0) < n {
                    segs.rotate_left(l0);
                    located.rotate_left(l0);
                }
            }
        }
        let mut pieces: Vec<PathSeg> = Vec::new();
        for (i, j) in runs_of(&located) {
            match located[i] {
                Some((k, t0, _)) => {
                    let t1 = located[j - 1].map_or(t0, |(_, _, t)| t);
                    let input = inputs[k];
                    let whole = |t: f64, at: f64| (t - at).abs() < 1e-6;
                    let piece = if t1 >= t0 {
                        if whole(t0, 0.0) && whole(t1, 1.0) {
                            input
                        } else {
                            input.subsegment(t0.max(0.0)..t1.min(1.0))
                        }
                    } else if whole(t0, 1.0) && whole(t1, 0.0) {
                        input.reverse()
                    } else {
                        input.subsegment(t1.max(0.0)..t0.min(1.0)).reverse()
                    };
                    // the sub-curve must retrace the engine's pieces: a cubic
                    // that crosses itself passes one point at two parameters,
                    // and `nearest` may have named the other branch
                    let retraces = [0.2, 0.4, 0.6, 0.8].iter().all(|&t| {
                        let q = piece.eval(t);
                        segs[i..j]
                            .iter()
                            .any(|s| s.nearest(q, 1e-9).distance_sq.sqrt() < eps * 4.0)
                    });
                    if retraces {
                        pieces.push(piece);
                    } else {
                        pieces.extend(&segs[i..j]);
                    }
                }
                None => pieces.extend(&segs[i..j]),
            }
        }
        if let Some(first) = pieces.first() {
            out.move_to(first.start());
            for p in &pieces {
                out.push(p.as_path_el());
            }
            out.close_path();
        }
        *sub = BezPath::new();
    };
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                flush(&mut sub, &mut out);
                sub.push(*el);
            }
            PathEl::ClosePath => {
                sub.push(*el);
                flush(&mut sub, &mut out);
            }
            _ => sub.push(*el),
        }
    }
    flush(&mut sub, &mut out);
    out
}

/// One operand's path data as a single closed path: arcs as cubics, every
/// subpath closed (fill semantics), nothing non-finite. `None` when nothing
/// parses.
fn curved_operand(paths: &[&str]) -> Option<BezPath> {
    let mut out = BezPath::new();
    for d in paths {
        let Ok(p) = BezPath::from_svg(&crate::spline::arcs_to_cubics(d)) else {
            continue;
        };
        let mut open = false;
        for el in p.elements() {
            if !el_finite(el) {
                continue;
            }
            match el {
                PathEl::MoveTo(_) => {
                    if open {
                        out.close_path();
                    }
                    open = true;
                }
                PathEl::ClosePath => open = false,
                _ => {}
            }
            out.push(*el);
        }
        if open {
            out.close_path();
        }
    }
    (!out.elements().is_empty()).then_some(out)
}

fn el_finite(el: &PathEl) -> bool {
    let ok = |p: &kurbo::Point| p.x.is_finite() && p.y.is_finite();
    match el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => ok(p),
        PathEl::QuadTo(a, p) => ok(a) && ok(p),
        PathEl::CurveTo(a, b, p) => ok(a) && ok(b) && ok(p),
        PathEl::ClosePath => true,
    }
}

/// The engine's contours as one path: outer contours wound positive and holes
/// negative (so either fill rule renders them alike), zero-area slivers gone.
/// Outer or hole is decided by containment depth here, not by the engine's own
/// flag: two lobes of a bowtie, contours that touch at the crossing, come back
/// with one flagged wrong, and a hole wound like an outer fills.
fn contours_to_path(contours: linesweeper::topology::Contours) -> BezPath {
    use kurbo::Shape;
    let mut out = BezPath::new();
    let mut parts: Vec<BezPath> = Vec::new();
    for c in contours.contours() {
        let area = c.path.area();
        if !area.is_finite() || area.abs() < 1e-9 {
            continue;
        }
        // thinner on average than the output precision (0.001) resolves: the
        // residue of near-coincident edges, invisible and a node to delete
        if area.abs() < 5e-4 * perimeter(&c.path) {
            continue;
        }
        parts.push(dedup_points(&c.path));
    }
    for (i, p) in parts.iter().enumerate() {
        // a point just inside this contour, beside an edge no other contour
        // shares; it is in the result iff it lies inside an even number of the
        // others (an outer at even depth)
        let others = || {
            parts
                .iter()
                .enumerate()
                .filter(move |(j, _)| *j != i)
                .map(|(_, o)| o)
        };
        let outer = match inside_point(p, others()) {
            Some(q) => others().filter(|o| o.winding(q) != 0).count() % 2 == 0,
            None => true,
        };
        if (p.area() > 0.0) != outer {
            out.extend(p.reverse_subpaths().elements().iter().copied());
        } else {
            out.extend(p.elements().iter().copied());
        }
    }
    out
}

/// A point strictly inside a closed contour: a segment's midpoint nudged to
/// whichever side the contour's own winding says is in. The segment is one
/// no other contour runs along — beside a shared edge the inside of one
/// contour is the inside of its neighbour too, and the depth read there is
/// the neighbour's. `None` when no segment qualifies (a degenerate contour).
fn inside_point<'a>(
    contour: &BezPath,
    others: impl Iterator<Item = &'a BezPath> + Clone,
) -> Option<kurbo::Point> {
    use kurbo::{ParamCurve, ParamCurveDeriv, ParamCurveNearest, Shape};
    let span = contour.bounding_box();
    let eps = 1e-6 * span.width().max(span.height()).max(1.0);
    let shared = |p: kurbo::Point| {
        others.clone().any(|o| {
            o.segments()
                .any(|s| s.nearest(p, 1e-9).distance_sq.sqrt() < eps * 4.0)
        })
    };
    for seg in contour.segments() {
        let mid = seg.eval(0.5);
        if shared(mid) {
            continue;
        }
        let d = match seg {
            kurbo::PathSeg::Line(l) => l.p1 - l.p0,
            kurbo::PathSeg::Quad(q) => q.deriv().eval(0.5).to_vec2(),
            kurbo::PathSeg::Cubic(c) => c.deriv().eval(0.5).to_vec2(),
        };
        if d.hypot() < 1e-12 {
            continue;
        }
        let n = kurbo::Vec2::new(-d.y, d.x) / d.hypot() * eps;
        for q in [mid + n, mid - n] {
            if contour.winding(q) != 0 {
                return Some(q);
            }
        }
    }
    None
}

/// The path without the zero-length segments the engine writes where a
/// contour passes through one point several times (a star's crossings, an exact
/// tangency): a `LineTo` landing on the current point, or a curve whose
/// handles and end all sit there, draws nothing and is a node an editor would
/// have to delete.
fn dedup_points(path: &BezPath) -> BezPath {
    let same = |a: kurbo::Point, b: kurbo::Point| (a - b).hypot() < 1e-9;
    let mut out = BezPath::new();
    let mut cur = kurbo::Point::ZERO;
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                cur = p;
                out.move_to(p);
            }
            PathEl::LineTo(p) if same(p, cur) => {}
            PathEl::QuadTo(a, p) if same(p, cur) && same(a, cur) => {}
            PathEl::CurveTo(a, b, p) if same(p, cur) && same(a, cur) && same(b, cur) => {}
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                cur = p;
                out.push(*el);
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    out
}

/// Absolute path data with the compiler's three decimals, curves kept. At
/// that precision a segment whose handles and end all round onto the pen draws
/// nothing and is dropped, and a contour left with fewer than two distinct
/// points is dropped whole.
fn serialize_curves(path: &BezPath) -> String {
    use crate::spline::fmt;
    let q = |p: &kurbo::Point| format!("{} {}", fmt(p.x), fmt(p.y));
    let mut s = String::new();
    let mut contour = String::new();
    let mut distinct: Vec<String> = Vec::new();
    let mut pen = String::new();
    let flush = |contour: &mut String, distinct: &mut Vec<String>, s: &mut String| {
        if distinct.len() >= 2 {
            if !s.is_empty() {
                s.push(' ');
            }
            s.push_str(contour);
        }
        contour.clear();
        distinct.clear();
    };
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                flush(&mut contour, &mut distinct, &mut s);
                pen = q(p);
                contour.push('M');
                contour.push_str(&pen);
                distinct.push(pen.clone());
            }
            PathEl::ClosePath => {
                contour.push_str(" Z");
                flush(&mut contour, &mut distinct, &mut s);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                let end = q(p);
                let handles: Vec<String> = match el {
                    PathEl::QuadTo(a, _) => vec![q(a)],
                    PathEl::CurveTo(a, b, _) => vec![q(a), q(b)],
                    _ => Vec::new(),
                };
                if end == pen && handles.iter().all(|h| *h == pen) {
                    continue;
                }
                contour.push(' ');
                contour.push(match el {
                    PathEl::LineTo(_) => 'L',
                    PathEl::QuadTo(..) => 'Q',
                    _ => 'C',
                });
                for h in &handles {
                    contour.push_str(h);
                    contour.push(' ');
                }
                contour.push_str(&end);
                if !distinct.contains(&end) {
                    distinct.push(end.clone());
                }
                pen = end;
            }
        }
    }
    flush(&mut contour, &mut distinct, &mut s);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the curve-preserving algebra (Incremental.md §11) -----------------

    /// Inside-set parity between a curved result and the flattened one, over a
    /// jittered grid: the fraction of samples where they disagree.
    fn disagreement(curved: &str, flat: &str) -> f64 {
        use kurbo::{Point, Shape};
        let c = BezPath::from_svg(curved).unwrap();
        let f = BezPath::from_svg(flat).unwrap();
        let bb = c.bounding_box().union(f.bounding_box());
        let n = 64;
        let mut differ = 0;
        for j in 0..n {
            for i in 0..n {
                let p = Point::new(
                    bb.x0 + (i as f64 + 0.37) * bb.width() / n as f64,
                    bb.y0 + (j as f64 + 0.61) * bb.height() / n as f64,
                );
                if (c.winding(p) != 0) != (f.winding(p) != 0) {
                    differ += 1;
                }
            }
        }
        differ as f64 / (n * n) as f64
    }

    fn both(ops: &[BoolOperand], op: BoolOp) -> (String, String) {
        let curved = boolean_svg_paths_curved(ops, op).unwrap().unwrap();
        let flat = boolean_svg_paths(ops, op, 0.05).unwrap();
        (curved, flat)
    }

    #[test]
    fn curved_union_keeps_the_untouched_cubics_to_the_bit() {
        // a rect and a circle (κ·30 = 16.569): the circle's two uncut quarter
        // cubics come back verbatim, the cut ones as sub-curves, the rect's
        // edges as lines; the compiled (flattened) region is the same
        let circle = "M140 35 C140 51.569 126.569 65 110 65 C93.431 65 80 51.569 80 35 C80 18.431 93.431 5 110 5 C126.569 5 140 18.431 140 35 Z";
        let ops = [
            BoolOperand {
                paths: vec!["M10 10 L110 10 L110 60 L10 60 Z"],
                even_odd: false,
            },
            BoolOperand {
                paths: vec![circle],
                even_odd: false,
            },
        ];
        let (curved, flat) = both(&ops, BoolOp::Union);
        assert!(
            curved.contains("C126.569 5 140 18.431 140 35 C140 51.569 126.569 65 110 65"),
            "{curved}"
        );
        assert!(curved.contains("L10 60 L10 10"), "{curved}");
        assert!(disagreement(&curved, &flat) < 0.01);
    }

    #[test]
    fn curved_holes_wind_against_their_outer_whatever_the_engine_flags() {
        // fuzz seed 991: a rounded rect minus a bowtie — the two lobes touch at
        // the crossing and the engine flags one of them as an outer; by
        // containment depth both are holes, wound against the rect
        let ops = [
            BoolOperand {
                paths: vec!["M-51.8747458326786,-28.703801098713406 L20.052843045327506,-28.703801098713406 C24.97799420512451,-28.703801098713406 28.97061809036714,-24.711177213470776 28.97061809036714,-19.786026053673773 L28.97061809036714,16.23630723709818 C28.97061809036714,21.161458396895185 24.97799420512451,25.154082282137814 20.052843045327506,25.154082282137814 L-51.8747458326786,25.154082282137814 C-56.7998969924756,25.154082282137814 -60.79252087771823,21.161458396895185 -60.79252087771823,16.23630723709818 L-60.79252087771823,-19.786026053673773 C-60.79252087771823,-24.711177213470776 -56.7998969924756,-28.703801098713406 -51.8747458326786,-28.703801098713406 Z"],
                even_odd: true,
            },
            BoolOperand {
                paths: vec!["M-41.1912828377701,-27.055190852382353 L9.36938005041901,23.50547203580676 L9.36938005041901,-27.055190852382353 L-41.1912828377701,23.50547203580676 Z"],
                even_odd: true,
            },
        ];
        let (curved, flat) = both(&ops, BoolOp::Subtract);
        assert!(disagreement(&curved, &flat) < 0.01, "{curved}");
        // three contours: the rect and two triangular holes
        assert_eq!(curved.matches('M').count(), 3, "{curved}");
    }

    #[test]
    fn curved_depth_is_read_beside_an_unshared_edge() {
        // fuzz seed 4389: a self-crossing polygon (evenodd) xor its copy nudged
        // by 1.6e-7 (nonzero) — contours sharing an edge, where a point beside
        // the shared edge reads the neighbour's depth
        let a = "M53.64925134705148,25.483293384577763 L42.80081259874452,13.719252978729596 L51.51712439924321,15.624231560470413 L42.10704570043318,11.502389092393573 L52.01581039096862,12.124392372247701 L48.892876296106905,16.964237742652568 L49.70840911733144,22.444498192591862 L51.63210420347156,17.72259067706239 L48.537956901556925,28.711522083213403 L48.58188483969366,13.50287440614132 Z";
        let b = "M53.649251511048455,25.483293477200945 L42.800812762741494,13.71925307135278 L51.51712456324018,15.624231653093597 L42.107045864430155,11.502389185016757 L52.01581055496559,12.124392464870885 L48.89287646010388,16.96423783527575 L49.708409281328414,22.444498285215044 L51.632104367468536,17.72259076968557 L48.5379570655539,28.711522175836585 L48.58188500369063,13.502874498764504 Z";
        let ops = [
            BoolOperand {
                paths: vec![a],
                even_odd: true,
            },
            BoolOperand {
                paths: vec![b],
                even_odd: false,
            },
        ];
        let (curved, flat) = both(&ops, BoolOp::Exclude);
        assert!(disagreement(&curved, &flat) < 0.02, "{curved}");
    }

    #[test]
    fn curved_xor_of_three_operands_with_coincident_boundaries() {
        // fuzz seed 995: ring ⊕ rotated ring (evenodd) ⊕ the ring again = the
        // rotated ring. Folding pairwise fed the engine its own output, whose
        // pieces lie within epsilon of the third operand's curves, and the
        // coincident-boundary decisions went wrong; one n-ary sweep over the
        // raw operands holds
        let ring = "M100.32244764704502,-16.784381700865644 C100.32244764704502,15.597971066912464 74.07136398968325,41.84905472427423 41.68901122190515,41.84905472427423 C9.306658454127039,41.84905472427423 -16.94442520323473,15.597971066912464 -16.94442520323473,-16.784381700865644 C-16.94442520323473,-49.16673446864375 9.306658454127039,-75.41781812600553 41.68901122190515,-75.41781812600553 C74.07136398968325,-75.41781812600553 100.32244764704502,-49.16673446864375 100.32244764704502,-16.784381700865644 Z M58.43461701337806,-16.784381700865644 C58.43461701337806,-35.54861440288821 50.93735392721385,-50.76003050842636 41.68901122190515,-50.76003050842636 C32.440668516596446,-50.76003050842636 24.943405430432232,-35.54861440288821 24.943405430432232,-16.784381700865644 C24.943405430432232,1.9798510011569235 32.440668516596446,17.191267106695072 41.68901122190515,17.191267106695072 C50.93735392721385,17.191267106695072 58.43461701337806,1.9798510011569235 58.43461701337806,-16.784381700865644 Z";
        let rotated = "M4.221785115792521,28.316470119630516 C-20.686727549045884,7.6238925227623255 -24.104418195459203,-29.343095142139877 -3.411840598591013,-54.25160780697827 C17.280736998277177,-79.16012047181668 54.24772466317937,-82.57781111822999 79.15623732801778,-61.885233521361805 C104.06474999285618,-41.192655924493614 107.4824406392695,-4.225668259591416 86.78986304240131,20.68284440524699 C66.09728544553312,45.5913570700854 29.130297780630926,49.00904771649869 4.221785115792521,28.316470119630516 Z M30.988437680151527,-3.903658149724059 C45.421889482003834,8.08683383264512 61.91332395926942,12.040151755737334 67.82308754082283,4.926324571655492 C73.73285112237622,-2.18750261242635 66.82303656551109,-17.67461326963806 52.389584763658775,-29.66510525200723 C37.95613296180646,-41.6555972343764 21.464698484540882,-45.608915157468616 15.554934902987476,-38.495087973386774 C9.645171321434077,-31.38126078930494 16.554985878299213,-15.89415013209323 30.988437680151527,-3.903658149724059 Z";
        let ops = [
            BoolOperand {
                paths: vec![ring],
                even_odd: false,
            },
            BoolOperand {
                paths: vec![rotated],
                even_odd: true,
            },
            BoolOperand {
                paths: vec![ring],
                even_odd: false,
            },
        ];
        let (curved, flat) = both(&ops, BoolOp::Exclude);
        assert!(disagreement(&curved, &flat) < 0.01, "{curved}");
        // the rotated ring, its two contours verbatim
        assert!(
            curved.contains("C-20.687 7.624 -24.104 -29.343 -3.412 -54.252"),
            "{curved}"
        );
        assert_eq!(curved.matches('M').count(), 2, "{curved}");
    }

    #[test]
    fn the_checked_algebra_keeps_the_curves_when_they_agree_with_the_render() {
        let circle = "M140 35 C140 51.569 126.569 65 110 65 C93.431 65 80 51.569 80 35 C80 18.431 93.431 5 110 5 C126.569 5 140 18.431 140 35 Z";
        let ops = [
            BoolOperand {
                paths: vec!["M10 10 L110 10 L110 60 L10 60 Z"],
                even_odd: false,
            },
            BoolOperand {
                paths: vec![circle],
                even_odd: false,
            },
        ];
        let d = boolean_svg_paths_checked(&ops, BoolOp::Union, 0.05).unwrap();
        assert!(d.contains("C126.569 5 140 18.431 140 35"), "{d}");
        // the gate itself: a region against a different one is a disagreement
        assert!(agree(
            "M0 0 L10 0 L10 10 L0 10 Z",
            "M0 0 L10 0 L10 10 L0 10 Z"
        ));
        assert!(!agree(
            "M0 0 L10 0 L10 10 L0 10 Z",
            "M0 0 L10 0 L10 5 L0 5 Z"
        ));
        assert!(agree("", ""));
        assert!(!agree("", "M0 0 L10 0 L10 10 Z"));
    }

    #[test]
    fn curved_degenerate_inputs_do_not_upset_it() {
        // nothing usable; a disjoint intersect (empty); a shape against itself
        // (empty difference, itself for union); an arc operand (as cubics)
        let none = [BoolOperand {
            paths: vec!["not a path"],
            even_odd: false,
        }];
        assert_eq!(
            boolean_svg_paths_curved(&none, BoolOp::Union).unwrap(),
            None
        );
        let a = "M0 0 L10 0 L10 10 L0 10 Z";
        let far = "M50 50 L60 50 L60 60 L50 60 Z";
        let ops = [
            BoolOperand {
                paths: vec![a],
                even_odd: false,
            },
            BoolOperand {
                paths: vec![far],
                even_odd: false,
            },
        ];
        assert_eq!(
            boolean_svg_paths_curved(&ops, BoolOp::Intersect)
                .unwrap()
                .unwrap(),
            ""
        );
        let twice = [
            BoolOperand {
                paths: vec![a],
                even_odd: false,
            },
            BoolOperand {
                paths: vec![a],
                even_odd: false,
            },
        ];
        assert_eq!(
            boolean_svg_paths_curved(&twice, BoolOp::Subtract)
                .unwrap()
                .unwrap(),
            ""
        );
        let union = boolean_svg_paths_curved(&twice, BoolOp::Union)
            .unwrap()
            .unwrap();
        assert_eq!(union.matches('M').count(), 1, "{union}");
        let arc = [BoolOperand { paths: vec!["M20 10 A10 10 0 0 1 10 20 A10 10 0 0 1 0 10 A10 10 0 0 1 10 0 A10 10 0 0 1 20 10 Z"], even_odd: false }];
        let d = boolean_svg_paths_curved(&arc, BoolOp::Union)
            .unwrap()
            .unwrap();
        assert!(d.contains("C20 15.523 15.523 20 10 20"), "{d}");
    }

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
