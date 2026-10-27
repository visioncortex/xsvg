//! `<x:textbox>`, `<x:p>` paragraphs and `<text inline-size>` (§6): the area and
//! region flows, their emission as positioned `<tspan>`s or outlined glyphs, run
//! highlights — and the text-layout seam that reports the same geometry to an
//! editor (docs/Incremental.md §6).

use super::*;
use crate::text::fit::bisect_fit;

/// A curved region flow's layout (§6.10) and its run table: styled `<tspan>`s
/// flow as runs (§6.11) — paint, typography, highlights — and text without any
/// takes the single-style path it always has.
fn region_layout(
    node: roxmltree::Node,
    style: &TextStyle,
    region: &dyn crate::Region,
    spec: &RegionSpec,
    m: &dyn Measurer,
) -> (AreaLayout, Vec<EmitAttrs>) {
    let (segments, styles, emits) = collect_runs(node, style);
    if emits.len() > 1 {
        (
            layout_region_runs(&segments, &styles, region, spec, m),
            emits,
        )
    } else {
        let text = collect_text_as(node, style);
        (layout_region(&text, style, region, spec, m), emits)
    }
}

/// `<text inline-size="W">…</text>` → `<text>` with one `<tspan>` per wrapped line.
/// Styled `<tspan>`s flow as runs (§6.11), their highlights and decorations
/// baked behind the text as in a box; text without any takes the single-style
/// flow it always has.
pub(super) fn emit_inline_size_text(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node, ctx);
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let max_w = attr_num(node, "inline-size", 0.0);
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let (segments, styles, emits) = collect_runs(node, &style);
    let lines = if emits.len() > 1 {
        // each forced flow wraps alone; the next starts one advance below the
        // last line of the one before
        let mut lines: Vec<PlacedLine> = Vec::new();
        let mut first = y;
        for flow in split_flows(&segments) {
            let ls = layout_flow_runs(&flow, &styles, x, first, max_w, m);
            let last = ls.last().expect("an empty flow lays one blank line");
            first = last.baseline + last.size * style.line_height;
            lines.extend(ls);
        }
        lines
    } else {
        layout_flow(&collect_text_as(node, &style), &style, x, y, max_w, m)
    };

    let anchor = node.attribute("text-anchor").unwrap_or("start");
    let filter = lower_filter_attr(node, out, ctx, || {
        lines_bbox(&lines, anchor, false, &style, style.size, &emits, m)
    });
    // a run's highlight and decoration bake behind the text, as in a box
    emit_run_backdrops(
        out,
        &lines,
        anchor,
        false,
        &style,
        style.size,
        node.attribute("fill").unwrap_or("#000"),
        &emits,
        Backdrops::All,
        m,
        ctx,
    );
    out.push_str("<text");
    copy_attrs(
        node,
        out,
        &["inline-size", "line-height", "text-transform", "filter"],
        ctx,
    );
    if let Some(f) = &filter {
        out.push_str(f);
    } else if let Some(f) = node.attribute("filter") {
        // as authored: `none`, a url() reference, a list the parser declines
        out.push_str(" filter=\"");
        push_escaped(out, &ctx.resolve_var(f), true);
        out.push('"');
    }
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    for line in &lines {
        emit_line(out, line, &style, style.size, gx, m, &emits, ctx);
    }
    out.push_str("</text>");
}

/// Which of a run's backdrops [`emit_run_backdrops`] draws. A filtered text
/// element (§8, *text*) draws its highlights first, outside the filter group,
/// and its decorations inside it with the glyphs: a highlight is a background,
/// and a background that casts a shadow reads as a sticker, not as text.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Backdrops {
    All,
    Highlights,
    Decorations,
}

/// Open the `<g filter="url(#…)">` a filtered multi-element text emitter wraps
/// its glyph output in — nothing when the element carries no lowered filter, so
/// plain output is byte-identical.
pub(super) fn open_filter_group(out: &mut String, filter: Option<&str>) {
    if let Some(f) = filter {
        out.push_str("<g");
        out.push_str(f);
        out.push('>');
    }
}

pub(super) fn close_filter_group(out: &mut String, filter: Option<&str>) {
    if filter.is_some() {
        out.push_str("</g>");
    }
}

/// The left edge, width and per-space justify slack of a laid-out line, from
/// the anchor the `<text>` will carry: the measured advance of its runs plus a
/// justified line's widened spaces (§6.9).
fn line_extent(
    line: &PlacedLine,
    anchor: &str,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
) -> (f64, f64, f64) {
    let natural: f64 = line
        .runs
        .iter()
        .map(|r| {
            let (st, rs) = emits
                .get(r.style)
                .map(|a| run_measure_style(a, style, size))
                .unwrap_or((style.clone(), size));
            line_advance(&r.text, &st, rs, m)
        })
        .sum();
    // a justified line widens its spaces, not its glyph gaps (§6.9)
    let extra = justify_space(line, style, size, emits, m).unwrap_or(0.0);
    let width = natural + spaces_in(&line.text) * extra;
    // the line's LEFT edge, whichever edge the anchor names
    let left = match (anchor, rtl) {
        ("middle", _) => line.x - width / 2.0,
        ("end", false) | ("start", true) => line.x - width,
        _ => line.x,
    };
    (left, width, extra)
}

/// A measurer without the font — the CLI without `--font-directory` — advances
/// nothing, and a filter region as wide as nothing clips the shadow to a sliver:
/// a zero advance on text that is not empty is replaced by a wide proportional
/// guess, 0.6 em per character, so the region holds the glyphs a renderer with
/// the font draws.
pub(super) fn unmeasured_advance(text: &str, advance: f64, size: f64) -> f64 {
    if advance <= 0.0 && !text.trim().is_empty() {
        text.chars().count() as f64 * 0.6 * size
    } else {
        advance
    }
}

/// Where laid-out lines draw, for a filter region (§8, *text*): the union of
/// each line's extent and em box, padded by an em all round — the renderer's
/// font need not be the measurer's, and a shadow clipped at the edge is worse
/// than a region a little large. `None` for no lines.
#[allow(clippy::too_many_arguments)]
pub(super) fn lines_bbox(
    lines: &[PlacedLine],
    anchor: &str,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
) -> Option<Rect> {
    let mut bb: Option<Rect> = None;
    for line in lines {
        let (left, width, _) = line_extent(line, anchor, rtl, style, size, emits, m);
        let width = unmeasured_advance(&line.text, width, line.size);
        let fm = m.font_metrics(style, line.size);
        let r = Rect::new(
            left,
            line.baseline - fm.ascent,
            left + width,
            line.baseline + fm.descent,
        );
        bb = Some(bb.map_or(r, |b| b.union(r)));
    }
    bb.map(|b| b.inflate(size, size))
}

/// `<x:textbox>` (Rung 3): explicit `align`/`valign`/`padding`/`fit`, or — with
/// `in="#id"` — bound to a referenced shape (rect → rectangular; any other shape →
/// flowed inside its outline, §6.10).
/// The block children of a textbox (§6.16): paragraphs, and lists whose
/// items flow as paragraphs with a gutter. Any block puts the box in
/// paragraph mode.
pub(super) fn blocks_of<'a>(node: roxmltree::Node<'a, 'a>) -> Vec<roxmltree::Node<'a, 'a>> {
    node.children()
        .filter(|c| {
            c.tag_name().namespace() == Some(XSVG_NS) && matches!(c.tag_name().name(), "p" | "list")
        })
        .collect()
}

