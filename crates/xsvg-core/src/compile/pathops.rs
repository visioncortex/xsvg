//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// `<x:offset in="#id" distance="d" join="round|miter|bevel">` (§7.7): grow
/// (positive `distance`) or shrink (negative) the referenced region by a
/// Minkowski offset, baked to one plain `<path>`. Like `<x:boolean>`, it is a
/// compile-time reference: `in` resolves to the target's compiled geometry, so
/// editing the target re-emits the offset (`baked_refs` already covers `in`).
pub(super) fn emit_offset(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    use crate::kurbo::Join;
    let Some(reference) = node.attribute("in") else {
        out.push_str("<!-- xsvg: <x:offset> requires in=\"#id\" -->");
        return;
    };
    let d = match ref_geometry(node, reference, ctx) {
        Ok(d) => d,
        Err(f) => {
            out.push_str(&format!(
                "<!-- xsvg: <x:offset in> target not found or not geometry ({}) -->",
                f.reason()
            ));
            return;
        }
    };
    let distance = attr_num(node, "distance", 0.0);
    let join = match node.attribute("join") {
        Some("miter") => Join::Miter,
        Some("bevel") => Join::Bevel,
        _ => Join::Round, // round = the true disc offset
    };
    let miter_limit = attr_num(node, "miter-limit", 4.0);
    let even_odd = node.attribute("fill-rule") == Some("evenodd");
    let skip = &["in", "distance", "join", "miter-limit"];
    match offset_svg_paths(
        &[&d],
        distance,
        join,
        miter_limit,
        even_odd,
        ctx.quality.tolerance(),
    ) {
        Some(od) if !od.is_empty() => {
            out.push_str("<path");
            copy_attrs(node, out, skip);
            out.push_str(&pos_attr(node, ctx));
            out.push_str(&format!(" d=\"{od}\"/>"));
        }
        Some(_) => {
            // legitimately empty (e.g. an inset larger than the region)
            out.push_str("<g");
            copy_attrs(node, out, skip);
            out.push_str(&pos_attr(node, ctx));
            out.push_str("/>");
        }
        None => out.push_str("<!-- xsvg: <x:offset> no usable geometry -->"),
    }
}

