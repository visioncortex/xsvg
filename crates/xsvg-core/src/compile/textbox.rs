//! `<x:textbox>`, `<x:p>` paragraphs and `<text inline-size>` (§6): the area and
//! region flows, their emission as positioned `<tspan>`s or outlined glyphs, run
//! highlights — and the text-layout seam that reports the same geometry to an
//! editor (docs/Incremental.md §6).

use super::*;

/// `<text inline-size="W">…</text>` → `<text>` with one `<tspan>` per wrapped line.
pub(super) fn emit_inline_size_text(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node, ctx);
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let max_w = attr_num(node, "inline-size", 0.0);
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let lines = layout_flow(&collect_text(node), &style, x, y, max_w, m);

    out.push_str("<text");
    copy_attrs(node, out, &["inline-size", "line-height"], ctx);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    let base = [EmitAttrs::default()];
    for line in &lines {
        emit_line(out, line, &style, style.size, gx, m, &base);
    }
    out.push_str("</text>");
}

/// `<x:textbox>` (Rung 3): explicit `align`/`valign`/`padding`/`fit`, or — with
/// `in="#id"` — bound to a referenced shape (rect → rectangular; any other shape →
/// flowed inside its outline, §6.10).
pub(super) fn emit_textbox(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node, ctx);
    let fill = node.attribute("fill").unwrap_or("#000");
    let gx = attr_num(node, "glyph-x-scale", 1.0);
    let tb: elements::Textbox = params_of(node);
    let outline = tb.outline || ctx.force_outline.get();
    let stroke = text_border_attrs(node, ctx);
    let pos = format!("{}{}{}", pos_attr(node, ctx), frame_attr(node, ctx), transform_attr(node));

    // Paragraph mode (§6.16): `<x:p>` children are separate paragraphs stacked with
    // space-before/after. Rectangular geometry only (own box or `in="#rect"`);
    // curved region flow ignores `<x:p>` and falls through to a single flow.
    let paras: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "p")
        .collect();
    if !paras.is_empty() {
        let geom = match node.attribute("in") {
            Some(r) => ctx
                .resolve_ref(node, r)
                .filter(|t| t.tag_name().name() == "rect"),
            None => Some(node),
        };
        if let Some(geom) = geom {
            let spec = textbox_area_spec(node, geom);
            // one keyed <g> wraps the per-paragraph <text>s: the TEXTBOX keeps a
            // rendered identity (and so a real bbox) of its own — the paragraphs
            // carry only their own <x:p> keys (found live: a multi-paragraph
            // textbox had no element to select, and its paragraphs were promoted
            // to free-standing objects)
            out.push_str("<g");
            out.push_str(&pos);
            out.push('>');
            emit_paragraphs(node, &paras, &style, fill, gx, &spec, out, ctx);
            out.push_str("</g>");
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
                outline,
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
            text_overflow: TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip")),
        };
        let layout = layout_region(&collect_text(node), &style, &region, &spec, ctx.m);
        write_area_text(
            out,
            &layout,
            &style,
            fill,
            &stroke,
            gx,
            ctx.m,
            &[EmitAttrs::default()],
            &pos,
            outline,
            ctx.outliner,
            ctx,
        );
        return;
    }

    let spec = textbox_area_spec(node, node);
    let (segments, styles, emits) = collect_runs(node, &style);
    let layout = layout_area_runs(&segments, &styles, &spec, ctx.m);
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
        outline,
        ctx.outliner,
        ctx,
    );
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
    out: &mut String,
    ctx: &Ctx,
) {
    let m = ctx.m;
    let cy = spec.y + spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let (items, block_top) = plan_paragraphs(node, paras, base_style, base_fill, spec, m);

    // pass 2: shift every line by `block_top`, clip to the box by baseline, emit one
    // <text> per paragraph
    for para in &items {
        let visible: Vec<PlacedLine> = para
            .lines
            .iter()
            .filter(|l| {
                let b = l.baseline + block_top;
                b >= cy - 1e-6 && b <= cy + ch + 1e-6
            })
            .cloned()
            .collect();
        if visible.is_empty() {
            continue;
        }
        let shifted: Vec<PlacedLine> = visible
            .iter()
            .map(|l| PlacedLine {
                baseline: l.baseline + block_top,
                ..l.clone()
            })
            .collect();
        emit_highlights(
            out,
            &shifted,
            para.anchor,
            para.rtl,
            &para.style,
            para.style.size,
            &para.emits,
            m,
            ctx,
        );
        out.push_str(&format!("<text text-anchor=\"{}\"", para.anchor));
        push_font_attrs(out, &para.style, para.style.size, para.fill, ctx);
        if para.rtl {
            out.push_str(" direction=\"rtl\"");
        }
        out.push_str(&pos_attr(para.src, ctx)); // per-paragraph source map
        out.push('>');
        for l in &shifted {
            emit_line(out, l, &para.style, para.style.size, gx, m, &para.emits);
        }
        out.push_str("</text>");
    }
}