pub(super) fn emit_textbox(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node, ctx);
    let tb: elements::Textbox = params_of(node);
    let fill = tb.fill.as_str();
    let gx = attr_num(node, "glyph-x-scale", 1.0);
    let outline = tb.outline || ctx.force_outline.get();
    let stroke = text_border_attrs(node, ctx);
    let pos = format!(
        "{}{}{}{}",
        pos_attr(node, ctx),
        frame_attr(node, ctx),
        transform_attr(node),
        ctx.carry_at(node.range().start, false) // `to_path`: the attributes it keeps
    );

    // Paragraph mode (§6.16): `<x:p>` children are separate paragraphs stacked with
    // space-before/after, and `<x:list>` children flow their items among them.
    // Rectangular geometry only (own box or `in="#rect"`); curved region flow
    // ignores blocks and falls through to a single flow.
    let paras = blocks_of(node);
    if !paras.is_empty() {
        let geom = match node.attribute("in") {
            Some(r) => ctx
                .resolve_ref(node, r)
                .filter(|t| t.tag_name().name() == "rect"),
            None => Some(node),
        };
        if let Some(geom) = geom {
            let spec = textbox_area_spec(node, geom);
            emit_paragraphs(
                node, &paras, &style, fill, gx, &spec, &pos, outline, out, ctx,
            );
            return;
        }
    }

    if let Some(reference) = node.attribute("in") {
        let Some(target) = ctx.resolve_ref(node, reference) else {
            out.push_str("<!-- xsvg: <x:textbox in> target not found -->");
            return;
        };
        // rect → reuse the rectangular path with the target's geometry (keeps fit/valign)
        if target.tag_name().name() == "rect" {
            let spec = textbox_area_spec(node, target);
            let (segments, styles, emits) = collect_runs(node, &style);
            let layout = layout_area_runs(&segments, &styles, &spec, ctx.m);
            let filter = lower_text_filter(node, &layout, &style, &emits, out, ctx);
            write_area_text(
                out,
                &layout,
                &style,
                fill,
                &stroke,
                gx,
                ctx.m,
                &emits,
                &pos,
                span_of(node),
                outline,
                filter.as_deref(),
                ctx.outliner,
                ctx,
            );
            return;
        }
        // any other shape — or an x: element's compiled output (e.g. a boolean
        // union) — flows text inside its filled outline via the raster region
        // (styled runs are not supported in curved region flow in v0)
        let geometry = match ref_geometry(node, reference, ctx) {
            Ok(d) => d,
            Err(f) => {
                out.push_str(&format!(
                    "<!-- xsvg: <x:textbox in> no region geometry ({}) -->",
                    f.reason()
                ));
                return;
            }
        };
        let region = ctx.shaper.rasterize(&geometry, (style.size / 3.0).max(1.0));
        let Some(region) = region else {
            out.push_str("<!-- xsvg: <x:textbox in> shape not rasterizable -->");
            return;
        };
        let spec = RegionSpec {
            padding: attr_num(node, "padding", 0.0),
            align: Align::parse(node.attribute("align").unwrap_or("start")),
            valign: VAlign::parse(node.attribute("valign").unwrap_or("top")),
            text_overflow: overflow_mode(node),
        };
        let (layout, emits) = region_layout(node, &style, &region, &spec, ctx.m);
        let filter = lower_text_filter(node, &layout, &style, &emits, out, ctx);
        write_area_text(
            out,
            &layout,
            &style,
            fill,
            &stroke,
            gx,
            ctx.m,
            &emits,
            &pos,
            span_of(node),
            outline,
            filter.as_deref(),
            ctx.outliner,
            ctx,
        );
        return;
    }

    let spec = textbox_area_spec(node, node);
    let (segments, styles, emits) = collect_runs(node, &style);
    let layout = layout_area_runs(&segments, &styles, &spec, ctx.m);
    let filter = lower_text_filter(node, &layout, &style, &emits, out, ctx);
    write_area_text(
        out,
        &layout,
        &style,
        fill,
        &stroke,
        gx,
        ctx.m,
        &emits,
        &pos,
        span_of(node),
        outline,
        filter.as_deref(),
        ctx.outliner,
        ctx,
    );
}

/// A textbox's `filter` (§8, *text*), lowered against the laid-out lines it is
/// about to write: the def goes before the element, the returned attribute on
/// the group [`write_area_text`] wraps the glyphs in.
fn lower_text_filter(
    node: roxmltree::Node,
    layout: &AreaLayout,
    style: &TextStyle,
    emits: &[EmitAttrs],
    out: &mut String,
    ctx: &Ctx,
) -> Option<String> {
    lower_filter_attr(node, out, ctx, || {
        lines_bbox(
            &layout.lines,
            layout.anchor.svg(),
            layout.rtl,
            style,
            layout.font_size,
            emits,
            ctx.m,
        )
    })
}

/// Paragraph flow (§6.16): lay each `<x:p>` as its own wrapped block, stacked
/// top-down inside the box with the paragraph gaps, then `valign` the whole
/// block. Each paragraph carries its own `align` / `text-indent` / `font-*` /
/// `fill`, and maps to its own source range. Live text (create-outlines is not
/// applied in paragraph mode).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_paragraphs(
    node: roxmltree::Node,
    paras: &[roxmltree::Node],
    base_style: &TextStyle,
    base_fill: &str,
    gx: f64,
    spec: &AreaSpec,
    pos: &str,
    outline: bool,
    out: &mut String,
    ctx: &Ctx,
) {
    let m = ctx.m;
    let cy = spec.y + spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let (items, block_top) = plan_paragraphs(node, paras, base_style, base_fill, spec, m);

    // list items' markers (§6.16): drawn shapes and one shared <text> of
    // generated markers, before the paragraphs, as a standalone list emits
    // them — the marker <text> carries no source key, so the editor's
    // char-cell tier reads the keyed paragraphs alone
    let mut shapes = String::new();
    let mut markers = String::new();
    // outlining (§6.12): each generated marker traced at its column, right-aligned
    let mut marker_paths: Option<Vec<(String, String)>> = outline.then(Vec::new);
    for para in &items {
        let Some(mk) = &para.marker else { continue };
        let Some(first) = para.laid.lines.first().map(|l| l.baseline + block_top) else {
            continue;
        };
        let outside = first < cy - 1e-6 || first > cy + ch + 1e-6;
        if outside && spec.text_overflow != TextOverflow::Visible {
            continue; // the item's first row is clipped: no marker
        }
        let stamp = overflow_stamp(outside, ctx);
        let fm = m.font_metrics(&para.laid.style, para.laid.style.size);
        match &mk.kind {
            super::list::Marker::None => {}
            super::list::Marker::Shape(tok) => {
                let cyy = first - fm.x_height * 0.5;
                shapes.push_str(
                    &super::list::shape_marker(tok, mk.col, cyy, mk.r, &ctx.paint(&mk.fill))
                        .replacen("/>", &format!("{stamp}/>"), 1),
                );
            }
            super::list::Marker::Text(t) => {
                if let Some(paths) = marker_paths.as_mut() {
                    let st = &para.laid.style;
                    let traced = ctx.outliner.advances(t, st, st.size).and_then(|adv| {
                        let w = adv.last().copied().unwrap_or(0.0);
                        ctx.outline(t, st, st.size, mk.col - w, first)
                    });
                    match traced {
                        Some(d) => paths.push((d, mk.fill.clone())),
                        None => {
                            ctx.record_miss(ToPathError::font(st, "no font file to trace"));
                            marker_paths = None;
                        }
                    }
                }
                markers.push_str(&format!(
                    "<tspan x=\"{}\" y=\"{}\" text-anchor=\"end\" fill=\"{}\" font-size=\"{}\"{stamp}>",
                    fmt(mk.col),
                    fmt(first),
                    ctx.paint(&mk.fill),
                    fmt(para.laid.style.size)
                ));
                push_escaped(&mut markers, t, false);
                markers.push_str("</tspan>");
            }
        }
    }
    // pass 2: the lines the box shows, placed and truncated as its text-overflow
    // asks; one <text> per paragraph
    let visible = visible_paragraph_lines(&items, block_top, cy, ch, spec.text_overflow, m);
    let backdrops = |out: &mut String, which: Backdrops| {
        for (para, shifted) in items.iter().zip(&visible) {
            emit_run_backdrops(
                out,
                shifted,
                para.laid.anchor.svg(),
                para.laid.rtl,
                &para.laid.style,
                para.laid.style.size,
                para.fill,
                &para.emits,
                which,
                m,
                ctx,
            );
        }
    };

    // the textbox's filter (§8, *text*): its region is where the paragraphs land
    let filter = lower_filter_attr(node, out, ctx, || {
        let mut bb: Option<Rect> = None;
        for (para, shifted) in items.iter().zip(&visible) {
            let r = lines_bbox(
                shifted,
                para.laid.anchor.svg(),
                para.laid.rtl,
                &para.laid.style,
                para.laid.style.size,
                &para.emits,
                m,
            );
            bb = match (bb, r) {
                (Some(a), Some(b)) => Some(a.union(b)),
                (a, b) => a.or(b),
            };
        }
        bb
    });
    // Create-outlines in paragraph mode (§6.12): every paragraph's lines traced
    // in its own style, run by run where a run paints, the markers with them;
    // live below when any line cannot be (the miss recorded for `to_path`)
    let traced: Option<Vec<Vec<PaintedPath>>> = if outline && marker_paths.is_some() {
        items
            .iter()
            .zip(&visible)
            .map(|(para, shifted)| {
                trace_lines(
                    shifted,
                    para.laid.anchor,
                    para.laid.rtl,
                    &para.laid.style,
                    para.laid.style.size,
                    gx,
                    &para.emits,
                    m,
                    ctx.outliner,
                    ctx,
                )
                .map(|(paths, _)| paths)
            })
            .collect()
    } else {
        None
    };
    if let Some(traced) = traced {
        out.push_str("<g");
        out.push_str(pos);
        out.push('>');
        if filter.is_some() {
            backdrops(out, Backdrops::Highlights);
        }
        open_filter_group(out, filter.as_deref());
        out.push_str(&shapes);
        for (d, fill) in marker_paths.unwrap_or_default() {
            out.push_str(&format!("<g fill=\"{}\"><path d=\"", ctx.paint(&fill)));
            push_escaped(out, &d, true);
            out.push_str("\"/></g>");
        }
        for ((para, shifted), paths) in items.iter().zip(&visible).zip(traced) {
            if shifted.is_empty() {
                continue;
            }
            emit_run_backdrops(
                out,
                shifted,
                para.laid.anchor.svg(),
                para.laid.rtl,
                &para.laid.style,
                para.laid.style.size,
                para.fill,
                &para.emits,
                if filter.is_some() {
                    Backdrops::Decorations
                } else {
                    Backdrops::All
                },
                m,
                ctx,
            );
            let ppos = pos_attr(para.src, ctx);
            if paths.iter().any(|p| p.fill.is_some() || p.border.is_some()) {
                push_painted_paths(out, &paths, para.fill, "", &ppos, ctx);
            } else {
                let paths: Vec<(String, bool)> =
                    paths.into_iter().map(|p| (p.d, p.overflow)).collect();
                push_outline_group(out, para.fill, "", &ppos, &paths, ctx);
            }
        }
        close_filter_group(out, filter.as_deref());
        out.push_str("</g>");
        return;
    }
    // one keyed <g> wraps the per-paragraph <text>s: the TEXTBOX keeps a
    // rendered identity (and so a real bbox) of its own — the paragraphs
    // carry only their own <x:p> keys (found live: a multi-paragraph
    // textbox had no element to select, and its paragraphs were promoted
    // to free-standing objects)
    out.push_str("<g");
    out.push_str(pos);
    out.push('>');
    if filter.is_some() {
        backdrops(out, Backdrops::Highlights);
    }
    open_filter_group(out, filter.as_deref());
    out.push_str(&shapes);
    if !markers.is_empty() {
        out.push_str("<text text-anchor=\"start\"");
        push_font_attrs(out, base_style, base_style.size, base_fill, ctx);
        out.push('>');
        out.push_str(&markers);
        out.push_str("</text>");
    }
    for (para, shifted) in items.iter().zip(&visible) {
        if shifted.is_empty() {
            continue;
        }
        emit_run_backdrops(
            out,
            shifted,
            para.laid.anchor.svg(),
            para.laid.rtl,
            &para.laid.style,
            para.laid.style.size,
            para.fill,
            &para.emits,
            if filter.is_some() {
                Backdrops::Decorations
            } else {
                Backdrops::All
            },
            m,
            ctx,
        );
        out.push_str(&format!("<text text-anchor=\"{}\"", para.laid.anchor.svg()));
        push_font_attrs(out, &para.laid.style, para.laid.style.size, para.fill, ctx);
        if para.laid.rtl {
            out.push_str(" direction=\"rtl\"");
        }
        // the paragraph's own spacing (§6.16), as the box's goes on a single flow
        if para.laid.style.letter_spacing != 0.0 {
            out.push_str(&format!(
                " letter-spacing=\"{}\"",
                fmt(para.laid.style.letter_spacing)
            ));
        }
        if para.laid.style.word_spacing != 0.0 {
            out.push_str(&format!(
                " word-spacing=\"{}\"",
                fmt(para.laid.style.word_spacing)
            ));
        }
        out.push_str(&pos_attr(para.src, ctx)); // per-paragraph source map
        out.push('>');
        for l in shifted {
            emit_line(
                out,
                l,
                &para.laid.style,
                para.laid.style.size,
                gx,
                m,
                &para.emits,
                ctx,
            );
        }
        out.push_str("</text>");
    }
    close_filter_group(out, filter.as_deref());
    out.push_str("</g>");
}

