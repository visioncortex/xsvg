//! Boolean operations on paths with the curves kept: the algebra behind an
//! editor's *convert to path* of an `<x:boolean>` (xsvg, docs/Incremental.md
//! §11). The engine is [`linesweeper`]; around it, what makes its answer fit
//! for a path a person will edit:
//!
//! - **one n-ary sweep** over the raw operands, each read under its own fill
//!   rule, so nothing the engine wrote is ever fed back to it;
//! - a **normalised frame** (moved to the origin, a small drawing scaled up,
//!   never down), since the engine's accuracy is absolute;
//! - a **near-coincident pre-check**: an operand running along another within
//!   10⁻⁷ of the extent is declined — there the engine's subdivision never
//!   settles and its memory grows without bound;
//! - **snap to input**: a stretch of the result lying on an input segment is
//!   written as that segment's exact sub-curve — the segment itself, to the
//!   bit, when it was not cut;
//! - **orientation by containment depth** (outer contours positive, holes
//!   negative, whatever the engine's own flag says), zero-length segments and
//!   sub-precision slivers removed;
//! - a **gate** ([`combine_checked`]): the answer is compared with the
//!   compiler's flattened result on a grid of samples and only written if the
//!   two agree.
//!
//! Operands are SVG path data with arcs already expanded to cubics (the
//! compiler's `arcs_to_cubics`); the result is absolute path data at three
//! decimals. See the README for the engine's known failure modes and their
//! measured rates, and `examples/fuzz.rs` (FUZZ.md) for the harness that measures them.

use kurbo::{BezPath, PathEl, Rect};

/// The operation (the Pathfinder set). `Subtract` removes every later operand
/// from the first; the others fold symmetrically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Union,
    Intersect,
    Subtract,
    Exclude,
}

impl Op {
    /// `None` for an unknown name.
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

/// One operand: the path data of one shape (several `d` strings when one
/// element lowered to several paths, an outlined text say), read as one region
/// under its own fill rule.
pub struct Operand<'a> {
    pub paths: Vec<&'a str>,
    pub even_odd: bool,
}

