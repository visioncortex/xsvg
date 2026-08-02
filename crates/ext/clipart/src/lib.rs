//! `clipart` — the first official xsvg extension (docs/Extensibility.md).
//!
//! Procedural diagram clipart: an outlined thick arrow and a speech-bubble
//! container. It is deliberately built on the *public* plugin ABI rather than
//! wired into the compiler, because an official extension that could reach past
//! the seam would stop being evidence that the seam works.
//!
//! Everything is emitted as explicit coordinates — no `transform` — so a shape
//! stays harvestable as geometry by `in="#id"`, `<x:boolean>`, and `<x:warp>`.

use xsvg_plugin::{esc, num, ElementSpec, ParamEnum, Params, Plugin, Request};

struct Clipart;

impl Plugin for Clipart {
    const NS: &'static str = "https://xsvg.visioncortex.org/ext/clipart";
    const NAME: &'static str = "clipart";
    const VERSION: &'static str = "0.1.0";

    fn elements() -> &'static [ElementSpec] {
        &[Arrow::SPEC, Callout::SPEC]
    }

    fn generate(tag: &str, req: &Request) -> Result<String, String> {
        match tag {
            "arrow" => arrow(&Arrow::from_attrs(req), req),
            "callout" => callout(&Callout::from_attrs(req), req),
            other => Err(format!("<{other}> is not a clipart element")),
        }
    }
}

/// Which ends of an arrow carry a head.
#[derive(ParamEnum, PartialEq, Default)]
enum Heads {
    #[default]
    End,
    Start,
    Both,
    None,
}

/// Which edge a callout's tail leaves from.
#[derive(ParamEnum, PartialEq, Default)]
enum Tail {
    None,
    Left,
    Right,
    Top,
    #[default]
    Bottom,
}

#[derive(Params)]
#[element(
    name = "arrow",
    doc = "An outlined thick arrow. Either give it two endpoints (x1,y1,x2,y2) or an origin plus length and angle."
)]
struct Arrow {
    #[param(default = 0.0)]
    x1: f64,
    #[param(default = 0.0)]
    y1: f64,
    /// Endpoint form. Absent means the origin/length/angle form is used instead,
    /// which is why these are optional rather than defaulted.
    x2: Option<f64>,
    y2: Option<f64>,
    /// Aliases for `x1`/`y1`. Declared, so the panel offers them and they cannot
    /// quietly become attributes that work but nothing knows about.
    #[param(doc = "alias for x1")]
    x: Option<f64>,
    #[param(doc = "alias for y1")]
    y: Option<f64>,
    #[param(default = 100.0, min = 0.0, max = 2000.0)]
    length: f64,
    #[param(default = 0.0, doc = "degrees clockwise from +x")]
    angle: f64,
    #[param(default = 16.0, min = 0.0, max = 200.0)]
    thickness: f64,
    #[param(min = 0.0, max = 400.0, doc = "defaults to 2.5x thickness")]
    head_width: Option<f64>,
    #[param(min = 0.0, max = 400.0, doc = "defaults to head-width")]
    head_length: Option<f64>,
    heads: Heads,
    #[param(
        default = 0.0,
        min = 0.0,
        max = 1.0,
        step = 0.05,
        doc = "chevron notch depth at the tail, as a fraction of head-length"
    )]
    notch: f64,
    #[param(default = 0.0, min = 0.0, max = 100.0, doc = "corner rounding")]
    radius: f64,
    #[param(color, default = "currentColor")]
    fill: String,
    #[param(color, doc = "outline colour")]
    stroke: Option<String>,
    #[param(default = 0.0, min = 0.0, max = 40.0, doc = "outline width; needs stroke to be visible")]
    outline: f64,
}

#[derive(Params)]
#[element(
    name = "callout",
    doc = "A rounded speech bubble sized to its children, with an optional tail.",
    children = "container"
)]
struct Callout {
    #[param(default = 12.0, min = 0.0, max = 200.0)]
    pad: f64,
    #[param(default = 8.0, min = 0.0, max = 100.0)]
    radius: f64,
    tail: Tail,
    #[param(default = 14.0, min = 0.0, max = 100.0)]
    tail_size: f64,
    #[param(default = 0.5, min = 0.0, max = 1.0, step = 0.05, doc = "0..1 along the tailed edge")]
    tail_offset: f64,
    #[param(color, default = "#fff")]
    fill: String,
    #[param(color, doc = "outline colour")]
    stroke: Option<String>,
    #[param(default = 0.0, min = 0.0, max = 40.0)]
    outline: f64,
}

xsvg_plugin::export_plugin!(Clipart);

/// A point in user space.
type P = (f64, f64);

