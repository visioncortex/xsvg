//! Cross-file `<use href="file.svg">` linking (§4): the dependency is resolved by
//! the platform's [`Resolver`], compiled with its own id scope, and baked into the
//! referrer as a viewport or a placed element.

use super::*;

/// `true` for a `<use href>` that points at another file — anything that isn't a
/// bare same-document `#fragment`. That's the trigger to bake a cross-file link (§4).
pub(super) fn is_external_href(href: &str) -> bool {
    !href.is_empty() && !href.starts_with('#')
}

/// A dependency box's intrinsic size: its `viewBox` extent, else its `width`/`height`
/// (units stripped), else a 100×100 fallback.
pub(super) fn intrinsic_size(root: roxmltree::Node) -> (f64, f64) {
    if let Some(vb) = root.attribute("viewBox") {
        let p: Vec<f64> = vb
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(parse_num)
            .collect();
        if p.len() == 4 && p[2] > 0.0 && p[3] > 0.0 {
            return (p[2], p[3]);
        }
    }
    let dim = |a: &str| root.attribute(a).and_then(parse_num).filter(|v| *v > 0.0);
    (
        dim("width").unwrap_or(100.0),
        dim("height").unwrap_or(100.0),
    )
}

/// The intrinsic extent of a by-id `<use>` target when it *declares* one — a nested
/// `<svg>`'s viewBox, or any element's explicit `width`/`height`. `None` for a plain
/// shape/group with no declared size (the caller then measures its geometry).
pub(super) fn target_extent(node: roxmltree::Node) -> Option<(f64, f64)> {
    if let Some(vb) = node.attribute("viewBox") {
        let p: Vec<f64> = vb
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter_map(parse_num)
            .collect();
        if p.len() == 4 && p[2] > 0.0 && p[3] > 0.0 {
            return Some((p[2], p[3]));
        }
    }
    let dim = |a: &str| node.attribute(a).and_then(parse_num).filter(|v| *v > 0.0);
    match (dim("width"), dim("height")) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    }
}

