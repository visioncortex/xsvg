//! Styled runs (§6.11): a text leaf's `<tspan>` tree collected into measured
//! segments and a style table, the resolved [`TextStyle`] of an element, and the
//! font / border attributes every text emitter writes.

use super::*;

/// Stroke/border paint attributes for a text element, honored on both live `<text>` and
/// outlined glyphs. The `x:border-width` / `x:border-color` convenience (§6.17) emits a
/// clean bordered-text effect: the stroke sits *behind* the fill (`paint-order="stroke"`,
/// round joins) so the border never eats into the letters, and `border-width` is the width
/// visible outside each glyph (the emitted `stroke-width` is doubled, since the fill covers
/// the inner half). Otherwise raw SVG stroke attributes pass through unchanged.
pub(super) fn text_border_attrs(node: roxmltree::Node, ctx: &Ctx) -> String {
    let bw = node
        .attribute((XSVG_NS, "border-width"))
        .and_then(parse_num);
    let bc = node.attribute((XSVG_NS, "border-color"));
    if has_text_border(node) {
        let w = bw.unwrap_or(3.0).max(0.0);
        let color = ctx.paint(bc.unwrap_or("#000000"));
        let mut s = format!(
            " stroke=\"{color}\" stroke-width=\"{}\" paint-order=\"stroke\" stroke-linejoin=\"round\"",
            fmt(w * 2.0)
        );
        if let Some(op) = node.attribute((XSVG_NS, "border-opacity")) {
            s.push_str(&format!(" stroke-opacity=\"{}\"", esc(op)));
        }
        return s;
    }
    // Raw passthrough of hand-written stroke attributes.
    let mut s = String::new();
    for name in RAW_STROKE_ATTRS {
        if let Some(v) = node.attribute(name) {
            s.push_str(&format!(" {name}=\"{}\"", ctx.paint(v)));
        }
    }
    s
}

/// Where a live `<text>` draws, estimated for its filter region (§8, *text*):
/// each chunk — the characters at one pen position — spans its measured advance
/// from the pen, which `x`/`y` on the `<text>` or a `<tspan>` reset and `dx`/`dy`
/// nudge, shifted back by the text-anchor in force; vertically the em box of
/// the chunk's style. Padded by an em all round, because the renderer's font
/// need not be the measurer's and a shadow clipped at the edge is worse than a
/// region a little large. `None` when the element has no text.
pub(super) fn text_bbox_estimate(node: roxmltree::Node, ctx: &Ctx) -> Option<Rect> {
    let first_num = |v: &str| {
        v.split(|c: char| c == ',' || c.is_whitespace())
            .find(|t| !t.is_empty())
            .and_then(parse_num)
    };
    let base = style_from(node, ctx);
    let mut pen = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    let mut bb: Option<Rect> = None;
    let mut pad = 0.0f64;
    for d in node.descendants().skip(1) {
        if d.is_element() {
            if let Some(x) = d.attribute("x").and_then(first_num) {
                pen.0 = x;
            }
            if let Some(y) = d.attribute("y").and_then(first_num) {
                pen.1 = y;
            }
            pen.0 += d.attribute("dx").and_then(first_num).unwrap_or(0.0);
            pen.1 += d.attribute("dy").and_then(first_num).unwrap_or(0.0);
            continue;
        }
        let Some(text) = d.text() else { continue };
        let chunk = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if chunk.is_empty() {
            continue;
        }
        // SVG inherits the font down a live <text>: the chunk's style is the
        // element's, with what its <tspan>s declare on the way down
        let el = d.parent().filter(|p| p.is_element()).unwrap_or(node);
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
        if let Some(v) = own("font-size").and_then(parse_num) {
            style.size = v;
        }
        let w = unmeasured_advance(
            &chunk,
            line_advance(&chunk, &style, style.size, ctx.m),
            style.size,
        );
        let anchor = el
            .ancestors()
            .find_map(|a| a.attribute("text-anchor"))
            .unwrap_or("start");
        let left = match anchor {
            "middle" => pen.0 - w / 2.0,
            "end" => pen.0 - w,
            _ => pen.0,
        };
        let fm = ctx.m.font_metrics(&style, style.size);
        let r = Rect::new(left, pen.1 - fm.ascent, left + w, pen.1 + fm.descent);
        bb = Some(bb.map_or(r, |b| b.union(r)));
        pad = pad.max(style.size);
        pen.0 += w;
    }
    bb.map(|b| b.inflate(pad, pad))
}