/// One planned paragraph of a §6.16 flow: lines laid in a RELATIVE frame
/// (paragraph 0's cap-top at 0) — shift baselines by the plan's `block_top`
/// to place them in the box.
pub(super) struct ParaPlan<'a> {
    pub(super) lines: Vec<PlacedLine>,
    pub(super) style: TextStyle,
    pub(super) fill: &'a str,
    pub(super) anchor: &'static str,
    /// The paragraph's styled-run emit table (`collect_runs`) — `<tspan>`
    /// overrides render styled in paragraph mode, same as a plain box.
    pub(super) emits: Vec<EmitAttrs>,
    /// Right-to-left paragraph (§6.4 `direction`): emits `direction="rtl"`.
    pub(super) rtl: bool,
    pub(super) src: roxmltree::Node<'a, 'a>,
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
    let default_align = node.attribute("align").unwrap_or("start");
    // paragraphs default to a visible gap — 0.35em, the same default rhythm
    // lists use for item-spacing — so Enter reads as a paragraph while a
    // forced line break (<tbreak/>) stays at the bare line advance; an
    // authored paragraph-spacing (zero included) wins
    let default_spacing = attr_num(node, "paragraph-spacing", base_style.size * 0.35);

    // per-paragraph style: the box style, with any font-* on the `<x:p>` overriding
    let para_style = |p: roxmltree::Node| {
        let mut st = base_style.clone();
        if let Some(v) = p.attribute("font-family") {
            st.family = v.to_string();
        }
        if let Some(v) = p
            .attribute("font-size")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.size = v;
        }
        if let Some(v) = p.attribute("font-weight") {
            st.weight = v.to_string();
        }
        if let Some(v) = p.attribute("font-style") {
            st.style = v.to_string();
        }
        if let Some(v) = p
            .attribute("line-height")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.line_height = v;
        }
        st
    };

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
    let plan_at = |scale: f64| -> (Vec<ParaPlan<'a>>, f64) {
        let mut items: Vec<ParaPlan> = Vec::with_capacity(paras.len());
        let mut last_baseline: Option<f64> = None; // relative frame: para 0's cap-top at 0
        let mut last_descent = 0.0f64;
        let mut prev_after = 0.0f64;
        for &p in paras.iter() {
            let mut pstyle = para_style(p);
            pstyle.size *= scale;
            let sb = attr_num(p, "space-before", 0.0) * scale;
            let sa = attr_num(p, "space-after", default_spacing) * scale;
            let pspec = AreaSpec {
                x: cx,
                // lay out in the paragraph's OWN frame (its cap-top at 0); it
                // moves onto the stack once its first line's size is known
                y: 0.0,
                width: cw,
                height: 1e5, // tall → no clip; valign=top places lines from `y`
                padding: 0.0,
                align: Align::parse(p.attribute("align").unwrap_or(default_align)),
                valign: VAlign::Top,
                fit: Fit::None,
                text_overflow: TextOverflow::Clip,
                text_indent: attr_num(p, "text-indent", 0.0) * scale,
                // a paragraph's own direction, else the box's
                rtl: p
                    .attribute("direction")
                    .or_else(|| node.attribute("direction"))
                    == Some("rtl"),
            };
            let (segments, pstyles, emits) = collect_runs(p, &pstyle);
            let mut layout = layout_area_runs(&segments, &pstyles, &pspec, m);
            // the advance INTO this paragraph is its first line's own (that
            // line's largest run size × line-height); an empty paragraph owns
            // one base-size row. Layout put the first baseline at that line's
            // cap-height, so the whole paragraph shifts by the difference.
            let first_size = layout.lines.first().map(|l| l.size).unwrap_or(pstyle.size);
            let last_size = layout.lines.last().map(|l| l.size).unwrap_or(pstyle.size);
            let cap0 = m.font_metrics(&pstyle, first_size).cap_height;
            let first_baseline = match last_baseline {
                None => cap0,
                Some(lb) => lb + prev_after + sb + first_size * pstyle.line_height,
            };
            let shift = first_baseline - cap0;
            for l in &mut layout.lines {
                l.baseline += shift;
            }
            last_baseline = Some(
                layout
                    .lines
                    .last()
                    .map(|l| l.baseline)
                    .unwrap_or(first_baseline),
            );
            last_descent = m.font_metrics(&pstyle, last_size).descent;
            items.push(ParaPlan {
                lines: layout.lines,
                style: pstyle,
                fill: p.attribute("fill").unwrap_or(base_fill),
                anchor: layout.anchor.svg(),
                emits,
                rtl: layout.rtl,
                src: p,
            });
            prev_after = sa;
        }
        let block_h = last_baseline.map(|lb| lb + last_descent).unwrap_or(0.0);
        (items, block_h)
    };

    // fit="shrink" (§6.1, extended to paragraph mode): binary-search the
    // block scale like `fit_size` searches the font size — the stacked height
    // drives it (each trial re-wraps to the width), plus `fit_size`'s width
    // condition: a lone unbreakable word is the only line greedy wrap lets
    // overflow, so it must fit the content width too. `fit-min` floors the
    // BASE size; per-paragraph overrides scale proportionally with it. Each
    // trial re-lays every paragraph (measures at the scaled size): the cost
    // is paid only by overflowing shrink boxes.
    let (items, block_h) = match spec.fit {
        Fit::Shrink { min } if base_style.size > 0.0 => {
            let fits = |s: f64| {
                let (items, h) = plan_at(s);
                h <= ch
                    && items.iter().all(|it| {
                        it.lines.iter().all(|l| {
                            l.words.1 - l.words.0 != 1
                                || m.measure(&l.text, &it.style, it.style.size)
                                    + it.style.letter_spacing
                                        * (l.text.chars().count().saturating_sub(1)) as f64
                                    <= cw + 1e-6
                        })
                    })
            };
            if fits(1.0) {
                plan_at(1.0)
            } else {
                let min_scale = (min / base_style.size).clamp(0.0, 1.0);
                let (mut lo, mut hi) = (min_scale, 1.0f64);
                for _ in 0..24 {
                    let mid = 0.5 * (lo + hi);
                    if fits(mid) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                plan_at(lo.max(min_scale))
            }
        }
        _ => plan_at(1.0),
    };
    let valign = VAlign::parse(node.attribute("valign").unwrap_or("top"));
    let block_top = match valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - block_h) / 2.0,
        VAlign::Bottom => cy + (ch - block_h),
    };
    (items, block_top)
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
        text_overflow: TextOverflow::parse(opts.text_overflow.as_str()),
        text_indent: opts.text_indent,
        rtl: node.attribute("direction") == Some("rtl"),
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
        text_overflow: TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip")),
    };
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let outline = node.attribute((XSVG_NS, "outline")) == Some("true");
    let (segments, styles, emits) = collect_runs(node, &style);
    let layout = layout_text_area_runs(&segments, &styles, &spec, m);
    write_area_text(
        out,
        &layout,
        &style,
        node.attribute("fill").unwrap_or("#000"),
        &text_border_attrs(node, ctx),
        gx,
        m,
        &emits,
        &pos_attr(node, ctx),
        outline,
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
    outline: bool,
    outliner: &dyn GlyphOutliner,
    ctx: &Ctx,
) {
    // Create-outlines (§6.12): emit each line as a <path> tracing its glyphs instead
    // of live <text>. All-or-nothing — if the outliner can't do any line (font bytes
    // unavailable) we fall through to live text. v0 uses the base style per line
    // (per-run styling / justify / glyph-x-scale don't apply) and anchors via the
    // measured line width.
    if outline {
        let mut paths = Vec::new();
        let mut ok = true;
        for line in &layout.lines {
            if line.text.is_empty() {
                continue; // blank line (e.g. from <tbreak/>) → nothing to trace
            }
            let w = m.measure(&line.text, style, layout.font_size);
            let start_x = match layout.anchor {
                Anchor::Start => line.x,
                Anchor::Middle => line.x - w / 2.0,
                Anchor::End => line.x - w,
            };
            match outliner.outline(&line.text, style, layout.font_size, start_x, line.baseline) {
                Some(d) => paths.push(d),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            push_outline_group(out, fill, stroke, pos, &paths, ctx);
            return;
        }
        // else: outliner unavailable → fall through to live <text> below
    }

    emit_highlights(
        out,
        &layout.lines,
        layout.anchor.svg(),
        layout.rtl,
        style,
        layout.font_size,
        emits,
        m,
        ctx,
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
        emit_line(out, line, style, layout.font_size, glyph_x_scale, m, emits);
    }
    out.push_str("</text>");
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

/// The `x:highlight` backgrounds of `lines` (§6.11): one `<rect>` per
/// highlighted run per line, emitted BEFORE the `<text>` so it sits behind
/// the glyphs. A run's box spans its advance (measured in the run's own
/// style, stretched like the line when justified) from the line's start
/// edge (`anchor` = the `<text>`'s text-anchor; under `rtl` the start edge
/// is the right one and runs lay leftward), and the em box of its size
/// (ascent above, descent below the baseline). Nothing is emitted for lines
/// without a highlighted run, so plain output is byte-identical.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_highlights(
    out: &mut String,
    lines: &[PlacedLine],
    anchor: &str,
    rtl: bool,
    style: &TextStyle,
    size: f64,
    emits: &[EmitAttrs],
    m: &dyn Measurer,
    ctx: &Ctx,
) {
    let scale = if style.size > 0.0 {
        size / style.size
    } else {
        1.0
    };
    // the run's measuring style: the base with the run's overrides folded in
    let run_style = |a: &EmitAttrs| -> (TextStyle, f64) {
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
    };
    for line in lines {
        if !line
            .runs
            .iter()
            .any(|r| emits.get(r.style).is_some_and(|a| a.highlight.is_some()))
        {
            continue;
        }
        let natural: f64 = line
            .runs
            .iter()
            .map(|r| {
                let (st, rs) = emits
                    .get(r.style)
                    .map(&run_style)
                    .unwrap_or((style.clone(), size));
                line_advance(&r.text, &st, rs, m)
            })
            .sum();
        let stretch = match line.justify_width {
            Some(w) if natural > 0.0 => w / natural,
            _ => 1.0,
        };
        let width = natural * stretch;
        // the line's LEFT edge, whichever edge the anchor names
        let left = match (anchor, rtl) {
            ("middle", _) => line.x - width / 2.0,
            ("end", false) | ("start", true) => line.x - width,
            _ => line.x,
        };
        // runs lay out from the start edge: rightward in ltr, leftward in rtl
        let mut x = if rtl { left + width } else { left };
        for r in &line.runs {
            let a = emits.get(r.style);
            let (st, rs) = a.map(&run_style).unwrap_or((style.clone(), size));
            let w = line_advance(&r.text, &st, rs, m) * stretch;
            if rtl {
                x -= w;
            }
            if let Some(color) = a.and_then(|a| a.highlight.as_deref()) {
                let fm = m.font_metrics(&st, rs);
                // a raised or lowered run carries its highlight with it
                let shift = a
                    .and_then(|a| a.shift.as_deref())
                    .map_or(0.0, |v| baseline_shift_px(v, fm.ascent + fm.descent, rs));
                out.push_str(&format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                    fmt(x),
                    fmt(line.baseline - shift - fm.ascent),
                    fmt(w),
                    fmt(fm.ascent + fm.descent),
                    ctx.paint(color)
                ));
            }
            if !rtl {
                x += w;
            }
        }
    }
}

