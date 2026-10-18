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
    if has_text_border(node) {
        return border_attrs(
            node.attribute((XSVG_NS, "border-width")),
            node.attribute((XSVG_NS, "border-color")),
            node.attribute((XSVG_NS, "border-opacity")),
            ctx,
        );
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

/// The bordered-text stroke (§6.17) for an `x:border-width` / `x:border-color` /
/// `x:border-opacity` triple — each value resolved by the caller, which may
/// have inherited it attribute by attribute (a textpath's runs, §6.13).
pub(super) fn border_attrs(
    width: Option<&str>,
    color: Option<&str>,
    opacity: Option<&str>,
    ctx: &Ctx,
) -> String {
    let w = width.and_then(parse_num).unwrap_or(3.0).max(0.0);
    let color = ctx.paint(color.unwrap_or("#000000"));
    let mut s = format!(
        " stroke=\"{color}\" stroke-width=\"{}\" paint-order=\"stroke\" stroke-linejoin=\"round\"",
        fmt(w * 2.0)
    );
    if let Some(op) = opacity {
        s.push_str(&format!(" stroke-opacity=\"{}\"", esc(op)));
    }
    s
}

/// One traced piece of outlined text (§6.12, §6.13): a run's glyphs, with the
/// paint it overrides — its fill, and its border's stroke attributes where they
/// differ from the element's — and whether its line lies outside a box that
/// shows its overflow (§6.6).
pub(super) struct PaintedPath {
    pub(super) d: String,
    pub(super) fill: Option<String>,
    pub(super) border: Option<String>,
    pub(super) overflow: bool,
}

/// Outlined text split into painted pieces, one `<g>` keyed as the element.
/// Without a border in play the pieces are one layer of paths, each overriding
/// the group's fill. With one — the element's (`stroke`) or a run's — they
/// paint in TWO layers, every border and then every fill: the order one path
/// gives. Stroked one after another, a later piece's border would cover the
/// edge of the glyph before it.
pub(super) fn push_painted_paths(
    out: &mut String,
    pieces: &[PaintedPath],
    fill: &str,
    stroke: &str,
    pos: &str,
    ctx: &Ctx,
) {
    let path = |out: &mut String, p: &PaintedPath, attrs: &str| {
        out.push_str("<path d=\"");
        push_escaped(out, &p.d, true);
        out.push('"');
        out.push_str(attrs);
        out.push_str(overflow_stamp(p.overflow, ctx));
        out.push_str("/>");
    };
    let fill_of = |p: &PaintedPath| {
        p.fill
            .as_deref()
            .map(|f| format!(" fill=\"{}\"", ctx.paint(f)))
            .unwrap_or_default()
    };
    if stroke.is_empty() && pieces.iter().all(|p| p.border.is_none()) {
        out.push_str(&format!("<g fill=\"{}\"{pos}>", ctx.paint(fill)));
        for p in pieces {
            path(out, p, &fill_of(p));
        }
        out.push_str("</g>");
        return;
    }
    out.push_str(&format!("<g fill=\"{}\"{pos}>", ctx.paint(fill)));
    out.push_str(&format!("<g fill=\"none\"{stroke}>"));
    for p in pieces {
        match &p.border {
            Some(b) => path(out, p, b),
            None if !stroke.is_empty() => path(out, p, ""),
            None => {}
        }
    }
    out.push_str("</g>");
    for p in pieces {
        path(out, p, &fill_of(p));
    }
    out.push_str("</g>");
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
    // The element's own cascade (query.rs): each property its own attribute, else
    // its own `<x:font>` token's (OVERRIDABLE defaults, §4.1), else the initial
    // value — except `font-family`, which continues up the ancestors (attribute,
    // then token) the way the browser's cascade would for plain text, because
    // compiled text is measured before the browser can inherit anything and the
    // measured family is then pinned on the output. Nothing else above a box may
    // reflow it.
    overlay(&element_levels(node, ctx.theme), &initial_style())
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
    /// A run's own border (§6.17): set when a `<tspan>` on its chain names any
    /// `x:border-*`, each attribute it leaves out taken from the enclosing
    /// `<tspan>`, else from the nearest bordered container (the box). Lowered
    /// on the run's `<tspan>` (live) or its traced piece (outlined).
    pub(super) border: Option<RunBorder>,
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
            && self.border.is_none()
    }

    /// The run's paint the outlined forms honour (§6.12): its fill or border.
    pub(super) fn paints(&self) -> bool {
        self.fill.is_some() || self.border.is_some()
    }

    /// Whether the run sets its own type: a face, a weight, a slant or a size.
    pub(super) fn types(&self) -> bool {
        self.weight.is_some()
            || self.style.is_some()
            || self.family.is_some()
            || self.size.is_some()
    }
}

/// `x:border-width` / `-color` / `-opacity` as resolved for one run.
#[derive(Clone, Default, PartialEq, Debug)]
pub(super) struct RunBorder {
    pub(super) width: Option<String>,
    pub(super) color: Option<String>,
    pub(super) opacity: Option<String>,
}

impl RunBorder {
    /// The border `node` itself names, over `self` attribute by attribute;
    /// `None` when it names none.
    fn over(&self, node: roxmltree::Node) -> Option<Self> {
        let get = |k: &str| node.attribute((XSVG_NS, k)).map(str::to_string);
        let (w, c, o) = (
            get("border-width"),
            get("border-color"),
            get("border-opacity"),
        );
        (w.is_some() || c.is_some() || o.is_some()).then(|| RunBorder {
            width: w.or_else(|| self.width.clone()),
            color: c.or_else(|| self.color.clone()),
            opacity: o.or_else(|| self.opacity.clone()),
        })
    }

    /// The stroke attributes, as [`text_border_attrs`] writes them.
    pub(super) fn attrs(&self, ctx: &Ctx) -> String {
        border_attrs(
            self.width.as_deref(),
            self.color.as_deref(),
            self.opacity.as_deref(),
            ctx,
        )
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
    // the border a run's own `x:border-*` completes: the nearest bordered
    // container's — the box around a paragraph, item or cell included
    let outer = node
        .ancestors()
        .find_map(|a| RunBorder::default().over(a))
        .unwrap_or_default();
    walk_runs(
        node,
        &EmitAttrs::default(),
        base,
        &outer,
        &mut segments,
        &mut styles,
        &mut emits,
    );
    (segments, styles, emits)
}

fn walk_runs(
    node: roxmltree::Node,
    ctx: &EmitAttrs,
    base: &TextStyle,
    outer: &RunBorder,
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
                    if let Some(b) = c.border.as_ref().unwrap_or(outer).over(child) {
                        c.border = Some(b);
                    }
                    if let Some(s) = child
                        .attribute("font-size")
                        .and_then(|v| run_size(v, base.size))
                    {
                        c.size = Some(s);
                    }
                    walk_runs(child, &c, base, outer, segments, styles, emits);
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
                    walk_runs(child, &c, base, outer, segments, styles, emits);
                }
                _ => walk_runs(child, ctx, base, outer, segments, styles, emits),
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
        border: ctx.border.clone(),
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
