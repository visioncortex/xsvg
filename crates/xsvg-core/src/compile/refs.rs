//! Reference resolution and geometry (§4): what `in="#id"`, `href="#id"` and the
//! boolean operands reach, and the compiled-output geometry of what they reach —
//! the target's own transform honoured, cycles and depth cut with a marker.

use super::*;

/// Resolve a `#id` (or bare `id`) reference to its element anywhere in the document.
/// Document-order scan for the element `r` names — the fallback for nodes of a
/// linked dependency; the entry document goes through [`Ctx::resolve_ref`]'s table.
pub(super) fn resolve_ref<'a>(
    node: roxmltree::Node<'a, 'a>,
    r: &str,
) -> Option<roxmltree::Node<'a, 'a>> {
    let id = r.strip_prefix('#').unwrap_or(r);
    node.document()
        .descendants()
        .find(|n| n.attribute("id") == Some(id))
}

/// Parse an SVG `transform` list into a kurbo [`Affine`] (§4 reference
/// resolution honors the target's own transform). Mirrors browser behavior for
/// invalid input: the whole list is ignored (identity).
pub(super) fn parse_transform(s: &str) -> crate::kurbo::Affine {
    use crate::kurbo::Affine;
    let mut total = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim_matches(|c: char| c == ',' || c.is_whitespace());
        let Some(close) = rest[open..].find(')') else {
            return Affine::IDENTITY;
        };
        let args: Vec<f64> = rest[open + 1..open + close]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(parse_num)
            .collect();
        let t = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [tx]) => Affine::translate((*tx, 0.0)),
            ("translate", [tx, ty]) => Affine::translate((*tx, *ty)),
            ("scale", [k]) => Affine::scale(*k),
            ("scale", [kx, ky]) => Affine::scale_non_uniform(*kx, *ky),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, cx, cy]) => {
                Affine::translate((*cx, *cy))
                    * Affine::rotate(a.to_radians())
                    * Affine::translate((-*cx, -*cy))
            }
            ("skewX", [a]) => Affine::new([1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", [a]) => Affine::new([1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => return Affine::IDENTITY,
        };
        total *= t;
        rest = &rest[open + close + 1..];
    }
    if total.is_finite() {
        total
    } else {
        Affine::IDENTITY
    }
}

/// Bounding box of a compiled (plain-SVG) subtree: the union of every descendant shape's
/// bbox (`<path>`, `<rect>`, `<circle>`, … via [`shape_to_path_d`]), plus `<image>` boxes,
/// nested `<svg>` viewports, and same-document `<use>` targets — with ancestor
/// `transform`s and stroke half-widths applied. Used only as a by-id `<use>` sizing hint,
/// so `<text>` (which would need font metrics) isn't measured. `None` when the subtree
/// has no measurable geometry.
pub(super) fn plain_subtree_bbox(
    node: roxmltree::Node,
    base: crate::kurbo::Affine,
) -> Option<crate::kurbo::Rect> {
    subtree_bbox(node, base, &mut Vec::new(), Stroke::NONE)
}

/// Inherited stroke state: whether a stroke is actually painted, and how wide.
#[derive(Clone, Copy)]
pub(super) struct Stroke {
    pub(super) painted: bool,
    pub(super) width: f64,
}

impl Stroke {
    const NONE: Self = Self {
        painted: false,
        width: 1.0,
    };

    /// Fold in an element's own `stroke` / `stroke-width` (attribute or inline style).
    pub(super) fn inherit(self, node: roxmltree::Node) -> Self {
        let get = |p: &str| {
            node.attribute(p)
                .map(str::trim)
                .or_else(|| style_decl(node, p))
        };
        Self {
            painted: match get("stroke") {
                Some(v) => v != "none",
                None => self.painted,
            },
            width: get("stroke-width")
                .and_then(parse_num)
                .unwrap_or(self.width),
        }
    }

    /// Half the stroke, in the *transformed* space — how far paint spills past the fill.
    /// Uses `sqrt(|det|)` so a rotated or non-uniformly scaled transform still gets a
    /// sensible single number.
    pub(super) fn outset(self, tf: crate::kurbo::Affine) -> f64 {
        if !self.painted || self.width <= 0.0 {
            return 0.0;
        }
        let scale = tf.determinant().abs().sqrt();
        if scale.is_finite() {
            self.width / 2.0 * scale
        } else {
            0.0
        }
    }
}

