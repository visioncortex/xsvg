//! WASM entry point for xsvg: parse the xsvg/SVG input, run lowering passes, and emit a
//! plain-SVG-subset string. Passes wired so far:
//!   • `<rect>` (sharp-cornered) → `<path>`
//!   • `<text inline-size>` → wrapped `<tspan>` lines (§6.2)
//!   • `<textArea>` → flowed text: align / display-align / line-increment / auto sizing (§6.3)
//!   • `<x:textbox>` → wrapped + aligned + shrink-to-fit text, incl. `in="#shape"` region
//!     flow and cap-height centering (§6.4–6.5, 6.10)
//!   • styled `<tspan>` runs (§6.11); create outlines `outline="true"` → `<path>` (§6.12);
//!     text on a path `<x:textpath>` skew + rainbow, with `baseline-shift` (§6.13)
//! Other `x:` extensions are recognized and skipped with a marker.
//!
//! **Platform seams.** Everything platform-specific is a trait the core calls, backed here
//! by JS callbacks: `Measurer` (canvas `measureText` + font metrics), `Shaper` (path
//! rasterize for region flow), and `GlyphOutliner` (opentype.js glyph outlines, incl.
//! path-warping). The core layout logic lives in `xsvg-core` and stays platform-free.

use wasm_bindgen::prelude::*;
use xsvg_core::{
    layout_area_measured, layout_flow, layout_region, layout_text_area_runs, line_advance,
    measure_runs, svg_path_bbox, warp_svg_path, Align, Anchor, AreaLayout, AreaSpec, DisplayAlign,
    EnvelopePreset, Fit, GlyphOutliner, LineIncrement, Measurer, PathEffect, PlacedLine,
    QualityProfile, RasterRegion, Rect, RegionSpec, Shaper, TextAlign, TextAreaSpec, TextOverflow,
    TextStyle, VAlign, WarpAxis,
};

const XSVG_NS: &str = "https://xsvg.visioncortex.org";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Maximum element nesting depth accepted. `roxmltree`'s parser recurses per level,
/// so pathologically deep input would overflow the stack (a hard abort, worse on
/// wasm's smaller stack). Real documents nest a few dozen deep; this is a generous
/// ceiling that still leaves a wide stack margin.
const MAX_NESTING_DEPTH: usize = 512;

/// Runs once when the module is instantiated: route Rust panics to `console.error`.
#[wasm_bindgen(start)]
pub fn on_start() {
    console_error_panic_hook::set_once();
}

/// Browser-backed `Measurer`. `measure(text, fontCss) -> width` and
/// `metrics(fontCss) -> [ascent, descent, capHeight, xHeight]` are canvas callbacks.
struct JsMeasurer<'a> {
    measure: &'a js_sys::Function,
    metrics: &'a js_sys::Function,
}

impl Measurer for JsMeasurer<'_> {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        let css = style.font_css(size);
        self.measure
            .call2(
                &JsValue::NULL,
                &JsValue::from_str(text),
                &JsValue::from_str(&css),
            )
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }

    fn font_metrics(&self, style: &TextStyle, size: f64) -> xsvg_core::FontMetrics {
        let default = xsvg_core::FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        };
        let css = style.font_css(size);
        let Ok(v) = self.metrics.call1(&JsValue::NULL, &JsValue::from_str(&css)) else {
            return default;
        };
        let arr = js_sys::Array::from(&v);
        let get = |i: u32, d: f64| {
            arr.get(i)
                .as_f64()
                .filter(|n| n.is_finite() && *n > 0.0)
                .unwrap_or(d)
        };
        xsvg_core::FontMetrics {
            ascent: get(0, default.ascent),
            descent: get(1, default.descent),
            cap_height: get(2, default.cap_height),
            x_height: get(3, default.x_height),
        }
    }
}

/// Browser-backed [`Shaper`]: `rasterize(pathD, rowH) => Float64Array` where the
/// array is `[minX, minY, width, height, rowH, l0, r0, l1, r1, …]` (a `NaN` pair for
/// an empty row). The browser flattens curves + scans via `getBBox`/`isPointInFill`.
/// `steppedBaseline(pathD, advances, align, start) => Float64Array` samples the
/// path's height profile for the §6.13 stair-step fallback (`[runStartX, y0, y1, …]`,
/// empty on failure).
struct JsShaper<'a> {
    rasterize: &'a js_sys::Function,
    stepped: &'a js_sys::Function,
}

impl Shaper for JsShaper<'_> {
    fn rasterize(&self, path_d: &str, row_h: f64) -> Option<RasterRegion> {
        let v = self
            .rasterize
            .call2(
                &JsValue::NULL,
                &JsValue::from_str(path_d),
                &JsValue::from_f64(row_h),
            )
            .ok()?;
        let arr = js_sys::Array::from(&v);
        if arr.length() < 6 {
            return None;
        }
        let g = |i: u32| arr.get(i).as_f64();
        let (minx, miny, w, h, rh) = (g(0)?, g(1)?, g(2)?, g(3)?, g(4)?);
        if !(w > 0.0 && h > 0.0 && rh > 0.0) {
            return None;
        }
        let mut rows = Vec::new();
        let mut i = 5;
        while i + 1 < arr.length() {
            let span = match (arr.get(i).as_f64(), arr.get(i + 1).as_f64()) {
                (Some(l), Some(r)) if l.is_finite() && r.is_finite() && r > l => Some((l, r)),
                _ => None,
            };
            rows.push(span);
            i += 2;
        }
        Some(RasterRegion::new(
            Rect {
                x: minx,
                y: miny,
                w,
                h,
            },
            miny,
            rh,
            rows,
        ))
    }

    fn baseline_samples(
        &self,
        path_d: &str,
        advances: &[f64],
        align: &str,
        start: f64,
    ) -> Option<(f64, Vec<f64>)> {
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(path_d));
        args.push(&js_sys::Float64Array::from(advances));
        args.push(&JsValue::from_str(align));
        args.push(&JsValue::from_f64(start));
        let v = self.stepped.apply(&JsValue::NULL, &args).ok()?;
        let arr = js_sys::Array::from(&v);
        // expected: [runStartX, y0 … y(n-1)] for n = advances.len() - 1 glyphs
        if arr.length() as usize != advances.len() {
            return None;
        }
        let mut vals = Vec::with_capacity(arr.length() as usize);
        for i in 0..arr.length() {
            vals.push(arr.get(i).as_f64().filter(|n| n.is_finite())?);
        }
        let x0 = vals.remove(0);
        Some((x0, vals))
    }
}

/// Push the run's style as the shared `(family, weight, style, size)` callback arguments
/// onto `args` — the common prefix of the `outline_run` / `outline_on_path` JS calls.
fn push_style_args(args: &js_sys::Array, style: &TextStyle, size: f64) {
    args.push(&JsValue::from_str(&style.family));
    args.push(&JsValue::from_str(&style.weight));
    args.push(&JsValue::from_str(&style.style));
    args.push(&JsValue::from_f64(size));
}

/// Browser-backed [`GlyphOutliner`]. `outline_run(text, family, weight, style, size, x,
/// baseline) => d | ""` returns a glyph outline (opentype.js), or `""` when the font's
/// bytes aren't available (→ fall back to live `<text>`). `outline_on_path(text, family,
/// weight, style, size, pathD, effect, baselineShift, align, start) => d | ""`
/// additionally warps the outline onto a path (§6.13 — the text-on-path specialization
/// of the §7 geometry pipeline).
struct JsOutliner<'a> {
    outline_run: &'a js_sys::Function,
    outline_on_path: &'a js_sys::Function,
}

