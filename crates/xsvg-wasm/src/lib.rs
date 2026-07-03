//! WASM entry point for xsvg: `compile(input, quality, measure) -> svg`.
//!
//! v0 scope (Plan.md §4 + the typography POC): parse the xsvg/SVG input, run
//! lowering passes, and emit a plain-SVG-subset string. Passes wired so far:
//!   • `<rect>` (sharp-cornered) → `<path>`
//!   • `<text inline-size>` → wrapped `<tspan>` lines (Syntax.md Rung 1)
//!   • `<x:textbox>` → wrapped + aligned + shrink-to-fit text (Syntax.md Rung 3)
//! Other `x:` extensions are recognized and skipped with a marker.
//!
//! Text layout needs font metrics, which v0 borrows from the browser: `compile`
//! takes a JS `measure(text, fontCss) -> number` callback (canvas `measureText`).
//! That callback is the browser implementation of the pure `Measurer` seam — the
//! core layout logic lives in `xsvg-core` and stays platform-free.

use wasm_bindgen::prelude::*;
use xsvg_core::{
    layout_area, layout_flow, layout_region, layout_text_area, line_advance, Align, AreaLayout,
    AreaSpec, DisplayAlign, Fit, LineIncrement, Measurer, PlacedLine, RasterRegion, Rect,
    RegionSpec, Shaper, TextAlign, TextAreaSpec, TextOverflow, TextStyle, VAlign,
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
struct JsShaper<'a> {
    rasterize: &'a js_sys::Function,
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
}

/// Everything a lowering pass needs from the platform: font metrics + shape raster.
struct Ctx<'a> {
    m: &'a dyn Measurer,
    shaper: &'a dyn Shaper,
}

/// WASM entry point. `measure(text, fontCss) => number`,
/// `metrics(fontCss) => [ascent, descent, capHeight, xHeight]`, and
/// `rasterize(pathD, rowH) => Float64Array` are browser callbacks. Throws on
/// malformed XML so the JS side can surface the error.
#[wasm_bindgen]
pub fn compile(
    input: &str,
    quality: &str,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    compile_impl(input, quality, &m, &shaper).map_err(|e| JsError::new(&e))
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(
    input: &str,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
) -> Result<String, String> {
    let q = xsvg_core::QualityProfile::parse(quality);
    check_nesting_depth(input, MAX_NESTING_DEPTH)?;
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!(
        "<!-- compiled by xsvg v0 (quality={}) -->\n",
        q.as_str()
    ));
    serialize(doc.root_element(), &mut out, true, &Ctx { m, shaper });
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
        emit_inline_size_text(node, out, ctx.m);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, ctx.m);
        return;
    }

    // Sharp-cornered <rect> → <path>. Rounded rects pass through unchanged.
    if name == "rect" && node.attribute("rx").is_none() && node.attribute("ry").is_none() {
        emit_rect_as_path(node, out);
        return;
    }

    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(&format!(" xmlns=\"{SVG_NS}\""));
    }
    copy_attrs(node, out, &[]);

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
fn emit_rect_as_path(node: roxmltree::Node, out: &mut String) {
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let w = attr_num(node, "width", 0.0);
    let h = attr_num(node, "height", 0.0);

    out.push_str("<path");
    copy_attrs(node, out, &["x", "y", "width", "height"]);
    out.push_str(&format!(" d=\"M{x},{y} h{w} v{h} h{} Z\"/>", -w));
}

