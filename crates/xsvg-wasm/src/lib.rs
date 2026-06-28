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
use xsvg_core::{fit_size, measure_words, wrap, Measurer, TextStyle};

const XSVG_NS: &str = "https://xsvg.dev/ns";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Runs once when the module is instantiated: route Rust panics to `console.error`.
#[wasm_bindgen(start)]
pub fn on_start() {
    console_error_panic_hook::set_once();
}

/// Browser-backed `Measurer`: calls the JS `measure(text, fontCss)` callback.
struct JsMeasurer<'a> {
    func: &'a js_sys::Function,
}

impl Measurer for JsMeasurer<'_> {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        let css = style.font_css(size);
        self.func
            .call2(&JsValue::NULL, &JsValue::from_str(text), &JsValue::from_str(&css))
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    }
}

/// WASM entry point. `measure` is a JS `(text, fontCss) => number` (canvas measureText).
/// Throws on malformed XML so the JS side can surface the error.
#[wasm_bindgen]
pub fn compile(input: &str, quality: &str, measure: &js_sys::Function) -> Result<String, JsError> {
    let m = JsMeasurer { func: measure };
    compile_impl(input, quality, &m).map_err(|e| JsError::new(&e))
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(input: &str, quality: &str, m: &dyn Measurer) -> Result<String, String> {
    let q = xsvg_core::QualityProfile::parse(quality);
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!("<!-- compiled by xsvg v0 (quality={}) -->\n", q.as_str()));
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

    // <text inline-size> → wrapped lines.
    if name == "text" && node.attribute("inline-size").is_some() {
        emit_inline_size_text(node, out, m);
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

    let text = collect_text(node);
    let measured = measure_words(&text, &style, m);
    let lines = wrap(&measured, max_w, 1.0);
    let advance = style.size * style.line_height;

    out.push_str("<text");
    copy_attrs(node, out, &["inline-size", "line-height"]);
    out.push('>');
    for (i, line) in lines.iter().enumerate() {
        let ly = y + i as f64 * advance;
        out.push_str(&format!("<tspan x=\"{}\" y=\"{}\">", fmt(x), fmt(ly)));
        push_escaped(out, line, false);
        out.push_str("</tspan>");
    }
    out.push_str("</text>");
}

/// `<x:textbox>` → wrapped, aligned, optionally shrink-to-fit `<text>`.
fn emit_textbox(node: roxmltree::Node, out: &mut String, m: &dyn Measurer) {
    let style = style_from(node);
    let bx = attr_num(node, "x", 0.0);
    let by = attr_num(node, "y", 0.0);
    let bw = attr_num(node, "width", 0.0);
    let bh = attr_num(node, "height", 0.0);
    let pad = attr_num(node, "padding", 0.0);
    let align = node.attribute("align").unwrap_or("start");
    let valign = node.attribute("valign").unwrap_or("top");
    let fit = node.attribute("fit").unwrap_or("none");
    let fit_min = attr_num(node, "fit-min", 6.0);
    let fill = node.attribute("fill").unwrap_or("#000");

    let (cx, cy, cw, ch) = (bx + pad, by + pad, bw - 2.0 * pad, bh - 2.0 * pad);

    let text = collect_text(node);
    let measured = measure_words(&text, &style, m);

    let size = if fit == "shrink" {
        fit_size(&measured, style.size, style.line_height, cw, ch, fit_min)
    } else {
        style.size
    };
    let scale = size / style.size;
    let lines = wrap(&measured, cw, scale);
    let advance = size * style.line_height;
    let block_h = lines.len() as f64 * advance;

    let (anchor, ax) = match align {
        "center" => ("middle", cx + cw / 2.0),
        "end" => ("end", cx + cw),
        _ => ("start", cx),
    };
    let top = match valign {
        "middle" => cy + (ch - block_h) / 2.0,
        "bottom" => cy + (ch - block_h),
        _ => cy,
    };
    let first_baseline = top + size * 0.8; // approx ascent

    out.push_str(&format!(
        "<text text-anchor=\"{anchor}\" font-family=\"{}\" font-size=\"{}\" font-weight=\"{}\" font-style=\"{}\" fill=\"{}\">",
        style.family, fmt(size), style.weight, style.style, fill
    ));
    for (i, line) in lines.iter().enumerate() {
        let ly = first_baseline + i as f64 * advance;
        out.push_str(&format!("<tspan x=\"{}\" y=\"{}\">", fmt(ax), fmt(ly)));
        push_escaped(out, line, false);
        out.push_str("</tspan>");
    }
    out.push_str("</text>");
}

// ---- helpers ---------------------------------------------------------------

fn style_from(node: roxmltree::Node) -> TextStyle {
    TextStyle {
        family: node.attribute("font-family").unwrap_or("sans-serif").to_string(),
        size: attr_num(node, "font-size", 16.0),
        weight: node.attribute("font-weight").unwrap_or("normal").to_string(),
        style: node.attribute("font-style").unwrap_or("normal").to_string(),
        line_height: attr_num(node, "line-height", 1.2),
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

/// Parse a leading numeric value, tolerating a trailing unit (e.g. `"13px"` → 13.0).
fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(s.len());
    s[..end].parse().ok()
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
}
