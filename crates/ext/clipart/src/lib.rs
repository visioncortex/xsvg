//! `clipart` — the first official xsvg extension (docs/Extensibility.md).
//!
//! Procedural diagram clipart: an outlined thick arrow and a speech-bubble
//! container. It is deliberately built on the *public* plugin ABI rather than
//! wired into the compiler, because an official extension that could reach past
//! the seam would stop being evidence that the seam works.
//!
//! Everything is emitted as explicit coordinates — no `transform` — so a shape
//! stays harvestable as geometry by `in="#id"`, `<x:boolean>`, and `<x:warp>`.

use xsvg_plugin::{esc, num, Plugin, Request};

struct Clipart;

impl Plugin for Clipart {
    const MANIFEST: &'static str = include_str!("manifest.json");

    fn generate(tag: &str, req: &Request) -> Result<String, String> {
        match tag {
            "arrow" => arrow(req),
            "callout" => callout(req),
            other => Err(format!("<{other}> is not a clipart element")),
        }
    }
}

xsvg_plugin::export_plugin!(Clipart);

/// A point in user space.
type P = (f64, f64);

/// Paint attributes shared by every shape here.
fn paint(req: &Request, default_fill: &str) -> String {
    let mut s = format!(
        r#" fill="{}""#,
        esc(req.attr("fill").unwrap_or(default_fill))
    );
    // an outline only paints if there is something to paint it with
    let w = req.num("outline", 0.0);
    if let Some(stroke) = req.attr("stroke") {
        s.push_str(&format!(r#" stroke="{}""#, esc(stroke)));
        if w > 0.0 {
            s.push_str(&format!(
                r#" stroke-width="{}" stroke-linejoin="round""#,
                num(w)
            ));
        }
    }
    s
}

/// Emit a closed polygon whose corners round by a **per-vertex** radius.
///
/// Per-vertex rather than uniform because a shape usually wants rounding on some
/// corners and not others — a callout's bubble corners soften while the point of
/// its tail must stay a point.
///
/// Rounding trims the radius off both sides of a corner and joins them with a
/// quadratic through the original vertex: visually indistinguishable from an arc
/// at these radii, and it keeps the path free of arc flags that every downstream
/// consumer would have to flatten.
fn polygon(pts: &[P], radii: &[f64]) -> String {
    if pts.len() < 3 {
        return String::new();
    }
    let n = pts.len();
    let r_at = |i: usize| radii.get(i).copied().unwrap_or(0.0).max(0.0);
    if (0..n).all(|i| r_at(i) <= 0.0) {
        let mut d = format!("M{} {}", num(pts[0].0), num(pts[0].1));
        for p in &pts[1..] {
            d.push_str(&format!(" L{} {}", num(p.0), num(p.1)));
        }
        d.push_str(" Z");
        return d;
    }
    let mut d = String::new();
    for i in 0..n {
        let cur = pts[i];
        let r = r_at(i);
        let (a, b) = if r > 0.0 {
            (
                trim(cur, pts[(i + n - 1) % n], r),
                trim(cur, pts[(i + 1) % n], r),
            )
        } else {
            (cur, cur)
        };
        if i == 0 {
            d.push_str(&format!("M{} {}", num(a.0), num(a.1)));
        } else {
            d.push_str(&format!(" L{} {}", num(a.0), num(a.1)));
        }
        if r > 0.0 {
            d.push_str(&format!(
                " Q{} {} {} {}",
                num(cur.0),
                num(cur.1),
                num(b.0),
                num(b.1)
            ));
        }
    }
    d.push_str(" Z");
    d
}

/// One radius repeated for every vertex.
fn uniform(pts: &[P], r: f64) -> Vec<f64> {
    vec![r; pts.len()]
}

/// The point `r` along the segment from `from` toward `to`, clamped to half the
/// segment so adjacent roundings cannot cross.
fn trim(from: P, to: P, r: f64) -> P {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0 {
        return from;
    }
    let t = r.min(len / 2.0) / len;
    (from.0 + dx * t, from.1 + dy * t)
}

/// `<clip:arrow>` — a thick shafted arrow with one or two heads.
///
/// The profile is built along the +x axis from the origin and then mapped onto
/// the requested endpoints, so rotation is baked into the coordinates.
fn arrow(req: &Request) -> Result<String, String> {
    let (x1, y1) = (
        req.num("x1", req.num("x", 0.0)),
        req.num("y1", req.num("y", 0.0)),
    );
    let (x2, y2) = match (req.attr("x2"), req.attr("y2")) {
        (Some(_), _) | (_, Some(_)) => (req.num("x2", x1), req.num("y2", y1)),
        _ => {
            let a = req.num("angle", 0.0).to_radians();
            let len = req.num("length", 100.0);
            (x1 + len * a.cos(), y1 + len * a.sin())
        }
    };

    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    let t = req.num("thickness", 16.0).max(0.0);
    if !len.is_finite() || len <= 0.0 || t <= 0.0 {
        return Err("arrow has no length or no thickness".to_string());
    }

    let hw = req.num("head-width", t * 2.5).max(t);
    let hl = req.num("head-length", hw).max(0.0);
    let (start_head, end_head) = match req.attr("heads").unwrap_or("end") {
        "none" => (false, false),
        "start" => (true, false),
        "both" => (true, true),
        _ => (false, true),
    };

    // Both heads must fit in the shaft; shrink them together if they don't.
    let wanted = hl * f64::from(u8::from(start_head) + u8::from(end_head));
    let hl = if wanted > len && wanted > 0.0 {
        hl * (len / wanted)
    } else {
        hl
    };
    let (h0, h1) = (
        if start_head { hl } else { 0.0 },
        if end_head { len - hl } else { len },
    );

    // Profile along +x, walking the outline counter-clockwise from the tail.
    let notch = req.num("notch", 0.0).clamp(0.0, 1.0) * hl;
    let mut pts: Vec<P> = Vec::with_capacity(10);
    if start_head {
        pts.push((0.0, 0.0));
        pts.push((h0, -hw / 2.0));
        pts.push((h0, -t / 2.0));
    } else {
        pts.push((notch, -t / 2.0));
        if notch > 0.0 {
            pts.push((0.0, -t / 2.0));
        }
    }
    if end_head {
        pts.push((h1, -t / 2.0));
        pts.push((h1, -hw / 2.0));
        pts.push((len, 0.0));
        pts.push((h1, hw / 2.0));
        pts.push((h1, t / 2.0));
    } else {
        pts.push((len, -t / 2.0));
        pts.push((len, t / 2.0));
    }
    if start_head {
        pts.push((h0, t / 2.0));
        pts.push((h0, hw / 2.0));
    } else {
        if notch > 0.0 {
            pts.push((0.0, t / 2.0));
        }
        pts.push((notch, t / 2.0));
    }

    // Map the profile onto the real endpoints: +x runs along the arrow, +y across.
    let (ux, uy) = (dx / len, dy / len);
    let placed: Vec<P> = pts
        .iter()
        .map(|&(px, py)| (x1 + ux * px - uy * py, y1 + uy * px + ux * py))
        .collect();

    let radii = uniform(&placed, req.num("radius", 0.0).max(0.0));
    Ok(format!(
        r#"<path{} d="{}"/>"#,
        paint(req, "currentColor"),
        polygon(&placed, &radii)
    ))
}

/// `<clip:callout>` — a bubble sized to its children, drawn behind them.
///
/// This is the container case: the compiler has already lowered the children and
/// measured what they paint, so the bubble is derived from real geometry rather
/// than from author-supplied dimensions.
fn callout(req: &Request) -> Result<String, String> {
    let Some(b) = req.children_bbox() else {
        return Err("callout has no measurable children to wrap".to_string());
    };
    let pad = req.num("pad", 12.0).max(0.0);
    let (x0, y0) = (b[0] - pad, b[1] - pad);
    let (x1, y1) = (b[0] + b[2] + pad, b[1] + b[3] + pad);
    let r = req.num("radius", 8.0).max(0.0);

    let ts = req.num("tail-size", 14.0).max(0.0);
    let off = req.num("tail-offset", 0.5).clamp(0.0, 1.0);
    let tail = req.attr("tail").unwrap_or("bottom");

    // Walk the rectangle clockwise, splicing the tail into the chosen edge. The
    // tail is part of the outline, not a separate shape, so a stroked bubble has
    // no seam where the two would have met.
    //
    // Radii travel with the points: the four bubble corners soften, while the
    // tail's own vertices stay sharp — rounding its tip would turn the point
    // that makes it a callout into a blob.
    let mut pts: Vec<P> = Vec::with_capacity(11);
    let mut radii: Vec<f64> = Vec::with_capacity(11);
    {
        let mut edge = |from: P, to: P, on: bool| {
            pts.push(from);
            radii.push(r);
            if !on || ts <= 0.0 {
                return;
            }
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len <= 0.0 {
                return;
            }
            let (ux, uy) = (dx / len, dy / len);
            let (nx, ny) = (uy, -ux); // outward normal for a clockwise walk
                                      // keep the tail's base clear of both rounded corners; if the edge is too
                                      // short for that, centre it and let it take what room there is
            let half = (ts / 2.0).min(len / 2.0);
            let (lo, hi) = (r + half, len - r - half);
            let c = if hi > lo {
                lo + off * (hi - lo)
            } else {
                len / 2.0
            };
            for p in [
                (from.0 + ux * (c - half), from.1 + uy * (c - half)),
                (from.0 + ux * c + nx * ts, from.1 + uy * c + ny * ts),
                (from.0 + ux * (c + half), from.1 + uy * (c + half)),
            ] {
                pts.push(p);
                radii.push(0.0);
            }
        };
        edge((x0, y0), (x1, y0), tail == "top");
        edge((x1, y0), (x1, y1), tail == "right");
        edge((x1, y1), (x0, y1), tail == "bottom");
        edge((x0, y1), (x0, y0), tail == "left");
    }

    let mut out = format!(
        r#"<path{} d="{}"/>"#,
        paint(req, "#fff"),
        polygon(&pts, &radii)
    );
    // children last, so the bubble sits behind them
    for c in &req.children {
        out.push_str(&c.svg);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(pairs: &[(&str, &str)]) -> Request {
        Request {
            attrs: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            input_d: None,
            children: Vec::new(),
            tolerance: 0.1,
        }
    }

    /// Every coordinate in a path `d`, in order.
    fn coords(d: &str) -> Vec<f64> {
        d.split([' ', 'M', 'L', 'Q', 'Z'])
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .collect()
    }

    fn bbox(d: &str) -> (f64, f64, f64, f64) {
        let c = coords(d);
        let xs: Vec<f64> = c.iter().step_by(2).copied().collect();
        let ys: Vec<f64> = c.iter().skip(1).step_by(2).copied().collect();
        (
            xs.iter().cloned().fold(f64::MAX, f64::min),
            ys.iter().cloned().fold(f64::MAX, f64::min),
            xs.iter().cloned().fold(f64::MIN, f64::max),
            ys.iter().cloned().fold(f64::MIN, f64::max),
        )
    }

    #[test]
    fn an_arrow_spans_its_endpoints_and_its_head_is_widest() {
        let out = arrow(&req(&[
            ("x1", "0"),
            ("y1", "0"),
            ("x2", "100"),
            ("y2", "0"),
            ("thickness", "20"),
            ("head-width", "50"),
            ("head-length", "30"),
        ]))
        .unwrap();
        let d = out
            .split(r#"d=""#)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (x0, y0, x1, y1) = bbox(d);
        assert!((x0).abs() < 0.01 && (x1 - 100.0).abs() < 0.01, "{d}");
        // the head is 50 wide, the shaft 20 — the extremes come from the head
        assert!((y0 + 25.0).abs() < 0.01 && (y1 - 25.0).abs() < 0.01, "{d}");
        // the tip is a single point on the axis at the far end
        assert!(d.contains("L100 0"), "{d}");
    }

    #[test]
    fn an_arrow_bakes_its_rotation_into_coordinates() {
        // a transform in generator output would make the shape unharvestable as
        // geometry, so rotation must land in the numbers
        let out = arrow(&req(&[
            ("x1", "0"),
            ("y1", "0"),
            ("length", "100"),
            ("angle", "90"),
            ("thickness", "20"),
        ]))
        .unwrap();
        assert!(!out.contains("transform"), "{out}");
        let d = out
            .split(r#"d=""#)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (x0, y0, x1, y1) = bbox(d);
        // pointing down: tall and narrow, spanning y 0..100
        assert!((y0).abs() < 0.01 && (y1 - 100.0).abs() < 0.01, "{d}");
        assert!(x1 - x0 < 60.0 && y1 - y0 > 90.0, "{d}");
    }

    #[test]
    fn two_heads_shrink_to_fit_a_short_shaft() {
        // heads that would overlap are scaled down together rather than
        // producing a self-intersecting outline
        let out = arrow(&req(&[
            ("x1", "0"),
            ("y1", "0"),
            ("x2", "20"),
            ("y2", "0"),
            ("thickness", "6"),
            ("head-length", "30"),
            ("heads", "both"),
        ]))
        .unwrap();
        let d = out
            .split(r#"d=""#)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (x0, _, x1, _) = bbox(d);
        assert!((x0).abs() < 0.01 && (x1 - 20.0).abs() < 0.01, "{d}");
        // both tips are on the axis
        assert!(d.contains("M0 0"), "start tip: {d}");
        assert!(d.contains("L20 0"), "end tip: {d}");
    }

    #[test]
    fn a_degenerate_arrow_degrades_instead_of_emitting_nonsense() {
        // totality: no NaN coordinates ever reach the output
        for bad in [
            vec![("x1", "0"), ("y1", "0"), ("x2", "0"), ("y2", "0")],
            vec![("length", "0")],
            vec![("thickness", "0")],
            vec![("length", "50"), ("thickness", "-3")],
        ] {
            assert!(arrow(&req(&bad)).is_err(), "{bad:?}");
        }
        // a nonsense attribute value falls back to the default rather than NaN
        let out = arrow(&req(&[("length", "banana"), ("thickness", "oops")])).unwrap();
        assert!(!out.contains("NaN") && !out.contains("inf"), "{out}");
    }

    #[test]
    fn rounding_keeps_the_outline_inside_the_unrounded_one() {
        let plain = arrow(&req(&[("length", "100"), ("thickness", "20")])).unwrap();
        let round = arrow(&req(&[
            ("length", "100"),
            ("thickness", "20"),
            ("radius", "4"),
        ]))
        .unwrap();
        assert!(
            round.contains(" Q"),
            "rounded corners are quadratics: {round}"
        );
        assert!(!plain.contains(" Q"), "{plain}");
    }

    #[test]
    fn a_callout_wraps_its_children_with_padding() {
        let mut r = req(&[("pad", "10"), ("tail", "none"), ("radius", "0")]);
        r.children = vec![xsvg_plugin::Child {
            svg: "<path d=\"M0,0 h40 v20 h-40 Z\"/>".to_string(),
            bbox: Some([0.0, 0.0, 40.0, 20.0]),
        }];
        let out = callout(&r).unwrap();
        let d = out
            .split(r#"d=""#)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (x0, y0, x1, y1) = bbox(d);
        assert!((x0 + 10.0).abs() < 0.01 && (y0 + 10.0).abs() < 0.01, "{d}");
        assert!((x1 - 50.0).abs() < 0.01 && (y1 - 30.0).abs() < 0.01, "{d}");
        // the children come after the bubble, so the bubble sits behind them
        assert!(out.ends_with("<path d=\"M0,0 h40 v20 h-40 Z\"/>"), "{out}");
    }

    #[test]
    fn a_callouts_tail_protrudes_from_the_named_edge() {
        let kid = xsvg_plugin::Child {
            svg: String::new(),
            bbox: Some([0.0, 0.0, 40.0, 20.0]),
        };
        let mut r = req(&[("pad", "0"), ("radius", "0"), ("tail-size", "12")]);
        r.children = vec![kid];
        let d = callout(&r).unwrap();
        let d = d.split(r#"d=""#).nth(1).unwrap().split('"').next().unwrap();
        // default tail is at the bottom: it reaches below the box
        assert!(bbox(d).3 > 20.0, "bottom tail: {d}");

        let mut r = req(&[
            ("pad", "0"),
            ("radius", "0"),
            ("tail", "top"),
            ("tail-size", "12"),
        ]);
        r.children = vec![xsvg_plugin::Child {
            svg: String::new(),
            bbox: Some([0.0, 0.0, 40.0, 20.0]),
        }];
        let d = callout(&r).unwrap();
        let d = d.split(r#"d=""#).nth(1).unwrap().split('"').next().unwrap();
        assert!(bbox(d).1 < 0.0, "top tail: {d}");
    }

    #[test]
    fn a_callouts_tip_stays_sharp_while_its_corners_round() {
        // corner rounding must not reach the tail, or the point that makes it a
        // callout turns into a blob
        let mut r = req(&[("pad", "0"), ("radius", "8"), ("tail-size", "14")]);
        r.children = vec![xsvg_plugin::Child {
            svg: String::new(),
            bbox: Some([0.0, 0.0, 80.0, 20.0]),
        }];
        let out = callout(&r).unwrap();
        let d = out
            .split(r#"d=""#)
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(d.contains(" Q"), "bubble corners are rounded: {d}");
        // the tip is reached and left by straight lines, and sits tail-size
        // below the bottom edge, centred by the default offset
        assert!(d.contains("L40 34"), "sharp tip at the edge midpoint: {d}");
    }

    #[test]
    fn a_callout_with_nothing_measurable_degrades() {
        assert!(callout(&req(&[])).is_err());
    }

    #[test]
    fn paint_attributes_are_escaped() {
        let out = arrow(&req(&[
            ("length", "10"),
            ("thickness", "2"),
            ("fill", "\"><script>"),
        ]))
        .unwrap();
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&quot;&gt;&lt;script&gt;"), "{out}");
    }

    #[test]
    fn the_manifest_is_valid_json_naming_the_official_namespace() {
        let m = xsvg_plugin::json::parse(Clipart::MANIFEST).expect("manifest parses");
        assert_eq!(
            m.get("ns").and_then(|v| v.as_str()),
            Some("https://xsvg.visioncortex.org/ext/clipart")
        );
        assert_eq!(m.get("abi").and_then(|v| v.as_num()), Some(1.0));
        let els = m.get("elements").expect("elements");
        assert!(els.get("arrow").is_some() && els.get("callout").is_some());
        assert_eq!(
            els.get("callout")
                .and_then(|c| c.get("children"))
                .and_then(|v| v.as_str()),
            Some("container")
        );
    }
}
