//! The plugin seam (docs/Extensibility.md): loadable geometry generators.
//!
//! The core does **not** load or run anything — it stays pure Rust with no wasm
//! runtime, exactly like the [`Measurer`](crate::Measurer) /
//! [`Resolver`](super::Resolver) seams. A host that can instantiate modules
//! implements [`GeneratorHost`]; the core asks it what an element is, builds the
//! request (resolving `in="#id"` geometry and compiling children itself), and
//! sanitizes whatever comes back.
//!
//! Dispatch is by **XML namespace URI**: a namespace bound by an `<x:import>`
//! goes to the host; anything else foreign still drops as editor metadata. The
//! core namespace can never be bound, so a plugin cannot shadow a built-in.

use super::XSVG_NS;

/// Whether an element consumes referenced geometry via `in="#id"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMode {
    /// No `in=` geometry is passed (any `in` attribute is an ordinary parameter).
    None,
    /// Passed when present and resolvable; absence is fine.
    Optional,
    /// Must resolve, else the element degrades with a marker.
    Required,
}

/// Whether an element is a leaf generator or a container whose children the core
/// compiles first (docs/Extensibility.md §4).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChildMode {
    /// A leaf: child content is ignored with a marker.
    None,
    /// Children are lowered by the core and handed over as [`ChildFragment`]s.
    Container,
}

/// What the host knows about `<ns:tag>` at this moment. The core asks *before*
/// building a request, because whether to compile children is the element's
/// property, not the document's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ElementInfo {
    /// The module is instantiated and claims this element.
    Ready {
        input: InputMode,
        children: ChildMode,
    },
    /// The namespace is imported but its module isn't instantiated yet — the host
    /// is expected to load it and recompile (docs/Extensibility.md §5).
    Pending,
    /// Module loaded, but it declares no such element.
    UnknownElement,
    /// No module bound for this namespace.
    Unbound,
}

/// One child of a container element, already lowered to the SVG subset by the core.
#[derive(Clone, Debug, PartialEq)]
pub struct ChildFragment {
    /// Compiled, subset-safe SVG. Splicing it verbatim preserves the child's
    /// `data-xsvg-pos` stamps, and with them inspector round-tripping.
    pub svg: String,
    /// `[x, y, width, height]` of the fragment's visible geometry, or `None` when
    /// nothing measurable was found (see [`fragment_bbox`]).
    pub bbox: Option<[f64; 4]>,
}

/// Everything a generator gets. Deliberately closed: no document access, no
/// callbacks, no way to resolve further references (docs/Extensibility.md §7).
#[derive(Clone, Debug, PartialEq)]
pub struct GenRequest {
    /// Non-reserved, unprefixed attributes in document order, with `var()` tokens
    /// already resolved against the theme (§4.1) — a plugin never sees a variable.
    pub attrs: Vec<(String, String)>,
    /// Geometry resolved from `in="#id"`, as an SVG path `d`.
    pub input_d: Option<String>,
    /// Compiled children, document order. Empty unless [`ChildMode::Container`].
    pub children: Vec<ChildFragment>,
    /// The active [`QualityProfile`](crate::QualityProfile) flattening tolerance.
    pub tolerance: f64,
}

/// A generator's answer: markup to splice (after sanitization), or a reason to
/// degrade. Totality means there is no third option.
#[derive(Clone, Debug, PartialEq)]
pub enum GenResult {
    Svg(String),
    Error(String),
}

/// Host-supplied registry of loaded generator modules.
///
/// Implementations must be **pure with respect to a compile**: the same request
/// must produce the same output, since incremental re-emission assumes a
/// fragment's output is a function of its subtree (docs/Incremental.md). Hosts
/// running WASM modules get this structurally; see docs/Extensibility.md §6 for
/// the fuel/instance-reset discipline that keeps it true.
pub trait GeneratorHost {
    /// What is `<ns:tag>`? Called before every dispatch; cheap by contract.
    fn element(&self, ns: &str, tag: &str) -> ElementInfo;

    /// Lower one element. Only called after [`ElementInfo::Ready`].
    fn generate(&self, ns: &str, tag: &str, req: &GenRequest) -> GenResult;
}

