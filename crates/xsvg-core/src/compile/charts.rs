//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// One pie/donut sector as SVG path data: outer radius `ro`, inner `ri` (0 = full
/// pie), proportional angles `a0`/`a1` (degrees, clockwise) about `(cx, cy)`, with
/// a **constant-width** `gap` between slices. The gap is a perpendicular offset of
/// each straight edge (not an angular inset), so adjacent slices leave a parallel
/// channel that points at the centre — instead of a wedge that flares outward. A
/// near-full span emits a circle/ring (avoiding SVG's degenerate 360° arc); a
/// slice the gap fully consumes returns "".
pub(super) fn sector_path(cx: f64, cy: f64, ro: f64, ri: f64, a0deg: f64, a1deg: f64, gap: f64) -> String {
    use std::f64::consts::{PI, TAU};
    let d2r = PI / 180.0;
    if a1deg - a0deg >= 359.999 {
        // full circle / ring (outer CW + inner CCW → nonzero leaves the ring)
        let ring = |r: f64, sweep: u8| {
            format!(
                "M{},{} A{r},{r} 0 1 {sweep} {},{} A{r},{r} 0 1 {sweep} {},{} Z",
                fmt(cx - r),
                fmt(cy),
                fmt(cx + r),
                fmt(cy),
                fmt(cx - r),
                fmt(cy),
                r = fmt(r),
                sweep = sweep
            )
        };
        return if ri > 0.0 {
            format!("{} {}", ring(ro, 1), ring(ri, 0))
        } else {
            ring(ro, 1)
        };
    }
    let h = (gap / 2.0).max(0.0);
    let (a0, a1) = (a0deg * d2r, a1deg * d2r);
    let (u0, u1) = ((a0.cos(), a0.sin()), (a1.cos(), a1.sin()));
    let n0 = (-u0.1, u0.0); // perpendicular into the slice, off the leading edge
    let n1 = (u1.1, -u1.0); // …off the trailing edge
                            // an edge point at signed radial distance `t`, shifted `h` off the radial
    let mk =
        |t: f64, u: (f64, f64), n: (f64, f64)| (cx + t * u.0 + h * n.0, cy + t * u.1 + h * n.1);
    let to = (ro * ro - h * h).max(0.0).sqrt(); // radial reach of the outer end
    let ti = if ri > h {
        (ri * ri - h * h).sqrt()
    } else {
        0.0
    };
    let (o0, o1) = (mk(to, u0, n0), mk(to, u1, n1));
    let (i0, i1) = (mk(ti, u0, n0), mk(ti, u1, n1));
    let ang = |p: (f64, f64)| (p.1 - cy).atan2(p.0 - cx);
    let pos = |mut d: f64| {
        while d < 0.0 {
            d += TAU;
        }
        d
    };
    let span_o = pos(ang(o1) - ang(o0));
    if span_o <= 1e-6 {
        return String::new(); // the gap consumed the slice
    }
    let large_o = u8::from(span_o > PI);
    if ri > h {
        // inner arc runs i1 → i0 the other way (sweep 0), so its span is i1−i0
        let large_i = u8::from(pos(ang(i1) - ang(i0)) > PI);
        format!(
            "M{},{} L{},{} A{ro},{ro} 0 {large_o} 1 {},{} L{},{} A{ri},{ri} 0 {large_i} 0 {},{} Z",
            fmt(i0.0),
            fmt(i0.1),
            fmt(o0.0),
            fmt(o0.1),
            fmt(o1.0),
            fmt(o1.1),
            fmt(i1.0),
            fmt(i1.1),
            fmt(i0.0),
            fmt(i0.1),
            ro = fmt(ro),
            ri = fmt(ri),
            large_o = large_o,
            large_i = large_i
        )
    } else {
        // full pie: the offset edges converge to a tiny notch (distance h) near
        // the centre rather than a single point
        format!(
            "M{},{} L{},{} A{ro},{ro} 0 {large_o} 1 {},{} L{},{} Z",
            fmt(i0.0),
            fmt(i0.1),
            fmt(o0.0),
            fmt(o0.1),
            fmt(o1.0),
            fmt(o1.1),
            fmt(i1.0),
            fmt(i1.1),
            ro = fmt(ro),
            large_o = large_o
        )
    }
}