impl GlyphOutliner for JsOutliner<'_> {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String> {
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(text));
        push_style_args(&args, style, size);
        args.push(&JsValue::from_f64(x));
        args.push(&JsValue::from_f64(baseline));
        let d = self
            .outline_run
            .apply(&JsValue::NULL, &args)
            .ok()?
            .as_string()?;
        (!d.is_empty()).then_some(d)
    }

    fn outline_on_path(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        path_d: &str,
        fx: &PathEffect,
    ) -> Option<String> {
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(text));
        push_style_args(&args, style, size);
        args.push(&JsValue::from_str(path_d));
        args.push(&JsValue::from_str(fx.effect));
        args.push(&JsValue::from_f64(fx.baseline_shift));
        args.push(&JsValue::from_str(fx.align));
        args.push(&JsValue::from_f64(fx.start));
        let d = self
            .outline_on_path
            .apply(&JsValue::NULL, &args)
            .ok()?
            .as_string()?;
        (!d.is_empty()).then_some(d)
    }
}

/// Everything a lowering pass needs from the platform: font metrics + shape raster +
/// glyph outliner, the quality profile (bake tolerances, §7.1), plus whether to emit
/// source-position attributes (`data-xsvg-pos`).
struct Ctx<'a> {
    m: &'a dyn Measurer,
    shaper: &'a dyn Shaper,
    outliner: &'a dyn GlyphOutliner,
    quality: QualityProfile,
    sourcemap: bool,
}

/// `" data-xsvg-pos=\"START-END\""` (byte offsets of `node` in the original xsvg
/// source) when the source map is enabled, else empty. Attached to each emitted
/// top-level element so a viewer can project a rendered element back to its source.
fn pos_attr(node: roxmltree::Node, ctx: &Ctx) -> String {
    if !ctx.sourcemap {
        return String::new();
    }
    let r = node.range();
    format!(" data-xsvg-pos=\"{}-{}\"", r.start, r.end)
}

/// WASM entry point. `measure(text, fontCss) => number`,
/// `metrics(fontCss) => [ascent, descent, capHeight, xHeight]`, and
/// `rasterize(pathD, rowH) => Float64Array` are browser callbacks. Throws on
/// malformed XML so the JS side can surface the error.
///
/// When `sourcemap` is true, every emitted top-level element carries a
/// `data-xsvg-pos="START-END"` attribute — the byte range of the originating xsvg
/// node in `input` — so an interactive viewer can project a rendered element back
/// to its authoring source. Synthesized subtrees (e.g. `<x:textbox>` → `<text>…`)
/// tag only their root element; a viewer resolves inner nodes via the nearest
/// ancestor carrying the attribute.
#[wasm_bindgen]
pub fn compile(
    input: &str,
    quality: &str,
    sourcemap: bool,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    stepped_baseline: &js_sys::Function,
    outline_run: &js_sys::Function,
    outline_on_path: &js_sys::Function,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper {
        rasterize,
        stepped: stepped_baseline,
    };
    let outliner = JsOutliner {
        outline_run,
        outline_on_path,
    };
    compile_impl(input, quality, sourcemap, &m, &shaper, &outliner).map_err(|e| JsError::new(&e))
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    let q = xsvg_core::QualityProfile::parse(quality);
    check_nesting_depth(input, MAX_NESTING_DEPTH)?;
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!(
        "<!-- compiled by xsvg v0 (quality={}) -->\n",
        q.as_str()
    ));
    serialize(
        doc.root_element(),
        &mut out,
        true,
        &Ctx {
            m,
            shaper,
            outliner,
            quality: q,
            sourcemap,
        },
    );
    Ok(out)
}

/// Recursively emit a node as SVG.
fn serialize(node: roxmltree::Node, out: &mut String, is_root: bool, ctx: &Ctx) {
    if !node.is_element() {
        if node.is_text() {
            if let Some(t) = node.text() {
                push_escaped(out, t, false);
            }
        }
        return; // comments, PIs, etc. are dropped
    }

    // xsvg extension elements.
    if node.tag_name().namespace() == Some(XSVG_NS) {
        match node.tag_name().name() {
            "textbox" => emit_textbox(node, out, ctx),
            "textpath" => emit_textpath(node, out, ctx),
            "warp" => emit_warp(node, out, ctx),
            other => out.push_str(&format!("<!-- xsvg: <x:{other}> not yet lowered -->")),
        }
        return;
    }

    // <xsvg> root is just an alias for <svg>.
    let name = match node.tag_name().name() {
        "xsvg" => "svg",
        other => other,
    };

    if name == "text" && node.attribute("inline-size").is_some() {
        emit_inline_size_text(node, out, ctx);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, ctx);
        return;
    }

    // Sharp-cornered <rect> → <path>. Rounded rects pass through unchanged.
    if name == "rect" && node.attribute("rx").is_none() && node.attribute("ry").is_none() {
        emit_rect_as_path(node, out, ctx);
        return;
    }

    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(&format!(" xmlns=\"{SVG_NS}\""));
    }
    copy_attrs(node, out, &[]);
    out.push_str(&pos_attr(node, ctx));

    if node.has_children() {
        out.push('>');
        for child in node.children() {
            serialize(child, out, false, ctx);
        }
        out.push_str(&format!("</{name}>"));
    } else {
        out.push_str("/>");
    }
}

/// `<rect x y width height …>` → equivalent `<path d=… …>`.
fn emit_rect_as_path(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let w = attr_num(node, "width", 0.0);
    let h = attr_num(node, "height", 0.0);

    out.push_str("<path");
    copy_attrs(node, out, &["x", "y", "width", "height"]);
    out.push_str(&pos_attr(node, ctx));
    out.push_str(&format!(" d=\"M{x},{y} h{w} v{h} h{} Z\"/>", -w));
}

/// `<text inline-size="W">…</text>` → `<text>` with one `<tspan>` per wrapped line.
fn emit_inline_size_text(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node);
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let max_w = attr_num(node, "inline-size", 0.0);
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let lines = layout_flow(&collect_text(node), &style, x, y, max_w, m);

    out.push_str("<text");
    copy_attrs(node, out, &["inline-size", "line-height"]);
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
fn emit_textbox(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node);
    let fill = node.attribute("fill").unwrap_or("#000");
    let gx = attr_num(node, "glyph-x-scale", 1.0);
    let outline = node.attribute("outline") == Some("true");
    let stroke = outline_stroke_attrs(node);
    let pos = pos_attr(node, ctx);

    if let Some(reference) = node.attribute("in") {
        let Some(target) = resolve_ref(node, reference) else {
            out.push_str("<!-- xsvg: <x:textbox in> target not found -->");
            return;
        };
        // rect → reuse the rectangular path with the target's geometry (keeps fit/valign)
        if target.tag_name().name() == "rect" {
            let spec = textbox_area_spec(node, target);
            let (segments, styles, emits) = collect_runs(node, &style);
            let layout = layout_area_measured(
                &measure_runs(&segments, &styles, ctx.m),
                &style,
                &spec,
                ctx.m,
            );
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
            );
            return;
        }
        // any other shape → flow inside its filled outline via the raster region
        // (styled runs are not supported in curved region flow in v0)
        let region = shape_to_path_d(target)
            .and_then(|d| ctx.shaper.rasterize(&d, (style.size / 3.0).max(1.0)));
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
        );
        return;
    }

    let spec = textbox_area_spec(node, node);
    let (segments, styles, emits) = collect_runs(node, &style);
    let layout = layout_area_measured(
        &measure_runs(&segments, &styles, ctx.m),
        &style,
        &spec,
        ctx.m,
    );
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
    );
}