/// Pass 2 of paragraph flow: the engine's [`clip_paragraphs`] over the plan.
pub(super) fn visible_paragraph_lines(
    items: &[ParaPlan],
    block_top: f64,
    cy: f64,
    ch: f64,
    overflow: TextOverflow,
    m: &dyn Measurer,
) -> Vec<Vec<PlacedLine>> {
    clip_paragraphs(
        items.iter().map(|p| &p.laid),
        block_top,
        cy,
        ch,
        overflow,
        m,
    )
}

/// One planned paragraph of a §6.16 flow: lines laid in a RELATIVE frame
/// (paragraph 0's cap-top at 0) — shift baselines by the plan's `block_top`
/// to place them in the box.
pub(super) struct ParaPlan<'a> {
    /// The stacked block: lines in the RELATIVE frame (paragraph 0's cap-top
    /// at 0 — shift by the plan's `block_top`), width, style and anchor.
    pub(super) laid: ParaLines,
    pub(super) fill: &'a str,
    /// The paragraph's styled-run emit table (`collect_runs`) — `<tspan>`
    /// overrides render styled in paragraph mode, same as a plain box.
    pub(super) emits: Vec<EmitAttrs>,
    pub(super) src: roxmltree::Node<'a, 'a>,
    /// A list item's marker in the gutter (§6.16, lists in a textbox).
    pub(super) marker: Option<PlanMarker>,
}

/// A list item's resolved marker, planned with the paragraph: what to draw,
/// the gutter column its right edge sits on, the paint and the shape radius.
pub(super) struct PlanMarker {
    pub(super) kind: super::list::Marker,
    pub(super) col: f64,
    pub(super) fill: String,
    pub(super) r: f64,
}

