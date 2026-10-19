//! Corners (§5.4): `x:corners` rounds, chamfers or scallops the corner anchors
//! of a `<path>`, `<rect>`, `<polygon>` or `<polyline>` at compile time — the
//! live corner of a design tool, as an attribute. A radius is a design property
//! here, not geometry: it can be a theme token, an agent can write "round the
//! top corners to 8" as one attribute, and the rect stays legible as a rect.
//! Uncompiled, the document renders sharp (§3).
//!
//! The geometry is Illustrator's: at a corner anchor P between two segments,
//! the fillet of radius r tangent to both trims each side by d = r / tan(θ/2),
//! θ the interior angle, and runs between the trim points; r is clamped so d
//! never exceeds half of either adjoining segment, so two corners on one edge
//! never overlap. A round corner is a circular arc (`A`, exact), a chamfer the
//! straight line between the trim points, an inverted corner the same arc
//! bulging the other way. Collinear anchors and the ends of an open subpath
//! have no corner.
//!
//! Anchors are the on-curve points of the path in order, subpaths concatenated,
//! index 0 the first `M`; on a closed subpath the end of a segment that lands on
//! the start is folded into anchor 0 and the `Z` segment is a segment like any
//! other — the closed form every compiled contour has, so an editor that folds
//! that way counts the same anchors.

use super::*;
use crate::kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv, PathEl, PathSeg, Point, Vec2};

/// How a corner is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CornerStyle {
    Round,
    Chamfer,
    Inverted,
}

impl CornerStyle {
    fn parse(v: &str) -> Option<Self> {
        match v.trim() {
            "round" => Some(Self::Round),
            "chamfer" => Some(Self::Chamfer),
            "inverted" => Some(Self::Inverted),
            _ => None,
        }
    }
}

/// What `x:corners` asks: one radius for every corner, or radii at anchors.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CornerSpec {
    pub(super) all: Option<f64>,
    pub(super) at: Vec<(usize, f64)>,
    pub(super) style: CornerStyle,
}

impl CornerSpec {
    fn radius(&self, anchor: usize) -> Option<f64> {
        self.at
            .iter()
            .find(|(i, _)| *i == anchor)
            .map(|(_, r)| *r)
            .or(self.all)
            .filter(|r| *r > 0.0)
    }
}

/// `x:corners` and `x:corner-style` on `node`, tokens resolved. `None` when the
/// element asks for no corners (absent, or nothing in it parses).
pub(super) fn corner_spec(node: roxmltree::Node, theme: &Theme) -> Option<CornerSpec> {
    let v = node.attribute((XSVG_NS, "corners"))?;
    let v = theme.resolve_var(v);
    let mut spec = CornerSpec {
        all: None,
        at: Vec::new(),
        style: node
            .attribute((XSVG_NS, "corner-style"))
            .and_then(CornerStyle::parse)
            .unwrap_or(CornerStyle::Round),
    };
    for (i, r) in super::indexed_items(&v) {
        let Some(r) = parse_num(r) else { continue };
        match i {
            Some(i) => spec.at.push((i, r.max(0.0))),
            None => spec.all = Some(r.max(0.0)),
        }
    }
    (spec.all.is_some() || !spec.at.is_empty()).then_some(spec)
}

/// One corner as compiled: for the editor's widget and readout.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerInfo {
    /// Anchor index, in the numbering above.
    pub anchor: usize,
    /// The sharp anchor.
    pub at: (f64, f64),
    /// The radius asked for.
    pub requested: f64,
    /// The radius drawn — smaller when the adjoining segments are too short.
    pub effective: f64,
    pub clamped: bool,
    /// Where the corner's curve leaves the incoming segment and joins the
    /// outgoing one.
    pub from: (f64, f64),
    pub to: (f64, f64),
    /// The arc's centre (`None` for a chamfer).
    pub center: Option<(f64, f64)>,
}

/// A subpath as segments, its anchors and whether it closes.
struct Sub {
    segs: Vec<PathSeg>,
    closed: bool,
}