/// The default host: nothing is bound, so every plugin element degrades with a
/// marker. Backs every pre-existing entry point.
pub struct NoGenerators;

impl GeneratorHost for NoGenerators {
    fn element(&self, _ns: &str, _tag: &str) -> ElementInfo {
        ElementInfo::Unbound
    }
    fn generate(&self, _ns: &str, _tag: &str, _req: &GenRequest) -> GenResult {
        GenResult::Error("no generator host".to_string())
    }
}

/// One `<x:import>`: a namespace bound to a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub ns: String,
    /// Module location, resolved by the host against the document base the same
    /// way cross-file `<use>` hrefs are (§4.2).
    pub src: String,
    /// SRI-style hash the host checks before instantiating. Optional, but a host
    /// SHOULD refuse a cross-origin `src` without one.
    pub integrity: Option<String>,
}

/// The document's namespace bindings, in declaration order.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct ImportTable(Vec<Import>);

impl ImportTable {
    pub fn as_slice(&self) -> &[Import] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `ns` is bound — i.e. whether an element in it dispatches to the
    /// host rather than dropping as unknown foreign markup.
    pub fn has(&self, ns: &str) -> bool {
        self.0.iter().any(|i| i.ns == ns)
    }
}

/// Collect `<x:import>` bindings from a document.
///
/// Two rules keep this trustworthy: the **core namespace is never bindable** (a
/// plugin must not be able to shadow a built-in), and a **namespace binds once**
/// (a second, conflicting `src` would make the document's meaning depend on
/// document order in a way nothing else here does). Both rejections are silent
/// in this scan and reported at the point of use — see [`check_import`].
pub(super) fn load_imports(doc: &roxmltree::Document) -> ImportTable {
    let mut table: Vec<Import> = Vec::new();
    for n in doc
        .descendants()
        .filter(|n| n.tag_name().namespace() == Some(XSVG_NS) && n.tag_name().name() == "import")
    {
        let (Some(ns), Some(src)) = (n.attribute("ns"), n.attribute("src")) else {
            continue;
        };
        if ns == XSVG_NS || ns.is_empty() || src.is_empty() {
            continue;
        }
        if table.iter().any(|i| i.ns == ns) {
            continue; // first binding wins
        }
        table.push(Import {
            ns: ns.to_string(),
            src: src.to_string(),
            integrity: n.attribute("integrity").map(str::to_string),
        });
    }
    ImportTable(table)
}

/// Why an `<x:import>` was not honored, for the marker it compiles to. Returns
/// `None` when the import is fine.
pub(super) fn check_import(n: roxmltree::Node, table: &ImportTable) -> Option<&'static str> {
    let (Some(ns), Some(src)) = (n.attribute("ns"), n.attribute("src")) else {
        return Some("import needs both ns and src");
    };
    if ns.is_empty() || src.is_empty() {
        return Some("import needs both ns and src");
    }
    if ns == XSVG_NS {
        return Some("the core namespace cannot be bound to a module");
    }
    // honored iff this node is the binding the table kept
    let kept = table.as_slice().iter().find(|i| i.ns == ns)?;
    (kept.src != src).then_some("namespace already bound to another module")
}

/// Public pre-scan: the imports a document declares, so a host can fetch and
/// instantiate modules **before** compiling instead of discovering them through a
/// throwaway round (docs/Extensibility.md §5).
///
/// Scoped to the document given: a cross-file dependency's own `<x:import>` is
/// not visible here and its plugin elements degrade — see §5 *Limitations*.
pub fn imports_impl(input: &str) -> Vec<Import> {
    let Ok(doc) = roxmltree::Document::parse(input) else {
        return Vec::new();
    };
    load_imports(&doc).0
}

// ---- Lowering ------------------------------------------------------------

use super::{
    attr_num, check_nesting_depth, parse_transform, pos_attr, push_escaped, ref_geometry,
    serialize, shape_to_path_d, svg_path_bbox, Ctx, MAX_NESTING_DEPTH, SVG_NS,
};
use crate::kurbo::{Affine, Rect};

/// Attributes the core owns on a plugin element: they drive layering, identity,
/// and placement, so they are handled here and never reach the generator.
const RESERVED_ATTRS: [&str; 3] = ["id", "transform", "in"];

