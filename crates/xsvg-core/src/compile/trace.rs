//! Convert to path (`to_path`, docs/Incremental.md §11): the compiler's own
//! output for one node with outlining forced, in the form an editor writes back
//! into the source in the node's place. Every attribute the conversion does not
//! spend is kept as authored — `id`, `x:label`, `transform`, `opacity`, a
//! `filter` list, a `var()` paint — so the theme, the filter and the editor's
//! tools see the converted element as they saw the text. Also the tracer for
//! point text (a plain `<text>`), which the box emitters never lay out, and the
//! shape-to-path conversion.

use super::*;

/// Why a node could not be converted: the font the outliner had no bytes for,
/// or a named limit.
#[derive(Clone, Debug, PartialEq)]
pub struct ToPathError {
    pub reason: String,
    pub family: Option<String>,
    pub weight: Option<String>,
    pub style: Option<String>,
}

impl ToPathError {
    pub(super) fn limit(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            family: None,
            weight: None,
            style: None,
        }
    }

    pub(super) fn font(style: &TextStyle, reason: &str) -> Self {
        Self {
            reason: reason.to_string(),
            family: Some(style.family.clone()),
            weight: Some(style.weight.clone()),
            style: Some(style.style.clone()),
        }
    }
}

/// The attributes a text element spends on its layout and paint: nothing of
/// them has a meaning on the paths that replace it.
const SPENT: &[&str] = &[
    "x",
    "y",
    "width",
    "height",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "text-transform",
    "text-anchor",
    "text-decoration",
    "dominant-baseline",
    "baseline-shift",
    "align",
    "valign",
    "padding",
    "fit",
    "fit-min",
    "text-overflow",
    "overflow",
    "text-indent",
    "paragraph-spacing",
    "direction",
    "in",
    "outline",
    "inline-size",
    "dx",
    "dy",
    "rotate",
    "textLength",
    "lengthAdjust",
    "text-align",
    "display-align",
    "line-increment",
    "effect",
    "start",
    "list",
    "marker",
    "marker-size",
    "marker-gap",
    "marker-fill",
    "indent",
    "item-spacing",
    "space-before",
    "space-after",
    // the group writes these, lowered: the fill, the border as a stroke
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linejoin",
    "stroke-linecap",
    "stroke-dasharray",
    "stroke-opacity",
    "paint-order",
];
const SPENT_X: &[&str] = &[
    "font",
    "outline",
    "border-width",
    "border-color",
    "border-opacity",
    "glyph-x-scale",
    "highlight",
];

/// The attributes of `node` a conversion carries onto the group that replaces
/// it, as authored: everything but what the text spent. `x:` attributes (an
/// editor's label, a layer, an order) keep their prefix — the markup goes back
/// into the source, where it is bound. `transform` rides along only when asked:
/// the box emitters write it themselves.
pub(super) fn carried_attrs(node: roxmltree::Node, with_transform: bool) -> String {
    let mut s = String::new();
    for a in node.attributes() {
        let (prefix, spent) = match a.namespace() {
            None => (
                "",
                SPENT.contains(&a.name()) || (a.name() == "transform" && !with_transform),
            ),
            Some(XSVG_NS) => ("x:", SPENT_X.contains(&a.name())),
            _ => continue,
        };
        if spent || (a.name().starts_with("on") && prefix.is_empty()) {
            continue;
        }
        s.push(' ');
        s.push_str(prefix);
        s.push_str(a.name());
        s.push_str("=\"");
        push_escaped(&mut s, a.value(), true);
        s.push('"');
    }
    s
}

/// The geometry attributes a shape spends on its `d`.
fn shape_geometry(name: &str) -> &'static [&'static str] {
    match name {
        "rect" => &["x", "y", "width", "height", "rx", "ry"],
        "circle" => &["cx", "cy", "r"],
        "ellipse" => &["cx", "cy", "rx", "ry"],
        "line" => &["x1", "y1", "x2", "y2"],
        _ => &["points"],
    }
}