pub(super) fn subtree_bbox(
    node: roxmltree::Node,
    base: crate::kurbo::Affine,
    seen: &mut Vec<String>,
    inherited: Stroke,
) -> Option<crate::kurbo::Rect> {
    let tag = node.tag_name().name();
    // Never-rendered subtrees (and explicitly hidden ones) would inflate the box. A
    // <symbol> is skipped here but *is* drawn when a <use> targets it — see below.
    if is_non_rendered(tag) || is_display_none(node) {
        return None;
    }
    let tf = base * element_transform(node);
    let stroke = inherited.inherit(node);
    let mut acc: Option<crate::kurbo::Rect> = None;
    let mut fold = |bb: crate::kurbo::Rect| {
        acc = Some(acc.map_or(bb, |a: crate::kurbo::Rect| a.union(bb)));
    };

    // A nested <svg> establishes a viewport and clips to it, so its extent *is* that
    // viewport rect. Recursing would measure its children in the unmapped coordinate
    // system — for a scaled viewBox (e.g. 0 0 1000 1000 shown at 20) wildly wrong.
    // Without a usable width/height we fall through and measure children as before.
    if tag == "svg" {
        if let Some(bb) = viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
            fold(bb);
            return acc;
        }
    }
    // Box-shaped, not path geometry (and <foreignObject>'s content isn't SVG).
    if matches!(tag, "image" | "foreignObject") {
        if let Some(bb) = viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
            fold(bb);
        }
        return acc;
    }
    // A same-document <use href="#id"> draws its target here, offset by x/y. (Cross-file
    // hrefs are already baked by this point, so only local ones remain.) The id stack is
    // the cycle guard — a <use> chain that loops back stops instead of recursing forever.
    if tag == "use" {
        if let Some(id) = local_href_id(node) {
            if !seen.iter().any(|s| s == id) {
                if let Some(target) = node
                    .document()
                    .descendants()
                    .find(|n| n.is_element() && n.attribute("id") == Some(id))
                {
                    let target_tag = target.tag_name().name();
                    let placed = tf
                        * crate::kurbo::Affine::translate((
                            attr_num(node, "x", 0.0),
                            attr_num(node, "y", 0.0),
                        ));
                    seen.push(id.to_string());
                    let bb = if matches!(target_tag, "symbol" | "svg") {
                        // The <use> sizes the viewport when it gives width/height;
                        // otherwise measure the (otherwise non-rendered) contents.
                        match viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
                            Some(bb) => Some(bb),
                            None => children_bbox(target, placed, seen, stroke),
                        }
                    } else {
                        subtree_bbox(target, placed, seen, stroke)
                    };
                    seen.pop();
                    if let Some(bb) = bb {
                        fold(bb);
                    }
                }
            }
        }
        return acc;
    }

    if let Some(bb) = shape_to_path_d(node)
        .and_then(|d| transform_d(&d, tf))
        .and_then(|d| svg_path_bbox(&d))
    {
        // Stroke straddles the outline, so paint reaches half a stroke past the fill.
        fold(bb.inflate(stroke.outset(tf), stroke.outset(tf)));
    }
    if let Some(bb) = children_bbox(node, tf, seen, stroke) {
        fold(bb);
    }
    acc
}

/// Union of `node`'s element children, measured in `tf`.
pub(super) fn children_bbox(
    node: roxmltree::Node,
    tf: crate::kurbo::Affine,
    seen: &mut Vec<String>,
    stroke: Stroke,
) -> Option<crate::kurbo::Rect> {
    let mut acc: Option<crate::kurbo::Rect> = None;
    for c in node.children().filter(roxmltree::Node::is_element) {
        if let Some(bb) = subtree_bbox(c, tf, seen, stroke) {
            acc = Some(acc.map_or(bb, |a: crate::kurbo::Rect| a.union(bb)));
        }
    }
    acc
}

/// The `#id` of a same-document `href` / `xlink:href`, if that's what it is.
pub(super) fn local_href_id<'a>(node: roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    node.attribute("href")
        .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))?
        .strip_prefix('#')
        .filter(|id| !id.is_empty())
}

/// Elements whose subtree is definition-only — referenced, never drawn in place.
pub(super) fn is_non_rendered(tag: &str) -> bool {
    matches!(
        tag,
        "defs"
            | "symbol"
            | "clipPath"
            | "mask"
            | "marker"
            | "pattern"
            | "filter"
            | "linearGradient"
            | "radialGradient"
            | "meshgradient"
            | "title"
            | "desc"
            | "metadata"
            | "style"
            | "script"
    )
}

/// `display:none` (attribute or inline style) — drawn nowhere, so it contributes no box.
/// `visibility:hidden` is deliberately *not* here: it still takes up layout space.
pub(super) fn is_display_none(node: roxmltree::Node) -> bool {
    node.attribute("display") == Some("none")
        || style_decl(node, "display").is_some_and(|v| v.trim() == "none")
}