/// Emit one line as an outer positioning `<tspan x y>` (carrying justify/glyph-scale
/// `textLength`) whose children are the line's styled runs — a bare string for base
/// runs, an inner `<tspan>` with the run's overrides for the rest.
pub(super) fn emit_line(
    out: &mut String,
    line: &PlacedLine,
    style: &TextStyle,
    size: f64,
    glyph_x_scale: f64,
    m: &dyn Measurer,
    emits: &[EmitAttrs],
) {
    out.push_str(&format!(
        "<tspan x=\"{}\" y=\"{}\"",
        fmt(line.x),
        fmt(line.baseline)
    ));
    if let Some(w) = line.justify_width {
        // Justification: stretch inter-word/glyph spacing (not glyph shapes) to fill
        // the content width. Takes precedence over glyph-x-scale on this line.
        out.push_str(&format!(
            " textLength=\"{}\" lengthAdjust=\"spacing\"",
            fmt(w)
        ));
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
                    emit_attr(out, "text-decoration", &a.decoration);
                    emit_attr(out, "baseline-shift", &a.shift);
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
    /// Justified lines render stretched to this width (SVG `textLength`).
    pub justify_width: Option<f64>,
    /// `[start, end)` into the paragraph's [`TextLayoutPara::words`]. Wrapping
    /// never splits a word, so the range is exact even for clipped-away
    /// neighbors; when `text` differs from `words[start..end].join(" ")` the
    /// line was rewritten by `text-overflow="ellipsis"` and carets must not be
    /// mapped through it.
    pub words: (usize, usize),
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

    let paras: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "p")
        .collect();
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
            text_overflow: TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip")),
        };
        let text = collect_text(node);
        let layout = layout_region(&text, &style, &region, &spec, ctx.m);
        let b = crate::Region::bounds(&region);
        let pad = spec.padding;
        let words: Vec<String> = text.split_whitespace().map(str::to_string).collect();
        return Some(TextLayout {
            content: (b.x + pad, b.y + pad, b.w - 2.0 * pad, b.h - 2.0 * pad),
            paras: vec![para_info(node, &style, &layout, words, ctx.m)],
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
    let (segments, styles, _) = collect_runs(node, style);
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
        paras: vec![para_info(node, style, &layout, words, m)],
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
    let out_paras = items
        .iter()
        .map(|para| {
            let words: Vec<String> = collect_text(para.src)
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let lines = para
                .lines
                .iter()
                .filter_map(|l| {
                    let b = l.baseline + block_top;
                    (b >= cy - 1e-6 && b <= cy + ch + 1e-6).then(|| TextLayoutLine {
                        text: l.text.clone(),
                        x: l.x,
                        baseline: b,
                        size: l.size,
                        justify_width: l.justify_width,
                        words: l.words,
                    })
                })
                .collect();
            let r = para.src.range();
            TextLayoutPara {
                span: (r.start, r.end),
                rtl: para.rtl,
                style: para.style.clone(),
                anchor: para.anchor.to_string(),
                metrics: m.font_metrics(&para.style, para.style.size),
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
                words: l.words,
            })
            .collect(),
        style: rendered,
    }
}
