//! Editable source objects become legal, flat SVG clip geometry. SVG clipPath
//! does not accept the groups/images produced by many xsvg pseudo shapes.
use super::*;
use std::collections::BTreeMap;

pub(super) fn emit_clip_path(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let previous = ctx.clip_geometry.replace(true);
    let outline = ctx.force_outline.replace(true);
    let _stops = ctx.suspend_stops();
    let mut body = String::new();
    for child in node.children().filter(|n| n.is_element()) {
        serialize(child, &mut body, false, ctx);
    }
    ctx.force_outline.set(outline);
    ctx.clip_geometry.set(previous);
    let source = format!(
        "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">{body}</svg>"
    );
    let Ok(parsed) = roxmltree::Document::parse(&source) else {
        out.push_str("<!-- xsvg: clipping geometry could not be lowered -->");
        return;
    };
    let mut definitions = String::new();
    let mut geometry = String::new();
    for child in parsed.root_element().children().filter(|n| n.is_element()) {
        flatten_clip(
            child,
            crate::kurbo::Affine::IDENTITY,
            &BTreeMap::new(),
            &source,
            &mut definitions,
            &mut geometry,
        );
    }
    out.push_str(&definitions);
    out.push_str("<clipPath");
    copy_attrs(node, out, &[], ctx);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    out.push_str(&geometry);
    out.push_str("</clipPath>");
}

/// SVG permits a clip <use> of a simple shape, but not a group. Expand the
/// latter with the same reference depth/fuel guards as geometry references.
pub(super) fn emit_clip_use(
    node: roxmltree::Node,
    href: &str,
    out: &mut String,
    ctx: &Ctx,
) -> bool {
    let Some(target) = ctx.resolve_ref(node, href) else {
        return false;
    };
    if target.tag_name().namespace() != Some(XSVG_NS)
        && !matches!(target.tag_name().name(), "g" | "a")
    {
        return false;
    }
    let key = href.trim_start_matches('#').to_string();
    if ctx.resolving.borrow().contains(&key)
        || ctx.resolving.borrow().len() >= MAX_REF_DEPTH
        || ctx.fuel.get() == 0
    {
        out.push_str("<!-- xsvg: clipping reference cycle or budget exceeded -->");
        return true;
    }
    ctx.fuel.set(ctx.fuel.get() - 1);
    ctx.resolving.borrow_mut().push(key);
    let [a, b, c, d, e, f] = use_placement(node).as_coeffs();
    out.push_str("<g");
    copy_attrs(
        node,
        out,
        &["href", "x", "y", "width", "height", "transform"],
        ctx,
    );
    out.push_str(&format!(" transform=\"matrix({a} {b} {c} {d} {e} {f})\">"));
    serialize(target, out, false, ctx);
    out.push_str("</g>");
    ctx.resolving.borrow_mut().pop();
    true
}

