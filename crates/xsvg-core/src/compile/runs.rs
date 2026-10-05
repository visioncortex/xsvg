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
    if bw.is_some() || bc.is_some() {
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
    for name in [
        "stroke",
        "stroke-width",
        "stroke-linejoin",
        "stroke-linecap",
        "stroke-dasharray",
        "stroke-opacity",
        "paint-order",
    ] {
        if let Some(v) = node.attribute(name) {
            s.push_str(&format!(" {name}=\"{}\"", ctx.paint(v)));
        }
    }
    s
}

/// Emit an outlined text group: `<g fill=… stroke=…><path d="…"/>…</g>` — the shared
/// output of create-outlines (§6.12) and text-on-path (§6.13). `paths` are the glyph
/// path `d` strings (one per line for a box, one for a warped run).
pub(super) fn push_outline_group(
    out: &mut String,
    fill: &str,
    stroke: &str,
    pos: &str,
    paths: &[String],
    ctx: &Ctx,
) {
    // `stroke` is the attribute string text_border_attrs built, values already
    // resolved and escaped; `fill` is a bare value.
    out.push_str(&format!("<g fill=\"{}\"{stroke}{pos}>", ctx.paint(fill)));
    for d in paths {
        out.push_str("<path d=\"");
        push_escaped(out, d, true);
        out.push_str("\"/>");
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
        family: s("font-family", "sans-serif"),
        size: num("font-size", 16.0),
        weight: s("font-weight", "normal"),
        style: s("font-style", "normal"),
        line_height: num("line-height", 1.2),
        letter_spacing: spacing("letter-spacing"),
        word_spacing: spacing("word-spacing"),
    }
}

/// Concatenate all descendant text content (styling flattened away). Used by the
/// single-style paths: `<text inline-size>` and curved-shape region flow.
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
}

impl EmitAttrs {
    pub(super) fn is_empty(&self) -> bool {
        self.paint_empty() && self.href.is_none() && self.highlight.is_none()
    }
    /// No paint/style override — only (perhaps) a link.
    pub(super) fn paint_empty(&self) -> bool {
        self.fill.is_none()
            && self.weight.is_none()
            && self.style.is_none()
            && self.family.is_none()
            && self.size.is_none()
            && self.decoration.is_none()
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
                    segments.push((t.to_string(), sid));
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