/// `<x:textpath in="#path" effect="skew|rainbow|stair">` (§6.13): outline the run and
/// warp it onto the referenced path via the [`GlyphOutliner::outline_on_path`] seam —
/// the text-on-path specialization of the geometry-transform pipeline (§7).
/// `baseline-shift` offsets the run from the path along the local normal (positive =
/// above); `align` / `start` place the run within the path's extent. Emits
/// `<g fill=… stroke=…>` + the warped `<path>`. `stair` (§6.13.3) is live `<text>` by
/// design — per-glyph stepped positions, no outliner — and doubles as skew's
/// degradation when no outline font is available; the last resort is a straight live
/// `<text>` at the element's x/y, so the document never breaks.
fn emit_textpath(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node);
    let fill = node.attribute("fill").unwrap_or("#000");
    let stroke = outline_stroke_attrs(node);
    let pos = pos_attr(node, ctx);
    let fx = PathEffect {
        effect: node.attribute("effect").unwrap_or("skew"),
        baseline_shift: attr_num(node, "baseline-shift", 0.0),
        align: node.attribute("align").unwrap_or("start"),
        start: attr_num(node, "start", 0.0),
    };
    let text = collect_text(node);

    let Some(reference) = node.attribute("in") else {
        out.push_str("<!-- xsvg: <x:textpath> requires in=\"#path\" -->");
        return;
    };
    let Some(path_d) = resolve_ref(node, reference).and_then(shape_to_path_d) else {
        out.push_str("<!-- xsvg: <x:textpath in> target not found or not a path -->");
        return;
    };

    // "stair" is live <text> by design (§6.13.3) — it never consults the outliner.
    if fx.effect != "stair" {
        if let Some(d) = ctx
            .outliner
            .outline_on_path(&text, &style, style.size, &path_d, &fx)
        {
            push_outline_group(out, fill, &stroke, &pos, &[d]);
            return;
        }
    }

    // Stair Step — authored (§6.13.3), or as skew's degradation without an outline
    // font (§6.13.1) — when the height profile can be sampled; anything else →
    // straight <text> at the element's x/y.
    if matches!(fx.effect, "stair" | "skew")
        && stepped_text(out, &text, &style, &fx, &path_d, fill, &pos, ctx)
    {
        return;
    }
    let (x, y) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    out.push_str("<text");
    push_font_attrs(out, &style, style.size, fill);
    out.push_str(&format!(" x=\"{}\" y=\"{}\"", fmt(x), fmt(y)));
    out.push_str(&pos);
    out.push('>');
    push_escaped(out, &text, false);
    out.push_str("</text>");
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
    text: &str,
    style: &TextStyle,
    fx: &PathEffect,
    path_d: &str,
    fill: &str,
    pos: &str,
    ctx: &Ctx,
) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
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
    advances.push(line_advance(text, style, style.size, ctx.m)); // total run width
    if advances.iter().any(|a| !a.is_finite()) {
        return false;
    }
    let Some((x0, ys)) = ctx
        .shaper
        .baseline_samples(path_d, &advances, fx.align, fx.start)
    else {
        return false;
    };
    if ys.len() != n || !x0.is_finite() {
        return false;
    }

    out.push_str("<text");
    push_font_attrs(out, style, style.size, fill);
    out.push_str(pos);
    // absolute per-glyph positions — spacing is baked in, so no spacing attrs
    let xs: Vec<String> = advances[..n].iter().map(|a| fmt(x0 + a)).collect();
    let ys: Vec<String> = ys.iter().map(|y| fmt(y - fx.baseline_shift)).collect();
    out.push_str(&format!(" x=\"{}\" y=\"{}\"", xs.join(" "), ys.join(" ")));
    out.push('>');
    push_escaped(out, text, false);
    out.push_str("</text>");
    true
}

/// `<x:warp field="…" bend="…" axis="h|v">` (§7.3): the generic geometry-warp
/// front-end. Children lower to pure `<path>` geometry, their union bbox builds the
/// field's envelope frame, and every path bakes through the §7.1 pipeline at the
/// quality tolerance. Children that cannot become path geometry (live text, rounded
/// rects, lines, images) are skipped with a marker — a warp never *silently* emits
/// unwarped content; an unknown/absent field or empty geometry emits the children
/// unwarped behind a marker. The element's own paint / `transform` ride on the
/// emitted `<g>` (an affine `transform` composes after the bake for free).
fn emit_warp(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
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
        .fold(None, |acc: Option<xsvg_core::kurbo::Rect>, &(a, b)| match (
            acc,
            svg_path_bbox(&inner[a..b]),
        ) {
            (Some(r), Some(n)) => Some(r.union(n)),
            (acc, n) => acc.or(n),
        });

    let field_name = node.attribute("field").unwrap_or("");
    let bend = attr_num(node, "bend", 0.0) / 100.0; // authored as −100…100 %
    let axis = WarpAxis::parse(node.attribute("axis").unwrap_or("h"));
    let field = bbox.and_then(|b| EnvelopePreset::new(field_name, bend, axis, b));

    out.push_str("<g");
    copy_attrs(node, out, &["field", "bend", "axis"]);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    match field {
        Some(f) => {
            let tol = ctx.quality.tolerance();
            let mut last = 0;
            for (a, b) in ranges {
                out.push_str(&inner[last..a]);
                // a path that fails to bake keeps its original geometry (§4 totality)
                match warp_svg_path(&inner[a..b], &f, tol) {
                    Some(d) => out.push_str(&d),
                    None => out.push_str(&inner[a..b]),
                }
                last = b;
            }
            out.push_str(&inner[last..]);
        }
        None => {
            out.push_str(&format!(
                "<!-- xsvg: <x:warp field=\"{field_name}\"> unknown field or no geometry — children unwarped -->"
            ));
            out.push_str(&inner);
        }
    }
    out.push_str("</g>");
}

/// Lower one `<x:warp>` child to pre-warp markup whose geometry is all `<path d>`:
/// basic shapes convert directly (the sharp-`rect` pass and `shape_to_path_d`),
/// everything else runs through the normal pipeline (so `outline="true"` text and
/// nested `<x:warp>`s compose). `Err(reason)` when the result still contains
/// geometry the bake cannot warp.
fn warp_child_markup(child: roxmltree::Node, ctx: &Ctx) -> Result<String, String> {
    let name = child.tag_name().name();
    if child.tag_name().namespace() != Some(XSVG_NS)
        && matches!(name, "circle" | "ellipse" | "polygon" | "polyline")
    {
        let d = shape_to_path_d(child).ok_or("degenerate shape")?;
        let mut s = String::from("<path");
        copy_attrs(child, &mut s, &["cx", "cy", "r", "rx", "ry", "points"]);
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
fn has_tag(s: &str, tag: &str) -> bool {
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

/// Byte ranges of every ` d="…"` attribute value in `s`. Sound on compiler output:
/// attribute values are quote-escaped, and text-bearing children are rejected before
/// this scan, so a raw `"` cannot appear inside a match.
fn find_path_d_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = s[i..].find(" d=\"") {
        let start = i + pos + 4;
        let Some(len) = s[start..].find('"') else {
            break;
        };
        out.push((start, start + len));
        i = start + len + 1;
    }
    out
}

/// Build the rectangular [`AreaSpec`] for a textbox, taking geometry from `geom`
/// (the textbox itself, or a referenced `rect`) and options from the textbox `node`.
fn textbox_area_spec(node: roxmltree::Node, geom: roxmltree::Node) -> AreaSpec {
    AreaSpec {
        x: attr_num(geom, "x", 0.0),
        y: attr_num(geom, "y", 0.0),
        width: attr_num(geom, "width", 0.0),
        height: attr_num(geom, "height", 0.0),
        padding: attr_num(node, "padding", 0.0),
        align: Align::parse(node.attribute("align").unwrap_or("start")),
        valign: VAlign::parse(node.attribute("valign").unwrap_or("top")),
        fit: fit_from(node.attribute("fit"), || attr_num(node, "fit-min", 6.0)),
        text_overflow: TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip")),
    }
}

/// Resolve a `#id` (or bare `id`) reference to its element anywhere in the document.
fn resolve_ref<'a>(node: roxmltree::Node<'a, 'a>, r: &str) -> Option<roxmltree::Node<'a, 'a>> {
    let id = r.strip_prefix('#').unwrap_or(r);
    node.document()
        .descendants()
        .find(|n| n.attribute("id") == Some(id))
}

/// Convert a fillable SVG shape element to a path `d` string (for rasterization).
/// `rect` is handled separately (rectangular fast path); returns `None` for shapes
/// with no fillable area (e.g. `<line>`).
fn shape_to_path_d(node: roxmltree::Node) -> Option<String> {
    match node.tag_name().name() {
        "path" => node.attribute("d").map(str::to_string),
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
        "polygon" | "polyline" => node.attribute("points").and_then(points_to_path_d),
        _ => None,
    }
}

/// `points="x0,y0 x1,y1 …"` → `"Mx0,y0 Lx1,y1 … Z"`. `None` if fewer than 2 points.
fn points_to_path_d(points: &str) -> Option<String> {
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

/// `<textArea>` (Rung 2, SVG Tiny 1.2 vocabulary): flowed text per the spec —
/// `text-align` (inline), `display-align` (block), `line-increment` (line height),
/// `auto` width/height, and `<tbreak/>` forced breaks.
fn emit_text_area(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node);
    let spec = TextAreaSpec {
        x: attr_num(node, "x", 0.0),
        y: attr_num(node, "y", 0.0),
        width: dim_attr(node, "width"),
        height: dim_attr(node, "height"),
        text_align: TextAlign::parse(node.attribute("text-align").unwrap_or("start")),
        display_align: DisplayAlign::parse(node.attribute("display-align").unwrap_or("auto")),
        line_increment: line_increment_attr(node),
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
        &outline_stroke_attrs(node),
        gx,
        m,
        &emits,
        &pos_attr(node, ctx),
        outline,
        ctx.outliner,
    );
}

/// Paint attributes carried onto an outlined `<g>` beyond `fill` — stroke and
/// paint-order — so create-outlines can render a stroked / keyline outline. Empty
/// for the common fill-only case; only meaningful when `outline="true"`.
fn outline_stroke_attrs(node: roxmltree::Node) -> String {
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
            s.push_str(&format!(" {name}=\"{v}\""));
        }
    }
    s
}