fn marker(out: &mut String, ns: &str, tag: &str, why: &str) {
    let short = ns.rsplit('/').next().unwrap_or(ns);
    out.push_str("<!-- xsvg: <");
    push_escaped(out, short, false);
    out.push(':');
    push_escaped(out, tag, false);
    out.push_str("> ");
    push_escaped(out, why, false);
    out.push_str(" -->");
}

/// Lower an element in an imported namespace (docs/Extensibility.md §3).
///
/// The core does the parts a sandboxed module must not: resolves `in="#id"`,
/// compiles children, resolves theme tokens, then sanitizes the result and
/// re-attaches the element's own identity and transform.
pub(super) fn emit_plugin(node: roxmltree::Node, ns: &str, out: &mut String, ctx: &Ctx) {
    let tag = node.tag_name().name();

    // A generator's own output is lowered once, never re-dispatched — otherwise a
    // module emitting its own element would recurse without bound.
    if ctx.in_plugin.get() {
        marker(out, ns, tag, "generators do not nest in generator output");
        return;
    }

    let (input, children) = match ctx.generators.element(ns, tag) {
        ElementInfo::Ready { input, children } => (input, children),
        ElementInfo::Pending => {
            marker(out, ns, tag, "module not loaded yet");
            return;
        }
        ElementInfo::UnknownElement => {
            marker(out, ns, tag, "not provided by the imported module");
            return;
        }
        ElementInfo::Unbound => {
            marker(out, ns, tag, "no generator bound for this namespace");
            return;
        }
    };

    // `in="#id"` geometry — the only reference a generator gets, pre-resolved.
    let mut input_d = None;
    if input != InputMode::None {
        match node.attribute("in") {
            Some(r) => match ref_geometry(node, r, ctx) {
                Ok(d) => input_d = Some(d),
                Err(e) => {
                    if input == InputMode::Required {
                        marker(out, ns, tag, e.reason());
                        return;
                    }
                }
            },
            None if input == InputMode::Required => {
                marker(out, ns, tag, "requires in=\"#id\" geometry");
                return;
            }
            None => {}
        }
    }

    // Children: lowered by the core exactly as if they stood alone, so nested
    // x: elements, themes, and references all behave normally.
    let mut kids = Vec::new();
    let has_element_children = node.children().any(|c| c.is_element());
    match children {
        ChildMode::Container => {
            for child in node.children().filter(|c| c.is_element()) {
                let mut buf = String::new();
                serialize(child, &mut buf, false, ctx);
                let bbox = fragment_bbox(&buf, ctx);
                kids.push(ChildFragment { svg: buf, bbox });
            }
        }
        ChildMode::None if has_element_children => {
            marker(out, ns, tag, "takes no children — content ignored");
        }
        ChildMode::None => {}
    }

    let attrs = node
        .attributes()
        .filter(|a| a.namespace().is_none())
        .filter(|a| !RESERVED_ATTRS.contains(&a.name()))
        .filter(|a| !a.name().starts_with("on"))
        .map(|a| (a.name().to_string(), ctx.resolve_var(a.value()).into_owned()))
        .collect();

    let req = GenRequest {
        attrs,
        input_d,
        children: kids,
        tolerance: ctx.quality.tolerance(),
    };

    let svg = match ctx.generators.generate(ns, tag, &req) {
        GenResult::Svg(s) => s,
        GenResult::Error(why) => {
            marker(out, ns, tag, &why);
            return;
        }
    };

    let Some(body) = sanitize(&svg, node, ctx) else {
        marker(out, ns, tag, "generator output is not well-formed XML");
        return;
    };

    // The wrapper carries what the core reserved, so the element stays
    // referenceable (`id`), placeable (`transform`), and clickable (sourcemap).
    out.push_str("<g");
    out.push_str(&pos_attr(node, ctx));
    for name in ["id", "transform"] {
        if let Some(v) = node.attribute(name) {
            out.push(' ');
            out.push_str(name);
            out.push_str("=\"");
            push_escaped(out, &ctx.resolve_var(v), true);
            out.push('"');
        }
    }
    out.push('>');
    out.push_str(&body);
    out.push_str("</g>");
}

