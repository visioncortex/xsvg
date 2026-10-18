//! `<x:textpath>` (§6.13): text set along a path, warped through a [`PathFrame`]
//! or stepped glyph by glyph.

use super::*;

/// `<x:textpath in="#path" effect="skew|rainbow|stair">` (§6.13): outline the run flat
/// (the [`GlyphOutliner`] seam supplies glyph geometry + advance width), then warp it
/// onto the referenced path **natively** — [`warp_text_on_path`] runs the §7.1 bake at
/// the quality tolerance, with cubic refit above `fast`. `baseline-shift` offsets the
/// run from the path along the local normal (positive = above); `align` / `start`
/// place the run within the path's extent. Emits `<g fill=… stroke=…>` + the warped
/// `<path>`. `stair` (§6.13.3) is live `<text>` by design — per-glyph stepped
/// positions, no outliner — and doubles as skew's degradation when no outline font is
/// available; the last resort is a straight live `<text>` at the element's x/y, so
/// the document never breaks.
pub(super) fn emit_textpath(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node, ctx);
    let fill = node.attribute("fill").unwrap_or("#000");
    let stroke = text_border_attrs(node, ctx);
    // `to_path` carries the element's attributes, its transform included: this
    // emitter writes none of its own
    let pos = format!(
        "{}{}",
        pos_attr(node, ctx),
        ctx.carry_at(node.range().start, true)
    );
    let tp: elements::Textpath = params_of(node);
    let fx = PathEffect {
        effect: &tp.effect,
        baseline_shift: tp.baseline_shift,
        align: tp.align.as_str(),
        start: tp.start,
    };
    let runs = paint_runs(node, &style, ctx);
    let text: String = runs.runs.iter().map(|r| r.text.as_str()).collect();
    let styled = runs.any_painted();

    let Some(reference) = node.attribute("in") else {
        out.push_str("<!-- xsvg: <x:textpath> requires in=\"#path\" -->");
        return;
    };
    let path_d = match ref_geometry(node, reference, ctx) {
        Ok(d) => d,
        Err(f) => {
            out.push_str(&format!(
                "<!-- xsvg: <x:textpath in> target not found or not a path ({}) -->",
                f.reason()
            ));
            return;
        }
    };

    // the element's filter (§8, *text*) on live output: the glyphs ride the
    // path, so its bbox grown by two ems (ascent, descent, the baseline shift,
    // and the measurer-vs-renderer slack) holds them
    let live_bbox = || {
        let pad = 2.0 * style.size + fx.baseline_shift.abs();
        svg_path_bbox(&path_d).map(|b| b.inflate(pad, pad))
    };

    // "follow" (§6.13.5) lowers to SVG's own <textPath> — live, selectable text
    // that follows the curve without deforming; no font bytes needed, only the
    // path's arc length for align placement.
    if fx.effect == "follow" {
        if ctx.force_outline.get() {
            ctx.record_miss(ToPathError::limit("effect `follow` has no outline form"));
        }
        if let Some(frame) = PathFrame::new(&path_d, ctx.quality.text_tolerance()) {
            let advance = line_advance(&text, &style, style.size, ctx.m);
            let offset = run_offset(frame.len(), advance, fx.align, fx.start);
            let filter = lower_filter_attr(node, out, ctx, live_bbox);
            out.push_str("<text");
            push_font_attrs(out, &style, style.size, fill, ctx);
            out.push_str(&stroke);
            out.push_str(&pos);
            if let Some(f) = &filter {
                out.push_str(f);
            }
            out.push_str("><textPath href=\"#");
            push_escaped(out, reference.strip_prefix('#').unwrap_or(reference), true);
            out.push('"');
            if offset != 0.0 {
                out.push_str(&format!(" startOffset=\"{}\"", fmt(offset)));
            }
            if fx.baseline_shift != 0.0 {
                out.push_str(&format!(" baseline-shift=\"{}\"", fmt(fx.baseline_shift)));
            }
            out.push('>');
            push_runs(out, &runs, 0, usize::MAX, ctx);
            out.push_str("</textPath></text>");
            return;
        }
        // degenerate path → the straight fallback below
    }

    // "stair"/"follow" are live <text> by design — they never consult the outliner.
    if !matches!(fx.effect, "stair" | "follow") {
        let flat = ctx.outline(&text, &style, style.size, 0.0, 0.0);
        let advances = ctx.outliner.advances(&text, &style, style.size);
        if (flat.is_none() || advances.is_none()) && ctx.force_outline.get() {
            ctx.record_miss(ToPathError::font(&style, "no font file to trace"));
        }
        if let (Some(flat), Some(advances)) = (flat, advances) {
            let advance = advances.last().copied().unwrap_or(0.0);
            // Refit is DISABLED (see §7.1): kurbo's fitter overshoots on dense
            // quantized glyph outlines (notches + hairline smears) and its Optimize
            // level dominates compile time. Polyline output at the graded tolerance
            // is the shipped form until a robust fitter lands.
            let warp = |flat: &str| {
                warp_text_on_path(
                    flat,
                    &path_d,
                    &fx,
                    advance,
                    ctx.quality.text_tolerance(),
                    false,
                )
            };
            // painted runs: each traced alone at its pen position in the whole
            // string — kerning across a run boundary kept — and bent by the
            // same field, so the pieces meet as the single path would
            let pieces = if styled {
                warp_runs(&runs.runs, &style, &advances, ctx, warp)
            } else {
                warp(&flat).map(|d| vec![(d, None)])
            };
            if let Some(pieces) = pieces {
                if ctx.recording_stops() {
                    record_textpath_stops(
                        &text,
                        &style,
                        &fx,
                        &path_d,
                        &advances,
                        span_of(node),
                        ctx,
                    );
                }
                // the baked glyphs are the exact region
                let filter = lower_filter_attr(node, out, ctx, || {
                    pieces
                        .iter()
                        .filter_map(|(d, _)| svg_path_bbox(d))
                        .reduce(|a, b| a.union(b))
                });
                open_filter_group(out, filter.as_deref());
                if styled {
                    let pieces: Vec<PaintedPath> = pieces
                        .into_iter()
                        .map(|(d, r)| PaintedPath {
                            d,
                            fill: r.and_then(|r| r.fill.clone()),
                            border: r
                                .filter(|r| r.border != runs.base_border)
                                .and_then(|r| r.border.clone()),
                            overflow: false,
                        })
                        .collect();
                    push_painted_paths(out, &pieces, fill, &stroke, &pos, ctx);
                } else {
                    let paths: Vec<(String, bool)> =
                        pieces.into_iter().map(|(d, _)| (d, false)).collect();
                    push_outline_group(out, fill, &stroke, &pos, &paths, ctx);
                }
                close_filter_group(out, filter.as_deref());
                return;
            }
        }
    }

    // Stair Step — authored (§6.13.3), or as the degradation of the height-profile
    // effects (skew, ribbon) without an outline font — when the profile can be
    // sampled; anything else → straight <text> at the element's x/y. One def
    // serves both: the fallback stays near the path too.
    if fx.effect == "stair" && ctx.force_outline.get() {
        ctx.record_miss(ToPathError::limit("effect `stair` has no outline form"));
    }
    let filter = lower_filter_attr(node, out, ctx, live_bbox);
    if matches!(fx.effect, "stair" | "skew" | "ribbon")
        && stepped_text(
            out,
            &runs,
            &style,
            &fx,
            &path_d,
            fill,
            &format!("{stroke}{pos}"),
            filter.as_deref(),
            ctx,
        )
    {
        return;
    }
    let (x, y) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    out.push_str("<text");
    push_font_attrs(out, &style, style.size, fill, ctx);
    out.push_str(&format!(" x=\"{}\" y=\"{}\"", fmt(x), fmt(y)));
    out.push_str(&stroke);
    out.push_str(&pos);
    if let Some(f) = &filter {
        out.push_str(f);
    }
    out.push('>');
    push_runs(out, &runs, 0, usize::MAX, ctx);
    out.push_str("</text>");
}