fn subpaths(path: &crate::kurbo::BezPath) -> Vec<Sub> {
    let mut subs: Vec<Sub> = Vec::new();
    let (mut start, mut cur) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                subs.push(Sub {
                    segs: Vec::new(),
                    closed: false,
                });
                start = p;
                cur = p;
            }
            PathEl::LineTo(p) => {
                if let Some(s) = subs.last_mut() {
                    s.segs.push(PathSeg::Line(crate::kurbo::Line::new(cur, p)));
                }
                cur = p;
            }
            PathEl::QuadTo(a, p) => {
                if let Some(s) = subs.last_mut() {
                    s.segs
                        .push(PathSeg::Quad(crate::kurbo::QuadBez::new(cur, a, p)));
                }
                cur = p;
            }
            PathEl::CurveTo(a, b, p) => {
                if let Some(s) = subs.last_mut() {
                    s.segs
                        .push(PathSeg::Cubic(crate::kurbo::CubicBez::new(cur, a, b, p)));
                }
                cur = p;
            }
            PathEl::ClosePath => {
                if let Some(s) = subs.last_mut() {
                    // the seam: a line home unless the last segment already lands there
                    if (cur - start).hypot() > 1e-9 {
                        s.segs
                            .push(PathSeg::Line(crate::kurbo::Line::new(cur, start)));
                    }
                    s.closed = true;
                }
                cur = start;
            }
        }
    }
    subs.retain(|s| !s.segs.is_empty());
    subs
}

/// The unit tangent of a segment at its end, pointing along it.
fn end_tangent(seg: &PathSeg) -> Option<Vec2> {
    let t = match seg {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(1.0).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(1.0).to_vec2(),
    };
    (t.hypot() > 1e-12).then(|| t / t.hypot())
}

/// The unit tangent of a segment at its start, pointing along it.
fn start_tangent(seg: &PathSeg) -> Option<Vec2> {
    let t = match seg {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(0.0).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(0.0).to_vec2(),
    };
    (t.hypot() > 1e-12).then(|| t / t.hypot())
}

fn arclen(seg: &PathSeg) -> f64 {
    match seg {
        PathSeg::Line(l) => l.arclen(1e-9),
        PathSeg::Quad(q) => q.arclen(1e-9),
        PathSeg::Cubic(c) => c.arclen(1e-9),
    }
}

/// The part of a segment from arc length `from` to arc length `to`.
fn trimmed(seg: &PathSeg, from: f64, to: f64) -> PathSeg {
    let total = arclen(seg);
    if total <= 0.0 {
        return *seg;
    }
    let t0 = if from <= 0.0 {
        0.0
    } else {
        seg.inv_arclen(from, 1e-9)
    };
    let t1 = if to >= total {
        1.0
    } else {
        seg.inv_arclen(to, 1e-9)
    };
    seg.subsegment(t0..t1)
}

/// A corner's geometry, or `None` where there is no corner to draw.
struct Fillet {
    info: CornerInfo,
    /// Trim of the incoming and outgoing segments, in arc length.
    d_in: f64,
    d_out: f64,
    /// Sweep flag of the round form (`inverted` flips it).
    sweep: bool,
}

fn fillet(
    anchor: usize,
    seg_in: &PathSeg,
    seg_out: &PathSeg,
    requested: f64,
    style: CornerStyle,
) -> Option<Fillet> {
    let p = seg_out.start();
    let (Some(t_in), Some(t_out)) = (end_tangent(seg_in), start_tangent(seg_out)) else {
        return None;
    };
    let u = -t_in; // toward the previous anchor
    let v = t_out; // toward the next
    let cross = u.cross(v);
    let cos = u.dot(v).clamp(-1.0, 1.0);
    if cross.abs() < 1e-9 {
        return None; // collinear (no corner) or a spike turning back on itself
    }
    let theta = cos.acos(); // interior angle
    let tan_half = (theta / 2.0).tan();
    if tan_half <= 1e-9 {
        return None;
    }
    let d_max = (arclen(seg_in) / 2.0).min(arclen(seg_out) / 2.0);
    let d = (requested / tan_half).min(d_max);
    if d <= 1e-9 {
        return None;
    }
    let effective = d * tan_half;
    let from = p + u * d;
    let to = p + v * d;
    // the centre sits on the bisector, r / sin(θ/2) from the apex; an inverted
    // corner's is mirrored across the chord
    let bis = (u + v) / (u + v).hypot();
    let mut center = p + bis * (effective / (theta / 2.0).sin());
    // going from `from` to `to` around the centre: clockwise (sweep) in y-down
    // space when the chord turns that way
    let mut sweep = (from - center).cross(to - center) > 0.0;
    if style == CornerStyle::Inverted {
        center = from + (to - center);
        sweep = !sweep;
    }
    Some(Fillet {
        info: CornerInfo {
            anchor,
            at: (p.x, p.y),
            requested,
            effective,
            clamped: effective + 1e-9 < requested,
            from: (from.x, from.y),
            to: (to.x, to.y),
            center: (style != CornerStyle::Chamfer).then_some((center.x, center.y)),
        },
        d_in: d,
        d_out: d,
        sweep,
    })
}