/// `<x:pie>` (§6.17): a pie/donut chart. Each `<x:slice>` gets an angular share
/// from `value` (default 1 → equal), an outer radius from the pie `r` overridable
/// per slice (`r` absolute or `grow` factor), and an `explode` offset out along
/// its bisector. `inner-radius` makes a donut; `gap` is the constant-width channel
/// between slices (a perpendicular edge offset, so it stays parallel, not a wedge);
/// `start` sets the first slice's angle (default −90°, top). Each slice bakes to
/// one `<path>` sector carrying its own source range.
pub(super) fn emit_pie(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let cx = attr_num(node, "cx", 0.0);
    let cy = attr_num(node, "cy", 0.0);
    let r = attr_num(node, "r", 100.0).max(0.0);
    let inner = attr_num(node, "inner-radius", 0.0).max(0.0).min(r);
    let start = attr_num(node, "start", -90.0);
    let gap = attr_num(node, "gap", 0.0).max(0.0);
    let pie_stroke = node.attribute("stroke");
    let pie_sw = attr_num(node, "stroke-width", 0.0);

    let slices: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "slice")
        .collect();
    if slices.is_empty() {
        out.push_str("<!-- xsvg: <x:pie> has no slices -->");
        return;
    }
    let total: f64 = slices
        .iter()
        .map(|s| attr_num(*s, "value", 1.0).max(0.0))
        .sum();
    let total = if total > 0.0 {
        total
    } else {
        slices.len() as f64
    };

    // a categorical default palette for slices without an explicit fill
    const PALETTE: [&str; 8] = [
        "#6366f1", "#0ea5e9", "#f59e0b", "#10b981", "#e11d48", "#8b5cf6", "#14b8a6", "#f472b6",
    ];
    let d2r = std::f64::consts::PI / 180.0;

    out.push_str("<g>"); // per-slice source mapping (each slice carries its own range)
    let mut angle = start;
    for (i, &s) in slices.iter().enumerate() {
        let value = attr_num(s, "value", 1.0).max(0.0);
        let (a0, a1) = (angle, angle + value / total * 360.0);
        angle = a1;
        // radius: explicit `r`, else pie r × `grow`
        let rr = match s.attribute("r").and_then(parse_num) {
            Some(v) => v.max(0.0),
            None => r * attr_num(s, "grow", 1.0).max(0.0),
        };
        // explode out along the slice's proportional bisector
        let ex = attr_num(s, "explode", 0.0);
        let mid = (a0 + a1) / 2.0 * d2r;
        let (ccx, ccy) = (cx + ex * mid.cos(), cy + ex * mid.sin());
        let fill = s
            .attribute("fill")
            .map(|v| resolve_var(v).into_owned())
            .unwrap_or_else(|| PALETTE[i % PALETTE.len()].to_string());
        let d = sector_path(ccx, ccy, rr, inner.min(rr), a0, a1, gap);
        if d.is_empty() {
            continue; // the gap consumed this slice
        }

        out.push_str("<g");
        out.push_str(&pos_attr(s, ctx));
        out.push('>');
        out.push_str(&format!("<path fill=\"{fill}\" d=\"{d}\""));
        let (stroke, sw) = (
            s.attribute("stroke").or(pie_stroke),
            attr_num(s, "stroke-width", pie_sw),
        );
        if let (Some(stroke), true) = (stroke, sw > 0.0) {
            out.push_str(&format!(
                " stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
                resolve_var(stroke),
                fmt(sw)
            ));
        }
        out.push_str("/></g>");
    }
    out.push_str("</g>");
}