/// A coordinate as the compiler writes one: three decimals, no trailing zeros.
fn fmt(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
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
pub fn combine(operands: &[Operand], op: Op) -> Result<Option<String>, String> {
    let paths: Vec<(BezPath, bool)> = operands
        .iter()
        .filter_map(|o| curved_operand(&o.paths).map(|p| (p, o.even_odd)))
        .collect();
    if paths.is_empty() {
        return Ok(None);
    }
    if near_coincident(&paths) {
        return Err(
            "operands near-coincident: the engine's subdivision does not terminate there".into(),
        );
    }
    let inputs: Vec<kurbo::PathSeg> = paths.iter().flat_map(|(p, _)| p.segments()).collect();
    let result = sweep_normalized(&paths, op)?;
    let snapped = prune_thin(&snap_to_inputs(&result, &inputs));
    Ok(Some(serialize_curves(&snapped)))
}

/// Whether one operand runs along another within a hair of it — a copy nudged
/// by less than 10⁻⁷ of the extent, as a round trip through another tool's
/// arithmetic leaves (the storms seen sat at 10⁻¹¹…2·10⁻⁹). Such a pair is
/// where the engine's robust subdivision never settles and its memory grows
/// without bound (a probe reached 90 GB), so the curve algebra declines it and
/// the flattened one answers. An exact copy is fine (its segments are
/// identical and merge); a shape that merely touches or shares one edge is
/// fine too: more than half of an operand's sampled boundary must lie that
/// close to the other's.
fn near_coincident(paths: &[(BezPath, bool)]) -> bool {
    use kurbo::{ParamCurve, ParamCurveNearest, Shape};
    let mut bb: Option<Rect> = None;
    for (p, _) in paths {
        let b = p.bounding_box();
        bb = Some(bb.map_or(b, |a| a.union(b)));
    }
    let bb = bb.unwrap_or(Rect::ZERO);
    let span = bb.width().max(bb.height()).max(1e-12);
    let hair = 1e-7 * span;
    // measured with the drawing at the origin: at a far-off position the
    // float noise of a nearest-point search exceeds the width of a hair
    let home = kurbo::Affine::translate((-bb.x0, -bb.y0));
    let segs: Vec<Vec<kurbo::PathSeg>> = paths
        .iter()
        .map(|(p, _)| (home * p.clone()).segments().collect())
        .collect();
    for (i, a) in segs.iter().enumerate() {
        for (j, b) in segs.iter().enumerate() {
            if i == j || a.is_empty() || b.is_empty() {
                continue;
            }
            // a's segments sampled at their midpoints (at most 64 of them)
            let step = a.len().div_ceil(64).max(1);
            let samples: Vec<kurbo::Point> = a.iter().step_by(step).map(|s| s.eval(0.5)).collect();
            let mut close = 0;
            let mut exact = 0;
            for q in &samples {
                let d = b
                    .iter()
                    .map(|s| s.nearest(*q, 1e-9).distance_sq)
                    .fold(f64::INFINITY, f64::min)
                    .sqrt();
                if d < hair {
                    close += 1;
                    if d < 1e-12 * span {
                        exact += 1;
                    }
                }
            }
            // identical copies sit exactly on each other and the engine merges
            // them; it is the hair-wide gap that does not settle
            if close * 2 > samples.len() && exact * 2 < close {
                return true;
            }
        }
    }
    false
}

/// The sweep in a canonical frame: the operands' joint extent moved to the
/// origin, and a drawing smaller than about a thousand units scaled up to
/// that, the result mapped back. The engine's accuracy is absolute (its
/// epsilon floors at 1e-6), so a small drawing is swept coarsely — a tiny one
/// panicked and stalled, an ordinary one stalled on near-coincident copies —
/// and a far-off one is at the mercy of its magnitude. A large drawing keeps
/// its scale: scaled down, the engine mistakes a shape's exact copy for a
/// neighbour and draws the seam.
fn sweep_normalized(paths: &[(BezPath, bool)], op: Op) -> Result<BezPath, String> {
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
    // scaled up to about a thousand units when smaller, never down
    let scale = if span < 1024.0 { 1024.0 / span } else { 1.0 };
    let to_frame = Affine::scale(scale) * Affine::translate((-bb.x0, -bb.y0));
    let framed: Vec<(BezPath, bool)> = paths
        .iter()
        .map(|(p, eo)| (to_frame * p.clone(), *eo))
        .collect();
    let result = sweep(&framed, op)?;
    Ok(to_frame.inverse() * result)
}

/// The curve-preserving result when it agrees with the flattened one, else the
/// flattened one: convert-to-path's answer. `flat` is the compiler's own
/// (flattened) path data for the same operands and op — `Some("")` for an
/// empty region, `None` for no usable geometry. The two are compared on a
/// jittered grid of samples over their joint extent; a disagreement beyond what
/// the flattening tolerance explains along the boundary means the engine
/// mis-resolved the topology (its known weakness is a boundary within epsilon
/// of another), and the conversion is then no worse than the render. An engine
/// error, a declined input, or — where the host unwinds — a panic in the
/// engine all answer with `flat` too.
pub fn combine_checked(operands: &[Operand], op: Op, flat: Option<&str>) -> Option<String> {
    let flat = flat.map(str::to_string);
    let curved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| combine(operands, op)));
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