/// Pass 1 of paragraph flow (§6.16), shared verbatim by emission and the
/// editor's text-layout seam (docs/Incremental.md §6) so the two can never
/// drift: lay each `<x:p>` as its own wrapped block, stack top-down with the
/// paragraph gaps, and compute the block's valigned top.
pub(super) fn plan_paragraphs<'a>(
    node: roxmltree::Node<'a, 'a>,
    paras: &[roxmltree::Node<'a, 'a>],
    base_style: &TextStyle,
    base_fill: &'a str,
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> (Vec<ParaPlan<'a>>, f64) {
    let cx = spec.x + spec.padding;
    let cy = spec.y + spec.padding;
    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let layout = |p: roxmltree::Node<'a, 'a>, in_list: bool, prop: &str| {
        block_layout(p, node, in_list, prop).and_then(|(v, _)| v)
    };
    // paragraphs default to a visible gap — 0.35em, the same default rhythm
    // lists use for item-spacing — so Enter reads as a paragraph while a
    // forced line break (<tbreak/>) stays at the bare line advance; an
    // authored paragraph-spacing (zero included) wins
    let declared: elements::Textbox = params_of(node);
    let default_spacing = declared.paragraph_spacing.unwrap_or(base_style.size * 0.35);

    // per-block style and fill: the box's, with what the `<x:p>` — or the item,
    // then its `<x:list>` — sets over it (query.rs, the same levels a query reads)
    let levels = |p: roxmltree::Node<'a, 'a>| block_levels(p).map(|(l, _)| l).unwrap_or_default();

    // pass 1 at a given SCALE (fit="shrink" searches it below): every text
    // metric — the font sizes, the paragraph gaps, the first-line indent —
    // scales together, so the block shrinks like a zoom while the box
    // geometry stays put. Returns the plans and the stacked block height.
    //
    // Stacking is BASELINE-advance based (the Slides convention): a
    // paragraph's first baseline sits one line advance (its own
    // `size × line-height`) below the previous paragraph's last baseline,
    // plus the explicit space-after/before gaps — so pressing Enter spaces
    // exactly like a wrapped line, never tighter. An EMPTY paragraph still
    // owns one row (a blank line the caret can sit on); it just emits no
    // glyphs. Styled runs and `<tbreak/>` forced breaks flow through
    // `collect_runs` → `layout_area_runs`, sharing the plain textbox's
    // pipeline.
    // the blocks as a flat list of paragraph-like units: a <x:p> as itself,
    // a <x:list>'s items each with its gutter (text column and marker) and
    // the list's own spacing between them (§6.16, lists in a textbox)
    struct Unit<'n> {
        src: roxmltree::Node<'n, 'n>,
        /// the list this item belongs to, for its style and spacing
        list: Option<roxmltree::Node<'n, 'n>>,
        /// the text column's inset from the content edge (unscaled)
        inset: f64,
        marker: Option<(
            super::list::Marker,
            f64, /* gutter col inset */
            Option<String>,
        )>,
        /// first item of its list: the gap before it is a paragraph gap
        first_in_list: bool,
    }
    let mut units: Vec<Unit<'a>> = Vec::new();
    for &b in paras.iter() {
        if b.tag_name().name() != "list" {
            units.push(Unit {
                src: b,
                list: None,
                inset: 0.0,
                marker: None,
                first_in_list: false,
            });
            continue;
        }
        let lp: elements::List = params_of(b);
        let lsize = attr_pos(b, "font-size", base_style.size);
        let indent_step = lp.indent.unwrap_or(lsize * 1.5).max(0.0);
        let marker_gap = lp.marker_gap.unwrap_or(lsize * 0.5);
        let mut counters: Vec<u32> = Vec::new();
        let mut first = true;
        for li in b
            .children()
            .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "li")
        {
            let level = attr_num(li, "indent", 0.0).max(0.0) as usize;
            let inset = (level as f64 + 1.0) * indent_step;
            if counters.len() <= level {
                counters.resize(level + 1, 0);
            }
            counters.truncate(level + 1);
            counters[level] += 1;
            let kind = li.attribute("list").unwrap_or(lp.list.as_str());
            let marker_attr = li.attribute("marker").or(lp.marker.as_deref());
            let marker = match marker_attr {
                Some(mk) => match super::list::shape_token(mk) {
                    Some(tok) => super::list::Marker::Shape(tok),
                    None => super::list::Marker::Text(mk.to_string()),
                },
                None => match kind {
                    "none" => super::list::Marker::None,
                    "number" => super::list::Marker::Text(super::list::number_marker(
                        level,
                        counters[level],
                    )),
                    _ => super::list::Marker::Shape(super::list::bullet_shape(level)),
                },
            };
            units.push(Unit {
                src: li,
                list: Some(b),
                inset,
                marker: Some((marker, inset - marker_gap, lp.marker_fill.clone())),
                first_in_list: first,
            });
            first = false;
        }
    }

    // one trial at a given SCALE (fit="shrink" searches it below): every text
    // metric — the font sizes, the gaps, the insets, the first-line indent —
    // scales together, so the stack shrinks like a zoom while the box geometry
    // stays put. The runs are re-collected at each scale so a run's size
    // resolves against its paragraph's scaled size, as emission will see it.
    let plan_at = |scale: f64| -> (Vec<ParaPlan<'a>>, f64) {
        let mut blocks: Vec<ParaUnit> = Vec::with_capacity(units.len());
        let mut extra: Vec<(&'a str, Vec<EmitAttrs>, Option<PlanMarker>, TextStyle)> =
            Vec::with_capacity(units.len());
        for (i, u) in units.iter().enumerate() {
            let p = u.src;
            // an item takes the list's style (font-* on <x:list>) under the
            // box's, then its own font-size and case; a paragraph its own font-*
            let block = levels(p);
            let mut pstyle = overlay(&block, base_style);
            pstyle.size *= scale;
            // the spacing is in the same units as the size: it shrinks with it
            pstyle.letter_spacing *= scale;
            pstyle.word_spacing *= scale;
            // spacing: between items of one list its item-spacing; into the
            // first item, and around a paragraph, the paragraph gaps; the gap
            // AFTER a list's last item is a paragraph gap, not its
            // item-spacing — the next block is a paragraph or another list
            let (space_before, space_after) = match u.list {
                Some(l) => {
                    let item_gap = attr_num(l, "item-spacing", pstyle.size / scale * 0.35) * scale;
                    let before = if u.first_in_list {
                        attr_num(l, "space-before", 0.0) * scale
                    } else {
                        0.0
                    };
                    let last_of_list =
                        i + 1 == units.len() || units.get(i + 1).is_some_and(|n| n.list != Some(l));
                    let after = if last_of_list {
                        attr_num(l, "space-after", default_spacing) * scale
                    } else {
                        item_gap
                    };
                    (before, after)
                }
                None => (
                    attr_num(p, "space-before", 0.0) * scale,
                    attr_num(p, "space-after", default_spacing) * scale,
                ),
            };
            let (segments, pstyles, emits) = collect_runs(p, &pstyle);
            blocks.push(ParaUnit {
                segments,
                styles: pstyles,
                space_before,
                space_after,
                inset: u.inset * scale,
                // align, indent and direction by the block rules a query reports
                // (query.rs `block_layout`): a paragraph's own over the box's, an
                // item always start-aligned and unindented
                align: Align::parse(layout(p, u.list.is_some(), "align").unwrap_or("start")),
                text_indent: layout(p, u.list.is_some(), "text-indent")
                    .and_then(parse_num)
                    .unwrap_or(0.0)
                    * scale,
                rtl: layout(p, u.list.is_some(), "direction") == Some("rtl"),
            });
            let marker = u.marker.as_ref().map(|(kind, col_inset, fill)| PlanMarker {
                kind: match kind {
                    super::list::Marker::None => super::list::Marker::None,
                    super::list::Marker::Shape(t) => super::list::Marker::Shape(t),
                    super::list::Marker::Text(t) => super::list::Marker::Text(t.clone()),
                },
                col: cx + col_inset * scale,
                fill: fill
                    .clone()
                    .or_else(|| u.list.and_then(|l| l.attribute("fill").map(str::to_string)))
                    .unwrap_or_else(|| base_fill.to_string()),
                r: pstyle.size
                    * 0.16
                    * u.list
                        .map_or(1.0, |l| attr_num(l, "marker-size", 1.0).max(0.0)),
            });
            let fill = level_value(&block, "fill").unwrap_or(base_fill);
            extra.push((fill, emits, marker, pstyle));
        }
        let (laid, block_h) = stack_paragraphs(&blocks, cx, cw, m);
        let items = laid
            .into_iter()
            .zip(extra)
            .zip(units.iter())
            .map(|((laid, (fill, emits, marker, _)), u)| ParaPlan {
                laid,
                fill,
                emits,
                src: u.src,
                marker,
            })
            .collect();
        (items, block_h)
    };

    // fit="shrink" (§6.1, extended to paragraph mode): bisect the stack's scale
    // like `fit_size` bisects the font size, on the engine's fit predicate
    // (stacked height within the box, widest lone word within the width).
    // `fit-min` floors the BASE size; per-paragraph overrides scale
    // proportionally with it. Each trial re-lays every paragraph: the cost is
    // paid only by overflowing shrink boxes.
    let (items, block_h) = match spec.fit {
        Fit::Shrink { min } if base_style.size > 0.0 => {
            let fits = |s: f64| {
                let (items, h) = plan_at(s);
                paragraphs_fit(items.iter().map(|it| &it.laid), h, cw, ch, m)
            };
            let min_scale = (min / base_style.size).clamp(0.0, 1.0);
            plan_at(bisect_fit(min_scale, 1.0, fits))
        }
        _ => plan_at(1.0),
    };
    let valign = VAlign::parse(node.attribute("valign").unwrap_or("top"));
    (items, block_top(valign, cy, ch, block_h))
}

/// Build the rectangular [`AreaSpec`] for a textbox, taking geometry from `geom`
/// (the textbox itself, or a referenced `rect`) and options from the textbox `node`.
pub(super) fn textbox_area_spec(node: roxmltree::Node, geom: roxmltree::Node) -> AreaSpec {
    // One declaration read twice, in its two roles: `geom` supplies the box (it
    // may be a referenced <rect> rather than the textbox itself), `node` the
    // layout options.
    let opts: elements::Textbox = params_of(node);
    let box_: elements::Textbox = params_of(geom);
    AreaSpec {
        x: box_.x,
        y: box_.y,
        width: box_.width,
        height: box_.height,
        padding: opts.padding,
        align: Align::parse(opts.align.as_str()),
        valign: VAlign::parse(opts.valign.as_str()),
        fit: match opts.fit {
            elements::FitParam::Shrink => Fit::Shrink { min: opts.fit_min },
            elements::FitParam::None => Fit::None,
        },
        text_overflow: match opts.overflow {
            elements::OverflowVisibility::Visible => TextOverflow::Visible,
            elements::OverflowVisibility::Hidden => {
                TextOverflow::parse(opts.text_overflow.as_str())
            }
        },
        text_indent: opts.text_indent,
        rtl: params_of::<elements::Textbox>(node).direction == elements::DirectionParam::Rtl,
    }
}

/// ` data-xsvg-overflow="1"` on what renders for a line the box would have
/// cropped (§6.6, `overflow="visible"`) — its `<tspan>`, its outline `<path>`,
/// its highlight and decoration rects, its list marker — in editor output only
/// (the source map is on). The compiler only marks; the host chooses the look
/// (a ghost while editing). An exported document carries no stamp.
pub(super) fn overflow_stamp(overflow: bool, ctx: &Ctx) -> &'static str {
    if overflow && ctx.sourcemap.get() {
        " data-xsvg-overflow=\"1\""
    } else {
        ""
    }
}

/// The truncation a box's `overflow` / `text-overflow` select (§6.6):
/// `overflow="visible"` crops nothing, whatever `text-overflow` says — CSS's
/// `text-overflow` acts only on content the box clips.
pub(super) fn overflow_mode(node: roxmltree::Node) -> TextOverflow {
    if node.attribute("overflow").map(str::trim) == Some("visible") {
        TextOverflow::Visible
    } else {
        TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip"))
    }
}

