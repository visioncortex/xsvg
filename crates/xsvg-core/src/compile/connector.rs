//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// `<x:connector from="#a" to="#b" route="…" arrow="…">` (§7.6): a line routed
/// between two elements' bounding boxes, lowered to a `<path>` with the
/// connector's own stroke. `route` ∈ straight | x-major | y-major | curve;
/// `arrow` ∈ end | start | both | none (default end). Endpoints resolve like
/// any reference (§4) and the route is recomputed from their boxes, so moving
/// an endpoint re-emits the connector (baked reference).
pub(super) fn emit_connector(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    use crate::kurbo::{Point, Rect};

    // §7.6 endpoints. `from`/`to` reference an element by id, optionally with a
    // `:side` suffix (`#b:left|right|top|bottom`) forcing which edge to attach to.
    // `from-point`/`to-point` give a raw `x,y` instead — the ref wins if both are set.
    #[derive(Clone, Copy)]
    enum Side {
        Left,
        Right,
        Top,
        Bottom,
    }
    let parse_side = |s: &str| match s {
        "left" => Some(Side::Left),
        "right" => Some(Side::Right),
        "top" => Some(Side::Top),
        "bottom" => Some(Side::Bottom),
        _ => None,
    };
    let parse_point = |s: &str| -> Option<Point> {
        let mut it = s
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty());
        let x = it.next()?.parse::<f64>().ok()?;
        let y = it.next()?.parse::<f64>().ok()?;
        if it.next().is_some() {
            return None; // more than two numbers — malformed
        }
        Some(Point::new(x, y))
    };
    // A resolved endpoint: its box (a point is a zero-size box) + an optional forced side.
    let resolve_end = |ref_attr: &str, pt_attr: &str| -> Option<(Rect, Option<Side>)> {
        if let Some(r) = node.attribute(ref_attr) {
            let (id, side) = match r.rsplit_once(':') {
                Some((base, suf)) => match parse_side(suf) {
                    Some(s) => (base, Some(s)),
                    None => (r, None), // `:` but not a side keyword — treat whole value as the id
                },
                None => (r, None),
            };
            let rect = ref_geometry(node, id, ctx)
                .ok()
                .and_then(|d| svg_path_bbox(&d))?;
            Some((rect, side))
        } else if let Some(pt) = node.attribute(pt_attr).and_then(|p| parse_point(p)) {
            Some((Rect::new(pt.x, pt.y, pt.x, pt.y), None))
        } else {
            None
        }
    };
    let (Some((a, side_a)), Some((b, side_b))) =
        (resolve_end("from", "from-point"), resolve_end("to", "to-point"))
    else {
        out.push_str("<!-- xsvg: <x:connector> endpoint not found or not geometry -->");
        return;
    };
    let (ca, cb) = (a.center(), b.center());
    // Midpoint of a box's named edge, and that edge's outward unit normal.
    let side_pt = |r: Rect, s: Side| match s {
        Side::Left => Point::new(r.x0, r.center().y),
        Side::Right => Point::new(r.x1, r.center().y),
        Side::Top => Point::new(r.center().x, r.y0),
        Side::Bottom => Point::new(r.center().x, r.y1),
    };
    let side_norm = |s: Side| match s {
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
    };
    // point on rect `r`'s edge along the ray from its center toward `t`
    let edge = |r: Rect, t: Point| -> Point {
        let c = r.center();
        let (dx, dy) = (t.x - c.x, t.y - c.y);
        if dx == 0.0 && dy == 0.0 {
            return c;
        }
        let sx = if dx != 0.0 {
            r.width() / 2.0 / dx.abs()
        } else {
            f64::INFINITY
        };
        let sy = if dy != 0.0 {
            r.height() / 2.0 / dy.abs()
        } else {
            f64::INFINITY
        };
        let s = sx.min(sy);
        Point::new(c.x + dx * s, c.y + dy * s)
    };
    let p = |pt: Point| format!("{},{}", fmt(pt.x), fmt(pt.y));

    let stroke = resolve_var(node.attribute("stroke").unwrap_or("#334155")).into_owned();
    let sw = attr_num(node, "stroke-width", 1.0);
    let size = attr_num(node, "arrow-size", (sw * 3.5).max(7.0)).max(0.0);

    let cubic_at = |p0: Point, p1: Point, p2: Point, p3: Point, t: f64| -> Point {
        let mt = 1.0 - t;
        let (a, b, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
        Point::new(
            a * p0.x + b * p1.x + c * p2.x + d * p3.x,
            a * p0.y + b * p1.y + c * p2.y + d * p3.y,
        )
    };
    // Walk the ACTUAL cubic back from the tip endpoint until the straight-line
    // (chord) distance to the tip equals `size`, and return that on-curve point
    // with its parameter t. Used as the arrowhead's base midpoint (so the base
    // sits on the curve and its axis follows the visible approach) and as the
    // split point that trims the line back to the base. `from_end` picks the tip.
    let chord_back = |p0: Point, p1: Point, p2: Point, p3: Point, from_end: bool| -> (Point, f64) {
        let tip = if from_end { p3 } else { p0 };
        let dist = |q: Point| ((q.x - tip.x).powi(2) + (q.y - tip.y).powi(2)).sqrt();
        let n = 96;
        let (mut prev_t, mut prev) = (if from_end { 1.0 } else { 0.0 }, tip);
        for i in 1..=n {
            let f = i as f64 / n as f64;
            let t = if from_end { 1.0 - f } else { f };
            let pt = cubic_at(p0, p1, p2, p3, t);
            let d = dist(pt);
            if d >= size {
                let dprev = dist(prev);
                let frac = if (d - dprev).abs() < 1e-9 {
                    0.0
                } else {
                    (size - dprev) / (d - dprev)
                };
                let tt = prev_t + (t - prev_t) * frac;
                return (cubic_at(p0, p1, p2, p3, tt), tt);
            }
            prev_t = t;
            prev = pt;
        }
        // curve shorter than the arrowhead: collapse to the far endpoint
        if from_end {
            (p0, 0.0)
        } else {
            (p3, 1.0)
        }
    };
    // The sub-cubic covering [t0, t1] of (p0..p3), as its own four control
    // points — de Casteljau twice. Lets us trim the drawn line back to where the
    // arrowhead begins so the stroke never protrudes past the sharp tip.
    let subcubic = |p0: Point, p1: Point, p2: Point, p3: Point, t0: f64, t1: f64| -> [Point; 4] {
        let lerp =
            |u: Point, v: Point, t: f64| Point::new(u.x + (v.x - u.x) * t, u.y + (v.y - u.y) * t);
        // clip to [t0, 1] first
        let (q0, q1, q2, q3) = if t0 > 0.0 {
            let a = lerp(p0, p1, t0);
            let b = lerp(p1, p2, t0);
            let c = lerp(p2, p3, t0);
            let d = lerp(a, b, t0);
            let e = lerp(b, c, t0);
            let f = lerp(d, e, t0);
            (f, e, c, p3)
        } else {
            (p0, p1, p2, p3)
        };
        // then take [0, s] of that, where s maps t1 into the clipped range
        let s = if t1 >= 1.0 {
            1.0
        } else {
            (t1 - t0) / (1.0 - t0)
        };
        let a = lerp(q0, q1, s);
        let b = lerp(q1, q2, s);
        let c = lerp(q2, q3, s);
        let d = lerp(a, b, s);
        let e = lerp(b, c, s);
        let f = lerp(d, e, s);
        [q0, a, d, f]
    };

    let arrow = node.attribute("arrow").unwrap_or("end");
    let arrow_start = arrow == "start" || arrow == "both";
    let arrow_end = arrow == "end" || arrow == "both";
    // Trim `from` toward `toward` by the arrowhead height, so the stroked line
    // stops at the triangle's base instead of running under (and past) the tip.
    // Clamped to the segment so a short segment doesn't invert.
    let trim = |from: Point, toward: Point| -> Point {
        let (dx, dy) = (toward.x - from.x, toward.y - from.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            return from;
        }
        let s = (size / len).min(1.0);
        Point::new(from.x + dx * s, from.y + dy * s)
    };

    // Each route yields its drawn path (already trimmed where an arrowhead sits)
    // plus the two endpoint tangents as (tip, adj) pairs: `adj` is the
    // neighbouring path point, so unit(tip − adj) is the direction the line
    // travels OUT of that end — the way its arrowhead points.
    let (d, start_tip, start_adj, end_tip, end_adj) =
        match node.attribute("route").unwrap_or("straight") {
            "x-major" => {
                let dir = if cb.x >= ca.x { 1.0 } else { -1.0 };
                let ax = match side_a {
                    Some(s) => side_pt(a, s),
                    None => Point::new(ca.x + dir * a.width() / 2.0, ca.y),
                };
                let bx = match side_b {
                    Some(s) => side_pt(b, s),
                    None => Point::new(cb.x - dir * b.width() / 2.0, cb.y),
                };
                let mx = (ax.x + bx.x) / 2.0;
                let (p1, p2) = (Point::new(mx, ax.y), Point::new(mx, bx.y));
                let s = if arrow_start { trim(ax, p1) } else { ax };
                let e = if arrow_end { trim(bx, p2) } else { bx };
                (
                    format!("M{} L{} L{} L{}", p(s), p(p1), p(p2), p(e)),
                    ax,
                    p1,
                    bx,
                    p2,
                )
            }
            "y-major" => {
                let dir = if cb.y >= ca.y { 1.0 } else { -1.0 };
                let ay = match side_a {
                    Some(s) => side_pt(a, s),
                    None => Point::new(ca.x, ca.y + dir * a.height() / 2.0),
                };
                let by = match side_b {
                    Some(s) => side_pt(b, s),
                    None => Point::new(cb.x, cb.y - dir * b.height() / 2.0),
                };
                let my = (ay.y + by.y) / 2.0;
                let (p1, p2) = (Point::new(ay.x, my), Point::new(by.x, my));
                let s = if arrow_start { trim(ay, p1) } else { ay };
                let e = if arrow_end { trim(by, p2) } else { by };
                (
                    format!("M{} L{} L{} L{}", p(s), p(p1), p(p2), p(e)),
                    ay,
                    p1,
                    by,
                    p2,
                )
            }
            "curve" => {
                let horiz = (cb.x - ca.x).abs() >= (cb.y - ca.y).abs();
                // Each endpoint's exit point + outward direction: a forced side gives
                // its edge midpoint and normal; otherwise the box edge along the
                // dominant axis toward the other end (the original auto behaviour).
                let exit = |r: Rect, c: Point, toward: Point, forced: Option<Side>| -> (Point, (f64, f64)) {
                    if let Some(s) = forced {
                        return (side_pt(r, s), side_norm(s));
                    }
                    if horiz {
                        let d = if toward.x >= c.x { 1.0 } else { -1.0 };
                        (Point::new(c.x + d * r.width() / 2.0, c.y), (d, 0.0))
                    } else {
                        let d = if toward.y >= c.y { 1.0 } else { -1.0 };
                        (Point::new(c.x, c.y + d * r.height() / 2.0), (0.0, d))
                    }
                };
                let (a0, an) = exit(a, ca, cb, side_a);
                let (b0, bn) = exit(b, cb, ca, side_b);
                let k = if horiz {
                    ((b0.x - a0.x).abs() * 0.5).max(36.0)
                } else {
                    ((b0.y - a0.y).abs() * 0.5).max(36.0)
                };
                let c1 = Point::new(a0.x + an.0 * k, a0.y + an.1 * k);
                let c2 = Point::new(b0.x + bn.0 * k, b0.y + bn.1 * k);
                let (s_base, t0) = chord_back(a0, c1, c2, b0, false);
                let (e_base, t1) = chord_back(a0, c1, c2, b0, true);
                // draw only the middle of the cubic, between the two bases
                let [q0, q1, q2, q3] = subcubic(
                    a0,
                    c1,
                    c2,
                    b0,
                    if arrow_start { t0 } else { 0.0 },
                    if arrow_end { t1 } else { 1.0 },
                );
                (
                    format!("M{} C{} {} {}", p(q0), p(q1), p(q2), p(q3)),
                    a0,
                    s_base,
                    b0,
                    e_base,
                )
            }
            _ => {
                // straight: clip the center-to-center line to each box edge (or the
                // forced side's midpoint)
                let a0 = match side_a {
                    Some(s) => side_pt(a, s),
                    None => edge(a, cb),
                };
                let b0 = match side_b {
                    Some(s) => side_pt(b, s),
                    None => edge(b, ca),
                };
                let s = if arrow_start { trim(a0, b0) } else { a0 };
                let e = if arrow_end { trim(b0, a0) } else { b0 };
                (format!("M{} L{}", p(s), p(e)), a0, b0, b0, a0)
            }
        };

    // Arrowhead as a computed triangle: tip AT the endpoint (no penetration),
    // base midpoint at `adj` (for the curve, an on-curve point at chord distance
    // `size` — so the base sits on the curve; for straight/orthogonal routes, a
    // point back along the segment). The drawn line stops at that base, so the
    // triangle alone forms the sharp tip.
    let head = |tip: Point, adj: Point| -> Option<String> {
        let (dx, dy) = (tip.x - adj.x, tip.y - adj.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 || size <= 0.0 {
            return None;
        }
        let (ux, uy) = (dx / len, dy / len);
        let base = Point::new(tip.x - ux * size, tip.y - uy * size);
        let (px, py) = (-uy * size * 0.45, ux * size * 0.45);
        Some(format!(
            "<path fill=\"{stroke}\" d=\"M{} L{},{} L{},{} Z\"/>",
            p(tip),
            fmt(base.x + px),
            fmt(base.y + py),
            fmt(base.x - px),
            fmt(base.y - py)
        ))
    };
    let mut heads = String::new();
    if arrow_start {
        if let Some(h) = head(start_tip, start_adj) {
            heads.push_str(&h);
        }
    }
    if arrow_end {
        if let Some(h) = head(end_tip, end_adj) {
            heads.push_str(&h);
        }
    }

    out.push_str("<path");
    copy_attrs(
        node,
        out,
        &["from", "to", "from-point", "to-point", "route", "arrow", "arrow-size", "fill"],
    );
    out.push_str(&pos_attr(node, ctx));
    out.push_str(&format!(" fill=\"none\" d=\"{d}\"/>"));
    out.push_str(&heads); // arrowheads paint on top of the line
}