/// The stages of [`combine`] — the engine's oriented
/// contours, after the snap-to-input pass, after pruning — as path data, for
/// the fuzz harness to see which stage a disagreement enters at.
#[cfg(feature = "diagnostics")]
#[doc(hidden)]
pub fn stages(operands: &[Operand], op: Op) -> Result<Option<[String; 3]>, String> {
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

/// [`combine`] without the snap-to-input and pruning passes:
/// the engine's own contours, oriented. A diagnostic for the fuzz harness, to
/// tell an engine disagreement from one the passes introduce.
#[cfg(feature = "diagnostics")]
#[doc(hidden)]
pub fn raw(operands: &[Operand], op: Op) -> Result<Option<String>, String> {
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
fn sweep(paths: &[(BezPath, bool)], op: Op) -> Result<BezPath, String> {
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
            Op::Union => (0..n).any(ins),
            Op::Intersect => (0..n).all(ins),
            Op::Subtract => ins(0) && !(1..n).any(ins),
            Op::Exclude => (0..n).filter(|&i| ins(i)).count() % 2 == 1,
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
            let mut snapped = BezPath::new();
            snapped.move_to(first.start());
            for p in &pieces {
                snapped.push(p.as_path_el());
            }
            snapped.close_path();
            // the snapped contour must be the same region: a run matched to
            // the wrong input (a copy's curve within epsilon) would not be
            let (a0, a1) = (sub.area(), snapped.area());
            let same = (a0 - a1).abs() <= 1e-3 * a0.abs().max(a1.abs()) + 1e-9;
            if same {
                out.extend(snapped.elements().iter().copied());
            } else {
                out.extend(sub.elements().iter().copied());
            }
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
        let Ok(p) = BezPath::from_svg(d) else {
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
    use fmt;
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

    /// The curved result and the truth sampled from the operands themselves,
    /// written as a polygon of the grid cells that are inside — enough for the
    /// disagreement measure, which samples the same way.
    fn both(ops: &[Operand], op: Op) -> (String, String) {
        let curved = combine(ops, op).unwrap().unwrap();
        (curved.clone(), truth_as_path(ops, op, &curved))
    }

    fn inside_operand(path: &BezPath, p: kurbo::Point, even_odd: bool) -> bool {
        use kurbo::Shape;
        let w = path.winding(p);
        if even_odd {
            w % 2 != 0
        } else {
            w != 0
        }
    }

    /// The truth over the curved result's extent, as one rect per inside cell
    /// of a 64×64 grid (cells are unioned by winding when sampled).
    fn truth_as_path(ops: &[Operand], op: Op, curved: &str) -> String {
        use kurbo::Shape;
        let paths: Vec<(BezPath, bool)> = ops
            .iter()
            .map(|o| (curved_operand(&o.paths).unwrap_or_default(), o.even_odd))
            .collect();
        let mut bb = BezPath::from_svg(curved).unwrap().bounding_box();
        for (p, _) in &paths {
            bb = bb.union(p.bounding_box());
        }
        let n = 64;
        let (sx, sy) = (bb.width() / n as f64, bb.height() / n as f64);
        let mut out = String::new();
        for j in 0..n {
            for i in 0..n {
                let p = kurbo::Point::new(
                    bb.x0 + (i as f64 + 0.37) * sx,
                    bb.y0 + (j as f64 + 0.61) * sy,
                );
                let ins: Vec<bool> = paths
                    .iter()
                    .map(|(path, eo)| inside_operand(path, p, *eo))
                    .collect();
                let t = match op {
                    Op::Union => ins.iter().any(|&b| b),
                    Op::Intersect => ins.iter().all(|&b| b),
                    Op::Subtract => ins[0] && !ins[1..].iter().any(|&b| b),
                    Op::Exclude => ins.iter().filter(|&&b| b).count() % 2 == 1,
                };
                if t {
                    let (x0, y0) = (bb.x0 + i as f64 * sx, bb.y0 + j as f64 * sy);
                    out.push_str(&format!(
                        "M{x0} {y0} L{} {y0} L{} {} L{x0} {} Z ",
                        x0 + sx,
                        x0 + sx,
                        y0 + sy,
                        y0 + sy
                    ));
                }
            }
        }
        out
    }

    #[test]
    fn curved_union_keeps_the_untouched_cubics_to_the_bit() {
        // a rect and a circle (κ·30 = 16.569): the circle's two uncut quarter
        // cubics come back verbatim, the cut ones as sub-curves, the rect's
        // edges as lines; the compiled (flattened) region is the same
        let circle = "M140 35 C140 51.569 126.569 65 110 65 C93.431 65 80 51.569 80 35 C80 18.431 93.431 5 110 5 C126.569 5 140 18.431 140 35 Z";
        let ops = [
            Operand {
                paths: vec!["M10 10 L110 10 L110 60 L10 60 Z"],
                even_odd: false,
            },
            Operand {
                paths: vec![circle],
                even_odd: false,
            },
        ];
        let (curved, flat) = both(&ops, Op::Union);
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
            Operand {
                paths: vec!["M-51.8747458326786,-28.703801098713406 L20.052843045327506,-28.703801098713406 C24.97799420512451,-28.703801098713406 28.97061809036714,-24.711177213470776 28.97061809036714,-19.786026053673773 L28.97061809036714,16.23630723709818 C28.97061809036714,21.161458396895185 24.97799420512451,25.154082282137814 20.052843045327506,25.154082282137814 L-51.8747458326786,25.154082282137814 C-56.7998969924756,25.154082282137814 -60.79252087771823,21.161458396895185 -60.79252087771823,16.23630723709818 L-60.79252087771823,-19.786026053673773 C-60.79252087771823,-24.711177213470776 -56.7998969924756,-28.703801098713406 -51.8747458326786,-28.703801098713406 Z"],
                even_odd: true,
            },
            Operand {
                paths: vec!["M-41.1912828377701,-27.055190852382353 L9.36938005041901,23.50547203580676 L9.36938005041901,-27.055190852382353 L-41.1912828377701,23.50547203580676 Z"],
                even_odd: true,
            },
        ];
        let (curved, flat) = both(&ops, Op::Subtract);
        assert!(disagreement(&curved, &flat) < 0.01, "{curved}");
        // three contours: the rect and two triangular holes
        assert_eq!(curved.matches('M').count(), 3, "{curved}");
    }

    #[test]
    fn curved_declines_a_hair_wide_pair_and_keeps_an_exact_copy() {
        // fuzz seed 4389: a self-crossing polygon (evenodd) xor its copy nudged
        // by 1.6e-7 (nonzero)
        let a = "M53.64925134705148,25.483293384577763 L42.80081259874452,13.719252978729596 L51.51712439924321,15.624231560470413 L42.10704570043318,11.502389092393573 L52.01581039096862,12.124392372247701 L48.892876296106905,16.964237742652568 L49.70840911733144,22.444498192591862 L51.63210420347156,17.72259067706239 L48.537956901556925,28.711522083213403 L48.58188483969366,13.50287440614132 Z";
        let b = "M53.649251511048455,25.483293477200945 L42.800812762741494,13.71925307135278 L51.51712456324018,15.624231653093597 L42.107045864430155,11.502389185016757 L52.01581055496559,12.124392464870885 L48.89287646010388,16.96423783527575 L49.708409281328414,22.444498285215044 L51.632104367468536,17.72259076968557 L48.5379570655539,28.711522175836585 L48.58188500369063,13.502874498764504 Z";
        let ops = [
            Operand {
                paths: vec![a],
                even_odd: true,
            },
            Operand {
                paths: vec![b],
                even_odd: false,
            },
        ];
        // a copy nudged by 1.6e-7 is a hair-wide pair: declined before the
        // engine (its subdivision never settles there), the flattened answer
        // stands — the depth fix itself stays covered by the fuzz corpus
        let declined = combine(&ops, Op::Exclude);
        assert!(
            matches!(&declined, Err(e) if e.contains("near-coincident")),
            "{declined:?}"
        );
        // and the gate hands back whatever flattened answer it was given
        let flat = "M0 0 L1 0 L1 1 Z";
        assert_eq!(
            combine_checked(&ops, Op::Exclude, Some(flat)).unwrap(),
            flat
        );
        // an exact copy is not declined: its segments merge
        let twice = [
            Operand {
                paths: vec![a],
                even_odd: false,
            },
            Operand {
                paths: vec![a],
                even_odd: false,
            },
        ];
        assert!(combine(&twice, Op::Union).is_ok());
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
            Operand {
                paths: vec![ring],
                even_odd: false,
            },
            Operand {
                paths: vec![rotated],
                even_odd: true,
            },
            Operand {
                paths: vec![ring],
                even_odd: false,
            },
        ];
        let (curved, flat) = both(&ops, Op::Exclude);
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
            Operand {
                paths: vec!["M10 10 L110 10 L110 60 L10 60 Z"],
                even_odd: false,
            },
            Operand {
                paths: vec![circle],
                even_odd: false,
            },
        ];
        let curved = combine(&ops, Op::Union).unwrap().unwrap();
        // the compiler's flattened answer for the same region, here stood in
        // for by the truth polygon: the curves are kept
        let flat = truth_as_path(&ops, Op::Union, &curved);
        let d = combine_checked(&ops, Op::Union, Some(&flat)).unwrap();
        assert!(d.contains("C126.569 5 140 18.431 140 35"), "{d}");
        // a flattened answer for a different region wins over the curves
        let other = "M0 0 L10 0 L10 10 L0 10 Z";
        assert_eq!(
            combine_checked(&ops, Op::Union, Some(other)).unwrap(),
            other
        );
        assert_eq!(combine_checked(&ops, Op::Union, None), None);
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
        let none = [Operand {
            paths: vec!["not a path"],
            even_odd: false,
        }];
        assert_eq!(combine(&none, Op::Union).unwrap(), None);
        let a = "M0 0 L10 0 L10 10 L0 10 Z";
        let far = "M50 50 L60 50 L60 60 L50 60 Z";
        let ops = [
            Operand {
                paths: vec![a],
                even_odd: false,
            },
            Operand {
                paths: vec![far],
                even_odd: false,
            },
        ];
        assert_eq!(combine(&ops, Op::Intersect).unwrap().unwrap(), "");
        let twice = [
            Operand {
                paths: vec![a],
                even_odd: false,
            },
            Operand {
                paths: vec![a],
                even_odd: false,
            },
        ];
        assert_eq!(combine(&twice, Op::Subtract).unwrap().unwrap(), "");
        let union = combine(&twice, Op::Union).unwrap().unwrap();
        assert_eq!(union.matches('M').count(), 1, "{union}");
        let arc = [Operand { paths: vec!["M20 10 A10 10 0 0 1 10 20 A10 10 0 0 1 0 10 A10 10 0 0 1 10 0 A10 10 0 0 1 20 10 Z"], even_odd: false }];
        let d = combine(&arc, Op::Union).unwrap().unwrap();
        assert!(d.contains("C20 15.523 15.523 20 10 20"), "{d}");
    }
}