/// `<textArea>` (Rung 2, SVG Tiny 1.2 vocabulary): flowed text per the spec —
/// `text-align` (inline), `display-align` (block), `line-increment` (line height),
/// `auto` width/height, and `<tbreak/>` forced breaks.
pub(super) fn emit_text_area(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node, ctx);
    let spec = TextAreaSpec {
        x: attr_num(node, "x", 0.0),
        y: attr_num(node, "y", 0.0),
        width: dim_attr(node, "width"),
        height: dim_attr(node, "height"),
        text_align: TextAlign::parse(node.attribute("text-align").unwrap_or("start")),
        display_align: DisplayAlign::parse(node.attribute("display-align").unwrap_or("auto")),
        line_increment: line_increment_attr(node, style.size),
        text_overflow: overflow_mode(node),
    };
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let outline = node.attribute((XSVG_NS, "outline")) == Some("true") || ctx.force_outline.get();
    let (segments, styles, emits) = collect_runs(node, &style);
    let layout = layout_text_area_runs(&segments, &styles, &spec, m);
    let filter = lower_text_filter(node, &layout, &style, &emits, out, ctx);
    write_area_text(
        out,
        &layout,
        &style,
        node.attribute("fill").unwrap_or("#000"),
        &text_border_attrs(node, ctx),
        gx,
        m,
        &emits,
        &format!(
            "{}{}",
            pos_attr(node, ctx),
            ctx.carry_at(node.range().start, true)
        ),
        span_of(node),
        outline,
        filter.as_deref(),
        ctx.outliner,
        ctx,
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_area_text(
    out: &mut String,
    layout: &AreaLayout,
    style: &TextStyle,
    fill: &str,
    stroke: &str,
    glyph_x_scale: f64,
    m: &dyn Measurer,
    emits: &[EmitAttrs],
    pos: &str,
    span: (usize, usize),
    outline: bool,
    filter: Option<&str>,
    outliner: &dyn GlyphOutliner,
    ctx: &Ctx,
) {
    let backdrops = |out: &mut String, which: Backdrops| {
        emit_run_backdrops(
            out,
            &layout.lines,
            layout.anchor.svg(),
            layout.rtl,
            style,
            layout.font_size,
            fill,
            emits,
            which,
            m,
            ctx,
        );
    };
    // Create-outlines (§6.12): emit each line as a <path> tracing its glyphs instead
    // of live <text>. All-or-nothing — if the outliner can't do any line (font bytes
    // unavailable) we fall through to live text. v0 uses the base style per line
    // (per-run styling / justify / glyph-x-scale don't apply) and anchors via the
    // measured line width.
    if outline {
        if let Some((paths, traced)) = trace_lines(
            &layout.lines,
            layout.anchor,
            layout.rtl,
            style,
            layout.font_size,
            glyph_x_scale,
            emits,
            m,
            outliner,
            ctx,
        ) {
            // a caret_stops query is listening: the stops of each traced line, at
            // the very placement the outline above was traced from (stops.rs)
            if ctx.recording_stops() {
                let metrics = m.font_metrics(style, layout.font_size);
                for (text, start_x, baseline) in &traced {
                    let run = outliner
                        .advances(text, style, layout.font_size)
                        .and_then(|adv| stop_run(text, *start_x, *baseline, &metrics, &adv));
                    if let Some(run) = run {
                        ctx.record_stops(span, run);
                    }
                }
            }
            // `to_path` replaces one node with one: a box with backdrops wraps
            // them and its paths in a group carrying the node's attributes
            let wrap = ctx.is_to_path_target(span.0) && has_backdrops(&layout.lines, emits);
            if wrap {
                out.push_str("<g");
                out.push_str(pos);
                out.push('>');
            }
            let pos = if wrap { "" } else { pos };
            if filter.is_some() {
                backdrops(out, Backdrops::Highlights);
            }
            open_filter_group(out, filter);
            backdrops(
                out,
                if filter.is_some() {
                    Backdrops::Decorations
                } else {
                    Backdrops::All
                },
            );
            if paths.iter().any(|p| p.fill.is_some() || p.border.is_some()) {
                push_painted_paths(out, &paths, fill, stroke, pos, ctx);
            } else {
                let paths: Vec<(String, bool)> =
                    paths.into_iter().map(|p| (p.d, p.overflow)).collect();
                push_outline_group(out, fill, stroke, pos, &paths, ctx);
            }
            close_filter_group(out, filter);
            if wrap {
                out.push_str("</g>");
            }
            return;
        }
        // else: outliner unavailable → fall through to live <text> below
    }

    if filter.is_some() {
        backdrops(out, Backdrops::Highlights);
    }
    open_filter_group(out, filter);
    backdrops(
        out,
        if filter.is_some() {
            Backdrops::Decorations
        } else {
            Backdrops::All
        },
    );
    out.push_str(&format!("<text text-anchor=\"{}\"", layout.anchor.svg()));
    push_font_attrs(out, style, layout.font_size, fill, ctx);
    if layout.rtl {
        out.push_str(" direction=\"rtl\"");
    }
    out.push_str(pos);
    out.push_str(stroke); // border/stroke (§6.17), values resolved + escaped where built
    if style.letter_spacing != 0.0 {
        out.push_str(&format!(
            " letter-spacing=\"{}\"",
            fmt(style.letter_spacing)
        ));
    }
    if style.word_spacing != 0.0 {
        out.push_str(&format!(" word-spacing=\"{}\"", fmt(style.word_spacing)));
    }
    out.push('>');
    for line in &layout.lines {
        emit_line(
            out,
            line,
            style,
            layout.font_size,
            glyph_x_scale,
            m,
            emits,
            ctx,
        );
    }
    out.push_str("</text>");
    close_filter_group(out, filter);
}

/// An outlined line's runs traced one by one (§6.12): each at its pen position
/// in the whole line — the line's own `advances`, so kerning across a run
/// boundary is kept and the pieces sit where one trace would put them — with
/// the paint it overrides. Whitespace-only runs draw nothing and are skipped.
/// `None` when the outliner declines any of it; the caller degrades.
#[allow(clippy::too_many_arguments)]
/// A traced line's `(text, start_x, baseline)`, for caret stops.
pub(super) type TracedLine = (String, f64, f64);

/// Whether any run on the lines has a highlight or a decoration to bake.
pub(super) fn has_backdrops(lines: &[PlacedLine], emits: &[EmitAttrs]) -> bool {
    lines.iter().any(|l| {
        l.runs.iter().any(|r| {
            emits
                .get(r.style)
                .is_some_and(|a| a.highlight.is_some() || a.decoration.is_some())
        })
    })
}

/// Trace laid-out lines to glyph paths (§6.12): each line from its anchored
/// left edge — the edge the backdrops and the live text use — whole in the base
/// style, or run by run when a run paints (its fill, its border) or sets its own
/// type (a weight, a slant, a face, a size), each run in its own style and the
/// pen advanced by that style's width. All or nothing: the first run the
/// outliner declines records the miss (`to_path` reports it) and returns
/// `None`, and the caller renders live. Also returns `(text, start_x,
/// baseline)` of each traced line, for caret stops.
#[allow(clippy::too_many_arguments)]
pub(super) fn trace_lines(
    lines: &[PlacedLine],
    anchor: Anchor,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    glyph_x_scale: f64,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
    outliner: &dyn GlyphOutliner,
    ctx: &Ctx,
) -> Option<(Vec<PaintedPath>, Vec<TracedLine>)> {
    let mut paths = Vec::new();
    let mut traced = Vec::new();
    let split = |l: &PlacedLine| {
        l.runs
            .iter()
            .any(|r| emits.get(r.style).is_some_and(|a| a.paints() || a.types()))
    };
    for line in lines {
        if line.text.is_empty() {
            continue; // blank line (e.g. from <tbreak/>) → nothing to trace
        }
        // a justified line widens its spaces by `extra` (§6.9); otherwise a
        // glyph-x-scale stretches the whole line from its left edge, as the
        // live line's textLength does (emit_line applies one or the other)
        let (left, width, extra) = line_extent(line, anchor.svg(), rtl, style, size, emits, m);
        let scale = if extra == 0.0 && glyph_x_scale > 0.0 && (glyph_x_scale - 1.0).abs() > 1e-6 {
            Some(glyph_x_scale)
        } else {
            None
        };
        let first = paths.len();
        let ok = if split(line) {
            trace_runs(
                line, left, width, extra, rtl, style, size, emits, outliner, ctx,
            )
            .map(|p| paths.extend(p))
        } else {
            trace_spaced(&line.text, style, size, left, line.baseline, extra, ctx).map(|d| {
                paths.push(PaintedPath {
                    d,
                    fill: None,
                    border: None,
                    overflow: line.overflow,
                })
            })
        };
        if ok.is_none() {
            if ctx.outline_miss.borrow().is_none() {
                ctx.record_miss(ToPathError::font(style, "no font file to trace"));
            }
            return None;
        }
        if let Some(gx) = scale {
            let about = crate::kurbo::Affine::translate((left, 0.0))
                * crate::kurbo::Affine::scale_non_uniform(gx, 1.0)
                * crate::kurbo::Affine::translate((-left, 0.0));
            for p in &mut paths[first..] {
                p.d = transform_d(&p.d, about)?;
            }
        }
        traced.push((line.text.clone(), left, line.baseline));
    }
    Some((paths, traced))
}

/// A run outlined with its spacing: whole in one call when it has none; else
/// glyph by glyph, each at its kerned pen (`advances`) plus the tracking before
/// it, the word gap and the justify slack after each space — the pens
/// `line_advance` lays the live text by. A glyph the font lacks is skipped as
/// the whole-run outline skips it; a font the outliner lacks is a miss.
fn trace_spaced(
    text: &str,
    style: &TextStyle,
    size: f64,
    x: f64,
    baseline: f64,
    extra_per_space: f64,
    ctx: &Ctx,
) -> Option<String> {
    let (ls, ws) = (style.letter_spacing, style.word_spacing + extra_per_space);
    if ls == 0.0 && ws == 0.0 {
        return ctx.outline(text, style, size, x, baseline);
    }
    let adv = ctx.outliner.advances(text, style, size)?;
    let mut d = String::new();
    let mut spacing = 0.0;
    let mut buf = [0u8; 4];
    for (i, ch) in text.chars().enumerate() {
        let pen = x + if i == 0 { 0.0 } else { adv[i - 1] } + spacing;
        if !ch.is_whitespace() {
            if let Some(g) = ctx.outline(ch.encode_utf8(&mut buf), style, size, pen, baseline) {
                d.push_str(&g);
            }
        }
        spacing += ls;
        if ch == ' ' {
            spacing += ws;
        }
    }
    (!d.is_empty()).then_some(d)
}

/// A line traced run by run: from the start edge (the left one, or the right
/// one under `rtl`, runs laying leftward), each run in its own measuring style
/// — its weight, slant, face and size over the box's — and the pen advanced by
/// the outliner's own width for it, so the geometry and the placement come from
/// one font.
#[allow(clippy::too_many_arguments)]
fn trace_runs(
    line: &PlacedLine,
    left: f64,
    width: f64,
    extra: f64,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    emits: &[EmitAttrs],
    outliner: &dyn GlyphOutliner,
    ctx: &Ctx,
) -> Option<Vec<PaintedPath>> {
    let mut pieces = Vec::new();
    let mut x = if rtl { left + width } else { left };
    for r in &line.runs {
        let a = emits.get(r.style);
        let (st, rs) = a
            .map(|a| run_measure_style(a, style, size))
            .unwrap_or((style.clone(), size));
        // the run's width as the renderer lays it: its glyphs, the tracking
        // after every glyph (the next run starts past the last one's), its
        // word gaps, the justify slack in its spaces
        let w = match outliner.advances(&r.text, &st, rs) {
            Some(adv) => {
                adv.last().copied().unwrap_or(0.0)
                    + r.text.chars().count() as f64 * st.letter_spacing
                    + spaces_in(&r.text) * (st.word_spacing + extra)
            }
            None => {
                ctx.record_miss(ToPathError::font(&st, "no font file to trace"));
                return None;
            }
        };
        if rtl {
            x -= w;
        }
        if !r.text.trim().is_empty() {
            let Some(d) = trace_spaced(&r.text, &st, rs, x, line.baseline, extra, ctx) else {
                ctx.record_miss(ToPathError::font(&st, "no font file to trace"));
                return None;
            };
            pieces.push(PaintedPath {
                d,
                fill: a.and_then(|a| a.fill.clone()),
                border: a.and_then(|a| a.border.as_ref()).map(|b| b.attrs(ctx)),
                overflow: line.overflow,
            });
        }
        if !rtl {
            x += w;
        }
    }
    Some(pieces)
}

/// How far a `baseline-shift` value raises a run, in user units (positive = up).
///
/// `baseline-shift` is emitted verbatim and the renderer moves the glyphs, so a
/// highlight baked beside them has to land where the renderer will put them. Lengths
/// are exact. `super`/`sub` and percentages are renderer-defined; this is Chromium's
/// model, measured: `super`/`sub` = ± half the em box (ascent + descent), and a
/// percentage is of the run's own font size. Another renderer may place a
/// super/subscript slightly differently from its highlight.
pub(super) fn baseline_shift_px(v: &str, em_box: f64, size: f64) -> f64 {
    let v = v.trim();
    match v {
        "super" => em_box / 2.0,
        "sub" => -em_box / 2.0,
        "baseline" => 0.0,
        _ => {
            if let Some(p) = v.strip_suffix('%') {
                p.trim().parse::<f64>().map_or(0.0, |p| p / 100.0 * size)
            } else if let Some(e) = v.strip_suffix("em") {
                e.trim().parse::<f64>().map_or(0.0, |e| e * size)
            } else {
                v.strip_suffix("px")
                    .unwrap_or(v)
                    .trim()
                    .parse::<f64>()
                    .unwrap_or(0.0)
            }
        }
    }
}

/// The measuring style of a run: the base with the run's overrides folded in,
/// its size riding the same fit scale as the base size.
pub(super) fn run_measure_style(a: &EmitAttrs, style: &TextStyle, size: f64) -> (TextStyle, f64) {
    let scale = if style.size > 0.0 {
        size / style.size
    } else {
        1.0
    };
    let mut st = style.clone();
    if let Some(w) = &a.weight {
        st.weight = w.clone();
    }
    if let Some(s) = &a.style {
        st.style = s.clone();
    }
    if let Some(f) = &a.family {
        st.family = f.clone();
    }
    let rs = a.size.map(|s| s * scale).unwrap_or(size);
    st.size = rs;
    (st, rs)
}

fn spaces_in(text: &str) -> f64 {
    text.matches(' ').count() as f64
}

/// The slack a justified line puts into each of its spaces (§6.9): the content
/// width less the line's natural advance (every run measured in its own style),
/// shared by the spaces. `None` when the line is not justified or has no space
/// to widen. Emission, the highlight rects and the layout seam all read it, so
/// the rendered line, its backdrops and the caret agree.
pub(super) fn justify_space(
    line: &PlacedLine,
    style: &TextStyle,
    size: f64,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
) -> Option<f64> {
    let w = line.justify_width?;
    let spaces = spaces_in(&line.text);
    if spaces == 0.0 {
        return None;
    }
    let natural: f64 = line
        .runs
        .iter()
        .map(|r| {
            let (st, rs) = emits
                .get(r.style)
                .map(|a| run_measure_style(a, style, size))
                .unwrap_or((style.clone(), size));
            line_advance(&r.text, &st, rs, m)
        })
        .sum();
    Some((w - natural) / spaces)
}

/// The backdrops of a line's styled runs, drawn as `<rect>`s BEHIND the `<text>`:
/// `x:highlight` fills the run's em box, and `text-decoration` draws the run's
/// underline and line-through as rects of the run's own colour. Decorations are
/// baked rather than left to the renderer because engines draw SVG decorations
/// per glyph once `letter-spacing` (WebKit) or `textLength` (Chromium) is in
/// play and leave the gaps bare — a dashed underline (found live) — and because
/// a rect spans exactly the advance the compiler measured, letter-spacing and a
/// justified line's widened spaces included, so it agrees with the glyphs, the
/// caret and the highlight. Position and thickness are the font's own lines
/// where a measurer has them ([`FontMetrics`]), proportions otherwise; a run's
/// decoration being its own colour, behind the glyphs reads as over them. Each
/// run is placed by walking the line from its start edge (`anchor` = the
/// `<text>`'s text-anchor; under `rtl` the start edge is the right one and runs
/// lay leftward); a raised or lowered run carries its backdrops with it.
/// Nothing is emitted for lines without a highlighted or decorated run, so plain
/// output is byte-identical. `which` selects highlights, decorations or both
/// ([`Backdrops`]).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_run_backdrops(
    out: &mut String,
    lines: &[PlacedLine],
    anchor: &str,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    fill: &str,
    emits: &[EmitAttrs],
    which: Backdrops,
    m: &dyn Measurer,
    ctx: &Ctx,
) {
    let run_style = |a: &EmitAttrs| run_measure_style(a, style, size);
    let highlights = which != Backdrops::Decorations;
    let decorations = which != Backdrops::Highlights;
    let has_backdrop = |a: &EmitAttrs| {
        (highlights && a.highlight.is_some()) || (decorations && a.decoration.is_some())
    };
    for line in lines {
        if !line
            .runs
            .iter()
            .any(|r| emits.get(r.style).is_some_and(has_backdrop))
        {
            continue;
        }
        let (left, width, extra) = line_extent(line, anchor, rtl, style, size, emits, m);
        // runs lay out from the start edge: rightward in ltr, leftward in rtl
        let mut x = if rtl { left + width } else { left };
        for r in &line.runs {
            let a = emits.get(r.style);
            let (st, rs) = a.map(&run_style).unwrap_or((style.clone(), size));
            let w = line_advance(&r.text, &st, rs, m) + spaces_in(&r.text) * extra;
            if rtl {
                x -= w;
            }
            if let Some(a) = a.filter(|a| has_backdrop(a)) {
                let fm = m.font_metrics(&st, rs);
                // a raised or lowered run carries its backdrops with it
                let shift = a
                    .shift
                    .as_deref()
                    .map_or(0.0, |v| baseline_shift_px(v, fm.ascent + fm.descent, rs));
                let base = line.baseline - shift;
                let rect = |out: &mut String, y: f64, h: f64, color: &str| {
                    out.push_str(&format!(
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"{}/>",
                        fmt(x),
                        fmt(y),
                        fmt(w),
                        fmt(h),
                        ctx.paint(color),
                        overflow_stamp(line.overflow, ctx)
                    ));
                };
                if let Some(color) = a.highlight.as_deref().filter(|_| highlights) {
                    rect(out, base - fm.ascent, fm.ascent + fm.descent, color);
                }
                if let Some(d) = a.decoration.as_deref().filter(|_| decorations) {
                    let color = a.fill.as_deref().unwrap_or(fill);
                    for token in d.split_whitespace() {
                        match token {
                            "underline" => rect(
                                out,
                                base + fm.underline_position,
                                fm.underline_thickness,
                                color,
                            ),
                            "line-through" => rect(
                                out,
                                base - fm.strikeout_position - fm.strikeout_thickness / 2.0,
                                fm.strikeout_thickness,
                                color,
                            ),
                            _ => {}
                        }
                    }
                }
            }
            if !rtl {
                x += w;
            }
        }
    }
}