/// Re-parse and re-emit generator output through the ordinary lowering path.
///
/// This is the security boundary, and it does not depend on the host: the
/// output goes through the same `serialize` that drops `<script>`/`<animate*>`,
/// strips `on*` handlers, and refuses foreign namespaces. Plugin markup is
/// therefore *xsvg*, not raw SVG — a generator may emit `<x:offset>` and let the
/// core lower it — with generator elements themselves blocked (no re-entry).
///
/// The source map is off inside: the fragment's byte ranges index the generator's
/// output string, not the document the viewer maps against, so stamping them
/// would point a click at nothing. The wrapper's own range covers it, exactly as
/// for a linked dependency (§4.2).
fn sanitize(svg: &str, node: roxmltree::Node, ctx: &Ctx) -> Option<String> {
    let wrapped = format!("<svg xmlns=\"{SVG_NS}\">{svg}</svg>");
    check_nesting_depth(&wrapped, MAX_NESTING_DEPTH).ok()?;
    let doc = roxmltree::Document::parse(&wrapped).ok()?;

    let prev_sourcemap = ctx.sourcemap.replace(false);
    let prev_in_plugin = ctx.in_plugin.replace(true);
    let mut lowered = String::new();
    for child in doc.root_element().children() {
        serialize(child, &mut lowered, false, ctx);
    }
    ctx.sourcemap.set(prev_sourcemap);
    ctx.in_plugin.set(prev_in_plugin);

    // Ids this module emits are namespaced to *this* element, so two instances of the
    // same generator cannot collide with each other either (docs/Sandbox.md §4).
    let scope = format!("x-plg-{}-", super::gen_id_seed(node, ctx));
    let (mut out, dropped) = confine(&lowered, &scope)?;

    // Say what was taken away. A generator whose output reached for the network is
    // either compromised or confused, and either way the drawing is now missing
    // something the module meant to draw — that is worth surfacing, not swallowing.
    if dropped > 0 {
        out.insert_str(
            0,
            &format!(
                "<!-- xsvg: {dropped} URL(s) in generator output pointed outside the document — dropped (docs/Sandbox.md) -->"
            ),
        );
    }
    Some(out)
}

/// The sandbox's output rules (docs/Sandbox.md §4), applied to generator markup
/// **after** the core has lowered it — the one place every emitter's output passes.
///
/// They used to live in `copy_attrs`, and that was the mistake: a generator may emit
/// xsvg, and xsvg text lowers through emitters that never call `copy_attrs`. A module
/// emitting `<x:textbox>` got a linked run's `javascript:` href, a highlight's remote
/// `url()` and a run's remote paint straight through. Enforcing the policy on the
/// finished markup makes it independent of which emitter produced it, including
/// emitters that do not exist yet.
///
/// Returns the rewritten markup and how many URLs were dropped.
fn confine(lowered: &str, scope: &str) -> Option<(String, u32)> {
    const XLINK: &str = "http://www.w3.org/1999/xlink";
    const XML: &str = "http://www.w3.org/XML/1998/namespace";
    let wrapped = format!("<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK}\">{lowered}</svg>");
    let doc = roxmltree::Document::parse(&wrapped).ok()?;
    let mut out = String::with_capacity(lowered.len());
    let mut dropped = 0u32;

    fn write(n: roxmltree::Node, out: &mut String, scope: &str, dropped: &mut u32) {
        if n.is_text() {
            push_escaped(out, n.text().unwrap_or(""), false);
            return;
        }
        if n.is_comment() {
            out.push_str("<!--");
            out.push_str(n.text().unwrap_or(""));
            out.push_str("-->");
            return;
        }
        if !n.is_element() {
            return; // processing instructions have no place in the output
        }
        let name = n.tag_name().name();
        if n.tag_name().namespace() != Some(SVG_NS) {
            return; // the serializer already drops these; never re-admit one
        }
        // `<foreignObject>` is an escape hatch into HTML parsing; a stylesheet would
        // restyle the *document*, not just this element, and can `@import`. Neither
        // has any business coming out of a geometry generator.
        if matches!(name, "foreignObject" | "style" | "script") {
            out.push_str(&format!("<!-- xsvg: <{name}> from a generator — dropped -->"));
            return;
        }
        out.push('<');
        out.push_str(name);
        for a in n.attributes() {
            let attr = match a.namespace() {
                None => a.name().to_string(),
                Some(XLINK) if a.name() == "href" => "href".to_string(),
                Some(XML) => format!("xml:{}", a.name()),
                Some(_) => continue,
            };
            if attr.starts_with("on") {
                continue; // handlers are outside the static subset for everyone
            }
            let mut value = a.value().to_string();
            if matches!(attr.as_str(), "href" | "src") {
                if !plugin_url_ok(&value) {
                    *dropped += 1;
                    continue; // drop the URL; the element stays, inert
                }
                if let Some(id) = value.trim().strip_prefix('#') {
                    value = format!("#{scope}{id}");
                }
            } else if attr == "id" {
                value = format!("{scope}{value}");
            }
            if value.contains("url(") {
                value = scope_urls(&value, scope, dropped);
            }
            out.push(' ');
            out.push_str(&attr);
            out.push_str("=\"");
            push_escaped(out, &value, true);
            out.push('"');
        }
        if n.has_children() {
            out.push('>');
            for c in n.children() {
                write(c, out, scope, dropped);
            }
            out.push_str("</");
            out.push_str(name);
            out.push('>');
        } else {
            out.push_str("/>");
        }
    }

    for c in doc.root_element().children() {
        write(c, &mut out, scope, &mut dropped);
    }
    Some((out, dropped))
}

