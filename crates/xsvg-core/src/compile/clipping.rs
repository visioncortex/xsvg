//! Editable source objects become legal, flat SVG clip geometry. SVG clipPath
//! does not accept the groups/images produced by many xsvg pseudo shapes.
use super::*;
use std::collections::BTreeMap;

pub(super) fn emit_clip_path(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let owner = match node.attribute("id") {
        Some(id) => id.to_string(),
        None => format!("xsvg-clip-{}", node.range().start),
    };
    let lower = Lower {
        ctx,
        origin: node,
        owner,
        made: std::cell::Cell::new(0),
    };
    let Some((definitions, geometry)) = lower.geometry(node, false) else {
        out.push_str("<!-- xsvg: clipping geometry could not be lowered -->");
        return;
    };
    out.push_str(&definitions);
    out.push_str("<clipPath");
    copy_attrs(node, out, &[], ctx);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    out.push_str(&geometry);
    out.push_str("</clipPath>");
}

/// A clip a wrapper asked for, and the wrapper's user space (clip root → it).
type GroupClip = (String, crate::kurbo::Affine);

const IDENTITY: crate::kurbo::Affine = crate::kurbo::Affine::IDENTITY;

fn same_space(a: crate::kurbo::Affine, b: crate::kurbo::Affine) -> bool {
    a.as_coeffs()
        .iter()
        .zip(b.as_coeffs())
        .all(|(x, y)| (x - y).abs() < 1e-9)
}

fn matrix(tf: crate::kurbo::Affine) -> String {
    let [a, b, c, d, e, f] = tf.as_coeffs();
    format!("matrix({a} {b} {c} {d} {e} {f})")
}

/// The id a `clip-path="url(#id)"` value names, if it is a local reference.
fn clip_ref(value: &str) -> Option<&str> {
    let inner = value.trim().strip_prefix("url(")?.strip_suffix(')')?;
    inner
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .strip_prefix('#')
}

/// One authored `<clipPath>` being lowered, and the clips derived for it.
struct Lower<'a, 'c> {
    ctx: &'a Ctx<'c>,
    /// the authored clipPath: where local references resolve from
    origin: roxmltree::Node<'a, 'a>,
    /// prefix of derived clip ids — the authored clip's own id
    owner: String,
    made: std::cell::Cell<usize>,
}

impl Lower<'_, '_> {
    /// Compile `clip`'s children to (definitions, flat clip shapes). A `copy`
    /// repeats a clip emitted elsewhere, so its shapes drop their ids.
    fn geometry(&self, clip: roxmltree::Node, copy: bool) -> Option<(String, String)> {
        let ctx = self.ctx;
        let previous = ctx.clip_geometry.replace(true);
        let outline = ctx.force_outline.replace(true);
        let mut body = String::new();
        {
            let _stops = ctx.suspend_stops();
            for child in clip.children().filter(|n| n.is_element()) {
                serialize(child, &mut body, false, ctx);
            }
        }
        ctx.force_outline.set(outline);
        ctx.clip_geometry.set(previous);
        let source = format!(
            "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"http://www.w3.org/1999/xlink\">{body}</svg>"
        );
        let parsed = roxmltree::Document::parse(&source).ok()?;
        let mut definitions = String::new();
        let mut geometry = String::new();
        let root = Inherit {
            copy,
            ..Inherit::default()
        };
        for child in parsed.root_element().children().filter(|n| n.is_element()) {
            self.flatten(child, &root, &source, &mut definitions, &mut geometry);
        }
        Some((definitions, geometry))
    }