/// Emit one line as an outer positioning `<tspan x y>` (carrying the justify
/// `word-spacing` or the glyph-scale `textLength`) whose children are the line's styled runs — a bare string for base
/// runs, an inner `<tspan>` with the run's overrides for the rest.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_line(
    out: &mut String,
    line: &PlacedLine,
    style: &TextStyle,
    size: f64,
    glyph_x_scale: f64,
    m: &dyn Measurer,
    emits: &[EmitAttrs],
    ctx: &Ctx,
) {
    out.push_str(&format!(
        "<tspan x=\"{}\" y=\"{}\"{}",
        fmt(line.x),
        fmt(line.baseline),
        overflow_stamp(line.overflow, ctx)
    ));
    if line.justify_width.is_some() {
        // Justification (§6.9): the slack goes into the spaces, as `word-spacing`
        // on the line — renderers draw a `textLength`-stretched line's decorations
        // per glyph and leave the stretched gaps bare (a dashed underline), and
        // widening the spaces is what typesetting does anyway. Inner runs inherit
        // it. Takes precedence over glyph-x-scale on this line.
        if let Some(extra) = justify_space(line, style, size, emits, m) {
            if extra.abs() > 1e-9 {
                out.push_str(&format!(
                    " word-spacing=\"{}\"",
                    fmt(style.word_spacing + extra)
                ));
            }
        }
    } else if glyph_x_scale > 0.0 && (glyph_x_scale - 1.0).abs() > 1e-6 && !line.text.is_empty() {
        // A non-positive scale is meaningless (would emit a zero/negative textLength);
        // treat it as "no scaling" rather than emitting invalid SVG.
        let len = line_advance(&line.text, style, size, m) * glyph_x_scale;
        out.push_str(&format!(
            " textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"",
            fmt(len)
        ));
    }
    out.push('>');
    for run in &line.runs {
        match emits.get(run.style) {
            Some(a) if !a.is_empty() => {
                // a linked run wraps in <a href>; its paint overrides (if any)
                // ride a nested <tspan> inside the link
                if let Some(h) = &a.href {
                    out.push_str("<a");
                    emit_attr(out, "href", &Some(h.clone()));
                    out.push('>');
                }
                if a.paint_empty() {
                    push_escaped(out, &run.text, false);
                } else {
                    out.push_str("<tspan");
                    emit_attr(out, "fill", &a.fill);
                    emit_attr(out, "font-weight", &a.weight);
                    emit_attr(out, "font-style", &a.style);
                    emit_attr(out, "font-family", &a.family);
                    if let Some(s) = a.size {
                        // the run's size rides the same fit scale as the base size
                        let scale = if style.size > 0.0 {
                            size / style.size
                        } else {
                            1.0
                        };
                        out.push_str(&format!(" font-size=\"{}\"", fmt(s * scale)));
                    }
                    emit_attr(out, "baseline-shift", &a.shift);
                    if let Some(b) = &a.border {
                        out.push_str(&b.attrs(ctx));
                    }
                    out.push('>');
                    push_escaped(out, &run.text, false);
                    out.push_str("</tspan>");
                }
                if a.href.is_some() {
                    out.push_str("</a>");
                }
            }
            _ => push_escaped(out, &run.text, false),
        }
    }
    out.push_str("</tspan>");
}