/// A stretch of a textpath's text in one paint (§6.13): the `fill` and the
/// border (§6.17) a `<tspan>` may give part of text on a path. Typography —
/// weight, size, family — stays the element's for every run: the effects shape
/// one run, and paint changes nothing about where a glyph sits.
#[derive(Clone, Debug, PartialEq)]
struct PaintRun {
    text: String,
    /// The run's own fill; `None` takes the element's.
    fill: Option<String>,
    /// The border in effect (built by [`border_attrs`]), the element's
    /// inherited attribute by attribute; `None` when no border is set.
    border: Option<String>,
}

/// A textpath's text split into [`PaintRun`]s, plus the element's own border.
struct PaintRuns {
    runs: Vec<PaintRun>,
    base_border: Option<String>,
}

impl PaintRuns {
    /// A run whose paint differs from the element's.
    fn painted(&self, r: &PaintRun) -> bool {
        r.fill.is_some() || r.border != self.base_border
    }

    fn any_painted(&self) -> bool {
        self.runs.iter().any(|r| self.painted(r))
    }

    /// ` fill="…"` and the border, where `r` overrides the element's.
    fn push_paint(&self, r: &PaintRun, out: &mut String, ctx: &Ctx) {
        if let Some(f) = &r.fill {
            out.push_str(&format!(" fill=\"{}\"", ctx.paint(f)));
        }
        if r.border != self.base_border {
            if let Some(b) = &r.border {
                out.push_str(b);
            }
        }
    }
}

