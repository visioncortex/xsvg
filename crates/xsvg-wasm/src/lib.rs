//! WASM entry point for xsvg: `compile(input, quality) -> svg`.
//!
//! v0 scope (Plan.md §4): parse the xsvg/SVG input, normalize it, and emit a
//! plain-SVG-subset string. The only real lowering wired up so far is
//! `<rect>` → `<path>` (a sharp-cornered rect); `x:`-namespaced extensions are
//! recognized and skipped with a marker (lowering passes land in later phases).
//! Everything else passes through. This proves the parse → transform → emit loop
//! end to end through WASM.

use wasm_bindgen::prelude::*;
use xsvg_core::QualityProfile;

const XSVG_NS: &str = "https://xsvg.dev/ns";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Runs once when the module is instantiated: route Rust panics to `console.error`.
#[wasm_bindgen(start)]
pub fn on_start() {
    console_error_panic_hook::set_once();
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(input: &str, quality: &str) -> Result<String, String> {
    let q = QualityProfile::parse(quality);
    let doc = roxmltree::Document::parse(input)
        .map_err(|e| format!("xsvg parse error: {e}"))?;

    let mut out = String::new();
    out.push_str(&format!("<!-- compiled by xsvg v0 (quality={}) -->\n", q.as_str()));
    serialize(doc.root_element(), &mut out, true);
    Ok(out)
}

/// WASM entry point. Throws (rejects) on malformed XML so the JS side can surface the error.
#[wasm_bindgen]
pub fn compile(input: &str, quality: &str) -> Result<String, JsError> {
    compile_impl(input, quality).map_err(|e| JsError::new(&e))
}

/// Recursively emit a node as SVG.
fn serialize(node: roxmltree::Node, out: &mut String, is_root: bool) {
    if !node.is_element() {
        if node.is_text() {
            if let Some(t) = node.text() {
                push_escaped(out, t, false);
            }
        }
        return; // comments, PIs, etc. are dropped
    }

    // xsvg extension elements are not lowered yet — mark and skip the subtree.
    if node.tag_name().namespace() == Some(XSVG_NS) {
        out.push_str(&format!(
            "<!-- xsvg: <x:{}> not yet lowered -->",
            node.tag_name().name()
        ));
        return;
    }

    // <xsvg> root is just an alias for <svg>.
    let name = match node.tag_name().name() {
        "xsvg" => "svg",
        other => other,
    };

    // Real lowering pass: sharp-cornered <rect> -> <path>.
    // Rounded rects (rx/ry) pass through unchanged for now.
    if name == "rect" && node.attribute("rx").is_none() && node.attribute("ry").is_none() {
        emit_rect_as_path(node, out);
        return;
    }

    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(&format!(" xmlns=\"{SVG_NS}\""));
    }
    for attr in node.attributes() {
        if attr.namespace() == Some(XSVG_NS) {
            continue; // drop x: attributes until their lowering exists
        }
        out.push(' ');
        out.push_str(attr.name());
        out.push_str("=\"");
        push_escaped(out, attr.value(), true);
        out.push('"');
    }

    if node.has_children() {
        out.push('>');
        for child in node.children() {
            serialize(child, out, false);
        }
        out.push_str(&format!("</{name}>"));
    } else {
        out.push_str("/>");
    }
}

/// Convert `<rect x y width height …>` into an equivalent `<path d=… …>`,
/// preserving all non-geometry, non-`x:` attributes.
fn emit_rect_as_path(node: roxmltree::Node, out: &mut String) {
    let x = attr_f64(node, "x", 0.0);
    let y = attr_f64(node, "y", 0.0);
    let w = attr_f64(node, "width", 0.0);
    let h = attr_f64(node, "height", 0.0);

    out.push_str("<path");
    for attr in node.attributes() {
        if attr.namespace() == Some(XSVG_NS) {
            continue;
        }
        if matches!(attr.name(), "x" | "y" | "width" | "height") {
            continue;
        }
        out.push(' ');
        out.push_str(attr.name());
        out.push_str("=\"");
        push_escaped(out, attr.value(), true);
        out.push('"');
    }
    out.push_str(&format!(" d=\"M{x},{y} h{w} v{h} h{} Z\"/>", -w));
}

fn attr_f64(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    node.attribute(name)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
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

    #[test]
    fn rect_becomes_path() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="20" width="30" height="40" fill="#f00"/></svg>"##;
        let out = compile_impl(svg, "balanced").unwrap();
        assert!(out.contains("<path"));
        assert!(out.contains(r#"d="M10,20 h30 v40 h-30 Z""#));
        assert!(out.contains(r##"fill="#f00""##));
        assert!(!out.contains("<rect"));
    }

    #[test]
    fn xsvg_extension_is_skipped() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.dev/ns"><x:vstroke d="M0,0"/></svg>"#;
        let out = compile_impl(svg, "fast").unwrap();
        assert!(out.contains("not yet lowered"));
        assert!(out.contains("quality=fast"));
    }

    #[test]
    fn malformed_errors() {
        assert!(compile_impl("<svg><unclosed></svg>", "balanced").is_err());
    }
}