/// `<x:warp field="…" bend="…" axis="h|v">` (§7.3): the generic geometry-warp
/// front-end. Children lower to pure `<path>` geometry, their union bbox builds the
/// field's envelope frame, and every path bakes through the §7.1 pipeline at the
/// quality tolerance. Children that cannot become path geometry (live text, lines,
/// images) are skipped with a marker — a warp never *silently* emits
/// unwarped content; an unknown/absent field or empty geometry emits the children
/// unwarped behind a marker. The element's own paint / `transform` ride on the
/// emitted `<g>` (an affine `transform` composes after the bake for free).
pub(super) fn emit_warp(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let mut inner = String::new();
    for child in node.children().filter(|c| c.is_element()) {
        match warp_child_markup(child, ctx) {
            Ok(s) => inner.push_str(&s),
            Err(why) => inner.push_str(&format!(
                "<!-- xsvg: <x:warp> skipped <{}>: {why} -->",
                child.tag_name().name()
            )),
        }
    }

    // the pre-warp union bbox of all path geometry = the envelope frame (§7.2)
    let ranges = find_path_d_ranges(&inner);
    let bbox = ranges
        .iter()
        .fold(None, |acc: Option<crate::kurbo::Rect>, &(a, b)| {
            match (acc, svg_path_bbox(&inner[a..b])) {
                (Some(r), Some(n)) => Some(r.union(n)),
                (acc, n) => acc.or(n),
            }
        });

    let field_name = node.attribute("field").unwrap_or("");
    let bend = attr_num(node, "bend", 0.0) / 100.0; // authored as −100…100 %
    let axis = WarpAxis::parse(node.attribute("axis").unwrap_or("h"));
    // the base field: an envelope preset, a corner-driven map, a spine follow, or
    // seeded noise (§7.3)
    let base: Option<Box<dyn Field>> = bbox.and_then(|b| match field_name {
        "perspective" => parse_corners(node)
            .and_then(|t| Homography::new(b, t))
            .map(|h| Box::new(h) as Box<dyn Field>),
        "free" => parse_corners(node)
            .and_then(|t| FreeDistort::new(b, t))
            .map(|f| Box::new(f) as Box<dyn Field>),
        "bend" => {
            // flow the children along a referenced spine (Inkscape's LPE Bend): the
            // envelope's left edge starts on the spine (placed by align/start) and
            // its vertical midline rides it
            let frame = node
                .attribute("in")
                .and_then(|r| ref_geometry(node, r, ctx).ok())
                .and_then(|d| PathFrame::new(&d, ctx.quality.tolerance()))?;
            let s0 = run_offset(
                frame.len(),
                b.width(),
                node.attribute("align").unwrap_or("start"),
                attr_num(node, "start", 0.0),
            );
            let anchor = crate::kurbo::Point::new(b.min_x(), b.center().y);
            Some(Box::new(BendField { frame, s0, anchor }) as Box<dyn Field>)
        }
        "roughen" => Some(
            Box::new(RoughenField::new(bend, attr_num(node, "detail", 10.0), b)) as Box<dyn Field>,
        ),
        name => EnvelopePreset::new(name, bend, axis, b).map(|p| Box::new(p) as Box<dyn Field>),
    });
    // the Warp-Options distortion sliders compose a projective taper after the field
    let dh = attr_num(node, "distort-h", 0.0) / 100.0;
    let dv = attr_num(node, "distort-v", 0.0) / 100.0;
    let field: Option<Box<dyn Field>> = match (base, bbox) {
        (Some(f), Some(b)) if dh != 0.0 || dv != 0.0 => {
            Some(Box::new(Chain(vec![f, Box::new(Taper::new(b, dh, dv))])))
        }
        (f, _) => f,
    };

    out.push_str("<g");
    copy_attrs(
        node,
        out,
        &[
            "field",
            "bend",
            "axis",
            "corners",
            "distort-h",
            "distort-v",
            "in",
            "detail",
            "align",
            "start",
        ],
    );
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    match field {
        Some(f) => {
            let tol = ctx.quality.tolerance();
            let mut last = 0;
            for (a, b) in ranges {
                out.push_str(&inner[last..a]);
                // a path that fails to bake keeps its original geometry (§4
                // totality). Refit is DISABLED (see §7.1): kurbo's fitter
                // overshoots on dense glyph outlines and dominates compile time.
                match warp_svg_path(&inner[a..b], f.as_ref(), tol, false) {
                    Some(d) => out.push_str(&d),
                    None => out.push_str(&inner[a..b]),
                }
                last = b;
            }
            out.push_str(&inner[last..]);
        }
        None => {
            out.push_str(&format!(
                "<!-- xsvg: <x:warp field=\"{field_name}\"> unknown field, bad corners, or no geometry — children unwarped -->"
            ));
            out.push_str(&inner);
        }
    }
    out.push_str("</g>");
}

/// `corners="x0,y0 x1,y1 x2,y2 x3,y3"` — the four target corners (**TL TR BR BL**)
/// for `field="perspective"` / `"free"`. `None` unless exactly 8 finite numbers.
pub(super) fn parse_corners(node: roxmltree::Node) -> Option<[crate::kurbo::Point; 4]> {
    let nums: Vec<f64> = node
        .attribute("corners")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(parse_num)
        .collect();
    if nums.len() != 8 {
        return None;
    }
    let p = |i: usize| crate::kurbo::Point::new(nums[2 * i], nums[2 * i + 1]);
    Some([p(0), p(1), p(2), p(3)])
}