/// The element's text as paint runs, in the case it renders in (§6.18): a
/// `<tspan>` sets its own `fill` and any of `x:border-width` / `-color` /
/// `-opacity`, inheriting the rest from its parent, the element at the top;
/// adjacent runs in one paint merge, so text with no painted run is one run.
fn paint_runs(node: roxmltree::Node, style: &TextStyle, ctx: &Ctx) -> PaintRuns {
    #[derive(Clone, Copy)]
    struct Paint<'a> {
        fill: Option<&'a str>,
        width: Option<&'a str>,
        color: Option<&'a str>,
        opacity: Option<&'a str>,
    }
    impl<'a> Paint<'a> {
        fn over(self, n: roxmltree::Node<'a, 'a>) -> Self {
            Paint {
                fill: n.attribute("fill").or(self.fill),
                width: n.attribute((XSVG_NS, "border-width")).or(self.width),
                color: n.attribute((XSVG_NS, "border-color")).or(self.color),
                opacity: n.attribute((XSVG_NS, "border-opacity")).or(self.opacity),
            }
        }
        fn border(self, ctx: &Ctx) -> Option<String> {
            (self.width.is_some() || self.color.is_some())
                .then(|| border_attrs(self.width, self.color, self.opacity, ctx))
        }
    }
    fn walk<'a>(n: roxmltree::Node<'a, 'a>, paint: Paint<'a>, ctx: &Ctx, out: &mut Vec<PaintRun>) {
        for c in n.children() {
            if let Some(t) = c.text().filter(|_| c.is_text()) {
                let fill = paint.fill.map(str::to_string);
                let border = paint.border(ctx);
                match out.last_mut() {
                    Some(r) if r.fill == fill && r.border == border => r.text.push_str(t),
                    _ if t.is_empty() => {}
                    _ => out.push(PaintRun {
                        text: t.to_string(),
                        fill,
                        border,
                    }),
                }
            } else if c.is_element() {
                let p = if c.tag_name().name() == "tspan" {
                    paint.over(c)
                } else {
                    paint
                };
                walk(c, p, ctx, out);
            }
        }
    }
    // the element's fill is the group's / the <text>'s: runs inherit it as `None`
    let none = Paint {
        fill: None,
        width: None,
        color: None,
        opacity: None,
    };
    let top = Paint {
        fill: None,
        ..none.over(node)
    };
    let mut runs = Vec::new();
    walk(node, top, ctx, &mut runs);
    for r in &mut runs {
        if let std::borrow::Cow::Owned(t) = style.text_transform.apply(&r.text) {
            r.text = t;
        }
    }
    PaintRuns {
        runs,
        base_border: top.border(ctx),
    }
}

/// The characters `[a, b)` of the runs' text, a painted run as a `<tspan>`
/// carrying its paint — what live text on a path (follow, stair, the straight
/// fallback) writes inside its `<text>`.
fn push_runs(out: &mut String, runs: &PaintRuns, a: usize, b: usize, ctx: &Ctx) {
    let mut at = 0;
    for r in &runs.runs {
        let n = r.text.chars().count();
        let (s, e) = (a.max(at), b.min(at + n));
        if s < e {
            let slice: String = r.text.chars().skip(s - at).take(e - s).collect();
            if runs.painted(r) {
                out.push_str("<tspan");
                runs.push_paint(r, out, ctx);
                out.push('>');
                push_escaped(out, &slice, false);
                out.push_str("</tspan>");
            } else {
                push_escaped(out, &slice, false);
            }
        }
        at += n;
    }
}