/// Paint attributes shared by every shape here.
fn paint(fill: &str, stroke: Option<&str>, width: f64) -> String {
    let mut s = format!(r#" fill="{}""#, esc(fill));
    // an outline only paints if there is something to paint it with
    if let Some(stroke) = stroke {
        s.push_str(&format!(r#" stroke="{}""#, esc(stroke)));
        if width > 0.0 {
            s.push_str(&format!(
                r#" stroke-width="{}" stroke-linejoin="round""#,
                num(width)
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
fn arrow(p: &Arrow, _req: &Request) -> Result<String, String> {
    let (x1, y1) = (p.x.unwrap_or(p.x1), p.y.unwrap_or(p.y1));
    let (x2, y2) = match (p.x2, p.y2) {
        (None, None) => {
            let a = p.angle.to_radians();
            (x1 + p.length * a.cos(), y1 + p.length * a.sin())
        }
        (x2, y2) => (x2.unwrap_or(x1), y2.unwrap_or(y1)),
    };

    let (dx, dy) = (x2 - x1, y2 - y1);
    let len = (dx * dx + dy * dy).sqrt();
    let t = p.thickness.max(0.0);
    if !len.is_finite() || len <= 0.0 || t <= 0.0 {
        return Err("arrow has no length or no thickness".to_string());
    }

    let hw = p.head_width.unwrap_or(t * 2.5).max(t);
    let hl = p.head_length.unwrap_or(hw).max(0.0);
    let (start_head, end_head) = match p.heads {
        Heads::None => (false, false),
        Heads::Start => (true, false),
        Heads::Both => (true, true),
        Heads::End => (false, true),
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
    let notch = p.notch.clamp(0.0, 1.0) * hl;
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

    let radii = uniform(&placed, p.radius.max(0.0));
    Ok(format!(
        r#"<path{} d="{}"/>"#,
        paint(&p.fill, p.stroke.as_deref(), p.outline),
        polygon(&placed, &radii)
    ))
}

/// `<clip:callout>` — a bubble sized to its children, drawn behind them.
///
/// This is the container case: the compiler has already lowered the children and
/// measured what they paint, so the bubble is derived from real geometry rather
/// than from author-supplied dimensions.
fn callout(p: &Callout, req: &Request) -> Result<String, String> {
    let Some(b) = req.children_bbox() else {
        return Err("callout has no measurable children to wrap".to_string());
    };
    let pad = p.pad.max(0.0);
    let (x0, y0) = (b[0] - pad, b[1] - pad);
    let (x1, y1) = (b[0] + b[2] + pad, b[1] + b[3] + pad);
    let r = p.radius.max(0.0);

    let ts = p.tail_size.max(0.0);
    let off = p.tail_offset.clamp(0.0, 1.0);

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
        edge((x0, y0), (x1, y0), p.tail == Tail::Top);
        edge((x1, y0), (x1, y1), p.tail == Tail::Right);
        edge((x1, y1), (x0, y1), p.tail == Tail::Bottom);
        edge((x0, y1), (x0, y0), p.tail == Tail::Left);
    }

    let mut out = format!(
        r#"<path{} d="{}"/>"#,
        paint(&p.fill, p.stroke.as_deref(), p.outline),
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

    /// Build a request, then the declared params the emitter actually takes —
    /// so a test exercises the same parsing the ABI does.
    fn arrow_of(pairs: &[(&str, &str)]) -> Result<String, String> {
        let r = req(pairs);
        arrow(&Arrow::from_attrs(&r), &r)
    }

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
        let out = arrow_of(&[
            ("x1", "0"),
            ("y1", "0"),
            ("x2", "100"),
            ("y2", "0"),
            ("thickness", "20"),
            ("head-width", "50"),
            ("head-length", "30"),
        ])
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
        let out = arrow_of(&[
            ("x1", "0"),
            ("y1", "0"),
            ("length", "100"),
            ("angle", "90"),
            ("thickness", "20"),
        ])
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
        let out = arrow_of(&[
            ("x1", "0"),
            ("y1", "0"),
            ("x2", "20"),
            ("y2", "0"),
            ("thickness", "6"),
            ("head-length", "30"),
            ("heads", "both"),
        ])
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
            assert!(arrow_of(&bad).is_err(), "{bad:?}");
        }
        // a nonsense attribute value falls back to the default rather than NaN
        let out = arrow_of(&[("length", "banana"), ("thickness", "oops")]).unwrap();
        assert!(!out.contains("NaN") && !out.contains("inf"), "{out}");
    }

    #[test]
    fn rounding_keeps_the_outline_inside_the_unrounded_one() {
        let plain = arrow_of(&[("length", "100"), ("thickness", "20")]).unwrap();
        let round = arrow_of(&[
            ("length", "100"),
            ("thickness", "20"),
            ("radius", "4"),
        ])
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
        let out = callout(&Callout::from_attrs(&r), &r).unwrap();
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
        let d = callout(&Callout::from_attrs(&r), &r).unwrap();
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
        let d = callout(&Callout::from_attrs(&r), &r).unwrap();
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
        let out = callout(&Callout::from_attrs(&r), &r).unwrap();
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
        let empty = req(&[]);
        assert!(callout(&Callout::from_attrs(&empty), &empty).is_err());
    }

    #[test]
    fn paint_attributes_are_escaped() {
        let out = arrow_of(&[
            ("length", "10"),
            ("thickness", "2"),
            ("fill", "\"><script>"),
        ])
        .unwrap();
        assert!(!out.contains("<script"), "{out}");
        assert!(out.contains("&quot;&gt;&lt;script&gt;"), "{out}");
    }

    #[test]
    fn the_manifest_is_generated_from_the_declarations() {
        let text = xsvg_plugin::manifest::<Clipart>();
        let m = xsvg_plugin::json::parse(&text).expect("manifest parses");
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

    #[test]
    fn every_attribute_the_code_reads_is_declared() {
        // The hand-written manifest this replaced omitted `x` and `y` — readable
        // aliases for x1/y1 that no property panel could see. With the struct as
        // the single source there is no field to read that isn't described, so
        // this pins the specific bug rather than the general property.
        let names: Vec<&str> = Arrow::SPEC.params.iter().map(|p| p.name).collect();
        for n in ["x", "y", "x1", "y1", "x2", "y2", "head-width", "head-length"] {
            assert!(names.contains(&n), "{n} is not declared: {names:?}");
        }
        // and the alias really is an alias
        let d = arrow_of(&[("x", "5"), ("y", "5"), ("length", "10"), ("thickness", "2")]).unwrap();
        assert!(d.contains("M5 4"), "x/y position the arrow: {d}");
    }
}