fn flatten_clip(
    node: roxmltree::Node,
    parent: crate::kurbo::Affine,
    inherited: &BTreeMap<String, String>,
    source: &str,
    definitions: &mut String,
    out: &mut String,
) {
    let name = node.tag_name().name();
    if matches!(
        name,
        "defs"
            | "clipPath"
            | "filter"
            | "mask"
            | "linearGradient"
            | "radialGradient"
            | "pattern"
            | "marker"
            | "style"
    ) {
        definitions.push_str(&source[node.range()]);
        return;
    }
    if node.attribute("display") == Some("none") {
        return;
    }
    let tf = parent
        * node
            .attribute("transform")
            .map(parse_transform)
            .unwrap_or(crate::kurbo::Affine::IDENTITY);
    let mut attrs = inherited.clone();
    // Carry inherited text layout and winding through wrappers, but not their
    // ids, paint, opacity or filter effects: clipping uses geometry, not colour.
    for a in node.attributes() {
        if matches!(
            a.name(),
            "font-family"
                | "font-size"
                | "font-style"
                | "font-weight"
                | "font-stretch"
                | "letter-spacing"
                | "word-spacing"
                | "text-anchor"
                | "dominant-baseline"
                | "visibility"
                | "clip-rule"
                | "clip-path"
        ) {
            attrs.insert(a.name().into(), a.value().into());
        }
    }
    if let Some(rule) = node.attribute("fill-rule") {
        if node.attribute("clip-rule").is_none() {
            attrs.insert("clip-rule".into(), rule.into());
        }
    }
    if let Some(style) = node.attribute("style") {
        let prior = attrs.remove("style").unwrap_or_default();
        attrs.insert("style".into(), format!("{prior};{style}"));
    }
    if matches!(name, "g" | "a" | "svg" | "symbol" | "switch") {
        for child in node.children().filter(|n| n.is_element()) {
            flatten_clip(child, tf, &attrs, source, definitions, out);
        }
        return;
    }
    if !matches!(
        name,
        "path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "text" | "use" | "image"
    ) {
        return;
    }
    // An image contributes its rectangular viewport, rather than its alpha.
    let output_name = if name == "image" { "rect" } else { name };
    for a in node.attributes() {
        if matches!(
            a.name(),
            "transform"
                | "filter"
                | "mask"
                | "opacity"
                | "fill"
                | "stroke"
                | "fill-opacity"
                | "stroke-opacity"
                | "style"
        ) {
            continue;
        }
        if name == "image"
            && !matches!(
                a.name(),
                "x" | "y" | "width" | "height" | "id" | "clip-path" | "data-xsvg-node"
            )
        {
            continue;
        }
        let key = if a.namespace() == Some("http://www.w3.org/1999/xlink") {
            format!("xlink:{}", a.name())
        } else {
            a.name().into()
        };
        attrs.insert(key, a.value().into());
    }
    let [a, b, c, d, e, f] = tf.as_coeffs();
    if tf != crate::kurbo::Affine::IDENTITY {
        attrs.insert(
            "transform".into(),
            format!("matrix({a} {b} {c} {d} {e} {f})"),
        );
    }
    out.push('<');
    out.push_str(output_name);
    for (key, value) in attrs {
        out.push(' ');
        out.push_str(&key);
        out.push_str("=\"");
        push_escaped(out, &value, true);
        out.push('"');
    }
    if name == "text" {
        out.push('>');
        for child in node.children() {
            out.push_str(&source[child.range()]);
        }
        out.push_str("</text>");
    } else {
        out.push_str("/>");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doubles::{BoxOutliner, Mono, NoOutliner, NoShaper};

    fn source(body: &str) -> String {
        format!("<svg xmlns=\"{SVG_NS}\" xmlns:x=\"{XSVG_NS}\"><defs><clipPath id=\"cut\">{body}</clipPath></defs><rect width=\"400\" height=\"300\" clip-path=\"url(#cut)\"/></svg>")
    }
    fn compile(body: &str) -> String {
        compile_impl(&source(body), "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap()
    }
    fn clip<'a>(doc: &'a roxmltree::Document<'a>) -> roxmltree::Node<'a, 'a> {
        doc.descendants()
            .find(|n| n.attribute("id") == Some("cut"))
            .unwrap()
    }

    #[test]
    fn clipping_nested_groups_compose_transform_and_keep_winding() {
        let out = compile(
            r#"<g transform="translate(20 30) rotate(25)"><g transform="scale(2 .5)"><rect width="60" height="40" rx="4"/><path fill-rule="evenodd" d="M0 0H20V20H0Z M5 5V15H15V5Z"/></g></g>"#,
        );
        let doc = roxmltree::Document::parse(&out).unwrap();
        let nodes: Vec<_> = clip(&doc).children().filter(|n| n.is_element()).collect();
        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().all(|n| n.tag_name().name() != "g"));
        let expected = parse_transform("translate(20 30) rotate(25) scale(2 .5)");
        for node in &nodes {
            let actual = parse_transform(node.attribute("transform").unwrap());
            for (a, b) in actual.as_coeffs().iter().zip(expected.as_coeffs()) {
                assert!((a - b).abs() < 1e-6);
            }
        }
        assert_eq!(nodes[1].attribute("clip-rule"), Some("evenodd"));
    }

    #[test]
    fn clipping_mesh_uses_exact_faces_without_paint_or_rasterization() {
        for body in [
            r##"<x:mesh x="10" y="20" width="80" height="40" cols="2" rows="1" fill="#f000 #0f0 #00f #ff0 #f0f #0ff"/>"##,
            r##"<x:mesh points="0,0 40,0 10,30 60,0 90,0 90,30" transform="rotate(30)"><x:face v="0 1 2" fill="#f00 #0f0 #00f"/><x:face v="3 4 5" fill="#0000"/></x:mesh>"##,
        ] {
            let out = compile(body);
            let doc = roxmltree::Document::parse(&out).unwrap();
            let nodes: Vec<_> = clip(&doc).children().filter(|n| n.is_element()).collect();
            assert_eq!(nodes.len(), 2, "{out}");
            assert!(nodes.iter().all(|n| n.tag_name().name() == "path"));
            assert!(!out.contains("data:image"));
            assert!(nodes
                .iter()
                .all(|n| n.attribute("opacity").is_none() && n.attribute("fill").is_none()));
        }
    }

    #[test]
    fn clipping_textbox_uses_glyphs_with_live_text_fallback() {
        let body = r#"<g transform="translate(30 50)"><x:textbox x="0" y="0" width="120" height="60" font-size="20">ABC</x:textbox></g>"#;
        let live = compile(body);
        let outlined =
            compile_impl(&source(body), "fast", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
        for (out, tag) in [(&live, "text"), (&outlined, "path")] {
            let doc = roxmltree::Document::parse(out).unwrap();
            let nodes: Vec<_> = clip(&doc).children().filter(|n| n.is_element()).collect();
            assert!(!nodes.is_empty(), "{out}");
            assert!(nodes.iter().all(|n| n.tag_name().name() == tag), "{out}");
            assert!(nodes.iter().all(|n| n.attribute("transform").is_some()));
        }
    }

    #[test]
    fn clipping_referenced_groups_expand_and_cycles_terminate() {
        let src = format!(
            r##"<svg xmlns="{SVG_NS}" xmlns:x="{XSVG_NS}"><defs><g id="drawing" transform="scale(2)"><rect width="10" height="20"/></g><clipPath id="cut"><use href="#drawing" x="30" y="40" transform="rotate(20)"/></clipPath></defs></svg>"##
        );
        let out = compile_impl(&src, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        let path = clip(&doc).first_element_child().unwrap();
        assert_eq!(path.tag_name().name(), "path");
        let expected = parse_transform("rotate(20) translate(30 40) scale(2)");
        for (a, b) in parse_transform(path.attribute("transform").unwrap())
            .as_coeffs()
            .iter()
            .zip(expected.as_coeffs())
        {
            assert!((a - b).abs() < 1e-6);
        }
        let cyclic = src.replace(
            "<rect width=\"10\" height=\"20\"/>",
            "<use href=\"#drawing\"/>",
        );
        assert!(compile_impl(&cyclic, "fast", false, &Mono, &NoShaper, &NoOutliner).is_ok());
    }

    #[test]
    fn clipping_boolean_and_pie_lower_to_flat_shapes() {
        for body in [
            r#"<x:boolean op="subtract" transform="translate(30 40)"><rect width="100" height="80"/><circle cx="50" cy="40" r="20"/></x:boolean>"#,
            r##"<x:pie cx="50" cy="50" r="40"><x:slice value="40" fill="#f00"/><x:slice value="60" fill="#00f"/></x:pie>"##,
        ] {
            let out = compile(body);
            let doc = roxmltree::Document::parse(&out).unwrap();
            let nodes: Vec<_> = clip(&doc).children().filter(|n| n.is_element()).collect();
            assert!(!nodes.is_empty(), "{out}");
            assert!(
                nodes
                    .iter()
                    .all(|n| matches!(n.tag_name().name(), "path" | "text")),
                "{out}"
            );
        }
    }
}