/// The value of one declaration in an inline `style="a: 1; b: 2"`, if present.
pub(super) fn style_decl<'a>(node: roxmltree::Node<'a, 'a>, prop: &str) -> Option<&'a str> {
    node.attribute("style")?.split(';').find_map(|decl| {
        let (name, value) = decl.split_once(':')?;
        (name.trim() == prop).then(|| value.trim())
    })
}

/// An element's transform: the presentation attribute, else a `style="transform:…"`.
/// CSS `px`/`deg` map 1:1 onto SVG user units / degrees, so they are simply stripped;
/// anything [`parse_transform`] doesn't recognise degrades to identity either way.
pub(super) fn element_transform(node: roxmltree::Node) -> crate::kurbo::Affine {
    if let Some(t) = node.attribute("transform") {
        return parse_transform(t);
    }
    match style_decl(node, "transform") {
        Some(v) => parse_transform(&v.replace("px", "").replace("deg", "")),
        None => crate::kurbo::Affine::IDENTITY,
    }
}

/// `x`/`y`/`width`/`height` as a rect — the viewport of a nested `<svg>`, or the box of
/// an `<image>`/`<foreignObject>`. `None` unless both extents are positive numbers.
pub(super) fn viewport_rect(node: roxmltree::Node) -> Option<crate::kurbo::Rect> {
    let dim = |a: &str| node.attribute(a).and_then(parse_num).filter(|v| *v > 0.0);
    let (w, h) = (dim("width")?, dim("height")?);
    let (x, y) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    Some(crate::kurbo::Rect::new(x, y, x + w, y + h))
}

/// Axis-aligned bounds of `r` after `tf` (the bbox of its four mapped corners).
pub(super) fn transform_rect_bbox(
    r: crate::kurbo::Rect,
    tf: crate::kurbo::Affine,
) -> Option<crate::kurbo::Rect> {
    let p = |x, y| tf * crate::kurbo::Point::new(x, y);
    let corners = [p(r.x0, r.y0), p(r.x1, r.y0), p(r.x1, r.y1), p(r.x0, r.y1)];
    if corners.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x);
        y1 = y1.max(c.y);
    }
    Some(crate::kurbo::Rect::new(x0, y0, x1, y1))
}

/// Apply an affine to path data. `None` if `d` fails to parse.
pub(super) fn transform_d(d: &str, a: crate::kurbo::Affine) -> Option<String> {
    if a == crate::kurbo::Affine::IDENTITY {
        return Some(d.to_string());
    }
    let mut p = crate::kurbo::BezPath::from_svg(d).ok()?;
    p.apply_affine(a);
    Some(p.to_svg())
}

/// A `<use>` operand's placement: its `transform` composed with the extra
/// `x`/`y` translation (per SVG, x/y append a translate after the transform).
pub(super) fn use_placement(use_el: roxmltree::Node) -> crate::kurbo::Affine {
    let t = use_el
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or(crate::kurbo::Affine::IDENTITY);
    t * crate::kurbo::Affine::translate((attr_num(use_el, "x", 0.0), attr_num(use_el, "y", 0.0)))
}

/// Why a reference failed to resolve to geometry — spelled into the caller's
/// marker so degradations are distinguishable (§4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RefFail {
    NotFound,
    NoGeometry,
    Cycle,
    Depth,
    Budget,
    InnerTransform,
}

impl RefFail {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "target not found",
            Self::NoGeometry => "target has no path geometry",
            Self::Cycle => "reference cycle",
            Self::Depth => "reference chain too deep",
            Self::Budget => "reference budget exhausted",
            Self::InnerTransform => "referenced output nests a transform",
        }
    }
}

/// Resolve an `in="#id"` / `<use href>` reference to **geometry** (§4): a plain
/// shape yields its source geometry, a plain `<g>` the union of its shape
/// descendants, and an `x:` element its **compiled output** — every `<path d>`
/// it emits, joined as one multi-subpath region — so composition works by
/// reference. The target's own `transform` is honored (applied to the borrowed
/// geometry). Cycles, chains deeper than [`MAX_REF_DEPTH`], and an exhausted
/// [`REF_FUEL`] budget degrade with a distinguishable [`RefFail`] (§4 totality).
pub(super) fn ref_geometry(node: roxmltree::Node, r: &str, ctx: &Ctx) -> Result<String, RefFail> {
    let target = ctx.resolve_ref(node, r).ok_or(RefFail::NotFound)?;
    target_geometry(target, ctx)
}