/// Lower one `<x:warp>` child to pre-warp markup whose geometry is all `<path d>`:
/// basic shapes (rect — sharp or rounded — circle, ellipse, polygon, polyline)
/// convert directly via `shape_to_path_d`, everything else runs through the normal
/// pipeline (so `outline="true"` text and nested `<x:warp>`s compose). `Err(reason)`
/// when the result still contains geometry the bake cannot warp.
pub(super) fn warp_child_markup(child: roxmltree::Node, ctx: &Ctx) -> Result<String, String> {
    let name = child.tag_name().name();
    if child.tag_name().namespace() != Some(XSVG_NS)
        && matches!(name, "rect" | "circle" | "ellipse" | "polygon" | "polyline")
    {
        // shape_to_path_d handles a rounded rect too (rx/ry → arc path), so a
        // <rect rx> converts here rather than falling through to a <rect> the
        // bake can't warp — it used to be dropped behind a skip marker.
        let d = shape_to_path_d(child).ok_or("degenerate shape")?;
        let mut s = String::from("<path");
        copy_attrs(
            child,
            &mut s,
            &[
                "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "points",
            ],
        );
        s.push_str(&pos_attr(child, ctx));
        s.push_str(&format!(" d=\"{d}\"/>"));
        return Ok(s);
    }
    let mut buf = String::new();
    serialize(child, &mut buf, false, ctx);
    for tag in [
        "text", "rect", "circle", "ellipse", "line", "image", "use", "polygon", "polyline",
    ] {
        if has_tag(&buf, tag) {
            return Err(format!("lowers to <{tag}> — needs path/outline form"));
        }
    }
    Ok(buf)
}

/// Whether `s` contains an opening `<tag …>` element (tag-boundary-aware, so
/// `"text"` does not match `<textArea>` and `"line"` not `<linearGradient>`).
pub(super) fn has_tag(s: &str, tag: &str) -> bool {
    let needle = format!("<{tag}");
    let mut i = 0;
    while let Some(pos) = s[i..].find(&needle) {
        let after = i + pos + needle.len();
        match s.as_bytes().get(after) {
            Some(b' ') | Some(b'>') | Some(b'/') | None => return true,
            _ => i = after,
        }
    }
    false
}

/// `<x:boolean op="union|intersect|subtract|exclude">` (§7.5): Pathfinder-style
/// path algebra. Each element child is one **operand** (lowered to path geometry
/// exactly like `<x:warp>` children — shapes convert, text participates outlined,
/// nested `x:` elements compose); `subtract` removes every later operand from the
/// first (*Minus Front*), the other ops fold symmetrically. Operands flatten at
/// the profile tolerance; the ops are integer-exact and deterministic. The result
/// is one region: paint comes from the element itself (per-child paint is
/// ignored), a legitimately empty result emits an empty `<g>`, and an unknown
/// `op` degrades behind a marker with the children un-operated.
pub(super) fn emit_boolean(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    // lower each child to path markup; one child = one operand. A <use href>
    // child is an operand **by reference** (§7.4): the target's geometry (§4
    // reference resolution — compiled output for x: targets) joins the algebra
    // while the target itself keeps rendering wherever it is.
    let mut markups: Vec<(String, bool)> = Vec::new(); // (markup, even_odd)
    let mut markers = String::new();
    for child in node.children().filter(|c| c.is_element()) {
        let even_odd = child.attribute("fill-rule") == Some("evenodd");
        if child.tag_name().name() == "use" && child.tag_name().namespace() != Some(XSVG_NS) {
            let href = child
                .attribute("href")
                .or_else(|| child.attribute((XLINK_NS, "href")));
            let placed = href
                .ok_or(RefFail::NotFound)
                .and_then(|r| ref_geometry(child, r, ctx))
                .and_then(|d| transform_d(&d, use_placement(child)).ok_or(RefFail::NoGeometry));
            match placed {
                Ok(d) => markups.push((format!("<path d=\"{d}\"/>"), even_odd)),
                Err(f) => markers.push_str(&format!(
                    "<!-- xsvg: <x:boolean> skipped <use> ({}) -->",
                    f.reason()
                )),
            }
            continue;
        }
        match warp_child_markup(child, ctx) {
            Ok(m) => {
                markups.push((m, even_odd));
                // stroke ink joins the operand's region (Illustrator expands
                // strokes before Pathfinder); nonzero operands only — evenodd
                // would XOR the overlap away
                match child_stroke_outline(child, ctx.quality.tolerance()) {
                    Some(stroke_d) if !even_odd => {
                        markups
                            .last_mut()
                            .unwrap()
                            .0
                            .push_str(&format!("<path d=\"{stroke_d}\"/>"));
                    }
                    Some(stroke_d) => {
                        // evenodd operand: resolve fill (evenodd) ∪ stroke
                        // (nonzero) NOW so the mixed rules never meet
                        let m = &markups.last().unwrap().0;
                        let fill_paths: Vec<&str> = find_path_d_ranges(m)
                            .into_iter()
                            .map(|(a, b)| &m[a..b])
                            .collect();
                        let ops = [
                            BoolOperand {
                                paths: fill_paths,
                                even_odd: true,
                            },
                            BoolOperand {
                                paths: vec![&stroke_d],
                                even_odd: false,
                            },
                        ];
                        if let Some(d) =
                            boolean_svg_paths(&ops, BoolOp::Union, ctx.quality.tolerance())
                        {
                            *markups.last_mut().unwrap() = (format!("<path d=\"{d}\"/>"), false);
                        }
                    }
                    None => {}
                }
            }
            Err(why) => markers.push_str(&format!(
                "<!-- xsvg: <x:boolean> skipped <{}>: {why} -->",
                child.tag_name().name()
            )),
        }
    }

    let op = BoolOp::parse(node.attribute("op").unwrap_or("union"));
    let Some(op) = op else {
        // unknown op → children un-operated behind a marker (never silent)
        out.push_str(&format!(
            "<!-- xsvg: <x:boolean op=\"{}\"> unknown op — children un-combined -->",
            node.attribute("op").unwrap_or("")
        ));
        out.push_str(&markers);
        out.push_str("<g");
        copy_attrs(node, out, &["op"]);
        out.push_str(&pos_attr(node, ctx));
        out.push('>');
        for (m, _) in &markups {
            out.push_str(m);
        }
        out.push_str("</g>");
        return;
    };

    let operands: Vec<BoolOperand> = markups
        .iter()
        .map(|(m, even_odd)| BoolOperand {
            paths: find_path_d_ranges(m)
                .into_iter()
                .map(|(a, b)| &m[a..b])
                .collect(),
            even_odd: *even_odd,
        })
        .filter(|o| !o.paths.is_empty())
        .collect();

    out.push_str(&markers);
    match boolean_svg_paths(&operands, op, ctx.quality.tolerance()) {
        Some(d) if !d.is_empty() => {
            out.push_str("<path");
            copy_attrs(node, out, &["op"]);
            out.push_str(&pos_attr(node, ctx));
            out.push_str(&format!(" d=\"{d}\"/>"));
        }
        Some(_) => {
            // a legitimately empty result (e.g. disjoint intersect)
            out.push_str("<g");
            copy_attrs(node, out, &["op"]);
            out.push_str(&pos_attr(node, ctx));
            out.push_str("/>");
        }
        None => {
            out.push_str("<!-- xsvg: <x:boolean> no usable geometry -->");
        }
    }
}