fn push_pt(out: &mut String, p: Point) {
    out.push_str(&fmt(p.x));
    out.push(' ');
    out.push_str(&fmt(p.y));
}

fn push_seg(out: &mut String, seg: &PathSeg) {
    match seg {
        PathSeg::Line(l) => {
            out.push_str(" L");
            push_pt(out, l.p1);
        }
        PathSeg::Quad(q) => {
            out.push_str(" Q");
            push_pt(out, q.p1);
            out.push(' ');
            push_pt(out, q.p2);
        }
        PathSeg::Cubic(c) => {
            out.push_str(" C");
            push_pt(out, c.p1);
            out.push(' ');
            push_pt(out, c.p2);
            out.push(' ');
            push_pt(out, c.p3);
        }
    }
}

fn push_corner(out: &mut String, f: &Fillet, style: CornerStyle) {
    let to = Point::new(f.info.to.0, f.info.to.1);
    match style {
        CornerStyle::Chamfer => {
            out.push_str(" L");
            push_pt(out, to);
        }
        _ => {
            let r = fmt(f.info.effective);
            out.push_str(&format!(" A{r} {r} 0 0 {} ", u8::from(f.sweep)));
            push_pt(out, to);
        }
    }
}

/// `d` with its corners applied, and each corner drawn. `None` when `d` does
/// not parse. A path with no corner to draw comes back re-serialised the same
/// way, so an element's geometry has one spelling whether or not any corner took.
pub(super) fn apply_corners(d: &str, spec: &CornerSpec) -> Option<(String, Vec<CornerInfo>)> {
    let path = crate::kurbo::BezPath::from_svg(d).ok()?;
    let mut out = String::with_capacity(d.len() * 2);
    let mut infos = Vec::new();
    let mut anchor = 0usize;
    for sub in subpaths(&path) {
        let n = sub.segs.len();
        // the corner at each anchor: closed subpaths at every anchor, open ones
        // at the interior anchors only
        let corner_at = |i: usize| -> Option<Fillet> {
            let (seg_in, seg_out) = if sub.closed {
                (&sub.segs[(i + n - 1) % n], &sub.segs[i % n])
            } else {
                if i == 0 || i >= n {
                    return None;
                }
                (&sub.segs[i - 1], &sub.segs[i])
            };
            let r = spec.radius(anchor + i)?;
            fillet(anchor + i, seg_in, seg_out, r, spec.style)
        };
        let anchors = if sub.closed { n } else { n + 1 };
        let fillets: Vec<Option<Fillet>> = (0..anchors).map(corner_at).collect();
        // each segment trimmed by the corner at its start and the one at its end
        let trim_start = |i: usize| fillets[i].as_ref().map_or(0.0, |f| f.d_out);
        let trim_end = |i: usize| {
            let next = if sub.closed { (i + 1) % n } else { i + 1 };
            fillets
                .get(next)
                .and_then(|f| f.as_ref())
                .map_or(0.0, |f| f.d_in)
        };
        let seg = |i: usize| {
            let s = &sub.segs[i];
            trimmed(s, trim_start(i), arclen(s) - trim_end(i))
        };
        let first = seg(0);
        out.push('M');
        push_pt(&mut out, first.start());
        for i in 0..n {
            let s = if i == 0 { first } else { seg(i) };
            // the closing segment's end is the start: the seam the contract folds
            let last_of_closed = sub.closed && i == n - 1;
            let ends_at_start = last_of_closed && fillets[0].is_none();
            // a segment two corners consumed entirely draws nothing
            let line = matches!(s, PathSeg::Line(_));
            let gone = line && (s.end() - s.start()).hypot() < 1e-9;
            let seam = line && ends_at_start; // the Z draws it
            if !(gone || seam) {
                push_seg(&mut out, &s);
            }
            let next = if sub.closed { (i + 1) % n } else { i + 1 };
            if let Some(f) = fillets.get(next).and_then(|f| f.as_ref()) {
                // a chamfer home is the Z itself
                if !(last_of_closed && spec.style == CornerStyle::Chamfer) {
                    push_corner(&mut out, f, spec.style);
                }
            }
        }
        if sub.closed {
            out.push_str(" Z");
        }
        infos.extend(fillets.into_iter().flatten().map(|f| f.info));
        anchor += anchors;
    }
    Some((out, infos))
}