/// The hand-written stroke attributes a text border (§6.17) replaces: with
/// `x:border-*` present none of them is emitted, so the border is the stroke.
pub(super) const RAW_STROKE_ATTRS: [&str; 7] = [
    "stroke",
    "stroke-width",
    "stroke-linejoin",
    "stroke-linecap",
    "stroke-dasharray",
    "stroke-opacity",
    "paint-order",
];

/// Whether `node` asks for a text border (§6.17): either `x:border-*` attribute.
pub(super) fn has_text_border(node: roxmltree::Node) -> bool {
    node.attribute((XSVG_NS, "border-width")).is_some()
        || node.attribute((XSVG_NS, "border-color")).is_some()
}

/// Emit an outlined text group: `<g fill=… stroke=…><path d="…"/>…</g>` — the shared
/// output of create-outlines (§6.12) and text-on-path (§6.13). `paths` are the glyph
/// path `d` strings (one per line for a box, one for a warped run).
pub(super) fn push_outline_group(
    out: &mut String,
    fill: &str,
    stroke: &str,
    pos: &str,
    paths: &[(String, bool)],
    ctx: &Ctx,
) {
    // `stroke` is the attribute string text_border_attrs built, values already
    // resolved and escaped; `fill` is a bare value. Each path's flag: its line
    // lies outside a box that shows its overflow (§6.6).
    out.push_str(&format!("<g fill=\"{}\"{stroke}{pos}>", ctx.paint(fill)));
    for (d, overflow) in paths {
        out.push_str("<path d=\"");
        push_escaped(out, d, true);
        out.push('"');
        out.push_str(overflow_stamp(*overflow, ctx));
        out.push_str("/>");
    }
    out.push_str("</g>");
}

/// Push the shared live-`<text>` paint/style attributes: ` font-family="…" font-size="…"
/// font-weight="…" font-style="…" fill="…"` (leads with a space). Callers add their own
/// prefix (e.g. `text-anchor`) and any suffix (`x`/`y`, `letter-spacing`).
pub(super) fn push_font_attrs(
    out: &mut String,
    style: &TextStyle,
    size: f64,
    fill: &str,
    ctx: &Ctx,
) {
    out.push_str(&format!(
        " font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\"",
        esc(&style.family),
        fmt(size),
        esc(&style.weight),
        esc(&style.style),
        ctx.paint(fill)
    ));
}

pub(super) fn style_from(node: roxmltree::Node, ctx: &Ctx) -> TextStyle {
    // A `<x:font>` token (via `x:font="name"`) supplies OVERRIDABLE defaults —
    // the element's own `font-*` attributes win over the token (§4.1).
    let tok = ctx.font_props(node).unwrap_or_default();
    let t = |k: &str| tok.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
    let s = |k: &str, d: &str| node.attribute(k).or_else(|| t(k)).unwrap_or(d).to_string();
    // `font-family` alone resolves the way the browser's cascade would for plain
    // text — the element's own attribute, else its own token's, else the nearest
    // ancestor's (attribute, then token), else `sans-serif` — because compiled text
    // is measured before the browser can inherit anything, and the measured family
    // is then pinned on the output (§4.1). Every other property stays the element's
    // own: nothing above a box may reflow it. `ancestors()` starts at the node, and
    // ends at this document's root, so a linked file never sees its host's family.
    let family = node
        .ancestors()
        .find_map(|a| {
            a.attribute("font-family").map(str::to_string).or_else(|| {
                ctx.font_props(a)?
                    .into_iter()
                    .find(|(p, _)| p == "font-family")
                    .map(|(_, v)| v)
            })
        })
        .unwrap_or_else(|| "sans-serif".to_string());
    let num = |k: &str, d: f64| {
        node.attribute(k)
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
            .or_else(|| t(k).and_then(parse_num).filter(|n| *n > 0.0))
            .unwrap_or(d)
    };
    let spacing = |k: &str| match node.attribute(k).or_else(|| t(k)) {
        None | Some("normal") => 0.0,
        Some(v) => parse_num(v).unwrap_or(0.0),
    };
    TextStyle {
        family,
        size: num("font-size", 16.0),
        weight: s("font-weight", "normal"),
        style: s("font-style", "normal"),
        line_height: num("line-height", 1.2),
        letter_spacing: spacing("letter-spacing"),
        word_spacing: spacing("word-spacing"),
        text_transform: node
            .attribute("text-transform")
            .or_else(|| t("text-transform"))
            .and_then(TextTransform::parse)
            .unwrap_or_default(),
    }
}

