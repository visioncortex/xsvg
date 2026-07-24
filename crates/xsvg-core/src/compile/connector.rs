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

    // §7.6 endpoints. `from`/`to` reference an element by id, optionally with an
    // `:anchor` suffix forcing which of the box's 9 connection points to attach to —
    // an edge midpoint (`left|right|top|bottom`), a corner (`left-top`, `right-bottom`,
    // …, either order), or `center`. `from-point`/`to-point` give a raw `x,y` instead
    // (the ref wins if both are set).
    // An anchor is a point on the box as (h, v) fractions of the half-extent from the
    // center: (-1,0)=left mid, (1,1)=right-bottom corner, (0,0)=center, etc.
    type Anchor = (f64, f64);
    // Parse `left|right|top|bottom` (edge), `center`, or a corner like `left-top` /
    // `top-left` (one horizontal + one vertical token, either order, `-`-separated).
    let parse_anchor = |s: &str| -> Option<Anchor> {
        if s == "center" {
            return Some((0.0, 0.0));
        }
        let (mut h, mut v) = (0.0, 0.0);
        let parts = s.split('-');
        let n = parts.clone().count();
        for part in parts {
            match part {
                "left" => h = -1.0,
                "right" => h = 1.0,
                "top" => v = -1.0,
                "bottom" => v = 1.0,
                _ => return None,
            }
        }
        // need at least one axis, at most one of each (reject "left-right", "left-left-top")
        if (h == 0.0 && v == 0.0) || n > 2 {
            return None;
        }
        Some((h, v))
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
    // A resolved endpoint: its box (a point is a zero-size box) + an optional forced anchor.
    let resolve_end = |ref_attr: &str, pt_attr: &str| -> Option<(Rect, Option<Anchor>)> {
        if let Some(r) = node.attribute(ref_attr) {
            let (id, anchor) = match r.rsplit_once(':') {
                Some((base, suf)) => match parse_anchor(suf) {
                    Some(an) => (base, Some(an)),
                    None => (r, None), // `:` but not an anchor keyword — treat whole value as the id
                },
                None => (r, None),
            };
            let rect = ref_geometry(node, id, ctx)
                .ok()
                .and_then(|d| svg_path_bbox(&d))?;
            Some((rect, anchor))
        } else if let Some(pt) = node.attribute(pt_attr).and_then(|p| parse_point(p)) {
            Some((Rect::new(pt.x, pt.y, pt.x, pt.y), None))
        } else {
            None
        }
    };
    let (Some((a, anchor_a)), Some((b, anchor_b))) =
        (resolve_end("from", "from-point"), resolve_end("to", "to-point"))
    else {
        out.push_str("<!-- xsvg: <x:connector> endpoint not found or not geometry -->");
        return;
    };
    let (ca, cb) = (a.center(), b.center());
    // The anchor point for (h, v), and its outward unit normal (0,0 at the center,
    // where callers fall back to aiming at the other endpoint).
    let anchor_pt = |r: Rect, (h, v): Anchor| {
        Point::new(r.center().x + h * r.width() / 2.0, r.center().y + v * r.height() / 2.0)
    };
    let anchor_norm = |(h, v): Anchor| -> Option<(f64, f64)> {
        let len = (h * h + v * v).sqrt();
        (len >= 1e-9).then_some((h / len, v / len))
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
    // How far a `route="curve"` bows out, in px — a FIXED amount (not scaled by the
    // endpoint distance), so wide and narrow connectors bow the same. Author-controllable.
    let bulge = attr_num(node, "bulge", 44.0).max(0.0);

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

    // Orthogonal "rail" waypoints from A's exit to B's exit, both leaving along the
    // major axis (`horiz` ⇒ x is major). `da`/`db` are the outward major-axis signs.
    // When the exits face each other with room between (A can reach a crossing line
    // before B), a 2-turn Z suffices. Otherwise — they face away, or B sits back past
    // A's exit (e.g. A right edge → B left edge with B *not* to the right, or coincident)
    // — the rail flips to a 4-turn detour: stub out of each box, then cross over on a
    // line that clears both boxes (midway between them when they don't overlap on the
    // minor axis, else around the nearer side). Mirrors Google Docs' elbow connectors.
    let stub = 18.0_f64;
    let elbow = |ax: Point, da: f64, bx: Point, db: f64, horiz: bool| -> Vec<Point> {
        let pt = |m: f64, n: f64| if horiz { Point::new(m, n) } else { Point::new(n, m) };
        let (am, an, bm, bn) = if horiz {
            (ax.x, ax.y, bx.x, bx.y)
        } else {
            (ax.y, ax.x, bx.y, bx.x)
        };
        if da * db < 0.0 && da * (bm - am) > 1e-6 {
            let mm = (am + bm) / 2.0; // crossing line sits in the gap between the exits
            return vec![pt(am, an), pt(mm, an), pt(mm, bn), pt(bm, bn)];
        }
        // Detour: a crossing line on the minor axis that clears both boxes.
        let (alo, ahi, blo, bhi) = if horiz {
            (a.y0, a.y1, b.y0, b.y1)
        } else {
            (a.x0, a.x1, b.x0, b.x1)
        };
        let midn = if alo < bhi && blo < ahi {
            // overlap on the minor axis → route around the nearer of the two far sides
            let (lo, hi) = (alo.min(blo) - stub, ahi.max(bhi) + stub);
            let mid = (an + bn) / 2.0;
            // Bias toward `lo` on a near-tie so the choice is stable against the ULP-level
            // float divergence between the native and wasm builds (a perfect tie must not
            // land on opposite sides per platform).
            if (mid - lo).abs() <= (hi - mid).abs() + 1e-6 { lo } else { hi }
        } else {
            (an + bn) / 2.0 // clear vertical band between them
        };
        let (pam, pbm) = (am + da * stub, bm + db * stub);
        vec![
            pt(am, an),
            pt(pam, an),
            pt(pam, midn),
            pt(pbm, midn),
            pt(pbm, bn),
            pt(bm, bn),
        ]
    };
    // Turn an orthogonal waypoint list into (path, tangents), trimming the first/last
    // segment back to the arrowhead base as needed. Works for 4- or 6-point rails.
    let build_rail = |pts: &[Point]| {
        let n = pts.len();
        let s = if arrow_start { trim(pts[0], pts[1]) } else { pts[0] };
        let e = if arrow_end { trim(pts[n - 1], pts[n - 2]) } else { pts[n - 1] };
        let mut d = format!("M{}", p(s));
        for q in &pts[1..n - 1] {
            d.push_str(&format!(" L{}", p(*q)));
        }
        d.push_str(&format!(" L{}", p(e)));
        (d, pts[0], pts[1], pts[n - 1], pts[n - 2])
    };
    // Outward sign of a forced anchor along one axis (0 ⇒ no component on that axis).
    let anchor_axis = |an: Option<Anchor>, horiz: bool| -> Option<f64> {
        an.and_then(|s| anchor_norm(s))
            .map(|(h, v)| if horiz { h } else { v })
            .filter(|s| s.abs() > 1e-9)
    };
    // The same orthogonal waypoints as `build_rail`, but with every corner rounded by a
    // quadratic fillet — the `curve` route's way of routing around when the endpoints
    // don't face with room, so it reads as a smooth cornered line rather than cutting
    // flat across the boxes. One *uniform* radius across all corners (so the fillets look
    // consistent): each corner may eat at most its fair share of the segments it touches
    // — half of a segment shared with another corner, all of one that ends at an endpoint
    // — and the tightest such share caps the shared radius.
    let round_rail = |pts: &[Point], r: f64| {
        let n = pts.len();
        // effective polyline with the arrowhead-trimmed ends baked in, so the end
        // segments' true lengths bound the radius too
        let mut q = pts.to_vec();
        if arrow_start {
            q[0] = trim(pts[0], pts[1]);
        }
        if arrow_end {
            q[n - 1] = trim(pts[n - 1], pts[n - 2]);
        }
        let mut rr = r;
        for i in 0..n - 1 {
            // how many of this segment's two ends are corners (endpoints 0 and n-1 aren't)
            let corners = (i != 0) as usize + (i != n - 2) as usize;
            if corners > 0 {
                rr = rr.min(q[i].distance(q[i + 1]) / corners as f64);
            }
        }
        let seg = |from: Point, to: Point| -> Point {
            let (dx, dy) = (to.x - from.x, to.y - from.y);
            let l = (dx * dx + dy * dy).sqrt();
            if l < 1e-9 { from } else { Point::new(from.x + dx / l * rr, from.y + dy / l * rr) }
        };
        let mut d = format!("M{}", p(q[0]));
        for i in 1..n - 1 {
            // approach the corner, then a quadratic through it to the departure point
            d.push_str(&format!(" L{} Q{} {}", p(seg(q[i], q[i - 1])), p(q[i]), p(seg(q[i], q[i + 1]))));
        }
        d.push_str(&format!(" L{}", p(q[n - 1])));
        (d, pts[0], pts[1], pts[n - 1], pts[n - 2])
    };

    // Each route yields its drawn path (already trimmed where an arrowhead sits)
    // plus the two endpoint tangents as (tip, adj) pairs: `adj` is the
    // neighbouring path point, so unit(tip − adj) is the direction the line
    // travels OUT of that end — the way its arrowhead points.
    let (d, start_tip, start_adj, end_tip, end_adj) =
        match node.attribute("route").unwrap_or("straight") {
            "x-major" => {
                let dir = if cb.x >= ca.x { 1.0 } else { -1.0 };
                let ax = match anchor_a {
                    Some(s) => anchor_pt(a, s),
                    None => Point::new(ca.x + dir * a.width() / 2.0, ca.y),
                };
                let bx = match anchor_b {
                    Some(s) => anchor_pt(b, s),
                    None => Point::new(cb.x - dir * b.width() / 2.0, cb.y),
                };
                // Outward x-direction: a forced anchor's own normal, else toward/away.
                let da = anchor_axis(anchor_a, true).unwrap_or(dir);
                let db = anchor_axis(anchor_b, true).unwrap_or(-dir);
                build_rail(&elbow(ax, da, bx, db, true))
            }
            "y-major" => {
                let dir = if cb.y >= ca.y { 1.0 } else { -1.0 };
                let ay = match anchor_a {
                    Some(s) => anchor_pt(a, s),
                    None => Point::new(ca.x, ca.y + dir * a.height() / 2.0),
                };
                let by = match anchor_b {
                    Some(s) => anchor_pt(b, s),
                    None => Point::new(cb.x, cb.y - dir * b.height() / 2.0),
                };
                let da = anchor_axis(anchor_a, false).unwrap_or(dir);
                let db = anchor_axis(anchor_b, false).unwrap_or(-dir);
                build_rail(&elbow(ay, da, by, db, false))
            }
            "curve" => {
                let horiz = (cb.x - ca.x).abs() >= (cb.y - ca.y).abs();
                // Each endpoint's exit point + outward direction: a forced side gives
                // its edge midpoint and normal; otherwise the box edge along the
                // dominant axis toward the other end (the original auto behaviour).
                let exit = |r: Rect, c: Point, toward: Point, forced: Option<Anchor>| -> (Point, (f64, f64)) {
                    if let Some(an) = forced {
                        let pt = anchor_pt(r, an);
                        // center has no edge normal — aim the tangent at the other endpoint
                        let dir = anchor_norm(an).unwrap_or_else(|| {
                            let (dx, dy) = (toward.x - pt.x, toward.y - pt.y);
                            let l = (dx * dx + dy * dy).sqrt();
                            if l < 1e-9 { (0.0, 0.0) } else { (dx / l, dy / l) }
                        });
                        return (pt, dir);
                    }
                    if horiz {
                        let d = if toward.x >= c.x { 1.0 } else { -1.0 };
                        (Point::new(c.x + d * r.width() / 2.0, c.y), (d, 0.0))
                    } else {
                        let d = if toward.y >= c.y { 1.0 } else { -1.0 };
                        (Point::new(c.x, c.y + d * r.height() / 2.0), (0.0, d))
                    }
                };
                let (a0, an) = exit(a, ca, cb, anchor_a);
                let (b0, bn) = exit(b, cb, ca, anchor_b);
                let unit = |dx: f64, dy: f64| {
                    let l = (dx * dx + dy * dy).sqrt();
                    if l < 1e-9 { (0.0, 0.0) } else { (dx / l, dy / l) }
                };
                // When the other anchor is *behind* the exit (dot < 0 — overlapping or
                // back-to-back boxes, the case the elbow routes flip for), a single cubic
                // can't both bow around and still meet the far edge along its normal; it
                // ends up cutting across the box. Route it as a rounded elbow instead (the
                // same rail path, corners filleted) so it reads as a smooth cornered line.
                let (cax, cay) = unit(b0.x - a0.x, b0.y - a0.y);
                let (cbx, cby) = unit(a0.x - b0.x, a0.y - b0.y);
                let behind = an.0 * cax + an.1 * cay < 0.0 || bn.0 * cbx + bn.1 * cby < 0.0;
                if behind {
                    let hz = an.0.abs() >= an.1.abs();
                    let da = (if hz { an.0 } else { an.1 }).signum();
                    let db = (if hz { bn.0 } else { bn.1 }).signum();
                    round_rail(&elbow(a0, da, b0, db, hz), 22.0)
                } else {
                    // Facing with room: a single smooth cubic. Tilt each handle from the
                    // exit toward the other anchor, so a same-side pair reads as a leaf
                    // (pointed at both ends) rather than bulging into a half-circle.
                    let da = unit(an.0 + cax * 0.7, an.1 + cay * 0.7);
                    let db = unit(bn.0 + cbx * 0.7, bn.1 + cby * 0.7);
                    // fixed bow; only clamped down for connectors shorter than the bulge
                    // itself, so a short link can't loop past its own endpoints
                    let dist = ((b0.x - a0.x).powi(2) + (b0.y - a0.y).powi(2)).sqrt();
                    let k = bulge.min(dist);
                    let c1 = Point::new(a0.x + da.0 * k, a0.y + da.1 * k);
                    let c2 = Point::new(b0.x + db.0 * k, b0.y + db.1 * k);
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
            }
            _ => {
                // straight: clip the center-to-center line to each box edge (or the
                // forced side's midpoint)
                let a0 = match anchor_a {
                    Some(s) => anchor_pt(a, s),
                    None => edge(a, cb),
                };
                let b0 = match anchor_b {
                    Some(s) => anchor_pt(b, s),
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
        &["from", "to", "from-point", "to-point", "route", "arrow", "arrow-size", "bulge", "fill"],
    );
    out.push_str(&pos_attr(node, ctx));
    out.push_str(&format!(" fill=\"none\" d=\"{d}\"/>"));
    out.push_str(&heads); // arrowheads paint on top of the line
}
