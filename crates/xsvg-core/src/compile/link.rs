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
/// stamp its output into `out`. Whole-file → a nested `<svg>` viewport (SVG's own fit);
/// `#id` → that compiled element, placed. Degrades with a marker on any failure.
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

    // Compile the dependency (recursively — its own <use>/x: elements lower too), with
    // base = its key and the key pushed so a link back to it is caught as a cycle.
    ctx.files.borrow_mut().push(key.clone());
    let prev_base = ctx.base.replace(key);
    // The dependency's node ranges index its *own* source, not the entry document, so
    // no source map inside it — the baked block resolves up to the `<use>` (§4.2).
    let prev_sm = ctx.sourcemap.replace(false);
    // Compiled-output refs (`#id`) never cross a file boundary, but the memo and cycle
    // stack are keyed by bare id — so give the dependency its own scope. Otherwise a
    // `#mark` here and a `#mark` in the referrer collide: the memo hands back the wrong
    // file's geometry, and the shared stack reports a false cycle. (`fuel`/`cuts` stay
    // shared on purpose — they are a whole-compile work budget.)
    let prev_resolved = ctx.resolved.replace(std::collections::HashMap::new());
    let prev_resolving = ctx.resolving.replace(Vec::new());
    let compiled = match session::parse_document(&source) {
        Ok(d) => {
            let mut s = String::new();
            serialize(d.root_element(), &mut s, true, ctx);
            Some(s)
        }
        Err(_) => None,
    };
    ctx.resolving.replace(prev_resolving);
    ctx.resolved.replace(prev_resolved);
    ctx.sourcemap.set(prev_sm);
    ctx.base.replace(prev_base);
    ctx.files.borrow_mut().pop();

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
            // whole file → nested <svg> viewport; the browser's own viewBox fit scales it.
            let inner: String = root
                .children()
                .map(|c| &compiled[c.range()])
                .collect::<Vec<_>>()
                .concat();
            // Read back out of the compiled dependency, so the parse has already
            // undone the escaping its own emission applied — re-escape on the way in.
            let vb = root
                .attribute("viewBox")
                .map(|v| format!(" viewBox=\"{}\"", esc(v)))
                .unwrap_or_default();
            // The referrer decides how its frame fits the dependency's viewBox:
            // `preserveAspectRatio` on the <use> is forwarded to the viewport it
            // becomes (default `xMidYMid meet` letterboxes; `none` stretches —
            // what an editor's free resize of a linked drawing wants). Forwarded only
            // when it is a value SVG defines: the attribute has a closed grammar, so
            // anything else is either a typo or an attempt to smuggle markup.
            let par = node
                .attribute("preserveAspectRatio")
                .filter(|v| valid_preserve_aspect_ratio(v))
                .map(|v| format!(" preserveAspectRatio=\"{}\"", v.trim()))
                .unwrap_or_default();
            out.push_str(&format!(
                "<svg{pos} x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{vb}{par}>{inner}</svg>",
                fmt(x),
                fmt(y),
                fmt(w.unwrap_or(iw)),
                fmt(h.unwrap_or(ih))
            ));
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