/// Emit ` name="value"` (escaped) when the override is present.
pub(super) fn emit_attr(out: &mut String, name: &str, value: &Option<String>) {
    if let Some(v) = value {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        push_escaped(out, v, true);
        out.push('"');
    }
}

/// A textArea dimension: absent or `"auto"` → `None` (auto), else a parsed length.
pub(super) fn dim_attr(node: roxmltree::Node, name: &str) -> Option<f64> {
    match node.attribute(name) {
        None | Some("auto") => None,
        Some(v) => parse_num(v),
    }
}

/// `line-increment` resolution for `<textArea>`: an explicit value keeps SVG
/// Tiny semantics; on `auto`, an authored `line-height` (xsvg's own spacing
/// vocabulary, in em) drives the step at `size × line-height` — so a
/// `<tbreak/>` respects the same line spacing every other text element uses —
/// and only when NEITHER is authored does the spec default (1.1 × size) apply.
pub(super) fn line_increment_attr(node: roxmltree::Node, size: f64) -> LineIncrement {
    match node.attribute("line-increment") {
        None | Some("auto") => node
            .attribute("line-height")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
            .map(|lh| LineIncrement::Fixed(size * lh))
            .unwrap_or(LineIncrement::Auto),
        Some(v) => parse_num(v)
            .map(LineIncrement::Fixed)
            .unwrap_or(LineIncrement::Auto),
    }
}

// ============================================================================
// Textbox measurement for editors (docs/Incremental.md §8; consumer: xsvg-kit's
// paste and auto-size): how big a box must be to hold what is in it.

/// The size a rectangular `<x:textbox>` needs for its content, laid out at the
/// authored width with height unbounded, `valign` top, no shrink and no ellipsis.
#[derive(Clone, Debug, PartialEq)]
pub struct TextboxSize {
    /// Outer height that just holds every line: the cap band of the stacked
    /// blocks — the first baseline sits at cap height below the top, the last
    /// line's descent at the bottom — plus padding above and below. Compiled at
    /// this height the box renders every line, unclipped and unshrunk, with the
    /// descenders inside the box (§6.5 clips by baseline, so a box up to one
    /// descent shorter still shows every line, hanging its last descenders out).
    pub height: f64,
    /// Outer width the content uses: the widest line plus padding on both sides,
    /// at most the authored width. Lines are measured in their paragraph's style
    /// at the line's size, so a line whose runs change weight or family is
    /// approximate; the height is exact.
    pub width: f64,
}

pub(super) fn measure_textbox(node: roxmltree::Node, ctx: &Ctx) -> Option<TextboxSize> {
    let style = style_from(node, ctx);
    let geom = match node.attribute("in") {
        // curved region flow has no height to grow into: not measurable
        Some(r) => ctx
            .resolve_ref(node, r)
            .filter(|t| t.tag_name().name() == "rect")?,
        None => node,
    };
    let mut spec = textbox_area_spec(node, geom);
    // the question is how tall the box must be, so lay it out unbounded, from the
    // top, at the authored size — the same tall stand-in `stack_paragraphs` uses
    spec.height = 1e5;
    spec.valign = VAlign::Top;
    spec.fit = Fit::None;
    spec.text_overflow = TextOverflow::Clip;
    let m = ctx.m;
    let pad = spec.padding;
    let top = spec.y + pad;
    // the bottom of the last line and the widest line, over whatever the box holds;
    // `shift` is where the lines' frame sits (paragraph plans are relative to the
    // block top, a single flow is absolute)
    let mut bottom: Option<f64> = None;
    let mut widest = 0.0f64;
    let mut take = |lines: &[PlacedLine], pstyle: &TextStyle, shift: f64| {
        for l in lines {
            widest = widest.max(line_advance(&l.text, pstyle, l.size, m));
        }
        if let Some(l) = lines.last() {
            bottom = Some(shift + l.baseline + m.font_metrics(pstyle, l.size).descent);
        }
    };
    let paras = blocks_of(node);
    if !paras.is_empty() {
        let (items, _) = plan_paragraphs(node, &paras, &style, "", &spec, m);
        for it in &items {
            take(&it.laid.lines, &it.laid.style, top);
        }
    } else {
        let (segments, styles, _) = collect_runs(node, &style);
        let layout = layout_area_runs(&segments, &styles, &spec, m);
        take(&layout.lines, &style, 0.0);
    }
    // an empty box is one line tall: cap height plus descent at its own size
    let bottom = bottom.unwrap_or_else(|| {
        let fm = m.font_metrics(&style, style.size);
        top + fm.cap_height + fm.descent
    });
    Some(TextboxSize {
        height: bottom - spec.y + pad,
        width: (widest + 2.0 * pad).min(spec.width),
    })
}

// ============================================================================
// Text-layout seam for editors (docs/Incremental.md §6; consumer: xsvg-kit).
// The compiler computes every PlacedLine it renders and used to discard them;
// this seam returns the same geometry so an editing host can draw a caret
// without becoming a second text-layout engine.

