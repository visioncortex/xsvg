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
    layout_area, layout_flow, layout_text_area, Align, AreaLayout, AreaSpec, DisplayAlign, Fit,
    LineIncrement, Measurer, PlacedLine, TextAlign, TextAreaSpec, TextOverflow, TextStyle, VAlign,
};

const XSVG_NS: &str = "https://xsvg.dev/ns";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

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

/// WASM entry point. `measure(text, fontCss) => number` and
/// `metrics(fontCss) => [ascent, descent, capHeight, xHeight]` are canvas callbacks.
/// Throws on malformed XML so the JS side can surface the error.
#[wasm_bindgen]
pub fn compile(
    input: &str,
    quality: &str,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    compile_impl(input, quality, &m).map_err(|e| JsError::new(&e))
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(input: &str, quality: &str, m: &dyn Measurer) -> Result<String, String> {
    let q = xsvg_core::QualityProfile::parse(quality);
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!(
        "<!-- compiled by xsvg v0 (quality={}) -->\n",
        q.as_str()
    ));
    serialize(doc.root_element(), &mut out, true, m);
    Ok(out)
}

/// Recursively emit a node as SVG.
fn serialize(node: roxmltree::Node, out: &mut String, is_root: bool, m: &dyn Measurer) {
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
            "textbox" => emit_textbox(node, out, m),
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
        emit_inline_size_text(node, out, m);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, m);
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
            serialize(child, out, false, m);
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

/// `<x:textbox>` (Rung 3): explicit `align`/`valign`/`padding`/`fit`.
fn emit_textbox(node: roxmltree::Node, out: &mut String, m: &dyn Measurer) {
    let style = style_from(node);
    let spec = AreaSpec {
        x: attr_num(node, "x", 0.0),
        y: attr_num(node, "y", 0.0),
        width: attr_num(node, "width", 0.0),
        height: attr_num(node, "height", 0.0),
        padding: attr_num(node, "padding", 0.0),
        align: Align::parse(node.attribute("align").unwrap_or("start")),
        valign: VAlign::parse(node.attribute("valign").unwrap_or("top")),
        fit: fit_from(node.attribute("fit"), || attr_num(node, "fit-min", 6.0)),
        text_overflow: TextOverflow::parse(node.attribute("text-overflow").unwrap_or("clip")),
    };
    let gx = attr_num(node, "glyph-x-scale", 1.0);
    let layout = layout_area(&collect_text(node), &style, &spec, m);
    write_area_text(
        out,
        &layout,
        &style,
        node.attribute("fill").unwrap_or("#000"),
        gx,
        m,
    );
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
        "<text text-anchor=\"{}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\">",
        layout.anchor.svg(), style.family, fmt(layout.font_size), style.weight, style.style, fill
    ));
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
    if (glyph_x_scale - 1.0).abs() > 1e-6 && !line.text.is_empty() {
        let len = m.measure(&line.text, style, size) * glyph_x_scale;
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

    fn compile_test(svg: &str) -> String {
        compile_impl(svg, "balanced", &Mono).unwrap()
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
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><x:textbox x="0" y="0" width="40" height="20" font-size="40" fit="shrink" fit-min="5" align="center" valign="middle">long label that must shrink</x:textbox></svg>"#;
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
        assert!(compile_impl("<svg><unclosed></svg>", "balanced", &Mono).is_err());
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
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><x:textbox x="0" y="0" width="50" height="50" font-size="0" fit="shrink">hi there friend</x:textbox></svg>"#,
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
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><textArea x="0" y="10" font-size="10" x:glyph-x-scale="1.5">hello</textArea></svg>"#;
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
        let box_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><x:textbox x="0" y="0" width="100" height="40" font-size="10" glyph-x-scale="1.5">hi</x:textbox></svg>"#;
        assert!(compile_test(box_svg).contains("textLength=\"15\""));

        // <text inline-size> takes it x:-prefixed (reused SVG element).
        let flow_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><text x="0" y="10" font-size="10" inline-size="500" x:glyph-x-scale="1.5">hi</text></svg>"#;
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
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><x:mesh/></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<!-- xsvg: <x:mesh> not yet lowered -->"),
            "{out}"
        );
    }

    #[test]
    fn xsvg_attributes_are_stripped() {
        // an x:-namespaced attribute on a passed-through element is dropped.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><circle cx="5" cy="5" r="3" x:note="hint"/></svg>"#;
        let out = compile_test(svg);
        assert!(out.contains("<circle"));
        assert!(out.contains("r=\"3\""));
        assert!(!out.contains("note"), "x: attr should be stripped: {out}");
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