/// Parse a `<x:plot>` domain attribute (`"20 25"`) into `(lo, hi)`; `auto`/absent/
/// malformed → the data extent (`data` min…max, padded when flat/empty).
pub(super) fn parse_domain(node: roxmltree::Node, attr: &str, data: &[f64]) -> (f64, f64) {
    if let Some(s) = node.attribute(attr) {
        if s != "auto" {
            let n: Vec<f64> = s.split_whitespace().filter_map(parse_num).collect();
            if n.len() == 2 && n[0] != n[1] {
                return (n[0], n[1]);
            }
        }
    }
    if data.is_empty() {
        return (0.0, 1.0);
    }
    let lo = data.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = data.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if lo == hi {
        (lo, lo + 1.0)
    } else {
        (lo, hi)
    }
}

/// Parse `"x,y x,y …"` data points into pixel-mappable pairs.
pub(super) fn parse_points(s: &str) -> Vec<(f64, f64)> {
    s.split_whitespace()
        .filter_map(|p| {
            let (a, b) = p.split_once(',')?;
            Some((parse_num(a)?, parse_num(b)?))
        })
        .collect()
}

/// `<x:plot>` (§7.9): a linear data coordinate frame. Maps `x-domain`/`y-domain`
/// (each explicit or `auto`) onto the pixel box, y inverted so larger is up, and
/// keeps stroke/marker sizes in pixels (unlike a `<g transform>`). Children:
/// `<x:bars>` of `<x:bar value label>` (bottom-aligned, mapped heights, evenly
/// spread) and `<x:line points>` (polyline + optional dots / area). `y-ticks`
/// draws gridlines with value labels.
pub(super) fn emit_plot(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let (mut px, mut py) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    let (mut pw, mut ph) = (
        attr_num(node, "width", 400.0),
        attr_num(node, "height", 200.0),
    );
    if let Some(r) = node.attribute("in") {
        if let Some(bb) = ref_geometry(node, r, ctx)
            .ok()
            .and_then(|d| svg_path_bbox(&d))
        {
            px = bb.x0;
            py = bb.y0;
            pw = bb.width();
            ph = bb.height();
        }
    }
    let is_x = |c: roxmltree::Node, name: &str| {
        c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == name
    };
    let bar_series: Vec<roxmltree::Node> = node.children().filter(|c| is_x(*c, "bars")).collect();
    let line_series: Vec<roxmltree::Node> = node.children().filter(|c| is_x(*c, "line")).collect();
    let bars_of = |bs: roxmltree::Node| bs.children().filter(move |c| is_x(*c, "bar")).count();

    // gather data extents for auto domains
    let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut has_bars = false;
    for &bs in &bar_series {
        for bar in bs.children().filter(|c| is_x(*c, "bar")) {
            ys.push(attr_num(bar, "value", 0.0));
            has_bars = true;
        }
    }
    for &ls in &line_series {
        for (x, y) in parse_points(ls.attribute("points").unwrap_or("")) {
            xs.push(x);
            ys.push(y);
        }
    }
    if has_bars {
        ys.push(0.0); // bars are measured from a zero baseline
    }
    let (yd0, yd1) = parse_domain(node, "y-domain", &ys);
    let (xd0, xd1) = parse_domain(node, "x-domain", &xs);
    let mapx = |dx: f64| px + (dx - xd0) / (xd1 - xd0) * pw;
    let mapy = |dy: f64| py + ph - (dy - yd0) / (yd1 - yd0) * ph;
    let clamp_y = |y: f64| y.clamp(py, py + ph);

    let grid = resolve_var(node.attribute("grid-color").unwrap_or("#e2e8f0")).into_owned();
    let grid_w = attr_num(node, "grid-width", 1.0).max(0.0);
    let label_fill = resolve_var(node.attribute("label-fill").unwrap_or("#64748b")).into_owned();
    let label_size = attr_num(node, "label-size", 11.0);

    out.push_str("<g");
    out.push_str(&pos_attr(node, ctx));
    out.push('>');

    // y gridlines + value labels
    let yticks = attr_num(node, "y-ticks", 0.0).max(0.0) as usize;
    if yticks >= 1 {
        for k in 0..=yticks {
            let v = yd0 + (yd1 - yd0) * k as f64 / yticks as f64;
            let gy = mapy(v);
            out.push_str(&format!(
                "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{grid}\" stroke-width=\"{}\"/>",
                fmt(px),
                fmt(gy),
                fmt(px + pw),
                fmt(gy),
                fmt(grid_w)
            ));
            out.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" text-anchor=\"end\" font-size=\"{}\" fill=\"{label_fill}\">{}</text>",
                fmt(px - 8.0),
                fmt(gy + label_size * 0.35),
                fmt(label_size),
                fmt(v)
            ));
        }
    }

    // bar series: bars fill their band, bottom-aligned, evenly spread
    for &bs in &bar_series {
        let n = bars_of(bs);
        if n == 0 {
            continue;
        }
        let gapf = attr_num(bs, "gap", 0.25).clamp(0.0, 0.95);
        let default_fill = bs.attribute("fill").unwrap_or("#6366f1");
        let band = pw / n as f64;
        let bw = band * (1.0 - gapf);
        for (i, bar) in bs.children().filter(|c| is_x(*c, "bar")).enumerate() {
            let v = attr_num(bar, "value", 0.0);
            let cxb = px + (i as f64 + 0.5) * band;
            let top = clamp_y(mapy(v));
            let bot = py + ph;
            let fill = bar
                .attribute("fill")
                .map(|x| resolve_var(x).into_owned())
                .unwrap_or_else(|| resolve_var(default_fill).into_owned());
            out.push_str("<g");
            out.push_str(&pos_attr(bar, ctx));
            out.push('>');
            out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{fill}\"/>",
                fmt(cxb - bw / 2.0),
                fmt(top),
                fmt(bw),
                fmt((bot - top).max(0.0)),
                fmt(attr_num(bs, "radius", 0.0))
            ));
            if let Some(label) = bar.attribute("label") {
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-size=\"{}\" fill=\"{label_fill}\">",
                    fmt(cxb),
                    fmt(bot + label_size + 4.0),
                    fmt(label_size)
                ));
                push_escaped(out, label, false);
                out.push_str("</text>");
            }
            out.push_str("</g>");
        }
    }

    // line series: mapped polyline + optional area fill + point markers
    for &ls in &line_series {
        let pts = parse_points(ls.attribute("points").unwrap_or(""));
        if pts.is_empty() {
            continue;
        }
        let mapped: Vec<(f64, f64)> = pts.iter().map(|&(x, y)| (mapx(x), mapy(y))).collect();
        let stroke = resolve_var(ls.attribute("stroke").unwrap_or("#0ea5e9")).into_owned();
        let sw = attr_num(ls, "stroke-width", 2.0);
        let poly = mapped
            .iter()
            .map(|&(x, y)| format!("{},{}", fmt(x), fmt(y)))
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str("<g");
        out.push_str(&pos_attr(ls, ctx));
        out.push('>');
        if let Some(area) = ls.attribute("area") {
            let fill = resolve_var(area).into_owned();
            let base = py + ph;
            out.push_str(&format!(
                "<path fill=\"{fill}\" d=\"M{},{} L{poly} L{},{} Z\" fill-opacity=\"0.15\"/>",
                fmt(mapped[0].0),
                fmt(base),
                fmt(mapped[mapped.len() - 1].0),
                fmt(base)
            ));
        }
        out.push_str(&format!(
            "<polyline fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\" points=\"{poly}\"/>",
            fmt(sw)
        ));
        if ls.attribute("marker").is_some() {
            let mr = attr_num(ls, "marker-size", sw * 1.8);
            let dot = ls
                .attribute("marker-fill")
                .map(|v| resolve_var(v).into_owned())
                .unwrap_or_else(|| stroke.clone());
            for &(x, y) in &mapped {
                out.push_str(&format!(
                    "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{dot}\"/>",
                    fmt(x),
                    fmt(y),
                    fmt(mr)
                ));
            }
        }
        out.push_str("</g>");
    }

    out.push_str("</g>");
}