/// Concatenate all descendant text content (styling flattened away), in the case
/// `style` renders it (§6.18). Used by the single-style paths: `<text
/// inline-size>`, curved-shape region flow and text on a path.
pub(super) fn collect_text_as(node: roxmltree::Node, style: &TextStyle) -> String {
    let text = collect_text(node);
    match style.text_transform.apply(&text) {
        std::borrow::Cow::Borrowed(_) => text,
        std::borrow::Cow::Owned(t) => t,
    }
}

/// Concatenate all descendant text content (styling flattened away), as authored.
pub(super) fn collect_text(node: roxmltree::Node) -> String {
    let mut s = String::new();
    for d in node.descendants() {
        if d.is_text() {
            if let Some(t) = d.text() {
                s.push_str(t);
            }
        }
    }
    s
}

/// Paint/style overrides a `<tspan>` run carries relative to the base `<text>`; only
/// attributes present are emitted.
#[derive(Clone, Default, PartialEq)]
pub(super) struct EmitAttrs {
    pub(super) fill: Option<String>,
    pub(super) weight: Option<String>,
    pub(super) style: Option<String>,
    pub(super) family: Option<String>,
    /// Per-run `font-size`, RESOLVED to user units at the authored scale (a
    /// `%`/`em` value is taken against the paragraph's base size). Layout
    /// measures the run at it and steps its line by the largest size on the
    /// line; emission scales it by the fitted/base ratio like the base size.
    pub(super) size: Option<f64>,
    /// `text-decoration` (underline / line-through / a space list): paint-only,
    /// no effect on advances, so layout never sees it — it rides to the output.
    pub(super) decoration: Option<String>,
    /// `baseline-shift` (`super` / `sub` / a length or percentage): a raised or
    /// lowered run, verbatim to the output; layout keeps the line's metrics.
    pub(super) shift: Option<String>,
    /// A hyperlink: the run lowers to an SVG `<a href>` around its text (read
    /// from `<tspan href>` — xsvg's run form — or an authored `<a>` child).
    /// Authored, so it passes verbatim like every authored URL (Sandbox.md).
    pub(super) href: Option<String>,
    /// `x:highlight` — a background colour behind the run's glyphs (the
    /// word-processor highlighter). SVG text has no background, so it bakes
    /// to a `<rect>` per run per line BEHIND the `<text>`; never a tspan attr.
    pub(super) highlight: Option<String>,
    /// `text-transform` on a run: applied to its text as the walk collects it
    /// (§6.18), so it never reaches the run table — two runs differing only in
    /// case share one style.
    pub(super) transform: Option<TextTransform>,
}

impl EmitAttrs {
    pub(super) fn is_empty(&self) -> bool {
        self.paint_empty()
            && self.href.is_none()
            && self.highlight.is_none()
            && self.decoration.is_none()
    }
    /// No paint/style override — only (perhaps) a link.
    pub(super) fn paint_empty(&self) -> bool {
        self.fill.is_none()
            && self.weight.is_none()
            && self.style.is_none()
            && self.family.is_none()
            && self.size.is_none()
            && self.shift.is_none()
    }
}

/// A run's `font-size`: absolute (user units), or relative to the paragraph's
/// base size when suffixed `%` or `em`. Non-positive or unparsable → none.
pub(super) fn run_size(v: &str, base: f64) -> Option<f64> {
    let v = v.trim();
    let n = if let Some(p) = v.strip_suffix('%') {
        parse_num(p).map(|n| base * n / 100.0)
    } else if let Some(e) = v.strip_suffix("em") {
        parse_num(e).map(|n| base * n)
    } else {
        parse_num(v)
    };
    n.filter(|n| *n > 0.0)
}