/// The geometry a single element contributes when referenced (or when swept up
/// by a group walk). Guards, budget, memo, and the element's own `transform`
/// live here; the per-kind harvests live in [`x_output_geometry`] /
/// [`group_geometry`] / [`shape_to_path_d`].
pub(super) fn target_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let own = target
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or(crate::kurbo::Affine::IDENTITY);
    // A generator element harvests like a built-in x: element: its compiled output
    // is scratch-serialized and every `d` it emits is joined (§Extensibility §2).
    let is_x = match target.tag_name().namespace() {
        Some(XSVG_NS) => true,
        Some(ns) => ctx.imports.has(ns),
        None => false,
    };
    if !is_x && target.tag_name().name() != "g" {
        let d = shape_to_path_d(target).ok_or(RefFail::NoGeometry)?;
        return transform_d(&d, own).ok_or(RefFail::NoGeometry);
    }
    let id = target.attribute("id").map(str::to_string);
    if let Some(id) = &id {
        if let Some(hit) = ctx.resolved.borrow().get(id) {
            return hit.clone().ok_or(RefFail::NoGeometry);
        }
        if ctx.resolving.borrow().contains(id) {
            ctx.cuts.set(ctx.cuts.get() + 1);
            return Err(RefFail::Cycle);
        }
    }
    if ctx.resolving.borrow().len() >= MAX_REF_DEPTH {
        ctx.cuts.set(ctx.cuts.get() + 1);
        return Err(RefFail::Depth);
    }
    if ctx.fuel.get() == 0 {
        ctx.cuts.set(ctx.cuts.get() + 1);
        return Err(RefFail::Budget);
    }
    ctx.fuel.set(ctx.fuel.get() - 1);
    let cuts_before = ctx.cuts.get();
    if let Some(id) = &id {
        ctx.resolving.borrow_mut().push(id.clone());
    }
    let harvested = if is_x {
        x_output_geometry(target, ctx)
    } else {
        group_geometry(target, ctx)
    };
    if id.is_some() {
        ctx.resolving.borrow_mut().pop();
    }
    let result = harvested.and_then(|d| transform_d(&d, own).ok_or(RefFail::NoGeometry));
    // a cut below means this result reflects what was on the stack, not the
    // target itself — valid here, poison anywhere else
    if ctx.cuts.get() == cuts_before {
        if let Some(id) = id {
            match &result {
                Ok(d) => {
                    ctx.resolved.borrow_mut().insert(id, Some(d.clone()));
                }
                Err(RefFail::NoGeometry) => {
                    ctx.resolved.borrow_mut().insert(id, None);
                }
                Err(_) => {}
            }
        }
    }
    result
}

/// Harvest an `x:` element's compiled output as geometry: scratch-serialize
/// (with glyph outlining forced, so referenced text contributes its glyph
/// geometry), reject nested transforms the d-only harvest cannot honor, resolve
/// winding-sensitive output through the boolean engine, and join the rest.
pub(super) fn x_output_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let prev = ctx.force_outline.replace(true);
    let mut buf = String::new();
    serialize(target, &mut buf, false, ctx);
    ctx.force_outline.set(prev);
    // the target's own transform reappears on its output wrapper (copy_attrs)
    // and is applied by target_geometry; any FURTHER transform nested in the
    // output would be silently lost by a d-only harvest — degrade loudly
    let expected = usize::from(target.attribute("transform").is_some());
    if buf.matches(" transform=\"").count() > expected {
        return Err(RefFail::InnerTransform);
    }
    let ranges = find_path_d_ranges(&buf);
    if ranges.is_empty() {
        return Err(RefFail::NoGeometry);
    }
    let ds: Vec<&str> = ranges.iter().map(|&(a, b)| &buf[a..b]).collect();
    let even: Vec<bool> = ranges
        .iter()
        .map(|&(a, b)| {
            let tag = buf[..a].rfind('<').unwrap_or(0);
            let end = buf[b..].find('>').map(|i| b + i).unwrap_or(buf.len());
            buf[tag..end].contains(r#"fill-rule="evenodd""#)
        })
        .collect();
    if even.iter().any(|&e| e) {
        // an evenodd path's painted region differs from its nonzero reading —
        // resolve through the boolean engine so borrowed = painted
        let ops: Vec<BoolOperand> = ds
            .iter()
            .zip(&even)
            .map(|(d, &e)| BoolOperand {
                paths: vec![d],
                even_odd: e,
            })
            .collect();
        return match boolean_svg_paths(&ops, BoolOp::Union, ctx.quality.tolerance()) {
            Some(d) if !d.is_empty() => Ok(d),
            _ => Err(RefFail::NoGeometry),
        };
    }
    Ok(ds.join(" "))
}