    /// A `clip-path` value that clips a leaf whose user space is `leaf` by
    /// every clip in `clips`, each taken in the space it was authored in. A
    /// clip already in the leaf's space is referenced as authored; any other
    /// is re-emitted as a derived `<clipPath>` carrying the change of space,
    /// chained to the next through its own `clip-path` (an intersection).
    fn chain(
        &self,
        clips: &[GroupClip],
        leaf: crate::kurbo::Affine,
        definitions: &mut String,
    ) -> Option<String> {
        let mut clips: Vec<&GroupClip> = clips.iter().collect();
        // one clip in the leaf's own space can end the chain untouched
        if let Some(i) = clips.iter().position(|(_, space)| same_space(*space, leaf)) {
            let own = clips.remove(i);
            clips.push(own);
        }
        let mut next: Option<String> = None;
        for (value, space) in clips.into_iter().rev() {
            let to_leaf = leaf.inverse() * *space;
            if next.is_none() && same_space(to_leaf, IDENTITY) {
                next = Some(value.clone());
                continue;
            }
            match self.derive(value, to_leaf, next.clone(), definitions, 0) {
                Some(id) => next = Some(format!("url(#{id})")),
                // not a clip this document can re-space: best effort, as authored
                None if next.is_none() => next = Some(value.clone()),
                None => {}
            }
        }
        next
    }

    /// Re-emit the clip `value` names with its geometry moved by `to_leaf`,
    /// intersected with `next`; returns the derived clip's id.
    fn derive(
        &self,
        value: &str,
        to_leaf: crate::kurbo::Affine,
        next: Option<String>,
        definitions: &mut String,
        depth: usize,
    ) -> Option<String> {
        let ctx = self.ctx;
        let id = clip_ref(value)?;
        let target = ctx.resolve_ref(self.origin, id)?;
        if target.tag_name().namespace() != Some(SVG_NS)
            || target.tag_name().name() != "clipPath"
            // fractions of the referencing element's box have no fixed space
            || target.attribute("clipPathUnits") == Some("objectBoundingBox")
            || depth >= MAX_REF_DEPTH
            || ctx.resolving.borrow().iter().any(|k| k == id)
            || ctx.fuel.get() == 0
        {
            return None;
        }
        ctx.fuel.set(ctx.fuel.get() - 1);
        ctx.resolving.borrow_mut().push(id.to_string());
        let lowered = self.geometry(target, true);
        // the target's own clip applies in the same (referencing) space
        let next = match target.attribute("clip-path") {
            Some(own) => self
                .derive(own, to_leaf, next.clone(), definitions, depth + 1)
                .map(|id| format!("url(#{id})"))
                .or(next),
            None => next,
        };
        ctx.resolving.borrow_mut().pop();
        let (defs, geometry) = lowered?;
        let n = self.made.get() + 1;
        self.made.set(n);
        let derived = format!("{}-clip{n}", self.owner);
        let tf = to_leaf
            * target
                .attribute("transform")
                .map(parse_transform)
                .unwrap_or(IDENTITY);
        definitions.push_str(&defs);
        definitions.push_str("<clipPath id=\"");
        push_escaped(definitions, &derived, true);
        definitions.push_str(&format!("\" transform=\"{}\"", matrix(tf)));
        if let Some(next) = next {
            definitions.push_str(" clip-path=\"");
            push_escaped(definitions, &next, true);
            definitions.push('"');
        }
        definitions.push('>');
        definitions.push_str(&geometry);
        definitions.push_str("</clipPath>");
        Some(derived)
    }
}

/// What a clip shape takes from the wrappers flattened away around it.
#[derive(Clone, Default)]
struct Inherit {
    tf: Option<crate::kurbo::Affine>,
    attrs: BTreeMap<String, String>,
    clips: Vec<GroupClip>,
    /// inside an expanded `<use>`: ids would repeat the referenced originals
    copy: bool,
}

/// SVG permits a clip <use> of a simple shape, but not a group. Expand the
/// latter with the same reference depth/fuel guards as geometry references.
/// Marks the scratch `<g>` an expanded clip `<use>` leaves for `flatten`.
const CLIP_COPY: &str = "data-xsvg-clip-copy";

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
    out.push_str(&format!(
        " {CLIP_COPY}=\"\" transform=\"matrix({a} {b} {c} {d} {e} {f})\">"
    ));
    serialize(target, out, false, ctx);
    out.push_str("</g>");
    ctx.resolving.borrow_mut().pop();
    true
}