/// `<text inline-size="W">…</text>` → `<text>` with one `<tspan>` per wrapped line.
fn emit_inline_size_text(node: roxmltree::Node, out: &mut String, m: &dyn Measurer) {
    let style = style_from(node);
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let max_w = attr_num(node, "inline-size", 0.0);
    let gx = attr_num_ns(node, "glyph-x-scale", 1.0);
    let lines = layout_flow(&collect_text(node), &style, x, y, max_w, m);

    out.push_str("<text");
    copy_attrs(node, out, &["inline-size", "line-height"]);
    out.push('>');
    for line in &lines {
        emit_tspan(out, line, &style, style.size, gx, m);
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

    if let Some(reference) = node.attribute("in") {
        let Some(target) = resolve_ref(node, reference) else {
            out.push_str("<!-- xsvg: <x:textbox in> target not found -->");
            return;
        };
        // rect → reuse the rectangular path with the target's geometry (keeps fit/valign)
        if target.tag_name().name() == "rect" {
            let spec = textbox_area_spec(node, target);
            let layout = layout_area(&collect_text(node), &style, &spec, ctx.m);
            write_area_text(out, &layout, &style, fill, gx, ctx.m);
            return;
        }
        // any other shape → flow inside its filled outline via the raster region
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
        write_area_text(out, &layout, &style, fill, gx, ctx.m);
        return;
    }

    let spec = textbox_area_spec(node, node);
    let layout = layout_area(&collect_text(node), &style, &spec, ctx.m);
    write_area_text(out, &layout, &style, fill, gx, ctx.m);
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
fn emit_text_area(node: roxmltree::Node, out: &mut String, m: &dyn Measurer) {
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
    let layout = layout_text_area(&collect_text_with_breaks(node), &style, &spec, m);
    write_area_text(
        out,
        &layout,
        &style,
        node.attribute("fill").unwrap_or("#000"),
        gx,
        m,
    );
}

fn write_area_text(
    out: &mut String,
    layout: &AreaLayout,
    style: &TextStyle,
    fill: &str,
    glyph_x_scale: f64,
    m: &dyn Measurer,
) {
    out.push_str(&format!(
        "<text text-anchor=\"{}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\"",
        layout.anchor.svg(), style.family, fmt(layout.font_size), style.weight, style.style, fill
    ));
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
        emit_tspan(out, line, style, layout.font_size, glyph_x_scale, m);
    }
    out.push_str("</text>");
}

/// Emit one `<tspan>`, scaling glyph widths via `textLength` when `glyph_x_scale != 1`.
fn emit_tspan(
    out: &mut String,
    line: &PlacedLine,
    style: &TextStyle,
    size: f64,
    glyph_x_scale: f64,
    m: &dyn Measurer,
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
    push_escaped(out, &line.text, false);
    out.push_str("</tspan>");
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

/// Concatenate all descendant text content.
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

/// Like [`collect_text`], but each `<tbreak/>` element becomes a `'\n'` (the
/// SVG Tiny 1.2 forced line break). Document order is preserved.
fn collect_text_with_breaks(node: roxmltree::Node) -> String {
    let mut s = String::new();
    for d in node.descendants() {
        if d.is_text() {
            if let Some(t) = d.text() {
                s.push_str(t);
            }
        } else if d.is_element() && d.tag_name().name() == "tbreak" {
            s.push('\n');
        }
    }
    s
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

    fn compile_test(svg: &str) -> String {
        compile_impl(svg, "balanced", &Mono, &NoShaper).unwrap()
    }

    /// Compile with the 60×60 `BoxShaper`, for `<x:textbox in>` region-flow tests.
    fn compile_shaped(svg: &str) -> String {
        compile_impl(svg, "balanced", &Mono, &BoxShaper).unwrap()
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
        assert!(compile_impl("<svg><unclosed></svg>", "balanced", &Mono, &NoShaper).is_err());
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
        let err = compile_impl(&svg, "balanced", &Mono, &NoShaper).unwrap_err();
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
        assert!(compile_impl(&svg, "balanced", &Mono, &NoShaper).is_ok());
        // and modest legitimate nesting is fine
        let nested = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(64),
            "</g>".repeat(64)
        );
        assert!(compile_impl(&nested, "balanced", &Mono, &NoShaper).is_ok());
    }

    #[test]
    fn glyph_x_scale_non_positive_is_ignored() {
        // 0 and negative scales must not emit a zero/negative textLength or NaN.
        for v in ["0", "-1.5"] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"https://xsvg.visioncortex.org\"><textArea x=\"0\" y=\"10\" font-size=\"10\" x:glyph-x-scale=\"{v}\">hello</textArea></svg>"
            );
            let out = compile_impl(&svg, "balanced", &Mono, &NoShaper).unwrap();
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
}