/// Walk a text container into styled segments for run layout: the `(text, style_id)`
/// stream (with `'\n'` for `<tbreak/>`), the layout style table (`styles[0]` = base),
/// and the parallel emit-attr table. `<tspan>` children introduce runs; nesting
/// composes (inner wins). Plain text collapses to a single base run.
pub(super) fn collect_runs(
    node: roxmltree::Node,
    base: &TextStyle,
) -> (Vec<(String, usize)>, Vec<TextStyle>, Vec<EmitAttrs>) {
    let mut segments = Vec::new();
    let mut styles = vec![base.clone()];
    let mut emits = vec![EmitAttrs::default()];
    walk_runs(
        node,
        &EmitAttrs::default(),
        base,
        &mut segments,
        &mut styles,
        &mut emits,
    );
    (segments, styles, emits)
}

pub(super) fn walk_runs(
    node: roxmltree::Node,
    ctx: &EmitAttrs,
    base: &TextStyle,
    segments: &mut Vec<(String, usize)>,
    styles: &mut Vec<TextStyle>,
    emits: &mut Vec<EmitAttrs>,
) {
    for child in node.children() {
        if child.is_text() {
            if let Some(t) = child.text() {
                if !t.is_empty() {
                    let sid = intern_run(ctx, base, styles, emits);
                    let tf = ctx.transform.unwrap_or(base.text_transform);
                    segments.push((tf.apply(t).into_owned(), sid));
                }
            }
        } else if child.is_element() {
            match child.tag_name().name() {
                "tbreak" => segments.push(("\n".to_string(), 0)),
                "tspan" => {
                    let mut c = ctx.clone();
                    let set = |slot: &mut Option<String>, v: Option<&str>| {
                        if let Some(v) = v {
                            *slot = Some(v.to_string());
                        }
                    };
                    set(&mut c.fill, child.attribute("fill"));
                    set(&mut c.weight, child.attribute("font-weight"));
                    set(&mut c.style, child.attribute("font-style"));
                    set(&mut c.family, child.attribute("font-family"));
                    set(&mut c.decoration, child.attribute("text-decoration"));
                    set(&mut c.shift, child.attribute("baseline-shift"));
                    set(&mut c.href, child.attribute("href"));
                    set(&mut c.highlight, child.attribute((XSVG_NS, "highlight")));
                    if let Some(tf) = child
                        .attribute("text-transform")
                        .and_then(TextTransform::parse)
                    {
                        c.transform = Some(tf);
                    }
                    if let Some(s) = child
                        .attribute("font-size")
                        .and_then(|v| run_size(v, base.size))
                    {
                        c.size = Some(s);
                    }
                    walk_runs(child, &c, base, segments, styles, emits);
                }
                // an authored SVG <a> around text: its href becomes the runs' link
                "a" => {
                    let mut c = ctx.clone();
                    if let Some(h) = child
                        .attribute("href")
                        .or_else(|| child.attribute((XLINK_NS, "href")))
                    {
                        c.href = Some(h.to_string());
                    }
                    walk_runs(child, &c, base, segments, styles, emits);
                }
                _ => walk_runs(child, ctx, base, segments, styles, emits),
            }
        }
    }
}

/// Intern the current override context into the style tables, returning its index
/// (0 = base). Overrides equal to the base weight/style/family are dropped; an
/// all-base context maps to the base run.
pub(super) fn intern_run(
    ctx: &EmitAttrs,
    base: &TextStyle,
    styles: &mut Vec<TextStyle>,
    emits: &mut Vec<EmitAttrs>,
) -> usize {
    let norm = EmitAttrs {
        fill: ctx.fill.clone(),
        weight: ctx.weight.clone().filter(|w| *w != base.weight),
        style: ctx.style.clone().filter(|s| *s != base.style),
        family: ctx.family.clone().filter(|f| *f != base.family),
        size: ctx.size.filter(|s| (*s - base.size).abs() > 1e-9),
        decoration: ctx.decoration.clone(),
        shift: ctx.shift.clone(),
        href: ctx.href.clone(),
        highlight: ctx.highlight.clone(),
        transform: None,
    };
    if norm.is_empty() {
        return 0;
    }
    if let Some(i) = emits.iter().position(|e| *e == norm) {
        return i;
    }
    let mut ts = base.clone();
    if let Some(s) = norm.size {
        ts.size = s;
    }
    if let Some(w) = &norm.weight {
        ts.weight = w.clone();
    }
    if let Some(s) = &norm.style {
        ts.style = s.clone();
    }
    if let Some(f) = &norm.family {
        ts.family = f.clone();
    }
    styles.push(ts);
    emits.push(norm);
    styles.len() - 1
}