/// A plain-shape boolean operand's stroke ink, expanded to fill geometry via
/// kurbo's stroke-to-fill (caps and joins from the attributes; dashes are not
/// supported). `None` when the child has no stroke to expand.
pub(super) fn child_stroke_outline(child: roxmltree::Node, tolerance: f64) -> Option<String> {
    if child.tag_name().namespace() == Some(XSVG_NS) {
        return None;
    }
    let paint = child.attribute("stroke")?;
    if paint == "none" {
        return None;
    }
    let width = attr_num(child, "stroke-width", 1.0);
    if width <= 0.0 {
        return None;
    }
    let d = shape_to_path_d(child)?;
    let path = crate::kurbo::BezPath::from_svg(&d).ok()?;
    use crate::kurbo::{stroke, Cap, Join, Stroke, StrokeOpts};
    let cap = match child.attribute("stroke-linecap") {
        Some("round") => Cap::Round,
        Some("square") => Cap::Square,
        _ => Cap::Butt,
    };
    let join = match child.attribute("stroke-linejoin") {
        Some("round") => Join::Round,
        Some("bevel") => Join::Bevel,
        _ => Join::Miter,
    };
    let mut style = Stroke::new(width).with_caps(cap).with_join(join);
    if let Some(dashes) = child.attribute("stroke-dasharray") {
        let pattern: Vec<f64> = dashes
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(parse_num)
            .filter(|v| *v >= 0.0)
            .collect();
        if !pattern.is_empty() && pattern.iter().any(|&v| v > 0.0) {
            style = style.with_dashes(attr_num(child, "stroke-dashoffset", 0.0), pattern);
        }
    }
    let out = stroke(path, &style, &StrokeOpts::default(), tolerance.max(0.01));
    Some(out.to_svg())
}