/// A basic shape as the `<path>` with the same attributes.
pub(super) fn emit_shape_as_path(node: roxmltree::Node, out: &mut String, ctx: &Ctx) -> bool {
    let Some(d) = shape_to_path_d(node) else {
        return false;
    };
    out.push_str("<path");
    copy_attrs(node, out, shape_geometry(node.tag_name().name()), ctx);
    // the editor's own attributes, which copy_attrs consumes
    for a in node.attributes().filter(|a| a.namespace() == Some(XSVG_NS)) {
        out.push_str(" x:");
        out.push_str(a.name());
        out.push_str("=\"");
        push_escaped(out, a.value(), true);
        out.push('"');
    }
    out.push_str(" d=\"");
    push_escaped(out, &d, true);
    out.push_str("\"/>");
    true
}

/// A plain `<text>` traced glyph by glyph (§6.12, point text): each chunk — the
/// characters at one pen position — outlined at the pen, which `x`/`y` on the
/// `<text>` or a `<tspan>` reset and `dx`/`dy` nudge, shifted back by the
/// `text-anchor` in force, in the font the chunk composes from the `<text>`
/// down; each chunk a path in its own paint (a `<tspan>`'s `fill`, its border).
/// What this walk cannot place faithfully is refused, not mis-traced: per-glyph
/// `x`/`y`/`dx`/`dy` lists, `rotate`, `textLength`, `baseline-shift`, a
/// `<textPath>`. `false` leaves the element to pass through live, the miss
/// recorded for `to_path`.
pub(super) fn emit_text_outlined(node: roxmltree::Node, out: &mut String, ctx: &Ctx) -> bool {
    let base = style_from(node, ctx);
    let miss = |why: ToPathError| {
        ctx.record_miss(why);
        false
    };
    let many = |v: &str| {
        v.split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .count()
            > 1
    };
    for d in node.descendants().filter(|d| d.is_element()) {
        let name = d.tag_name().name();
        if d != node && !matches!(name, "tspan" | "a") {
            return miss(ToPathError::limit(format!(
                "a <{name}> inside <text> is not traced"
            )));
        }
        for a in ["rotate", "textLength", "lengthAdjust", "baseline-shift"] {
            if d.attribute(a).is_some() {
                return miss(ToPathError::limit(format!(
                    "`{a}` on <{name}> is not traced"
                )));
            }
        }
        for a in ["x", "y", "dx", "dy"] {
            if d.attribute(a).is_some_and(many) {
                return miss(ToPathError::limit(format!(
                    "a per-glyph `{a}` list on <{name}> is not traced"
                )));
            }
        }
    }

    let first_num = |v: &str| {
        v.split(|c: char| c == ',' || c.is_whitespace())
            .find(|t| !t.is_empty())
            .and_then(parse_num)
    };
    // pass 1: every piece at its pen, grouped into SVG text chunks — a chunk
    // starts at an absolute `x`/`y` and is what `text-anchor` aligns as a whole
    struct Piece {
        text: String,
        style: TextStyle,
        x: f64,
        baseline: f64,
        width: f64,
        chunk: usize,
        fill: Option<String>,
        border: Option<String>,
    }
    let mut pen = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    let mut chunks: Vec<&str> = vec![node.attribute("text-anchor").unwrap_or("start")];
    let mut pieces: Vec<Piece> = Vec::new();
    let mut at_start = true;
    for d in node.descendants().skip(1) {
        if d.is_element() {
            let absolute = d.attribute("x").is_some() || d.attribute("y").is_some();
            if let Some(x) = d.attribute("x").and_then(first_num) {
                pen.0 = x;
            }
            if let Some(y) = d.attribute("y").and_then(first_num) {
                pen.1 = y;
            }
            pen.0 += d.attribute("dx").and_then(first_num).unwrap_or(0.0);
            pen.1 += d.attribute("dy").and_then(first_num).unwrap_or(0.0);
            if absolute {
                chunks.push(
                    d.ancestors()
                        .find_map(|a| a.attribute("text-anchor"))
                        .unwrap_or("start"),
                );
            }
            continue;
        }
        let Some(text) = d.text() else { continue };
        // SVG collapses runs of white space; the element's leading space goes
        let mut chunk = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.starts_with(char::is_whitespace) && !at_start && !chunk.is_empty() {
            chunk.insert(0, ' ');
        }
        if text.ends_with(char::is_whitespace) && !chunk.is_empty() {
            chunk.push(' ');
        }
        if chunk.trim().is_empty() {
            continue;
        }
        at_start = false;
        let el = d.parent().filter(|p| p.is_element()).unwrap_or(node);
        let style = chunk_style(node, el, &base);
        let text = style.text_transform.apply(&chunk).into_owned();
        let Some(adv) = ctx.outliner.advances(&text, &style, style.size) else {
            return miss(ToPathError::font(&style, "no font file to trace"));
        };
        let width = adv.last().copied().unwrap_or(0.0);
        // the piece's own paint: a <tspan>'s fill and border, up to the <text>,
        // whose own are the group's
        let own = |pred: &dyn Fn(roxmltree::Node) -> bool| {
            el.ancestors().take_while(|a| *a != node).find(|a| pred(*a))
        };
        pieces.push(Piece {
            text,
            style,
            x: pen.0,
            baseline: pen.1,
            width,
            chunk: chunks.len() - 1,
            fill: own(&|a| a.attribute("fill").is_some())
                .and_then(|a| a.attribute("fill"))
                .map(str::to_string),
            border: own(&|a| has_text_border(a)).map(|a| text_border_attrs(a, ctx)),
        });
        pen.0 += width;
    }
    // pass 2: each chunk shifted back by its anchor over its whole extent, then traced
    let shift: Vec<f64> = (0..chunks.len())
        .map(|c| {
            let of: Vec<&Piece> = pieces.iter().filter(|p| p.chunk == c).collect();
            let (Some(first), Some(last)) = (of.first(), of.last()) else {
                return 0.0;
            };
            let extent = last.x + last.width - first.x;
            match chunks[c] {
                "middle" => -extent / 2.0,
                "end" => -extent,
                _ => 0.0,
            }
        })
        .collect();
    let mut traced: Vec<PaintedPath> = Vec::with_capacity(pieces.len());
    for p in &pieces {
        let Some(d) = ctx.outline(
            &p.text,
            &p.style,
            p.style.size,
            p.x + shift[p.chunk],
            p.baseline,
        ) else {
            return miss(ToPathError::font(&p.style, "no font file to trace"));
        };
        traced.push(PaintedPath {
            d,
            fill: p.fill.clone(),
            border: p.border.clone(),
            overflow: false,
        });
    }
    let pieces = traced;
    if pieces.is_empty() {
        return miss(ToPathError::limit(
            "the <text> holds no characters to trace",
        ));
    }

    let start = node.range().start;
    let pos = format!(
        "{}{}{}",
        pos_attr(node, ctx),
        transform_attr(node),
        ctx.carry_at(start, false)
    );
    let stroke = text_border_attrs(node, ctx);
    match node.attribute("fill") {
        Some(fill) => push_painted_paths(out, &pieces, fill, &stroke, &pos, ctx),
        None => {
            // no fill of its own: the paths inherit, as the text did
            out.push_str(&format!("<g{stroke}{pos}>"));
            for p in &pieces {
                out.push_str("<path d=\"");
                push_escaped(out, &p.d, true);
                out.push('"');
                if let Some(f) = &p.fill {
                    out.push_str(&format!(" fill=\"{}\"", ctx.paint(f)));
                }
                if let Some(b) = &p.border {
                    out.push_str(b);
                }
                out.push_str("/>");
            }
            out.push_str("</g>");
        }
    }
    true
}

/// The font a chunk of a live `<text>` renders in: the element's, with what the
/// `<tspan>`s on the way down declare (SVG inherits the font down the element).
pub(super) fn chunk_style(
    node: roxmltree::Node,
    el: roxmltree::Node,
    base: &TextStyle,
) -> TextStyle {
    let own = |name: &str| {
        el.ancestors()
            .take_while(|a| *a != node)
            .find_map(|a| a.attribute(name))
    };
    let mut style = base.clone();
    if let Some(v) = own("font-weight") {
        style.weight = v.to_string();
    }
    if let Some(v) = own("font-style") {
        style.style = v.to_string();
    }
    if let Some(v) = own("font-family") {
        style.family = v.to_string();
    }
    if let Some(v) = own("font-size").and_then(parse_num).filter(|n| *n > 0.0) {
        style.size = v;
    }
    style
}