/// Bake a cross-file `<use>` link (§4): resolve the referenced file, compile it, and
/// stamp its output into `out`. Whole-file → a viewport `<g>` (§5.3, the fit folded
/// into its transform); `#id` → that compiled element, placed. Degrades with a marker on any failure.
pub(super) fn emit_link(node: roxmltree::Node, href: &str, out: &mut String, ctx: &Ctx) {
    if ctx.files.borrow().len() > MAX_LINK_DEPTH {
        out.push_str("<!-- xsvg: <use> link nesting too deep -->");
        return;
    }
    let (file, frag) = match href.split_once('#') {
        Some((f, g)) => (f, (!g.is_empty()).then_some(g)),
        None => (href, None),
    };
    let base = ctx.base.borrow().clone();
    let Some((key, source)) = ctx.resolver.resolve(&base, file) else {
        out.push_str("<!-- xsvg: <use> external target not resolved -->");
        return;
    };
    if ctx.files.borrow().iter().any(|f| f == &key) {
        out.push_str("<!-- xsvg: <use> cyclic link skipped -->");
        return;
    }

    // Each file owns its tokens, imports and reference memo. Only the link
    // ancestry and resolution budget cross the boundary; source identities and
    // caret positions belong to the containing <use>, never to its baked nodes.
    let compiled = match session::Parsed::new(&source) {
        Ok(parsed) => {
            let seams = Backend {
                m: ctx.m,
                shaper: ctx.shaper,
                outliner: ctx.outliner,
                resolver: ctx.resolver,
                keys: &NoKeys,
                generators: ctx.generators,
            };
            let child = Ctx::new(&seams, ctx.quality, false, &key, &parsed);
            let mut files = ctx.files.borrow().clone();
            files.push(key);
            child.files.replace(files);
            child.fuel.set(ctx.fuel.get());
            child.cuts.set(ctx.cuts.get());
            child.force_outline.set(ctx.force_outline.get());
            child.clip_geometry.set(ctx.clip_geometry.get());
            child.in_plugin.set(ctx.in_plugin.get());
            let mut s = String::new();
            serialize(parsed.doc.root_element(), &mut s, true, &child);
            ctx.fuel.set(child.fuel.get());
            ctx.cuts.set(child.cuts.get());
            if let Some(miss) = child.outline_miss.into_inner() {
                ctx.record_miss(miss);
            }
            Some(s)
        }
        Err(_) => None,
    };

    let Some(compiled) = compiled else {
        out.push_str("<!-- xsvg: <use> dependency parse error -->");
        return;
    };
    let Ok(dep) = roxmltree::Document::parse(&compiled) else {
        out.push_str("<!-- xsvg: <use> dependency re-parse error -->");
        return;
    };
    let root = dep.root_element();
    let (iw, ih) = intrinsic_size(root);
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let w = node.attribute("width").and_then(parse_num);
    let h = node.attribute("height").and_then(parse_num);
    let pos = pos_attr(node, ctx);
    match frag {
        None => {
            // whole file → a viewport (§5.3): the dependency's root children under
            // one clipped <g> that fits its viewBox to the frame — the referrer's
            // preserveAspectRatio deciding how (the default letterboxes; `none`
            // stretches, what an editor's free resize of a linked drawing wants) —
            // and carries the <use>'s own transform, opacity, filter and paint.
            let inner: String = root
                .children()
                .map(|c| &compiled[c.range()])
                .collect::<Vec<_>>()
                .concat();
            let vp = Viewport {
                x,
                y,
                w: w.unwrap_or(iw),
                h: h.unwrap_or(ih),
                view_box: view_box_of(root),
                par: node
                    .attribute("preserveAspectRatio")
                    .and_then(Par::parse)
                    .unwrap_or(Par::DEFAULT),
                clip: true,
            };
            let close = open_viewport(out, &vp, node, &["href", "x", "y", "width", "height"], ctx);
            // The root's inherited paint and font still apply after its viewport
            // has been fitted. Dropping the <svg> must not drop its presentation.
            let mut attrs = String::new();
            for attr in root.attributes() {
                if [
                    "id",
                    "x",
                    "y",
                    "width",
                    "height",
                    "viewBox",
                    "preserveAspectRatio",
                    "version",
                ]
                .contains(&attr.name())
                {
                    continue;
                }
                let prefix = match attr.namespace() {
                    None => "",
                    Some(XML_NS) => "xml:",
                    _ => continue,
                };
                attrs.push_str(&format!(" {prefix}{}=\"", attr.name()));
                push_escaped(&mut attrs, attr.value(), true);
                attrs.push('"');
            }
            if !attrs.is_empty() {
                out.push_str(&format!("<g{attrs}>"));
            }
            out.push_str(&inner);
            if !attrs.is_empty() {
                out.push_str("</g>");
            }
            out.push_str(close);
        }
        Some(id) => {
            let Some(target) = dep.descendants().find(|n| n.attribute("id") == Some(id)) else {
                out.push_str("<!-- xsvg: <use> #id not found in dependency -->");
                return;
            };
            let el = &compiled[target.range()];
            // Size against the *target's own* extent so `logo.xsvg#icon` at width=72 makes
            // the icon 72 — not 72 of the whole file. Prefer a declared extent (a nested
            // <svg>'s viewBox, or explicit width+height), else the drawn geometry's bbox,
            // else the file. When width/height is given, also re-anchor the element's
            // top-left to (x,y) before scaling; with neither, keep the plain translate.
            let (ox, oy, bw, bh) = match target_extent(target) {
                Some((w, h)) => (0.0, 0.0, w, h),
                None => match plain_subtree_bbox(target, crate::kurbo::Affine::IDENTITY) {
                    Some(r) => (r.x0, r.y0, r.width(), r.height()),
                    None => (0.0, 0.0, iw, ih),
                },
            };
            let scale = w
                .map(|w| w / bw)
                .or_else(|| h.map(|h| h / bh))
                .filter(|s| s.is_finite() && *s > 0.0);
            let tf = match scale {
                Some(s) => format!(
                    "translate({},{}) scale({})",
                    fmt(x - s * ox),
                    fmt(y - s * oy),
                    fmt(s)
                ),
                None => format!("translate({},{})", fmt(x), fmt(y)),
            };
            out.push_str(&format!("<g{pos} transform=\"{tf}\">{el}</g>"));
        }
    }
}