/// Each run's glyphs traced at its pen position in the whole string (from the
/// whole string's `advances`, so the pieces sit exactly where one trace would
/// put them) and bent by `warp`. Runs with nothing to draw (whitespace) are
/// skipped; `None` when any trace or bake fails — the caller degrades.
fn warp_runs<'r>(
    runs: &'r [PaintRun],
    style: &TextStyle,
    advances: &[f64],
    ctx: &Ctx,
    warp: impl Fn(&str) -> Option<String>,
) -> Option<Vec<(String, Option<&'r PaintRun>)>> {
    let mut pieces = Vec::new();
    let mut at = 0usize;
    for r in runs {
        let n = r.text.chars().count();
        if !r.text.trim().is_empty() {
            let x0 = if at == 0 { 0.0 } else { *advances.get(at - 1)? };
            let flat = ctx.outline(&r.text, style, style.size, x0, 0.0)?;
            pieces.push((warp(&flat)?, Some(r)));
        }
        at += n;
    }
    Some(pieces)
}

/// The stepped-baseline degradation of skew (§6.13.1, Illustrator's *Stair Step*):
/// each glyph of the live run is absolutely positioned on the path's height profile
/// via per-glyph `x`/`y` lists — upright glyphs, stepped baseline, still selectable.
/// Glyph offsets come from kerned prefix advances plus the letter/word-spacing gaps
/// (§6.8), so positions match what the renderer would produce. Returns `false` (and
/// writes nothing) when the run is empty or the profile can't be sampled.
#[allow(clippy::too_many_arguments)]
fn stepped_text(
    out: &mut String,
    runs: &PaintRuns,
    style: &TextStyle,
    fx: &PathEffect,
    path_d: &str,
    fill: &str,
    pos: &str,
    filter: Option<&str>,
    ctx: &Ctx,
) -> bool {
    let full: String = runs.runs.iter().map(|r| r.text.as_str()).collect();
    let text = full.trim();
    if text.is_empty() {
        return false;
    }
    // where the trimmed text sits in the runs, in characters
    let lead = full.chars().take_while(|c| c.is_whitespace()).count();
    let n = text.chars().count();
    let mut advances = Vec::with_capacity(n + 1);
    let mut spaces_before = 0usize;
    for (i, (bi, ch)) in text.char_indices().enumerate() {
        advances.push(
            ctx.m.measure(&text[..bi], style, style.size)
                + i as f64 * style.letter_spacing
                + spaces_before as f64 * style.word_spacing,
        );
        if ch == ' ' {
            spaces_before += 1;
        }
    }
    let total = line_advance(text, style, style.size, ctx.m); // total run width
    if !total.is_finite() || advances.iter().any(|a| !a.is_finite()) {
        return false;
    }
    // sample the path's height profile natively (§6.13.1)
    let Some(frame) = PathFrame::new(path_d, ctx.quality.text_tolerance()) else {
        return false;
    };
    let x0 = frame.x0() + run_offset(frame.x1() - frame.x0(), total, fx.align, fx.start);
    if !x0.is_finite() {
        return false;
    }

    out.push_str("<text");
    push_font_attrs(out, style, style.size, fill, ctx);
    out.push_str(pos);
    if let Some(f) = filter {
        out.push_str(f);
    }
    // absolute per-glyph positions — spacing is baked in, so no spacing attrs
    let xs: Vec<String> = advances.iter().map(|a| fmt(x0 + a)).collect();
    let ys: Vec<String> = advances
        .iter()
        .map(|a| fmt(frame.y_at(x0 + a) - fx.baseline_shift))
        .collect();
    out.push_str(&format!(" x=\"{}\" y=\"{}\"", xs.join(" "), ys.join(" ")));
    out.push('>');
    // the <text>'s per-glyph x/y lists position the characters inside its
    // <tspan> runs too, in order
    push_runs(out, runs, lead, lead + n, ctx);
    out.push_str("</text>");
    true
}