impl Lower<'_, '_> {
    fn flatten(
        &self,
        node: roxmltree::Node,
        parent: &Inherit,
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
        let container = matches!(name, "g" | "a" | "svg" | "symbol" | "switch");
        if !container
            && !matches!(
                name,
                "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "polygon"
                    | "polyline"
                    | "text"
                    | "use"
                    | "image"
            )
        {
            return;
        }
        let mut inherit = parent.clone();
        let tf = parent.tf.unwrap_or(IDENTITY)
            * node
                .attribute("transform")
                .map(parse_transform)
                .unwrap_or(IDENTITY);
        inherit.tf = Some(tf);
        inherit.copy |= node.attribute(CLIP_COPY).is_some();
        let attrs = &mut inherit.attrs;
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
            ) {
                attrs.insert(a.name().into(), a.value().into());
            }
        }
        // `fill-rule` reaches here only from an xsvg shape (a plain SVG one's is
        // dropped at the source: SVG ignores it in a clip) — it is that shape's
        // winding, which a clip spells `clip-rule`.
        if let Some(rule) = node.attribute("fill-rule") {
            if node.attribute("clip-rule").is_none() {
                attrs.insert("clip-rule".into(), rule.into());
            }
        }
        if let Some(style) = node.attribute("style") {
            let prior = attrs.remove("style").unwrap_or_default();
            attrs.insert("style".into(), format!("{prior};{style}"));
        }
        // a stylesheet rule on a wrapper's class (`display:none`) still selects
        if let Some(class) = node.attribute("class") {
            let merged = match attrs.remove("class") {
                Some(prior) => format!("{prior} {class}"),
                None => class.into(),
            };
            attrs.insert("class".into(), merged);
        }
        // A clip applies in the user space of the element that asks for it.
        if let Some(clip) = node.attribute("clip-path") {
            inherit.clips.push((clip.into(), tf));
        }
        if container {
            for child in node.children().filter(|n| n.is_element()) {
                self.flatten(child, &inherit, source, definitions, out);
            }
            return;
        }
        let Inherit {
            mut attrs,
            clips,
            copy,
            ..
        } = inherit;
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
                    | "class"
                    | "clip-path"
            ) || (copy && a.name() == "id")
            {
                continue;
            }
            if name == "image"
                && !matches!(
                    a.name(),
                    "x" | "y" | "width" | "height" | "id" | "data-xsvg-node"
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
        if let Some(value) = self.chain(&clips, tf, definitions) {
            attrs.insert("clip-path".into(), value);
        }
        if !same_space(tf, IDENTITY) {
            attrs.insert("transform".into(), matrix(tf));
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
        // text keeps its runs; a shape keeps its description and drop markers
        let keeps_children = name == "text" || (name != "image" && node.has_children());
        if keeps_children {
            out.push('>');
            for child in node.children() {
                out.push_str(&source[child.range()]);
            }
            out.push_str(&format!("</{output_name}>"));
        } else {
            out.push_str("/>");
        }
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
            r#"<g transform="translate(20 30) rotate(25)"><g transform="scale(2 .5)"><rect width="60" height="40" rx="4"/><path clip-rule="evenodd" d="M0 0H20V20H0Z M5 5V15H15V5Z"/></g></g>"#,
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

    fn shapes<'a>(doc: &'a roxmltree::Document<'a>, id: &str) -> Vec<roxmltree::Node<'a, 'a>> {
        doc.descendants()
            .find(|n| n.attribute("id") == Some(id))
            .unwrap()
            .children()
            .filter(|n| n.is_element())
            .collect()
    }

    #[test]
    fn clipping_plain_fill_rule_is_not_a_clip_rule() {
        // SVG ignores fill-rule in a clip: a plain shape, or a plain wrapper,
        // must not start clipping even-odd because of it
        let out = compile(
            r#"<path fill-rule="evenodd" d="M0 0H20V20H0Z"/><g fill-rule="evenodd"><circle r="5"/></g>"#,
        );
        assert!(!out.contains("clip-rule"), "{out}");
        // an xsvg shape's own winding is its geometry, so it does carry over
        let out = compile(
            r#"<x:boolean op="union" fill-rule="evenodd"><rect width="10" height="10"/><rect x="5" width="10" height="10"/></x:boolean>"#,
        );
        assert!(out.contains(r#"clip-rule="evenodd""#), "{out}");
    }

    #[test]
    fn clipping_group_clip_keeps_its_own_space() {
        let src = format!(
            r##"<svg xmlns="{SVG_NS}"><clipPath id="a"><circle id="dot" r="9"/></clipPath><clipPath id="b"><circle r="30"/></clipPath><clipPath id="cut"><g clip-path="url(#a)"><circle r="5"/><g clip-path="url(#b)" transform="scale(2)"><circle r="4" transform="translate(50 0)"/></g></g></clipPath></svg>"##
        );
        let out = compile_impl(&src, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        let doc = roxmltree::Document::parse(&out).unwrap();
        let nodes = shapes(&doc, "cut");
        // same space as the group: the clip is referenced as authored
        assert_eq!(nodes[0].attribute("clip-path"), Some("url(#a)"));
        // the moved shape gets derived clips: b in the inner group's space
        // (its own transform undone), chained to a in the outer group's
        let first = clip_ref(nodes[1].attribute("clip-path").unwrap()).unwrap();
        assert_ne!(first, "a");
        let chain: Vec<_> = std::iter::successors(
            doc.descendants().find(|n| n.attribute("id") == Some(first)),
            |n| {
                let next = clip_ref(n.attribute("clip-path")?)?;
                doc.descendants().find(|m| m.attribute("id") == Some(next))
            },
        )
        .collect();
        assert_eq!(chain.len(), 2, "{out}");
        let want = [
            parse_transform("translate(-50 0)"),
            parse_transform("translate(-50 0) scale(.5)"),
        ];
        let mut got: Vec<_> = chain
            .iter()
            .map(|n| parse_transform(n.attribute("transform").unwrap()))
            .collect();
        got.sort_by(|p, q| q.as_coeffs()[0].total_cmp(&p.as_coeffs()[0]));
        for (g, w) in got.iter().zip(want) {
            assert!(same_space(*g, w), "{out}");
        }
        // a derived copy never repeats the original's ids
        assert_eq!(out.matches(r#"id="dot""#).count(), 1, "{out}");
        // a clip that references itself through a group still terminates
        let cyclic = src.replace(
            r#"<circle id="dot" r="9"/>"#,
            r##"<g clip-path="url(#cut)" transform="scale(3)"><circle r="9"/></g>"##,
        );
        assert!(compile_impl(&cyclic, "fast", false, &Mono, &NoShaper, &NoOutliner).is_ok());
    }

    #[test]
    fn clipping_expanded_use_does_not_repeat_ids() {
        let src = format!(
            r##"<svg xmlns="{SVG_NS}"><defs><g id="d"><circle id="k" r="5"/></g><clipPath id="cut"><use href="#d"/><circle id="own" r="2"/></clipPath></defs></svg>"##
        );
        let out = compile_impl(&src, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        assert_eq!(out.matches(r#"id="k""#).count(), 1, "{out}");
        assert_eq!(out.matches(r#"id="own""#).count(), 1, "{out}");
        assert!(!out.contains(CLIP_COPY), "{out}");
    }

    #[test]
    fn clipping_keeps_wrapper_classes_and_shape_children() {
        let out = compile(
            r#"<g class="h"><circle class="k" r="5"/></g><circle r="7"><title>hole</title></circle>"#,
        );
        let doc = roxmltree::Document::parse(&out).unwrap();
        let nodes = shapes(&doc, "cut");
        assert_eq!(nodes[0].attribute("class"), Some("h k"));
        let title = nodes[1].first_element_child().unwrap();
        assert_eq!(title.tag_name().name(), "title");
    }
}