/// Emit an outlined text group: `<g fill=… stroke=…><path d="…"/>…</g>` — the shared
/// output of create-outlines (§6.12) and text-on-path (§6.13). `paths` are the glyph
/// path `d` strings (one per line for a box, one for a warped run).
fn push_outline_group(out: &mut String, fill: &str, stroke: &str, pos: &str, paths: &[String]) {
    out.push_str(&format!("<g fill=\"{fill}\"{stroke}{pos}>"));
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
fn push_font_attrs(out: &mut String, style: &TextStyle, size: f64, fill: &str) {
    out.push_str(&format!(
        " font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\"",
        style.family,
        fmt(size),
        style.weight,
        style.style,
        fill
    ));
}

#[allow(clippy::too_many_arguments)]
fn write_area_text(
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
            push_outline_group(out, fill, stroke, pos, &paths);
            return;
        }
        // else: outliner unavailable → fall through to live <text> below
    }

    out.push_str(&format!("<text text-anchor=\"{}\"", layout.anchor.svg()));
    push_font_attrs(out, style, layout.font_size, fill);
    out.push_str(pos);
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

/// Emit one line as an outer positioning `<tspan x y>` (carrying justify/glyph-scale
/// `textLength`) whose children are the line's styled runs — a bare string for base
/// runs, an inner `<tspan>` with the run's overrides for the rest.
fn emit_line(
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
                out.push_str("<tspan");
                emit_attr(out, "fill", &a.fill);
                emit_attr(out, "font-weight", &a.weight);
                emit_attr(out, "font-style", &a.style);
                emit_attr(out, "font-family", &a.family);
                out.push('>');
                push_escaped(out, &run.text, false);
                out.push_str("</tspan>");
            }
            _ => push_escaped(out, &run.text, false),
        }
    }
    out.push_str("</tspan>");
}

/// Emit ` name="value"` (escaped) when the override is present.
fn emit_attr(out: &mut String, name: &str, value: &Option<String>) {
    if let Some(v) = value {
        out.push(' ');
        out.push_str(name);
        out.push_str("=\"");
        push_escaped(out, v, true);
        out.push('"');
    }
}

fn fit_from(fit: Option<&str>, min: impl FnOnce() -> f64) -> Fit {
    if fit == Some("shrink") {
        Fit::Shrink { min: min() }
    } else {
        Fit::None
    }
}

/// A textArea dimension: absent or `"auto"` → `None` (auto), else a parsed length.
fn dim_attr(node: roxmltree::Node, name: &str) -> Option<f64> {
    match node.attribute(name) {
        None | Some("auto") => None,
        Some(v) => parse_num(v),
    }
}

fn line_increment_attr(node: roxmltree::Node) -> LineIncrement {
    match node.attribute("line-increment") {
        None | Some("auto") => LineIncrement::Auto,
        Some(v) => parse_num(v)
            .map(LineIncrement::Fixed)
            .unwrap_or(LineIncrement::Auto),
    }
}

// ---- helpers ---------------------------------------------------------------

fn style_from(node: roxmltree::Node) -> TextStyle {
    TextStyle {
        family: node
            .attribute("font-family")
            .unwrap_or("sans-serif")
            .to_string(),
        size: attr_pos(node, "font-size", 16.0),
        weight: node
            .attribute("font-weight")
            .unwrap_or("normal")
            .to_string(),
        style: node.attribute("font-style").unwrap_or("normal").to_string(),
        line_height: attr_pos(node, "line-height", 1.2),
        letter_spacing: spacing_attr(node, "letter-spacing"),
        word_spacing: spacing_attr(node, "word-spacing"),
    }
}

/// A `letter-spacing`/`word-spacing` value: `normal` or absent → 0, else a length.
fn spacing_attr(node: roxmltree::Node, name: &str) -> f64 {
    match node.attribute(name) {
        None | Some("normal") => 0.0,
        Some(v) => parse_num(v).unwrap_or(0.0),
    }
}