/// A plain `<g>` target: every shape descendant contributes, transforms compose
/// down the tree (each child applies its own inside [`target_geometry`]),
/// nested `x:` elements resolve to their compiled output. Children that carry
/// no geometry (text, defs, live `<use>`) are skipped, not fatal.
pub(super) fn group_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let mut ds = Vec::new();
    for child in target.children().filter(|c| c.is_element()) {
        // a passthrough <use> stays a live reference (§5), even inside a group
        if child.tag_name().name() == "use" && child.tag_name().namespace() != Some(XSVG_NS) {
            continue;
        }
        if let Ok(d) = target_geometry(child, ctx) {
            ds.push(d);
        }
    }
    if ds.is_empty() {
        return Err(RefFail::NoGeometry);
    }
    Ok(ds.join(" "))
}

/// Convert a fillable SVG shape element to a path `d` string (for rasterization).
/// `rect` is handled separately (rectangular fast path); returns `None` for shapes
/// with no fillable area (e.g. `<line>`).
pub(super) fn shape_to_path_d(node: roxmltree::Node) -> Option<String> {
    match node.tag_name().name() {
        "path" => node.attribute("d").map(str::to_string),
        "rect" => {
            let (x, y, w, h) = (
                attr_num(node, "x", 0.0),
                attr_num(node, "y", 0.0),
                attr_num(node, "width", 0.0),
                attr_num(node, "height", 0.0),
            );
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            // rx/ry default to each other per SVG, clamped to the half-extent
            let rx = attr_num(node, "rx", attr_num(node, "ry", 0.0)).clamp(0.0, w / 2.0);
            let ry = attr_num(node, "ry", attr_num(node, "rx", 0.0)).clamp(0.0, h / 2.0);
            Some(if rx > 0.0 && ry > 0.0 {
                format!(
                    "M{},{y} h{} a{rx},{ry} 0 0,1 {rx},{ry} v{} a{rx},{ry} 0 0,1 -{rx},{ry} h-{} a{rx},{ry} 0 0,1 -{rx},-{ry} v-{} a{rx},{ry} 0 0,1 {rx},-{ry} Z",
                    x + rx,
                    w - 2.0 * rx,
                    h - 2.0 * ry,
                    w - 2.0 * rx,
                    h - 2.0 * ry
                )
            } else {
                format!("M{x},{y} h{w} v{h} h-{w} Z")
            })
        }
        "circle" => {
            let (cx, cy, r) = (
                attr_num(node, "cx", 0.0),
                attr_num(node, "cy", 0.0),
                attr_num(node, "r", 0.0),
            );
            (r > 0.0).then(|| {
                format!(
                    "M{},{} a{r},{r} 0 1,0 {},0 a{r},{r} 0 1,0 {},0 Z",
                    cx - r,
                    cy,
                    2.0 * r,
                    -2.0 * r
                )
            })
        }
        "ellipse" => {
            let (cx, cy, rx, ry) = (
                attr_num(node, "cx", 0.0),
                attr_num(node, "cy", 0.0),
                attr_num(node, "rx", 0.0),
                attr_num(node, "ry", 0.0),
            );
            (rx > 0.0 && ry > 0.0).then(|| {
                format!(
                    "M{},{} a{rx},{ry} 0 1,0 {},0 a{rx},{ry} 0 1,0 {},0 Z",
                    cx - rx,
                    cy,
                    2.0 * rx,
                    -2.0 * rx
                )
            })
        }
        "line" => {
            let (x1, y1, x2, y2) = (
                attr_num(node, "x1", 0.0),
                attr_num(node, "y1", 0.0),
                attr_num(node, "x2", 0.0),
                attr_num(node, "y2", 0.0),
            );
            Some(format!("M{x1},{y1} L{x2},{y2}"))
        }
        "polygon" | "polyline" => node.attribute("points").and_then(points_to_path_d),
        _ => None,
    }
}

/// `points="x0,y0 x1,y1 …"` → `"Mx0,y0 Lx1,y1 … Z"`. `None` if fewer than 2 points.
pub(super) fn points_to_path_d(points: &str) -> Option<String> {
    let nums: Vec<f64> = points
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(parse_num)
        .collect();
    if nums.len() < 4 {
        return None;
    }
    let mut d = String::new();
    for (i, pair) in nums.chunks_exact(2).enumerate() {
        d.push_str(if i == 0 { "M" } else { "L" });
        d.push_str(&format!("{},{}", fmt(pair[0]), fmt(pair[1])));
        d.push(' ');
    }
    d.push('Z');
    Some(d)
}