/// Whether a URL is one generator output may name: a same-document fragment, or an
/// inline image. Everything else — a remote origin, `javascript:`, any other scheme —
/// would let untrusted output *act* instead of draw.
fn plugin_url_ok(url: &str) -> bool {
    let u = url.trim();
    u.starts_with('#') || u.to_ascii_lowercase().starts_with("data:image/")
}

/// Namespace every `url(#id)` in a value to `scope`, and neutralize any `url(…)` that
/// points out of the document. Applies to every attribute, not just `style`: `fill`,
/// `stroke` and `filter` all take a `url(…)`, so a paint server is the same egress
/// channel wearing a different attribute name.
fn scope_urls(value: &str, scope: &str, dropped: &mut u32) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find("url(") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 4..];
        let Some(close) = after.find(')') else {
            break; // malformed; leave the remainder to the copy below
        };
        let target = after[..close].trim().trim_matches(['"', '\''].as_slice());
        if let Some(id) = target.strip_prefix('#') {
            out.push_str(&format!("url(#{scope}{id})"));
        } else {
            *dropped += 1;
            out.push_str("none");
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The visible extent of a compiled fragment, for a container to lay out against.
///
/// Works on *emitted* markup, so it sees shapes rather than source elements:
/// path geometry, image/`foreignObject` boxes, and live `<text>` measured through
/// the same [`Measurer`](crate::Measurer) seam the text pipeline uses — a text
/// child reports its line box, not its glyph outlines. Transforms compose down
/// the tree; strokes inflate the box by half their width.
///
/// `None` means nothing measurable was found (an empty group, a live `<use>`, a
/// filter-only effect); the container decides how to degrade.
fn fragment_bbox(svg: &str, ctx: &Ctx) -> Option<[f64; 4]> {
    let wrapped = format!("<svg xmlns=\"{SVG_NS}\">{svg}</svg>");
    check_nesting_depth(&wrapped, MAX_NESTING_DEPTH).ok()?;
    let doc = roxmltree::Document::parse(&wrapped).ok()?;
    let mut acc: Option<Rect> = None;
    for child in doc.root_element().children() {
        accumulate_bbox(child, Affine::IDENTITY, ctx, &mut acc);
    }
    let r = acc?;
    (r.width().is_finite() && r.height().is_finite())
        .then(|| [r.x0, r.y0, r.width(), r.height()])
}

fn accumulate_bbox(n: roxmltree::Node, parent: Affine, ctx: &Ctx, acc: &mut Option<Rect>) {
    if !n.is_element() {
        return;
    }
    let aff = match n.attribute("transform") {
        Some(t) => parent * parse_transform(t),
        None => parent,
    };
    let own = match n.tag_name().name() {
        "text" => text_bbox(n, ctx),
        "image" | "foreignObject" | "svg" => box_attrs(n),
        // a live <use> resolves in the browser; <defs> paints nothing
        "use" | "defs" | "clipPath" | "mask" | "marker" => None,
        _ => shape_to_path_d(n).as_deref().and_then(svg_path_bbox),
    };
    if let Some(r) = own {
        let r = inflate_stroke(n, r);
        let r = aff.transform_rect_bbox(r);
        *acc = Some(match *acc {
            Some(a) => a.union(r),
            None => r,
        });
    }
    if n.tag_name().name() != "defs" {
        for c in n.children() {
            accumulate_bbox(c, aff, ctx, acc);
        }
    }
}

fn box_attrs(n: roxmltree::Node) -> Option<Rect> {
    let (w, h) = (attr_num(n, "width", 0.0), attr_num(n, "height", 0.0));
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (x, y) = (attr_num(n, "x", 0.0), attr_num(n, "y", 0.0));
    Some(Rect::new(x, y, x + w, y + h))
}

/// A `<text>` element's box: the union of its **positioned runs**.
///
/// Multi-line output puts each line on its own `<tspan x y>`, so measuring the
/// element's whole string as one run would report a box as wide as all the lines
/// concatenated and place it wherever `<text>` happened to sit. Each positioned
/// tspan is measured separately; an element with none is one run of its own text.
fn text_bbox(n: roxmltree::Node, ctx: &Ctx) -> Option<Rect> {
    let positioned: Vec<roxmltree::Node> = n
        .descendants()
        .filter(|d| {
            d.is_element()
                && d.tag_name().name() == "tspan"
                && (d.has_attribute("x") || d.has_attribute("y"))
        })
        .collect();
    if positioned.is_empty() {
        return run_bbox(n, ctx);
    }
    positioned
        .into_iter()
        .filter_map(|s| run_bbox(s, ctx))
        .reduce(|a, b| a.union(b))
}

/// One positioned run: measured advance horizontally, font ascent and descent
/// vertically, shifted for `text-anchor`.
fn run_bbox(n: roxmltree::Node, ctx: &Ctx) -> Option<Rect> {
    // direct text children only — `Node::text()` also answers for an *element*,
    // returning its first child's text, which would count runs twice
    let text: String = n
        .children()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect();
    if text.trim().is_empty() {
        return None;
    }
    let mut style = crate::TextStyle {
        size: inherited(n, "font-size").and_then(super::parse_num).unwrap_or(16.0),
        ..Default::default()
    };
    if let Some(f) = inherited(n, "font-family") {
        style.family = f.to_string();
    }
    if let Some(w) = inherited(n, "font-weight") {
        style.weight = w.to_string();
    }
    if let Some(s) = inherited(n, "font-style") {
        style.style = s.to_string();
    }
    if style.size <= 0.0 || !style.size.is_finite() {
        return None;
    }
    let w = ctx.m.measure(&text, &style, style.size);
    if !w.is_finite() || w < 0.0 {
        return None;
    }
    let fm = ctx.m.font_metrics(&style, style.size);
    let x = inherited(n, "x").and_then(super::parse_num).unwrap_or(0.0);
    let y = inherited(n, "y").and_then(super::parse_num).unwrap_or(0.0);
    let x0 = match inherited(n, "text-anchor") {
        Some("middle") => x - w / 2.0,
        Some("end") => x - w,
        _ => x,
    };
    Some(Rect::new(x0, y - fm.ascent, x0 + w, y + fm.descent))
}

/// An inheritable presentation attribute, looked up on the element then its
/// ancestors — emitters routinely hoist `font-family` onto a wrapping `<g>`.
fn inherited<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.ancestors().find_map(|a| a.attribute(name))
}

/// Widen a box by half the stroke it is painted with — a stroked shape's visible
/// extent is not its geometric one, and a container placing a frame around a
/// child would otherwise clip it.
fn inflate_stroke(n: roxmltree::Node, r: Rect) -> Rect {
    match inherited(n, "stroke") {
        None | Some("none") | Some("transparent") => r,
        Some(_) => {
            let w = inherited(n, "stroke-width")
                .and_then(super::parse_num)
                .unwrap_or(1.0);
            if w.is_finite() && w > 0.0 {
                r.inflate(w / 2.0, w / 2.0)
            } else {
                r
            }
        }
    }
}