/// Concatenate all descendant text content (styling flattened away). Used by the
/// single-style paths: `<text inline-size>` and curved-shape region flow.
fn collect_text(node: roxmltree::Node) -> String {
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
/// attributes present are emitted. `font-size` is intentionally not overridable in
/// v0 (mixed sizes would perturb line-height/baseline).
#[derive(Clone, Default, PartialEq)]
struct EmitAttrs {
    fill: Option<String>,
    weight: Option<String>,
    style: Option<String>,
    family: Option<String>,
}

impl EmitAttrs {
    fn is_empty(&self) -> bool {
        self.fill.is_none()
            && self.weight.is_none()
            && self.style.is_none()
            && self.family.is_none()
    }
}

/// Walk a text container into styled segments for run layout: the `(text, style_id)`
/// stream (with `'\n'` for `<tbreak/>`), the layout style table (`styles[0]` = base),
/// and the parallel emit-attr table. `<tspan>` children introduce runs; nesting
/// composes (inner wins). Plain text collapses to a single base run.
fn collect_runs(
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

fn walk_runs(
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
fn intern_run(
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
    };
    if norm.is_empty() {
        return 0;
    }
    if let Some(i) = emits.iter().position(|e| *e == norm) {
        return i;
    }
    let mut ts = base.clone();
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

/// Copy a node's attributes (skipping `x:`-namespaced ones and any in `skip`).
fn copy_attrs(node: roxmltree::Node, out: &mut String, skip: &[&str]) {
    for attr in node.attributes() {
        if attr.namespace() == Some(XSVG_NS) || skip.contains(&attr.name()) {
            continue;
        }
        out.push(' ');
        out.push_str(attr.name());
        out.push_str("=\"");
        push_escaped(out, attr.value(), true);
        out.push('"');
    }
}

fn attr_num(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    node.attribute(name).and_then(parse_num).unwrap_or(default)
}

/// Read an `x:`-namespaced numeric attribute (e.g. `x:glyph-x-scale` on a plain
/// SVG element), falling back to `default`.
fn attr_num_ns(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    node.attribute((XSVG_NS, name))
        .and_then(parse_num)
        .unwrap_or(default)
}

/// `attr_num` but falls back to `default` for non-positive values (e.g. a 0 or
/// negative `font-size`), keeping degenerate input out of the layout math.
fn attr_pos(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    let v = attr_num(node, name, default);
    if v > 0.0 {
        v
    } else {
        default
    }
}

/// Reject pathologically nested input *before* the recursive XML parser sees it, so
/// deep nesting returns a clean error instead of overflowing the stack. A single
/// left-to-right scan tracking element depth; comments / CDATA / PIs / declarations
/// and self-closing tags do not add depth. This is a safety bound, not a validator —
/// `roxmltree` still does the real parsing (and rejects malformed input) afterward.
fn check_nesting_depth(input: &str, max: usize) -> Result<(), String> {
    let mut rest = input;
    let mut depth = 0usize;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map(|j| &after[j + 3..]).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("![CDATA[") {
            rest = after.find("]]>").map(|j| &after[j + 3..]).unwrap_or("");
        } else if rest.starts_with('!') || rest.starts_with('?') {
            rest = rest.find('>').map(|j| &rest[j + 1..]).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix('/') {
            depth = depth.saturating_sub(1);
            rest = after.find('>').map(|j| &after[j + 1..]).unwrap_or("");
        } else {
            let end = rest.find('>').unwrap_or(rest.len());
            if !rest[..end].ends_with('/') {
                depth += 1;
                if depth > max {
                    return Err(format!("xsvg: element nesting depth exceeds {max}"));
                }
            }
            rest = rest.get(end + 1..).unwrap_or("");
        }
    }
    Ok(())
}

/// Parse a leading numeric value, tolerating a trailing unit (e.g. `"13px"` → 13.0).
/// Rejects non-finite results (`inf`/`NaN`).
fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(s.len());
    s[..end].parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Format a float without a trailing `.0` for whole numbers (tidier path/coords).
fn fmt(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Minimal XML escaping. `in_attr` also escapes double quotes.
fn push_escaped(out: &mut String, s: &str, in_attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if in_attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic measurer: width = char count × 0.5 × size.
    struct Mono;
    impl Measurer for Mono {
        fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
            text.chars().count() as f64 * 0.5 * size
        }
    }

    /// No shape rasterizer (the default for tests that don't use `in=`).
    struct NoShaper;
    impl Shaper for NoShaper {
        fn rasterize(&self, _d: &str, _row_h: f64) -> Option<RasterRegion> {
            None
        }
    }

    /// Pretends every shape is a 60×60 box, so `in=`-region flow can be exercised
    /// without a browser (the real raster comes from the browser / fixtures).
    struct BoxShaper;
    impl Shaper for BoxShaper {
        fn rasterize(&self, _d: &str, row_h: f64) -> Option<RasterRegion> {
            let n = (60.0 / row_h).ceil().max(1.0) as usize;
            Some(RasterRegion::new(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: 60.0,
                    h: 60.0,
                },
                0.0,
                row_h,
                vec![Some((0.0, 60.0)); n],
            ))
        }
    }

    /// No glyph outliner (the default for tests not exercising `outline`).
    struct NoOutliner;
    impl GlyphOutliner for NoOutliner {
        fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, _x: f64, _b: f64) -> Option<String> {
            None
        }
    }

    /// Stub outliner: a deterministic 1×1 box path at the run origin, so the outline
    /// emit path can be exercised without a real font.
    struct BoxOutliner;
    impl GlyphOutliner for BoxOutliner {
        fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, x: f64, b: f64) -> Option<String> {
            Some(format!("M{x},{b} h1 v-1 h-1 Z"))
        }
        /// Marker "warped" path encoding that the reference path, size, and the full
        /// [`PathEffect`] reached the seam — enough to prove the `<x:textpath>` wiring.
        fn outline_on_path(
            &self,
            _t: &str,
            _s: &TextStyle,
            sz: f64,
            path_d: &str,
            fx: &PathEffect,
        ) -> Option<String> {
            Some(format!(
                "M0,0 L{},{} Z {} {} {} {}",
                path_d.len(),
                sz,
                fx.effect,
                fx.baseline_shift,
                fx.align,
                fx.start
            ))
        }
    }

    /// Stub [`Shaper`] with a stepped-baseline sampler: the run starts at
    /// `100 + start`, and glyph `i` sits at `y = 50 + i` — deterministic markers for
    /// the §6.13 stair-step fallback (rasterize stays unavailable).
    struct StepShaper;
    impl Shaper for StepShaper {
        fn rasterize(&self, _d: &str, _row_h: f64) -> Option<RasterRegion> {
            None
        }
        fn baseline_samples(
            &self,
            _path_d: &str,
            advances: &[f64],
            _align: &str,
            start: f64,
        ) -> Option<(f64, Vec<f64>)> {
            let n = advances.len().checked_sub(1)?;
            Some((100.0 + start, (0..n).map(|i| 50.0 + i as f64).collect()))
        }
    }

    fn compile_test(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap()
    }

    /// Compile with the 60×60 `BoxShaper`, for `<x:textbox in>` region-flow tests.
    fn compile_shaped(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &BoxShaper, &NoOutliner).unwrap()
    }

    /// Compile with the stub `BoxOutliner`, for `outline` (create-outlines) tests.
    fn compile_outlined(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap()
    }

    /// Compile with the source map on (`data-xsvg-pos` attributes emitted).
    fn compile_mapped(svg: &str) -> String {
        compile_impl(svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap()
    }

    #[test]
    fn rect_becomes_path() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="20" width="30" height="40" fill="#f00"/></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains("<path"));
        assert!(out.contains(r#"d="M10,20 h30 v40 h-30 Z""#));
        assert!(out.contains(r##"fill="#f00""##));
        assert!(!out.contains("<rect"));
    }

    #[test]
    fn inline_size_wraps_into_tspans() {
        // 6 words, each 3 chars → 1.5 wide at size 1; force several lines at width 5.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="5" y="10" font-size="10" inline-size="40">one two six ten cat dog</text></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("<tspan"));
        // more than one line emitted
        assert!(out.matches("<tspan").count() >= 2, "expected wrap: {out}");
        assert!(out.contains("<text"));
    }

    #[test]
    fn textbox_shrinks_to_fit() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="40" height="20" font-size="40" fit="shrink" fit-min="5" align="center" valign="middle">long label that must shrink</x:textbox></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains(r#"text-anchor="middle""#));
        // font-size must have been reduced from the authored 40
        let size = out
            .split("font-size=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap();
        assert!(size < 40.0 && size >= 5.0, "expected shrink, got {size}");
    }

    #[test]
    fn malformed_errors() {
        assert!(compile_impl(
            "<svg><unclosed></svg>",
            "balanced",
            false,
            &Mono,
            &NoShaper,
            &NoOutliner
        )
        .is_err());
    }

    #[test]
    fn text_area_wraps_and_uses_text_align() {
        // SVG Tiny 1.2: text-align (not text-anchor); explicit width wraps.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="40" height="100" font-size="10" text-align="center">one two three four five</textArea></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("<text"));
        assert!(out.contains(r#"text-anchor="middle""#)); // text-align:center → anchor middle
        assert!(out.matches("<tspan").count() >= 2);
        assert!(!out.contains("<textArea"));
    }

    #[test]
    fn text_area_auto_width_does_not_wrap() {
        // lacuna width = auto ⇒ single line, no wrapping
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="10" font-size="10">one two three four five</textArea></svg>"#;
        let out = compile_test(svg);
        assert_eq!(
            out.matches("<tspan").count(),
            1,
            "auto width must not wrap: {out}"
        );
    }

    #[test]
    fn degenerate_text_does_not_panic() {
        // empty textArea → empty <text>, no tspans
        let a = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="50" height="50"></textArea></svg>"#,
        );
        assert!(a.contains("<text") && !a.contains("<tspan"));

        // textArea with no width/height (lacuna = auto) → single unwrapped line
        let b = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea>hello world</textArea></svg>"#,
        );
        assert_eq!(b.matches("<tspan").count(), 1);

        // inline-size 0 → one word per line
        let c = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="0">a b c</text></svg>"#,
        );
        assert_eq!(c.matches("<tspan").count(), 3);

        // font-size 0 → falls back to default, no NaN from a 0/0 scale
        let d = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="50" height="50" font-size="0" fit="shrink">hi there friend</x:textbox></svg>"#,
        );
        assert!(d.contains("<text") && !d.contains("NaN"));

        // non-finite numeric attribute → default
        let e = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="1e999" y="10" font-size="10" inline-size="40">x y z</text></svg>"#,
        );
        assert!(!e.contains("inf") && !e.contains("NaN"));
    }

    #[test]
    fn text_overflow_ellipsis_emits_marker() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="80" height="30" font-size="12" text-overflow="ellipsis">this paragraph is far too tall to fit inside the short box provided here</textArea></svg>"#;
        assert!(compile_test(svg).contains('…'), "expected ellipsis marker");
        // default (clip) emits no marker
        let clip = svg.replace(" text-overflow=\"ellipsis\"", "");
        assert!(!compile_test(&clip).contains('…'));
    }

    #[test]
    fn tbreak_forces_lines() {
        // auto width would be a single line; each <tbreak/> forces a new one.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" font-size="10">one<tbreak/>two<tbreak/>three</textArea></svg>"#;
        let out = compile_test(svg);
        assert_eq!(out.matches("<tspan").count(), 3, "tbreak lines: {out}");
    }

    #[test]
    fn glyph_x_scale_emits_text_length() {
        // x:glyph-x-scale on a plain textArea → textLength/lengthAdjust per line.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="10" font-size="10" x:glyph-x-scale="1.5">hello</textArea></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("lengthAdjust=\"spacingAndGlyphs\""), "{out}");
        // "hello" = 5 chars × 0.5 × 10 = 25; scaled ×1.5 = 37.5
        assert!(out.contains("textLength=\"37.5\""), "{out}");

        // no scale attribute → no textLength emitted
        let plain = compile_test(
            r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="10" font-size="10">hello</textArea></svg>"#,
        );
        assert!(!plain.contains("textLength"));
    }

    #[test]
    fn glyph_x_scale_on_textbox_and_inline_size() {
        // <x:textbox> takes it unprefixed; "hi" = 2 × 0.5 × 10 = 10, ×1.5 = 15.
        let box_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" glyph-x-scale="1.5">hi</x:textbox></svg>"#;
        assert!(compile_test(box_svg).contains("textLength=\"15\""));

        // <text inline-size> takes it x:-prefixed (reused SVG element).
        let flow_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><text x="0" y="10" font-size="10" inline-size="500" x:glyph-x-scale="1.5">hi</text></svg>"#;
        assert!(compile_test(flow_svg).contains("textLength=\"15\""));
    }

    // ---- degradation / passthrough contract ----

    #[test]
    fn rounded_rect_passes_through() {
        // rx/ry present → not lowered to a path; emitted as <rect> unchanged.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" y="2" width="3" height="4" rx="2"/></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<rect"),
            "rounded rect should pass through: {out}"
        );
        assert!(out.contains("rx=\"2\""));
        assert!(!out.contains("<path"));
    }

    #[test]
    fn xsvg_root_aliases_to_svg() {
        let svg = r#"<xsvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g/></xsvg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<svg"),
            "root <xsvg> should become <svg>: {out}"
        );
        assert!(!out.contains("<xsvg"));
        assert!(out.contains(&format!("xmlns=\"{SVG_NS}\"")));
    }

    #[test]
    fn unknown_extension_degrades_to_marker() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:mesh/></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<!-- xsvg: <x:mesh> not yet lowered -->"),
            "{out}"
        );
    }

    #[test]
    fn xsvg_attributes_are_stripped() {
        // an x:-namespaced attribute on a passed-through element is dropped.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><circle cx="5" cy="5" r="3" x:note="hint"/></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("<circle"));
        assert!(out.contains("r=\"3\""));
        assert!(!out.contains("note"), "x: attr should be stripped: {out}");
    }

    #[test]
    fn letter_spacing_affects_layout_and_is_emitted() {
        // Mono: char = 0.5 × size. At size 10, "aa bb" = 10 + 5 + 10 = 25 wide (4 gaps).
        let base = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10">aa bb</textArea></svg>"#;
        assert_eq!(compile_test(base).matches("<tspan").count(), 1);

        // letter-spacing=3 adds 4·3 = 12 → 37 > 25, so it must wrap to two lines,
        // and the attribute is written onto the synthesized <text>.
        let spaced = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10" letter-spacing="3">aa bb</textArea></svg>"#;
        let out = compile_test(spaced);
        assert_eq!(out.matches("<tspan").count(), 2, "{out}");
        assert!(out.contains("letter-spacing=\"3\""), "{out}");

        // On <text inline-size> the attribute is forwarded verbatim (passthrough).
        let flow = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="500" letter-spacing="2">hi there</text></svg>"#;
        assert!(compile_test(flow).contains("letter-spacing=\"2\""));
    }

    #[test]
    fn word_spacing_affects_layout_and_is_emitted() {
        // Mono: char = 0.5 × size. At size 10, "aa bb" = 10 + 5 + 10 = 25 (one gap).
        let base = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10">aa bb</textArea></svg>"#;
        assert_eq!(compile_test(base).matches("<tspan").count(), 1);

        // word-spacing=6 widens the inter-word gap: 25 + 6 = 31 > 25 → wraps, and
        // the attribute lands on the synthesized <text>.
        let spaced = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10" word-spacing="6">aa bb</textArea></svg>"#;
        let out = compile_test(spaced);
        assert_eq!(out.matches("<tspan").count(), 2, "{out}");
        assert!(out.contains("word-spacing=\"6\""), "{out}");

        // Forwarded on <text inline-size> too.
        let flow = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="500" word-spacing="4">hi there</text></svg>"#;
        assert!(compile_test(flow).contains("word-spacing=\"4\""));
    }

    #[test]
    fn justify_emits_textlength_spacing_on_full_lines() {
        // Mono: char = 0.5 × size. Wraps to several lines at width 40, size 10.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="40" font-size="10" text-align="justify">aa bb cc dd ee ff</textArea></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains(r#"text-anchor="start""#), "{out}");
        assert!(
            out.contains(r#"textLength="40" lengthAdjust="spacing""#),
            "expected justified full lines: {out}"
        );
        // start alignment (no justify) emits no textLength
        let plain = svg.replace(" text-align=\"justify\"", "");
        assert!(!compile_test(&plain).contains("textLength"));
    }

    #[test]
    fn justify_on_textbox_uses_content_width() {
        // width 60, padding 10 → content width 40 is the justify target.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="60" height="200" padding="10" align="justify" font-size="10">aa bb cc dd ee ff</x:textbox></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains(r#"textLength="40" lengthAdjust="spacing""#),
            "{out}"
        );
    }

    #[test]
    fn textbox_in_rect_binds_to_referenced_geometry() {
        // The textbox has no geometry of its own; it takes the rect's box.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><rect id="card" x="10" y="10" width="200" height="80"/><x:textbox in="#card" align="center" valign="middle">label</x:textbox></svg>"##;
        let out = compile_test(svg); // rect fast path — no shaper needed
        assert!(out.contains("<text") && !out.contains("<x:textbox"));
        assert!(out.contains(r#"text-anchor="middle""#));
        // centered in content box: x=10, width 200 → anchor x = 10 + 100 = 110
        assert!(out.contains(r#"x="110""#), "{out}");
    }

    #[test]
    fn textbox_in_shape_flows_region() {
        // Non-rect target → region flow via the (test) shaper; emits <text>/<tspan>.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><circle id="blob" cx="30" cy="30" r="30"/><x:textbox in="#blob" align="start" font-size="10">one two three four five six seven</x:textbox></svg>"##;
        let out = compile_shaped(svg);
        assert!(out.contains("<text") && !out.contains("<x:textbox"));
        assert!(out.contains(r#"text-anchor="start""#));
        assert!(
            out.matches("<tspan").count() >= 2,
            "expected flowed lines: {out}"
        );
    }

    #[test]
    fn textbox_in_missing_target_degrades_to_marker() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox in="#ghost">hi</x:textbox></svg>"##;
        let out = compile_shaped(svg);
        assert!(out.contains("target not found"), "{out}");
        assert!(!out.contains("<text"));
    }

    #[test]
    fn deep_nesting_errors_instead_of_aborting() {
        // Far past the stack-overflow threshold; the depth guard must turn this into
        // a clean Err rather than a hard abort (see check_nesting_depth).
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(5000),
            "</g>".repeat(5000)
        );
        let err = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap_err();
        assert!(err.contains("nesting depth"), "{err}");
    }

    #[test]
    fn many_siblings_and_self_closing_are_not_rejected() {
        // Depth guard must count nesting, not element count: thousands of siblings
        // (net depth 1) stay well under the limit.
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
            "<rect/>".repeat(3000)
        );
        assert!(compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
        // and modest legitimate nesting is fine
        let nested = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(64),
            "</g>".repeat(64)
        );
        assert!(compile_impl(&nested, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
    }

    #[test]
    fn glyph_x_scale_non_positive_is_ignored() {
        // 0 and negative scales must not emit a zero/negative textLength or NaN.
        for v in ["0", "-1.5"] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"https://xsvg.visioncortex.org\"><textArea x=\"0\" y=\"10\" font-size=\"10\" x:glyph-x-scale=\"{v}\">hello</textArea></svg>"
            );
            let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
            assert!(out.contains("<text") && !out.contains("<textArea"));
            assert!(
                !out.contains("textLength"),
                "scale {v} still emitted: {out}"
            );
            assert!(!out.contains("NaN"));
        }
    }

    #[test]
    fn tbreak_only_textarea_does_not_panic() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="50" font-size="10"><tbreak/></textArea></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("<text") && !out.contains("NaN"));
    }

    #[test]
    fn negative_geometry_does_not_panic_or_leak_nan() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="-40" height="-20" padding="-5" fit="shrink" align="center" valign="middle" font-size="10">hi there friend</x:textbox></svg>"#;
        assert!(!compile_test(svg).contains("NaN"));
    }

    #[test]
    fn styled_tspan_runs_emit_nested_tspans() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10" fill="#111">Ship <tspan font-weight="bold" fill="#e11">fast</tspan> today</textArea></svg>"##;
        let out = compile_test(svg);
        // base attrs live on the <text>; the bold run is a nested <tspan> with overrides
        assert!(out.contains(r##"fill="#111""##), "base fill missing: {out}");
        assert_eq!(out.matches("font-weight=\"bold\"").count(), 1, "{out}");
        assert!(out.contains(r##"fill="#e11""##), "run fill missing: {out}");
        assert!(out.contains("fast"), "{out}");
        // base-styled parts stay bare text (not wrapped)
        assert!(out.contains("Ship ") && out.contains("today"));
    }

    #[test]
    fn plain_textarea_has_no_inner_tspans() {
        // no runs → one outer <tspan> per line, nothing nested
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="500" font-size="10">hello world</textArea></svg>"#;
        assert_eq!(compile_test(svg).matches("<tspan").count(), 1);
    }

    #[test]
    fn outline_emits_paths_not_text() {
        // outline="true" with a working outliner → <g><path>, no <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="#111">Hi</x:textbox></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"#111\""), "{out}");
        assert!(out.contains("<path d=\""), "{out}");
        assert!(!out.contains("<text") && !out.contains("<x:textbox"));
    }

    #[test]
    fn textarea_x_outline_emits_paths() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="200" font-size="10" x:outline="true">one two</textArea></svg>"##;
        let out = compile_outlined(svg);
        assert!(
            out.contains("<path d=\"") && !out.contains("<text"),
            "{out}"
        );
    }

    #[test]
    fn outline_carries_stroke_onto_the_group() {
        // stroke/stroke-width on an outlined box propagate to the outline <g> (a
        // keyline outline), and fill="none" is honored — the live <text> branch is
        // unaffected (it has no stroke path).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="none" stroke="#fff" stroke-width="1.5" stroke-linejoin="round">Hi</x:textbox></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"none\""), "{out}");
        assert!(out.contains("stroke=\"#fff\""), "{out}");
        assert!(out.contains("stroke-width=\"1.5\""), "{out}");
        assert!(out.contains("stroke-linejoin=\"round\""), "{out}");
        assert!(out.contains("<path d=\""), "{out}");
    }

    #[test]
    fn outline_falls_back_to_text_without_a_font() {
        // outline requested but the outliner has no font (returns None) → live <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true">Hi</x:textbox></svg>"##;
        let out = compile_test(svg); // NoOutliner
        assert!(
            out.contains("<text") && !out.contains("<path d=\""),
            "{out}"
        );
    }

    #[test]
    fn textpath_emits_warped_path() {
        // <x:textpath in="#p"> with a working warper → <g><path>, no <text> (§6.13)
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 C40,0 80,40 120,20" fill="none"/><x:textpath in="#p" effect="skew" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"#111\""), "{out}");
        assert!(out.contains("<path d=\"M0,0 L"), "{out}");
        assert!(!out.contains("<text"), "{out}");
    }

    #[test]
    fn textpath_carries_stroke() {
        // fill="none" + stroke → a keyline outline on the warped path
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="none" stroke="#0af" stroke-width="1.5">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"none\""), "{out}");
        assert!(out.contains("stroke=\"#0af\""), "{out}");
    }

    #[test]
    fn textpath_falls_back_to_text_without_a_font() {
        // no path-warping backend (NoOutliner → outline_on_path defaults to None) → live <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" x="0" y="20" font-size="20">wave</x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains(">wave</text>"), "{out}");
        assert!(!out.contains("<path d=\"M0,0 L"), "{out}");
    }

    #[test]
    fn textpath_missing_target_is_a_comment() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textpath in="#nope" font-size="20">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(
            out.contains("<!-- xsvg: <x:textpath in> target not found"),
            "{out}"
        );
    }

    #[test]
    fn textpath_forwards_effect_and_baseline_shift() {
        // effect="rainbow" + baseline-shift reach the outliner seam (§6.13.2)
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,60 A60,60 0 0 1 120,60" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="8" font-size="20" fill="#111">arc</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("rainbow 8"), "{out}");
    }

    #[test]
    fn textpath_defaults_to_skew_with_zero_shift() {
        // no effect / baseline-shift attributes → skew, shift 0 at the seam
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("skew 0"), "{out}");
    }

    #[test]
    fn textpath_forwards_align_and_start() {
        // align/start placement options reach the outliner seam (§6.13)
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" align="middle" start="12" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("skew 0 middle 12"), "{out}");
    }

    #[test]
    fn textpath_skew_degrades_to_stepped_baseline() {
        // No outline font, but a height-profile sampler → live <text> whose glyphs are
        // individually placed (Stair Step, §6.13.1). Mono(0.5·size): prefix advances of
        // "abc" at size 10 are 0/5/10; StepShaper starts the run at 100+start and steps
        // y by one per glyph; baseline-shift subtracts.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="skew" baseline-shift="2" start="10" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_impl(svg, "balanced", false, &Mono, &StepShaper, &NoOutliner).unwrap();
        assert!(out.contains(r#"x="110 115 120""#), "{out}");
        assert!(out.contains(r#"y="48 49 50""#), "{out}");
        assert!(out.contains(">abc</text>"), "{out}");

        // letter-spacing widens the per-glyph gaps in the baked positions (§6.8)
        let ls = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="skew" letter-spacing="2" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_impl(ls, "balanced", false, &Mono, &StepShaper, &NoOutliner).unwrap();
        assert!(out.contains(r#"x="100 107 114""#), "{out}");
    }

    #[test]
    fn textpath_stair_is_an_authored_effect() {
        // effect="stair" chooses the stepped live-<text> lowering even when an
        // outline font IS available — it never consults the outliner (§6.13.3).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="stair" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_impl(svg, "balanced", false, &Mono, &StepShaper, &BoxOutliner).unwrap();
        assert!(out.contains(r#"x="100 105 110""#), "{out}");
        assert!(out.contains(r#"y="50 51 52""#), "{out}");
        assert!(!out.contains("<path d=\"M0,0 L"), "outliner ran: {out}");

        // with no height-profile sampler either → straight <text>, never a panic
        let out = compile_impl(svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
    }

    #[test]
    fn textpath_rainbow_without_font_stays_straight() {
        // stepped degradation is skew-only; rainbow with no outliner → straight <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_impl(svg, "balanced", false, &Mono, &StepShaper, &NoOutliner).unwrap();
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
        assert!(!out.contains("x=\"100"), "{out}");
    }

    #[test]
    fn textpath_stepped_skips_empty_and_whitespace_text() {
        // nothing to place → the straight fallback, never a panic or an empty x list
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" font-size="10">   </x:textpath></svg>"##;
        let out = compile_impl(svg, "balanced", false, &Mono, &StepShaper, &NoOutliner).unwrap();
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
    }

    #[test]
    fn textpath_degenerate_input_never_panics_or_leaks_nan() {
        // Degenerate baseline-shift values (garbage, ±inf, NaN, unit suffix) collapse
        // to a finite number at the seam — never NaN (§4 totality).
        for bad in ["garbage", "1e999", "-1e999", "NaN", "inf"] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="{bad}" font-size="20">wave</x:textpath></svg>"##
            );
            let out = compile_outlined(&svg);
            assert!(out.contains("rainbow 0"), "shift={bad}: {out}");
            assert!(!out.contains("NaN"), "shift={bad}: {out}");
        }
        // A unit suffix parses its numeric prefix (13px → 13), like every length attr.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="13px" font-size="20">wave</x:textpath></svg>"##;
        assert!(compile_outlined(svg).contains("rainbow 13"));
    }

    #[test]
    fn textpath_empty_and_degenerate_targets_do_not_panic() {
        // empty run → still total (the stub warps ""; a real backend may fall back)
        let empty = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20"></x:textpath></svg>"##;
        compile_outlined(empty);
        // zero-length path data still reaches the seam verbatim (degeneracy is the
        // backend's to detect — it returns None and the element falls back)
        let point = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20">dot</x:textpath></svg>"##;
        compile_outlined(point);
        // a target with no fillable geometry (<line>) → comment marker, no panic
        let line = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><line id="p" x1="0" y1="20" x2="120" y2="20"/><x:textpath in="#p" effect="rainbow" font-size="20">hi</x:textpath></svg>"##;
        assert!(compile_outlined(line).contains("not found or not a path"));
    }

    // ---- <x:warp> (§7.3) ----

    const XW: &str =
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">"##;

    #[test]
    fn warp_arch_bends_a_rect() {
        // bbox 100×40, bend 100% → A = 25: the top edge's center maps to (50, −25),
        // the corners stay pinned (u = ±1 → Δ = 0), and the rect is now a polyline.
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="100" height="40" fill="#f00"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<g"), "{out}");
        assert!(!out.contains("<rect"), "{out}");
        assert!(out.contains(r##"fill="#f00""##), "{out}");
        assert!(out.contains("M0,0"), "{out}");
        assert!(out.contains("50,-25"), "apex missing: {out}");
        assert!(out.matches('L').count() > 8, "no subdivision: {out}");
    }

    #[test]
    fn warp_skips_live_text_child_with_a_marker() {
        // no outline font → the textbox lowers to live <text>, which a warp must
        // not silently emit unwarped
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi</x:textbox></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<x:warp> skipped <textbox>"), "{out}");
        assert!(!out.contains("<text "), "{out}");
    }

    #[test]
    fn warp_warps_outlined_text_child() {
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">hi</x:textbox></x:warp></svg>"##
        );
        let out = compile_outlined(&svg);
        assert!(!out.contains("<text"), "{out}");
        assert!(!out.contains("skipped"), "{out}");
        assert!(out.contains("<path d=\"M"), "{out}");
    }

    #[test]
    fn warp_circle_child_becomes_a_warped_path() {
        let svg = format!(
            r##"{XW}<x:warp field="flag" bend="60"><circle cx="50" cy="50" r="40" fill="#0af"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("<circle"), "{out}");
        assert!(out.contains(r##"fill="#0af""##), "{out}");
        assert!(out.contains("<path"), "{out}");
    }

    #[test]
    fn warp_unknown_field_marks_and_passes_through() {
        let svg = format!(
            r##"{XW}<x:warp field="bogus" bend="50"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("unknown field or no geometry"), "{out}");
        assert!(out.contains("h100"), "child not passed through: {out}");
    }

    #[test]
    fn warp_quality_grades_segment_count() {
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="200" height="60"/></x:warp></svg>"##
        );
        let fast = compile_impl(&svg, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        let hi = compile_impl(&svg, "highest", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        assert!(
            hi.matches('L').count() > fast.matches('L').count(),
            "highest ({}) !> fast ({})",
            hi.matches('L').count(),
            fast.matches('L').count()
        );
    }

    #[test]
    fn warp_nested_composes() {
        // the inner warp bakes first (recursive serialize), the outer re-bakes its
        // emitted paths — nesting composes innermost-first
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="40"><x:warp field="flag" bend="40"><rect x="0" y="0" width="100" height="30"/></x:warp></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.matches("<g").count() >= 2, "{out}");
        assert!(out.contains("<path"), "{out}");
        assert!(!out.contains("skipped"), "{out}");
    }

    #[test]
    fn warp_degenerate_input_never_panics() {
        // no children / no geometry / garbage bend / rounded rect (unwarpable)
        for svg in [
            format!(r##"{XW}<x:warp field="arch" bend="50"></x:warp></svg>"##),
            format!(
                r##"{XW}<x:warp field="arch" bend="garbage"><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##
            ),
            format!(
                r##"{XW}<x:warp field="wave" bend="100"><rect x="0" y="0" width="10" height="10" rx="3"/></x:warp></svg>"##
            ),
            format!(r##"{XW}<x:warp><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##),
        ] {
            let out = compile_test(&svg);
            assert!(!out.contains("NaN"), "{out}");
        }
    }

    #[test]
    fn xml_special_chars_are_escaped() {
        // text content and attribute values must be entity-escaped.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" aria-label="a &quot;b&quot; &amp; c">x &lt; y &amp; z</text></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("x &lt; y &amp; z"), "text not escaped: {out}");
        assert!(
            out.contains("&quot;b&quot;") && out.contains("&amp; c"),
            "attr not escaped: {out}"
        );
    }

    // ---- source map (data-xsvg-pos) ----

    #[test]
    fn sourcemap_off_by_default() {
        // The default compile (sourcemap=false) must not pollute the output.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="10" y="20" width="30" height="40"/></svg>"#;
        assert!(!compile_test(svg).contains("data-xsvg-pos"));
    }

    #[test]
    fn sourcemap_tags_elements_with_valid_source_ranges() {
        // A passthrough <rect>→<path>, a synthesized <x:textbox>→<text>, plus the
        // <svg> root all get tagged; every range is well-formed and in-bounds, and
        // maps back to the authoring node's markup.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><rect id="r" x="10" y="20" width="30" height="40"/><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi there</x:textbox></svg>"##;
        let out = compile_mapped(svg);
        let n = svg.len();

        let ranges: Vec<(usize, usize)> = out
            .split("data-xsvg-pos=\"")
            .skip(1)
            .map(|s| {
                let r = s.split('"').next().unwrap();
                let (a, b) = r.split_once('-').unwrap();
                (a.parse().unwrap(), b.parse().unwrap())
            })
            .collect();

        // svg root + rect + textbox
        assert!(ranges.len() >= 3, "expected >=3 tagged elements: {out}");
        for &(a, b) in &ranges {
            assert!(a < b && b <= n, "bad range {a}-{b} (len {n}): {out}");
        }
        // ranges point back at the authoring nodes
        let slices: Vec<&str> = ranges.iter().map(|&(a, b)| &svg[a..b]).collect();
        assert!(
            slices.iter().any(|s| s.contains("rect")),
            "no range covers the <rect>: {slices:?}"
        );
        assert!(
            slices.iter().any(|s| s.contains("x:textbox")),
            "no range covers the <x:textbox>: {slices:?}"
        );
    }
}