/// One rendered line of a textbox flow, in absolute user units.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayoutLine {
    /// Exactly the rendered text (wrap words joined by single spaces; an
    /// `ellipsis` rewrite included — see [`TextLayoutLine::words`]).
    pub text: String,
    /// Line anchor x — interpret through the paragraph's `anchor`.
    pub x: f64,
    /// Absolute baseline y.
    pub baseline: f64,
    /// The line's rendered font size — its largest run's (§6.11 mixed sizes);
    /// the paragraph `style.size` for a plain line. A caret on it is this tall.
    pub size: f64,
    /// Justified lines span this width: their slack went into `word_spacing`.
    pub justify_width: Option<f64>,
    /// The per-space advance the line renders with — the paragraph's
    /// `word_spacing`, plus a justified line's share of slack per space. Caret x
    /// on any line is the prefix advance with this per space: exact, not an
    /// interpolation.
    pub word_spacing: f64,
    /// `[start, end)` into the paragraph's [`TextLayoutPara::words`]. Wrapping
    /// never splits a word, so the range is exact even for clipped-away
    /// neighbors; when `text` differs from `words[start..end].join(" ")` the
    /// line was rewritten by `text-overflow="ellipsis"` and carets must not be
    /// mapped through it.
    pub words: (usize, usize),
    /// The line renders outside its box (§6.6, `overflow="visible"`): a
    /// cropping box would have dropped it. The editor ghosts what it draws.
    pub overflow: bool,
}

/// One paragraph flow of a textbox: the whole box for plain / `in=` boxes,
/// one `<x:p>` in paragraph mode (§6.16).
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayoutPara {
    /// Byte range of the paragraph's source element (`<x:p>`, or the textbox
    /// itself outside paragraph mode) — hosts resolve it to a node via their
    /// span table, exactly like `data-xsvg-pos`.
    pub span: (usize, usize),
    /// Right-to-left paragraph (§6.4 `direction`): `anchor` "start" is the
    /// RIGHT edge and the glyphs lay leftward; hosts read glyph cells there.
    pub rtl: bool,
    /// Resolved paragraph style. `size` is the RENDERED size — after
    /// `fit="shrink"` — so measuring prefixes of a line's text in this style
    /// (plus `letter_spacing` per gap / `word_spacing` per space, exactly the
    /// renderer's math in [`line_advance`]) yields caret x offsets.
    pub style: TextStyle,
    /// `start` | `middle` | `end` — how each line's `x` anchors its text.
    pub anchor: String,
    /// Vertical metrics at the rendered size (caret height = ascent + descent).
    pub metrics: FontMetrics,
    /// The paragraph's wrap words: its text split on whitespace. Mapping a word
    /// back to source offsets is the same whitespace walk over the host's own
    /// text content.
    pub words: Vec<String>,
    /// Rendered lines only — lines clipped by the box are absent (their word
    /// ranges are simply never covered).
    pub lines: Vec<TextLayoutLine>,
}

/// The rendered text geometry of one `<x:textbox>`.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    /// Content rect `(x, y, width, height)`: the box after padding for
    /// rectangular flows, the region's padded bounds for curved region flow.
    pub content: (f64, f64, f64, f64),
    pub paras: Vec<TextLayoutPara>,
}

/// The layout half of `emit_textbox`'s dispatch — same branches, same order.
pub(super) fn layout_textbox(node: roxmltree::Node, ctx: &Ctx) -> Option<TextLayout> {
    let style = style_from(node, ctx);

    let paras = blocks_of(node);
    if !paras.is_empty() {
        let geom = match node.attribute("in") {
            Some(r) => ctx
                .resolve_ref(node, r)
                .filter(|t| t.tag_name().name() == "rect"),
            None => Some(node),
        };
        if let Some(geom) = geom {
            let spec = textbox_area_spec(node, geom);
            return Some(paragraphs_layout_info(node, &paras, &style, &spec, ctx.m));
        }
        // curved region flow ignores <x:p> and falls through, like emission
    }

    if let Some(reference) = node.attribute("in") {
        let target = ctx.resolve_ref(node, reference)?;
        if target.tag_name().name() == "rect" {
            let spec = textbox_area_spec(node, target);
            return Some(area_layout_info(node, &style, &spec, ctx.m));
        }
        let geometry = ref_geometry(node, reference, ctx).ok()?;
        let region = ctx
            .shaper
            .rasterize(&geometry, (style.size / 3.0).max(1.0))?;
        let spec = RegionSpec {
            padding: attr_num(node, "padding", 0.0),
            align: Align::parse(node.attribute("align").unwrap_or("start")),
            valign: VAlign::parse(node.attribute("valign").unwrap_or("top")),
            text_overflow: overflow_mode(node),
        };
        let (layout, emits) = region_layout(node, &style, &region, &spec, ctx.m);
        let b = crate::Region::bounds(&region);
        let pad = spec.padding;
        let words: Vec<String> = collect_text_as(node, &style)
            .split_whitespace()
            .map(str::to_string)
            .collect();
        return Some(TextLayout {
            content: (b.x + pad, b.y + pad, b.w - 2.0 * pad, b.h - 2.0 * pad),
            paras: vec![para_info(node, &style, &layout, words, &emits, ctx.m)],
        });
    }

    let spec = textbox_area_spec(node, node);
    Some(area_layout_info(node, &style, &spec, ctx.m))
}

/// Rectangular flow (own box or `in="#rect"`): the same `collect_runs` →
/// [`layout_area_runs`] pipeline emission runs. The flat word list matches
/// the layout's global word ranges ('\n' is whitespace to `measure_runs`).
pub(super) fn area_layout_info(
    node: roxmltree::Node,
    style: &TextStyle,
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> TextLayout {
    let (segments, styles, emits) = collect_runs(node, style);
    let measured = measure_runs(&segments, &styles, m);
    let words: Vec<String> = measured
        .words
        .iter()
        .map(|w| w.pieces.iter().map(|p| p.text.as_str()).collect())
        .collect();
    let layout = layout_area_runs(&segments, &styles, spec, m);
    let p = spec.padding;
    TextLayout {
        content: (
            spec.x + p,
            spec.y + p,
            spec.width - 2.0 * p,
            spec.height - 2.0 * p,
        ),
        paras: vec![para_info(node, style, &layout, words, &emits, m)],
    }
}

/// Paragraph mode (§6.16): the shared `plan_paragraphs` pass plus the same
/// shift-and-clip emission applies.
pub(super) fn paragraphs_layout_info(
    node: roxmltree::Node,
    paras: &[roxmltree::Node],
    base_style: &TextStyle,
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> TextLayout {
    let (items, block_top) = plan_paragraphs(node, paras, base_style, "", spec, m);
    let cy = spec.y + spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let p = spec.padding;
    let visible = visible_paragraph_lines(&items, block_top, cy, ch, spec.text_overflow, m);
    let out_paras = items
        .iter()
        .zip(&visible)
        .map(|(para, shown)| {
            // in the case each run renders (§6.18), like the lines below
            let (segments, _, _) = collect_runs(para.src, &para.laid.style);
            let words: Vec<String> = segments
                .iter()
                .map(|(t, _)| t.as_str())
                .collect::<String>()
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let lines = shown
                .iter()
                .map(|l| TextLayoutLine {
                    text: l.text.clone(),
                    x: l.x,
                    baseline: l.baseline,
                    size: l.size,
                    justify_width: l.justify_width,
                    word_spacing: para.laid.style.word_spacing
                        + justify_space(l, &para.laid.style, para.laid.style.size, &para.emits, m)
                            .unwrap_or(0.0),
                    words: l.words,
                    overflow: l.overflow,
                })
                .collect();
            let r = para.src.range();
            TextLayoutPara {
                span: (r.start, r.end),
                rtl: para.laid.rtl,
                style: para.laid.style.clone(),
                anchor: para.laid.anchor.svg().to_string(),
                metrics: m.font_metrics(&para.laid.style, para.laid.style.size),
                words,
                lines,
            }
        })
        .collect();
    TextLayout {
        content: (
            spec.x + p,
            spec.y + p,
            spec.width - 2.0 * p,
            spec.height - 2.0 * p,
        ),
        paras: out_paras,
    }
}

/// Wrap one [`AreaLayout`] (already clipped/ellipsized, absolute coordinates)
/// as a single-paragraph [`TextLayoutPara`] spanning `node`.
pub(super) fn para_info(
    node: roxmltree::Node,
    style: &TextStyle,
    layout: &AreaLayout,
    words: Vec<String>,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
) -> TextLayoutPara {
    let mut rendered = style.clone();
    rendered.size = layout.font_size;
    let r = node.range();
    TextLayoutPara {
        span: (r.start, r.end),
        rtl: layout.rtl,
        anchor: layout.anchor.svg().to_string(),
        metrics: m.font_metrics(style, layout.font_size),
        words,
        lines: layout
            .lines
            .iter()
            .map(|l| TextLayoutLine {
                text: l.text.clone(),
                x: l.x,
                baseline: l.baseline,
                size: l.size,
                justify_width: l.justify_width,
                word_spacing: style.word_spacing
                    + justify_space(l, style, layout.font_size, emits, m).unwrap_or(0.0),
                words: l.words,
                overflow: l.overflow,
            })
            .collect(),
        style: rendered,
    }
}
