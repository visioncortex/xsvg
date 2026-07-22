//! The xsvg compiler core: parse xsvg/SVG input, run the lowering passes, and emit a
//! plain-SVG-subset string. Platform-agnostic — no wasm/JS or native font code lives
//! here. Everything platform-specific is a trait the core calls (`Measurer`, `Shaper`,
//! `GlyphOutliner`, all defined in `xsvg-core`); the `xsvg-wasm` crate backs them with
//! browser callbacks and `xsvg-cli` backs them with native font libraries. Both are thin
//! adapters over the `compile_impl` / `compile_fragment_impl` entry points here.
pub(crate) use crate::{
    boolean_svg_paths, filter_primitives, layout_area, layout_area_measured, layout_flow,
    layout_region, layout_text_area_runs, line_advance, measure_runs, offset_svg_paths,
    parse_filter_functions, run_offset, svg_path_bbox, warp_svg_path, warp_text_on_path, Align,
    Anchor, AreaLayout, AreaSpec, BendField, BoolOp, BoolOperand, Chain, DisplayAlign,
    EnvelopePreset, Field, Fit, FreeDistort, GlyphOutliner, Homography, LineIncrement, Measurer,
    PathEffect, PathFrame, PlacedLine, QualityProfile, RegionSpec, RoughenField, Shaper, Taper,
    TextAlign, TextAreaSpec, TextOverflow, TextStyle, VAlign, WarpAxis,
};

const XSVG_NS: &str = "https://xsvg.visioncortex.org";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Maximum element nesting depth accepted. `roxmltree`'s parser recurses per level,
/// so pathologically deep input would overflow the stack (a hard abort, worse on
/// wasm's smaller stack). Real documents nest a few dozen deep; this is a generous
/// ceiling that still leaves a wide stack margin.
const MAX_NESTING_DEPTH: usize = 512;

/// Depth cap for **reference chains** (`ref_geometry` recursion). Much lower
/// than [`MAX_NESTING_DEPTH`]: each chain link recurses through a full emitter
/// (frames of kilobytes, not the parser's small ones), so 512 links would
/// overflow a 1–2 MB stack. Real derivation chains are a handful of links; 32
/// is far beyond that, and a deeper chain degrades with a marker (§4 totality)
/// instead of trapping.
const MAX_REF_DEPTH: usize = 32;

/// Total reference resolutions allowed per compile call. The memo makes legit
/// documents consume roughly one unit per distinct id'd target; only
/// cycle-poisoned fan-out (unmemoizable by design) burns fuel combinatorially,
/// and this is the bound that stops it.
const REF_FUEL: u32 = 65_536;

/// Depth cap for cross-file `<use>` links — how deep the dependency DAG may nest
/// before a link degrades with a marker (§4 totality; guards against a very deep
/// or resolver-misbehaving chain, independent of the same-document [`MAX_REF_DEPTH`]).
const MAX_LINK_DEPTH: usize = 16;

/// Platform seam for cross-file `<use href="file.svg">` linking (§4). Given the
/// referrer's canonical key (`base`) and the raw `href`, the host resolves the path
/// relative to `base`, enforces its own security model (same-origin `fetch` in the
/// browser, disk reads natively), and returns the dependency's **canonical key** plus
/// its **source text** — or `None` to degrade. Keys only need to be stable and
/// comparable (the cycle guard compares them); they are typically the resolved path/URL.
pub trait Resolver {
    fn resolve(&self, base: &str, href: &str) -> Option<(String, String)>;
}

/// A resolver that links nothing — every external `<use>` degrades with a marker.
/// Backs the non-linking entry point ([`compile_impl`]) and callers without deps.
pub struct NoResolver;
impl Resolver for NoResolver {
    fn resolve(&self, _base: &str, _href: &str) -> Option<(String, String)> {
        None
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
    /// Whether to emit `data-xsvg-pos` source ranges. Toggled **off** while a linked
    /// dependency is compiled: its element ranges index the *dependency* file, not the
    /// entry document the viewer maps against, so baked content carries no range and a
    /// click resolves up to the wrapper — i.e. to the `<use>` in the entry source (§4.2).
    sourcemap: std::cell::Cell<bool>,
    /// ids currently being resolved as compiled-output references — the cycle
    /// guard for `ref_geometry` (a target that is already on this stack degrades
    /// with a marker instead of recursing forever). Its length is also the
    /// reference-chain depth, capped at [`MAX_REF_DEPTH`] (§4 totality: sibling
    /// chains must not overflow the stack either).
    resolving: std::cell::RefCell<Vec<String>>,
    /// per-compile memo of **context-free** `ref_geometry` results (id → joined
    /// `d`). Only resolutions untouched by a cycle/depth cut are cached — a cut
    /// result depends on what was on the stack, so it must not leak into other
    /// contexts. Collapses diamond-shaped reference fan-out from exponential to
    /// linear; dies with this compile call, so it can never go stale.
    resolved: std::cell::RefCell<std::collections::HashMap<String, Option<String>>>,
    /// counts cycle/depth cuts; `ref_geometry` snapshots it around a resolution
    /// to decide whether the result was context-free (memoizable).
    cuts: std::cell::Cell<u64>,
    /// remaining reference resolutions for this compile call ([`REF_FUEL`]).
    /// The depth cap bounds how deep a resolution tree goes; this bounds how
    /// WIDE — cycle-poisoned fan-out defeats the memo (cut results are never
    /// cached), so without fuel a document could branch 2^depth resolutions.
    fuel: std::cell::Cell<u32>,
    /// forces `outline="true"` semantics while scratch-serializing a reference
    /// target, so referenced text contributes glyph geometry instead of nothing.
    force_outline: std::cell::Cell<bool>,
    /// Cross-file `<use>` linking seam (§4).
    resolver: &'a dyn Resolver,
    /// canonical keys of the files currently being linked — the cross-file cycle
    /// guard (a link back to an ancestor degrades). Its length is the link depth,
    /// capped at [`MAX_LINK_DEPTH`].
    files: std::cell::RefCell<Vec<String>>,
    /// canonical key of the file being serialized, so relative hrefs resolve against it.
    base: std::cell::RefCell<String>,
}

/// `" data-xsvg-pos=\"START-END\""` (byte offsets of `node` in the original xsvg
/// source) when the source map is enabled, else empty. Attached to each emitted
/// top-level element so a viewer can project a rendered element back to its source.
fn pos_attr(node: roxmltree::Node, ctx: &Ctx) -> String {
    if !ctx.sourcemap.get() {
        return String::new();
    }
    let r = node.range();
    format!(" data-xsvg-pos=\"{}-{}\"", r.start, r.end)
}
/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
/// Cross-file `<use>` links degrade (no resolver) — use [`compile_linked_impl`] to link.
pub fn compile_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_linked_impl(input, quality, sourcemap, m, shaper, outliner, &NoResolver, "")
}

/// Compile entry with cross-file `<use>` linking: `resolver` fetches dependencies and
/// `base` is the entry document's canonical key (relative hrefs resolve against it, and
/// a dependency that links back to it is a cycle). See [`Resolver`].
#[allow(clippy::too_many_arguments)]
pub fn compile_linked_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
) -> Result<String, String> {
    let q = crate::QualityProfile::parse(quality);
    check_nesting_depth(input, MAX_NESTING_DEPTH)?;
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;
    load_theme(&doc); // §4.1 color/type tokens, resolved during serialization

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
            sourcemap: std::cell::Cell::new(sourcemap),
            resolving: std::cell::RefCell::new(Vec::new()),
            resolved: std::cell::RefCell::new(std::collections::HashMap::new()),
            cuts: std::cell::Cell::new(0),
            fuel: std::cell::Cell::new(REF_FUEL),
            force_outline: std::cell::Cell::new(false),
            resolver,
            files: std::cell::RefCell::new(vec![base.to_string()]),
            base: std::cell::RefCell::new(base.to_string()),
        },
    );
    Ok(out)
}

/// The direct element child of the document root whose source byte range contains
/// `offset` — the **fragment unit** of incremental compilation (docs/Incremental.md).
fn top_level_at<'a>(
    doc: &'a roxmltree::Document<'a>,
    offset: usize,
) -> Option<roxmltree::Node<'a, 'a>> {
    doc.root_element()
        .children()
        .filter(|c| c.is_element())
        .find(|c| {
            let r = c.range();
            r.start <= offset && offset < r.end
        })
}

/// Incremental compilation (docs/Incremental.md): re-emit only the top-level
/// element containing byte `offset`. Emission is a pure function of the subtree
/// plus anything it references, so the result is **byte-identical to the span the
/// full compile would produce** for that element (enforced by test) — the caller
/// can splice it over the previous output surgically. Errors when `offset` doesn't
/// fall inside a top-level element.
pub fn compile_fragment_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_fragment_linked_impl(
        input, quality, sourcemap, offset, m, shaper, outliner, &NoResolver, "",
    )
}

/// [`compile_fragment_impl`] with cross-file linking, so an incremental re-emit of a
/// `<use href="file.svg">` stays byte-identical to its span in the full linked compile
/// (the fragment invariant, docs/Incremental.md). Same `resolver`/`base` as
/// [`compile_linked_impl`].
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_linked_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
) -> Result<String, String> {
    let q = crate::QualityProfile::parse(quality);
    check_nesting_depth(input, MAX_NESTING_DEPTH)?;
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;
    let node = top_level_at(&doc, offset)
        .ok_or_else(|| format!("no top-level element at byte offset {offset}"))?;
    let mut out = String::new();
    serialize(
        node,
        &mut out,
        false,
        &Ctx {
            m,
            shaper,
            outliner,
            quality: q,
            sourcemap: std::cell::Cell::new(sourcemap),
            resolving: std::cell::RefCell::new(Vec::new()),
            resolved: std::cell::RefCell::new(std::collections::HashMap::new()),
            cuts: std::cell::Cell::new(0),
            fuel: std::cell::Cell::new(REF_FUEL),
            force_outline: std::cell::Cell::new(false),
            resolver,
            files: std::cell::RefCell::new(vec![base.to_string()]),
            base: std::cell::RefCell::new(base.to_string()),
        },
    );
    Ok(out)
}

/// Source byte range of the fragment unit containing `offset` (for the caller's
/// identity bookkeeping — ranges shift by the edit delta between compiles).
pub fn fragment_range_impl(input: &str, offset: usize) -> Option<(usize, usize)> {
    let doc = roxmltree::Document::parse(input).ok()?;
    top_level_at(&doc, offset).map(|n| {
        let r = n.range();
        (r.start, r.end)
    })
}

/// Source byte ranges of every *other* top-level element that must be re-emitted
/// when the fragment at `offset` changes — the **transitive closure** over
/// compile-time-baked `in="#id"` references (textbox / textpath / warp bend).
/// Chains are real since `x:` elements can be `in`-targets themselves (their
/// compiled output is the referenced geometry): editing `#a` re-emits
/// `<x:warp id="w" in="#a">`, whose changed output re-emits `<x:textpath
/// in="#w">`, and so on to a fixpoint. Live references (`href`, `url(#…)`
/// paints) re-resolve in the DOM and are deliberately NOT reported. Results are
/// in document order.
/// The references this node **bakes** at compile time: an `in="#id"`
/// attribute (textbox / textpath / warp bend), or the `href` of a `<use>` that
/// is a direct child of `<x:boolean>` (an operand by reference — §7.4). A
/// passthrough `<use>` anywhere else is a live reference the browser resolves,
/// so it is deliberately not reported.
fn baked_refs<'a>(n: roxmltree::Node<'a, 'a>) -> Vec<&'a str> {
    let mut out = Vec::new();
    if let Some(r) = n.attribute("in") {
        out.push(r);
    }
    // <x:connector from to> bakes both endpoint boxes into the routed path (§7.6)
    if n.tag_name().namespace() == Some(XSVG_NS) && n.tag_name().name() == "connector" {
        out.extend(n.attribute("from"));
        out.extend(n.attribute("to"));
    }
    if n.tag_name().name() == "use"
        && n.tag_name().namespace() != Some(XSVG_NS)
        && n.parent().is_some_and(|p| {
            p.tag_name().namespace() == Some(XSVG_NS) && p.tag_name().name() == "boolean"
        })
    {
        out.extend(
            n.attribute("href")
                .or_else(|| n.attribute((XLINK_NS, "href"))),
        );
    }
    // a fill referencing a <meshgradient> is compiled, not live (§8.2)
    if let Some(id) = n
        .attribute("fill")
        .and_then(|f| f.strip_prefix("url(#"))
        .and_then(|f| f.strip_suffix(')'))
    {
        if resolve_ref(n, id).is_some_and(|t| t.tag_name().name() == "meshgradient") {
            out.push(id);
        }
    }
    out
}

pub fn dependents_impl(input: &str, offset: usize) -> Vec<(usize, usize)> {
    let Ok(doc) = roxmltree::Document::parse(input) else {
        return Vec::new();
    };
    let Some(target) = top_level_at(&doc, offset) else {
        return Vec::new();
    };
    // ids whose (compiled) geometry has changed; grows as dependents join
    let mut live: Vec<&str> = target
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    if live.is_empty() {
        return Vec::new();
    }
    let tops: Vec<roxmltree::Node> = doc
        .root_element()
        .children()
        .filter(|c| c.is_element() && c.range() != target.range())
        .collect();
    let mut included = vec![false; tops.len()];
    loop {
        let mut changed = false;
        for (i, tl) in tops.iter().enumerate() {
            if included[i] {
                continue;
            }
            let refs_live = tl.descendants().any(|n| {
                baked_refs(n)
                    .iter()
                    .any(|r| live.contains(&r.strip_prefix('#').unwrap_or(r)))
            });
            if refs_live {
                included[i] = true;
                changed = true;
                // its output changed, so ids it defines go live too
                for id in tl.descendants().filter_map(|n| n.attribute("id")) {
                    if !live.contains(&id) {
                        live.push(id);
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut out: Vec<(usize, usize)> = tops
        .iter()
        .zip(&included)
        .filter(|(_, inc)| **inc)
        .map(|(tl, _)| {
            let r = tl.range();
            (r.start, r.end)
        })
        .collect();
    out.sort_unstable();
    out
}

/// Recursively emit a node as SVG.
/// Lower a `filter` attribute holding CSS filter functions (§8) to a real
/// `<filter>` definition: the def is emitted immediately before the element
/// (self-contained per fragment — §Incremental), and the returned markup is
/// the substitute ` filter="url(#…)"` attribute. `None` leaves the attribute
/// as authored: absent, `none`, a `url(#…)` reference, or a list the parser
/// declines (unknown function, invalid argument) — browsers still honor those
/// live, mirroring CSS's whole-declaration-invalid rule. sRGB interpolation is
/// pinned so lowered output matches live browser rendering; the region gets
/// the ±10% margin so strokes outside the fill bbox survive.
fn lower_filter_attr(node: roxmltree::Node, out: &mut String) -> Option<String> {
    let fns = parse_filter_functions(node.attribute("filter")?)?;
    let id = format!("x-flt-{}", node.range().start);
    // pointwise functions need only the stroke margin. Blur/drop-shadow spill:
    // on a plain shape the spill is computed EXACTLY (3σ per blur + the shadow
    // offset + half the stroke width) as a userSpaceOnUse region; on
    // unmeasurable content (groups) the region falls back to ±50% of the bbox
    // — the one bound a bbox-relative region can express.
    let bleeds = fns.iter().any(|f| f.bleeds());
    let region = if !bleeds {
        " x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"120%\"".to_string()
    } else if let Some(bb) = shape_to_path_d(node).and_then(|d| svg_path_bbox(&d)) {
        let mut m = attr_num(node, "stroke-width", 1.0) / 2.0;
        for f in &fns {
            m += match f {
                crate::AdjustFn::Blur(r) => 3.0 * r,
                crate::AdjustFn::DropShadow { dx, dy, r, .. } => 3.0 * r + dx.abs().max(dy.abs()),
                _ => 0.0,
            };
        }
        format!(
            " filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            fmt(bb.x0 - m),
            fmt(bb.y0 - m),
            fmt(bb.width() + 2.0 * m),
            fmt(bb.height() + 2.0 * m)
        )
    } else {
        " x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"".to_string()
    };
    out.push_str(&format!(
        "<filter id=\"{id}\" color-interpolation-filters=\"sRGB\"{region}>{}</filter>",
        filter_primitives(&fns)
    ));
    Some(format!(" filter=\"url(#{id})\""))
}

/// The compile-time z-band of a layer (§5.1): `x:layer="background"` sinks
/// behind everything (−1), `"foreground"` floats in front (+1), any other
/// value is the content band (0). `None` when the element carries no
/// `x:layer` attribute at all.
fn layer_band(node: roxmltree::Node) -> Option<i32> {
    node.attribute((XSVG_NS, "layer")).map(|v| match v {
        "background" => -1,
        "foreground" => 1,
        _ => 0,
    })
}

/// The layer visibility toggle: `x:hidden` (any value but `false`) compiles
/// the element and its subtree to nothing — hide a layer without deleting it.
fn is_hidden(node: roxmltree::Node) -> bool {
    matches!(node.attribute((XSVG_NS, "hidden")), Some(v) if v != "false")
}

/// Artboard metadata (§5.2): a `<g x:artboard="Label">` is a named frame — a
/// slide — that the viewer/preview can zoom to and page through. It compiles to
/// a plain `<g>` (renders normally in any viewer) carrying
/// `data-xsvg-artboard="Label"`, plus `data-xsvg-frame="x y w h"` when an
/// explicit `x:frame` is given (else the tools use the group's bbox). Returns
/// the attribute string to append, or empty when the element isn't an artboard.
fn artboard_attr(node: roxmltree::Node) -> String {
    let Some(label) = node.attribute((XSVG_NS, "artboard")) else {
        return String::new();
    };
    let mut s = String::from(" data-xsvg-artboard=\"");
    push_escaped(&mut s, label, true);
    s.push('"');
    if let Some(frame) = node.attribute((XSVG_NS, "frame")) {
        let nums: Vec<f64> = frame
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(parse_num)
            .collect();
        if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
            s.push_str(&format!(
                " data-xsvg-frame=\"{} {} {} {}\"",
                fmt(nums[0]),
                fmt(nums[1]),
                fmt(nums[2]),
                fmt(nums[3])
            ));
        }
    }
    s
}

fn serialize(node: roxmltree::Node, out: &mut String, is_root: bool, ctx: &Ctx) {
    if !node.is_element() {
        if node.is_text() {
            if let Some(t) = node.text() {
                push_escaped(out, t, false);
            }
        }
        return; // comments, PIs, etc. are dropped
    }

    // x:hidden (§5.1) — the layer eyeball toggle; the subtree compiles to
    // nothing (never the root).
    if !is_root && is_hidden(node) {
        return;
    }

    // xsvg extension elements.
    if node.tag_name().namespace() == Some(XSVG_NS) {
        match node.tag_name().name() {
            "textbox" => emit_textbox(node, out, ctx),
            "textpath" => emit_textpath(node, out, ctx),
            "warp" => emit_warp(node, out, ctx),
            "boolean" => emit_boolean(node, out, ctx),
            "mesh" => emit_mesh(node, out, ctx),
            "connector" => emit_connector(node, out, ctx),
            "offset" => emit_offset(node, out, ctx),
            "list" => emit_list(node, out, ctx),
            "table" => emit_table(node, out, ctx),
            "pie" => emit_pie(node, out, ctx),
            "plot" => emit_plot(node, out, ctx),
            "theme" => {} // §4.1 definitions — loaded up front, emit nothing
            other => out.push_str(&format!("<!-- xsvg: <x:{other}> not yet lowered -->")),
        }
        return;
    }

    // Foreign-namespace elements (editor metadata like sodipodi:/inkscape:) can't
    // be re-emitted faithfully — we declare no xmlns for them — so they drop with
    // a marker rather than being silently reparented into the SVG namespace.
    if let Some(ns) = node.tag_name().namespace() {
        if ns != SVG_NS {
            out.push_str(&format!(
                "<!-- xsvg: foreign-namespace <{}> dropped -->",
                node.tag_name().name()
            ));
            return;
        }
    }

    // Cross-file link (§4): a <use> whose href points at another file (not a same-
    // document #fragment) is baked here — the dependency is compiled and stamped in.
    // A same-document <use href="#id"> stays a live reference (passthrough below).
    if node.tag_name().name() == "use" {
        if let Some(href) = node
            .attribute("href")
            .or_else(|| node.attribute((XLINK_NS, "href")))
        {
            if is_external_href(href) {
                emit_link(node, href, out, ctx);
                return;
            }
        }
    }

    // <xsvg> root is just an alias for <svg>.
    let name = match node.tag_name().name() {
        "xsvg" => "svg",
        other => other,
    };

    // The static-subset deny list (§9, Plan R6): script and animation cannot
    // exist in the output contract, so they drop with a marker instead of
    // passing through.
    if matches!(
        name,
        "script" | "animate" | "animateMotion" | "animateTransform" | "set" | "discard"
    ) {
        out.push_str(&format!(
            "<!-- xsvg: <{name}> outside the static subset — dropped -->"
        ));
        return;
    }

    if name == "text" && node.attribute("inline-size").is_some() {
        emit_inline_size_text(node, out, ctx);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, ctx);
        return;
    }

    // A fill referencing an SVG 2 <meshgradient> (the Inkscape dialect no
    // browser renders) compiles to the mesh pipeline, clipped by the shape.
    if let Some(mg) = mesh_fill_target(node) {
        if emit_meshgradient_fill(node, mg, out, ctx) {
            return;
        }
    }

    // Sharp-cornered <rect> → <path>. Rounded rects pass through unchanged.
    if name == "rect" && node.attribute("rx").is_none() && node.attribute("ry").is_none() {
        emit_rect_as_path(node, out, ctx);
        return;
    }

    let filter_sub = if is_root {
        None
    } else {
        lower_filter_attr(node, out)
    };
    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(&format!(" xmlns=\"{SVG_NS}\""));
    }
    match &filter_sub {
        Some(sub) => {
            copy_attrs(node, out, &["filter"]);
            out.push_str(sub);
        }
        None => copy_attrs(node, out, &[]),
    }
    out.push_str(&pos_attr(node, ctx));
    if !is_root {
        out.push_str(&artboard_attr(node));
    }

    if node.has_children() {
        out.push('>');
        let elems: Vec<roxmltree::Node> = node.children().filter(|c| c.is_element()).collect();
        let restack = elems.iter().any(|c| {
            c.attribute((XSVG_NS, "layer")).is_some() || c.attribute((XSVG_NS, "order")).is_some()
        });
        if restack {
            // Layer restacking (§5.1): stable-sort direct children by
            // (band, order, document index). Loose content is band 0 / order
            // 0, so it keeps its place and only bucketed or explicitly-ordered
            // layers move. Non-element nodes (whitespace) drop in a restacked
            // container — insignificant for a group.
            let mut idx: Vec<usize> = (0..elems.len()).collect();
            idx.sort_by(|&a, &b| {
                let ka = (
                    layer_band(elems[a]).unwrap_or(0),
                    attr_num_ns(elems[a], "order", 0.0),
                );
                let kb = (
                    layer_band(elems[b]).unwrap_or(0),
                    attr_num_ns(elems[b], "order", 0.0),
                );
                ka.0.cmp(&kb.0)
                    .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
                    .then(a.cmp(&b))
            });
            for &i in &idx {
                serialize(elems[i], out, false, ctx);
            }
        } else {
            for child in node.children() {
                serialize(child, out, false, ctx);
            }
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

    let filter_sub = lower_filter_attr(node, out);
    out.push_str("<path");
    match &filter_sub {
        Some(sub) => {
            copy_attrs(node, out, &["x", "y", "width", "height", "filter"]);
            out.push_str(sub);
        }
        None => copy_attrs(node, out, &["x", "y", "width", "height"]),
    }
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
    let outline = node.attribute("outline") == Some("true") || ctx.force_outline.get();
    let stroke = text_border_attrs(node);
    let pos = pos_attr(node, ctx);

    // Paragraph mode (§6.16): `<x:p>` children are separate paragraphs stacked with
    // space-before/after. Rectangular geometry only (own box or `in="#rect"`);
    // curved region flow ignores `<x:p>` and falls through to a single flow.
    let paras: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "p")
        .collect();
    if !paras.is_empty() {
        let geom = match node.attribute("in") {
            Some(r) => resolve_ref(node, r).filter(|t| t.tag_name().name() == "rect"),
            None => Some(node),
        };
        if let Some(geom) = geom {
            let spec = textbox_area_spec(node, geom);
            emit_paragraphs(node, &paras, &style, fill, gx, &spec, out, ctx);
            return;
        }
    }

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
        // any other shape — or an x: element's compiled output (e.g. a boolean
        // union) — flows text inside its filled outline via the raster region
        // (styled runs are not supported in curved region flow in v0)
        let geometry = match ref_geometry(node, reference, ctx) {
            Ok(d) => d,
            Err(f) => {
                out.push_str(&format!(
                    "<!-- xsvg: <x:textbox in> no region geometry ({}) -->",
                    f.reason()
                ));
                return;
            }
        };
        let region = ctx.shaper.rasterize(&geometry, (style.size / 3.0).max(1.0));
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

/// Paragraph flow (§6.16): lay each `<x:p>` as its own wrapped block, stacked
/// top-down inside the box with the paragraph gaps, then `valign` the whole
/// block. Each paragraph carries its own `align` / `text-indent` / `font-*` /
/// `fill`, and maps to its own source range. Live text (create-outlines is not
/// applied in paragraph mode).
fn emit_paragraphs(
    node: roxmltree::Node,
    paras: &[roxmltree::Node],
    base_style: &TextStyle,
    base_fill: &str,
    gx: f64,
    spec: &AreaSpec,
    out: &mut String,
    ctx: &Ctx,
) {
    let m = ctx.m;
    let cx = spec.x + spec.padding;
    let cy = spec.y + spec.padding;
    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let default_align = node.attribute("align").unwrap_or("start");
    let default_spacing = attr_num(node, "paragraph-spacing", 0.0);

    // per-paragraph style: the box style, with any font-* on the `<x:p>` overriding
    let para_style = |p: roxmltree::Node| {
        let mut st = base_style.clone();
        if let Some(v) = p.attribute("font-family") {
            st.family = v.to_string();
        }
        if let Some(v) = p
            .attribute("font-size")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.size = v;
        }
        if let Some(v) = p.attribute("font-weight") {
            st.weight = v.to_string();
        }
        if let Some(v) = p.attribute("font-style") {
            st.style = v.to_string();
        }
        if let Some(v) = p
            .attribute("line-height")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.line_height = v;
        }
        st
    };

    struct Para<'a> {
        lines: Vec<PlacedLine>,
        style: TextStyle,
        fill: &'a str,
        anchor: &'static str,
        src: roxmltree::Node<'a, 'a>,
    }
    // pass 1: lay each paragraph in a relative frame (para 0's cap-top at 0),
    // accumulating the gaps
    let mut items: Vec<Para> = Vec::with_capacity(paras.len());
    let mut cursor = 0.0f64; // cap-top of the current paragraph
    let mut prev_after = 0.0f64;
    for (i, &p) in paras.iter().enumerate() {
        let ptext = collect_text(p);
        let pstyle = para_style(p);
        let sb = attr_num(p, "space-before", 0.0);
        let sa = attr_num(p, "space-after", default_spacing);
        if i > 0 {
            cursor += prev_after + sb;
        }
        let pspec = AreaSpec {
            x: cx,
            y: cursor,
            width: cw,
            height: 1e5, // tall → no clip; valign=top places lines from `cursor`
            padding: 0.0,
            align: Align::parse(p.attribute("align").unwrap_or(default_align)),
            valign: VAlign::Top,
            fit: Fit::None,
            text_overflow: TextOverflow::Clip,
            text_indent: attr_num(p, "text-indent", 0.0),
        };
        let layout = layout_area(&ptext, &pstyle, &pspec, m);
        let pfm = m.font_metrics(&pstyle, pstyle.size);
        let n = layout.lines.len().max(1);
        let ph = pfm.cap_height + pfm.descent + (n - 1) as f64 * (pstyle.size * pstyle.line_height);
        items.push(Para {
            lines: layout.lines,
            style: pstyle,
            fill: p.attribute("fill").unwrap_or(base_fill),
            anchor: layout.anchor.svg(),
            src: p,
        });
        cursor += ph;
        prev_after = sa;
    }
    let block_h = cursor;
    let valign = VAlign::parse(node.attribute("valign").unwrap_or("top"));
    let block_top = match valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - block_h) / 2.0,
        VAlign::Bottom => cy + (ch - block_h),
    };

    // pass 2: shift every line by `block_top`, clip to the box by baseline, emit one
    // <text> per paragraph
    let base = [EmitAttrs::default()];
    for para in &items {
        let visible: Vec<PlacedLine> = para
            .lines
            .iter()
            .filter(|l| {
                let b = l.baseline + block_top;
                b >= cy - 1e-6 && b <= cy + ch + 1e-6
            })
            .cloned()
            .collect();
        if visible.is_empty() {
            continue;
        }
        out.push_str(&format!("<text text-anchor=\"{}\"", para.anchor));
        push_font_attrs(out, &para.style, para.style.size, para.fill);
        out.push_str(&pos_attr(para.src, ctx)); // per-paragraph source map
        out.push('>');
        for line in &visible {
            let mut l = line.clone();
            l.baseline += block_top;
            emit_line(out, &l, &para.style, para.style.size, gx, m, &base);
        }
        out.push_str("</text>");
    }
}

/// `<x:textpath in="#path" effect="skew|rainbow|stair">` (§6.13): outline the run flat
/// (the [`GlyphOutliner`] seam supplies glyph geometry + advance width), then warp it
/// onto the referenced path **natively** — [`warp_text_on_path`] runs the §7.1 bake at
/// the quality tolerance, with cubic refit above `fast`. `baseline-shift` offsets the
/// run from the path along the local normal (positive = above); `align` / `start`
/// place the run within the path's extent. Emits `<g fill=… stroke=…>` + the warped
/// `<path>`. `stair` (§6.13.3) is live `<text>` by design — per-glyph stepped
/// positions, no outliner — and doubles as skew's degradation when no outline font is
/// available; the last resort is a straight live `<text>` at the element's x/y, so
/// the document never breaks.
fn emit_textpath(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let style = style_from(node);
    let fill = node.attribute("fill").unwrap_or("#000");
    let stroke = text_border_attrs(node);
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
    let path_d = match ref_geometry(node, reference, ctx) {
        Ok(d) => d,
        Err(f) => {
            out.push_str(&format!(
                "<!-- xsvg: <x:textpath in> target not found or not a path ({}) -->",
                f.reason()
            ));
            return;
        }
    };

    // "follow" (§6.13.5) lowers to SVG's own <textPath> — live, selectable text
    // that follows the curve without deforming; no font bytes needed, only the
    // path's arc length for align placement.
    if fx.effect == "follow" {
        if let Some(frame) = PathFrame::new(&path_d, ctx.quality.text_tolerance()) {
            let advance = line_advance(&text, &style, style.size, ctx.m);
            let offset = run_offset(frame.len(), advance, fx.align, fx.start);
            out.push_str("<text");
            push_font_attrs(out, &style, style.size, fill);
            out.push_str(&pos);
            out.push_str("><textPath href=\"#");
            push_escaped(out, reference.strip_prefix('#').unwrap_or(reference), true);
            out.push('"');
            if offset != 0.0 {
                out.push_str(&format!(" startOffset=\"{}\"", fmt(offset)));
            }
            if fx.baseline_shift != 0.0 {
                out.push_str(&format!(" baseline-shift=\"{}\"", fmt(fx.baseline_shift)));
            }
            out.push('>');
            push_escaped(out, &text, false);
            out.push_str("</textPath></text>");
            return;
        }
        // degenerate path → the straight fallback below
    }

    // "stair"/"follow" are live <text> by design — they never consult the outliner.
    if !matches!(fx.effect, "stair" | "follow") {
        let flat = ctx.outliner.outline(&text, &style, style.size, 0.0, 0.0);
        let advance = ctx.outliner.advance_width(&text, &style, style.size);
        if let (Some(flat), Some(advance)) = (flat, advance) {
            // Refit is DISABLED (see §7.1): kurbo's fitter overshoots on dense
            // quantized glyph outlines (notches + hairline smears) and its Optimize
            // level dominates compile time. Polyline output at the graded tolerance
            // is the shipped form until a robust fitter lands.
            if let Some(d) = warp_text_on_path(
                &flat,
                &path_d,
                &fx,
                advance,
                ctx.quality.text_tolerance(),
                false,
            ) {
                push_outline_group(out, fill, &stroke, &pos, &[d]);
                return;
            }
        }
    }

    // Stair Step — authored (§6.13.3), or as the degradation of the height-profile
    // effects (skew, ribbon) without an outline font — when the profile can be
    // sampled; anything else → straight <text> at the element's x/y.
    if matches!(fx.effect, "stair" | "skew" | "ribbon")
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
    let total = line_advance(text, style, style.size, ctx.m); // total run width
    if !total.is_finite() || advances.iter().any(|a| !a.is_finite()) {
        return false;
    }
    // sample the path's height profile natively (§6.13.1)
    let Some(frame) = PathFrame::new(path_d, ctx.quality.text_tolerance()) else {
        return false;
    };
    let x0 = frame.x0() + run_offset(frame.x1() - frame.x0(), total, fx.align, fx.start);
    if !x0.is_finite() {
        return false;
    }

    out.push_str("<text");
    push_font_attrs(out, style, style.size, fill);
    out.push_str(pos);
    // absolute per-glyph positions — spacing is baked in, so no spacing attrs
    let xs: Vec<String> = advances.iter().map(|a| fmt(x0 + a)).collect();
    let ys: Vec<String> = advances
        .iter()
        .map(|a| fmt(frame.y_at(x0 + a) - fx.baseline_shift))
        .collect();
    out.push_str(&format!(" x=\"{}\" y=\"{}\"", xs.join(" "), ys.join(" ")));
    out.push('>');
    push_escaped(out, text, false);
    out.push_str("</text>");
    true
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
        text_indent: attr_num(node, "text-indent", 0.0),
    }
}

/// Resolve a `#id` (or bare `id`) reference to its element anywhere in the document.
fn resolve_ref<'a>(node: roxmltree::Node<'a, 'a>, r: &str) -> Option<roxmltree::Node<'a, 'a>> {
    let id = r.strip_prefix('#').unwrap_or(r);
    node.document()
        .descendants()
        .find(|n| n.attribute("id") == Some(id))
}

/// Parse an SVG `transform` list into a kurbo [`Affine`] (§4 reference
/// resolution honors the target's own transform). Mirrors browser behavior for
/// invalid input: the whole list is ignored (identity).
fn parse_transform(s: &str) -> crate::kurbo::Affine {
    use crate::kurbo::Affine;
    let mut total = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest[..open].trim_matches(|c: char| c == ',' || c.is_whitespace());
        let Some(close) = rest[open..].find(')') else {
            return Affine::IDENTITY;
        };
        let args: Vec<f64> = rest[open + 1..open + close]
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(parse_num)
            .collect();
        let t = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine::new([*a, *b, *c, *d, *e, *f]),
            ("translate", [tx]) => Affine::translate((*tx, 0.0)),
            ("translate", [tx, ty]) => Affine::translate((*tx, *ty)),
            ("scale", [k]) => Affine::scale(*k),
            ("scale", [kx, ky]) => Affine::scale_non_uniform(*kx, *ky),
            ("rotate", [a]) => Affine::rotate(a.to_radians()),
            ("rotate", [a, cx, cy]) => {
                Affine::translate((*cx, *cy))
                    * Affine::rotate(a.to_radians())
                    * Affine::translate((-*cx, -*cy))
            }
            ("skewX", [a]) => Affine::new([1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0]),
            ("skewY", [a]) => Affine::new([1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]),
            _ => return Affine::IDENTITY,
        };
        total *= t;
        rest = &rest[open + close + 1..];
    }
    if total.is_finite() {
        total
    } else {
        Affine::IDENTITY
    }
}

/// `true` for a `<use href>` that points at another file — anything that isn't a
/// bare same-document `#fragment`. That's the trigger to bake a cross-file link (§4).
fn is_external_href(href: &str) -> bool {
    !href.is_empty() && !href.starts_with('#')
}

/// A dependency box's intrinsic size: its `viewBox` extent, else its `width`/`height`
/// (units stripped), else a 100×100 fallback.
fn intrinsic_size(root: roxmltree::Node) -> (f64, f64) {
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
    (dim("width").unwrap_or(100.0), dim("height").unwrap_or(100.0))
}

/// The intrinsic extent of a by-id `<use>` target when it *declares* one — a nested
/// `<svg>`'s viewBox, or any element's explicit `width`/`height`. `None` for a plain
/// shape/group with no declared size (the caller then measures its geometry).
fn target_extent(node: roxmltree::Node) -> Option<(f64, f64)> {
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

/// Bounding box of a compiled (plain-SVG) subtree: the union of every descendant shape's
/// bbox (`<path>`, `<rect>`, `<circle>`, … via [`shape_to_path_d`]), plus `<image>` boxes,
/// nested `<svg>` viewports, and same-document `<use>` targets — with ancestor
/// `transform`s and stroke half-widths applied. Used only as a by-id `<use>` sizing hint,
/// so `<text>` (which would need font metrics) isn't measured. `None` when the subtree
/// has no measurable geometry.
fn plain_subtree_bbox(
    node: roxmltree::Node,
    base: crate::kurbo::Affine,
) -> Option<crate::kurbo::Rect> {
    subtree_bbox(node, base, &mut Vec::new(), Stroke::NONE)
}

/// Inherited stroke state: whether a stroke is actually painted, and how wide.
#[derive(Clone, Copy)]
struct Stroke {
    painted: bool,
    width: f64,
}

impl Stroke {
    const NONE: Self = Self {
        painted: false,
        width: 1.0,
    };

    /// Fold in an element's own `stroke` / `stroke-width` (attribute or inline style).
    fn inherit(self, node: roxmltree::Node) -> Self {
        let get = |p: &str| {
            node.attribute(p)
                .map(str::trim)
                .or_else(|| style_decl(node, p))
        };
        Self {
            painted: match get("stroke") {
                Some(v) => v != "none",
                None => self.painted,
            },
            width: get("stroke-width").and_then(parse_num).unwrap_or(self.width),
        }
    }

    /// Half the stroke, in the *transformed* space — how far paint spills past the fill.
    /// Uses `sqrt(|det|)` so a rotated or non-uniformly scaled transform still gets a
    /// sensible single number.
    fn outset(self, tf: crate::kurbo::Affine) -> f64 {
        if !self.painted || self.width <= 0.0 {
            return 0.0;
        }
        let scale = tf.determinant().abs().sqrt();
        if scale.is_finite() {
            self.width / 2.0 * scale
        } else {
            0.0
        }
    }
}

fn subtree_bbox(
    node: roxmltree::Node,
    base: crate::kurbo::Affine,
    seen: &mut Vec<String>,
    inherited: Stroke,
) -> Option<crate::kurbo::Rect> {
    let tag = node.tag_name().name();
    // Never-rendered subtrees (and explicitly hidden ones) would inflate the box. A
    // <symbol> is skipped here but *is* drawn when a <use> targets it — see below.
    if is_non_rendered(tag) || is_display_none(node) {
        return None;
    }
    let tf = base * element_transform(node);
    let stroke = inherited.inherit(node);
    let mut acc: Option<crate::kurbo::Rect> = None;
    let mut fold = |bb: crate::kurbo::Rect| {
        acc = Some(acc.map_or(bb, |a: crate::kurbo::Rect| a.union(bb)));
    };

    // A nested <svg> establishes a viewport and clips to it, so its extent *is* that
    // viewport rect. Recursing would measure its children in the unmapped coordinate
    // system — for a scaled viewBox (e.g. 0 0 1000 1000 shown at 20) wildly wrong.
    // Without a usable width/height we fall through and measure children as before.
    if tag == "svg" {
        if let Some(bb) = viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
            fold(bb);
            return acc;
        }
    }
    // Box-shaped, not path geometry (and <foreignObject>'s content isn't SVG).
    if matches!(tag, "image" | "foreignObject") {
        if let Some(bb) = viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
            fold(bb);
        }
        return acc;
    }
    // A same-document <use href="#id"> draws its target here, offset by x/y. (Cross-file
    // hrefs are already baked by this point, so only local ones remain.) The id stack is
    // the cycle guard — a <use> chain that loops back stops instead of recursing forever.
    if tag == "use" {
        if let Some(id) = local_href_id(node) {
            if !seen.iter().any(|s| s == id) {
                if let Some(target) = node
                    .document()
                    .descendants()
                    .find(|n| n.is_element() && n.attribute("id") == Some(id))
                {
                    let target_tag = target.tag_name().name();
                    let placed = tf
                        * crate::kurbo::Affine::translate((
                            attr_num(node, "x", 0.0),
                            attr_num(node, "y", 0.0),
                        ));
                    seen.push(id.to_string());
                    let bb = if matches!(target_tag, "symbol" | "svg") {
                        // The <use> sizes the viewport when it gives width/height;
                        // otherwise measure the (otherwise non-rendered) contents.
                        match viewport_rect(node).and_then(|r| transform_rect_bbox(r, tf)) {
                            Some(bb) => Some(bb),
                            None => children_bbox(target, placed, seen, stroke),
                        }
                    } else {
                        subtree_bbox(target, placed, seen, stroke)
                    };
                    seen.pop();
                    if let Some(bb) = bb {
                        fold(bb);
                    }
                }
            }
        }
        return acc;
    }

    if let Some(bb) = shape_to_path_d(node)
        .and_then(|d| transform_d(&d, tf))
        .and_then(|d| svg_path_bbox(&d))
    {
        // Stroke straddles the outline, so paint reaches half a stroke past the fill.
        fold(bb.inflate(stroke.outset(tf), stroke.outset(tf)));
    }
    if let Some(bb) = children_bbox(node, tf, seen, stroke) {
        fold(bb);
    }
    acc
}

/// Union of `node`'s element children, measured in `tf`.
fn children_bbox(
    node: roxmltree::Node,
    tf: crate::kurbo::Affine,
    seen: &mut Vec<String>,
    stroke: Stroke,
) -> Option<crate::kurbo::Rect> {
    let mut acc: Option<crate::kurbo::Rect> = None;
    for c in node.children().filter(roxmltree::Node::is_element) {
        if let Some(bb) = subtree_bbox(c, tf, seen, stroke) {
            acc = Some(acc.map_or(bb, |a: crate::kurbo::Rect| a.union(bb)));
        }
    }
    acc
}

/// The `#id` of a same-document `href` / `xlink:href`, if that's what it is.
fn local_href_id<'a>(node: roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    node.attribute("href")
        .or_else(|| node.attribute(("http://www.w3.org/1999/xlink", "href")))?
        .strip_prefix('#')
        .filter(|id| !id.is_empty())
}

/// Elements whose subtree is definition-only — referenced, never drawn in place.
fn is_non_rendered(tag: &str) -> bool {
    matches!(
        tag,
        "defs"
            | "symbol"
            | "clipPath"
            | "mask"
            | "marker"
            | "pattern"
            | "filter"
            | "linearGradient"
            | "radialGradient"
            | "meshgradient"
            | "title"
            | "desc"
            | "metadata"
            | "style"
            | "script"
    )
}

/// `display:none` (attribute or inline style) — drawn nowhere, so it contributes no box.
/// `visibility:hidden` is deliberately *not* here: it still takes up layout space.
fn is_display_none(node: roxmltree::Node) -> bool {
    node.attribute("display") == Some("none")
        || style_decl(node, "display").is_some_and(|v| v.trim() == "none")
}

/// The value of one declaration in an inline `style="a: 1; b: 2"`, if present.
fn style_decl<'a>(node: roxmltree::Node<'a, 'a>, prop: &str) -> Option<&'a str> {
    node.attribute("style")?.split(';').find_map(|decl| {
        let (name, value) = decl.split_once(':')?;
        (name.trim() == prop).then(|| value.trim())
    })
}

/// An element's transform: the presentation attribute, else a `style="transform:…"`.
/// CSS `px`/`deg` map 1:1 onto SVG user units / degrees, so they are simply stripped;
/// anything [`parse_transform`] doesn't recognise degrades to identity either way.
fn element_transform(node: roxmltree::Node) -> crate::kurbo::Affine {
    if let Some(t) = node.attribute("transform") {
        return parse_transform(t);
    }
    match style_decl(node, "transform") {
        Some(v) => parse_transform(&v.replace("px", "").replace("deg", "")),
        None => crate::kurbo::Affine::IDENTITY,
    }
}

/// `x`/`y`/`width`/`height` as a rect — the viewport of a nested `<svg>`, or the box of
/// an `<image>`/`<foreignObject>`. `None` unless both extents are positive numbers.
fn viewport_rect(node: roxmltree::Node) -> Option<crate::kurbo::Rect> {
    let dim = |a: &str| node.attribute(a).and_then(parse_num).filter(|v| *v > 0.0);
    let (w, h) = (dim("width")?, dim("height")?);
    let (x, y) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    Some(crate::kurbo::Rect::new(x, y, x + w, y + h))
}

/// Axis-aligned bounds of `r` after `tf` (the bbox of its four mapped corners).
fn transform_rect_bbox(
    r: crate::kurbo::Rect,
    tf: crate::kurbo::Affine,
) -> Option<crate::kurbo::Rect> {
    let p = |x, y| tf * crate::kurbo::Point::new(x, y);
    let corners = [p(r.x0, r.y0), p(r.x1, r.y0), p(r.x1, r.y1), p(r.x0, r.y1)];
    if corners.iter().any(|c| !c.x.is_finite() || !c.y.is_finite()) {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for c in corners {
        x0 = x0.min(c.x);
        y0 = y0.min(c.y);
        x1 = x1.max(c.x);
        y1 = y1.max(c.y);
    }
    Some(crate::kurbo::Rect::new(x0, y0, x1, y1))
}

/// Bake a cross-file `<use>` link (§4): resolve the referenced file, compile it, and
/// stamp its output into `out`. Whole-file → a nested `<svg>` viewport (SVG's own fit);
/// `#id` → that compiled element, placed. Degrades with a marker on any failure.
fn emit_link(node: roxmltree::Node, href: &str, out: &mut String, ctx: &Ctx) {
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
    let compiled = match roxmltree::Document::parse(&source) {
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
            let vb = root
                .attribute("viewBox")
                .map(|v| format!(" viewBox=\"{v}\""))
                .unwrap_or_default();
            out.push_str(&format!(
                "<svg{pos} x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{vb}>{inner}</svg>",
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

/// Apply an affine to path data. `None` if `d` fails to parse.
fn transform_d(d: &str, a: crate::kurbo::Affine) -> Option<String> {
    if a == crate::kurbo::Affine::IDENTITY {
        return Some(d.to_string());
    }
    let mut p = crate::kurbo::BezPath::from_svg(d).ok()?;
    p.apply_affine(a);
    Some(p.to_svg())
}

/// A `<use>` operand's placement: its `transform` composed with the extra
/// `x`/`y` translation (per SVG, x/y append a translate after the transform).
fn use_placement(use_el: roxmltree::Node) -> crate::kurbo::Affine {
    let t = use_el
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or(crate::kurbo::Affine::IDENTITY);
    t * crate::kurbo::Affine::translate((attr_num(use_el, "x", 0.0), attr_num(use_el, "y", 0.0)))
}

/// Why a reference failed to resolve to geometry — spelled into the caller's
/// marker so degradations are distinguishable (§4).
#[derive(Clone, Copy, Debug, PartialEq)]
enum RefFail {
    NotFound,
    NoGeometry,
    Cycle,
    Depth,
    Budget,
    InnerTransform,
}

impl RefFail {
    fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "target not found",
            Self::NoGeometry => "target has no path geometry",
            Self::Cycle => "reference cycle",
            Self::Depth => "reference chain too deep",
            Self::Budget => "reference budget exhausted",
            Self::InnerTransform => "referenced output nests a transform",
        }
    }
}

/// Resolve an `in="#id"` / `<use href>` reference to **geometry** (§4): a plain
/// shape yields its source geometry, a plain `<g>` the union of its shape
/// descendants, and an `x:` element its **compiled output** — every `<path d>`
/// it emits, joined as one multi-subpath region — so composition works by
/// reference. The target's own `transform` is honored (applied to the borrowed
/// geometry). Cycles, chains deeper than [`MAX_REF_DEPTH`], and an exhausted
/// [`REF_FUEL`] budget degrade with a distinguishable [`RefFail`] (§4 totality).
fn ref_geometry(node: roxmltree::Node, r: &str, ctx: &Ctx) -> Result<String, RefFail> {
    let target = resolve_ref(node, r).ok_or(RefFail::NotFound)?;
    target_geometry(target, ctx)
}

/// The geometry a single element contributes when referenced (or when swept up
/// by a group walk). Guards, budget, memo, and the element's own `transform`
/// live here; the per-kind harvests live in [`x_output_geometry`] /
/// [`group_geometry`] / [`shape_to_path_d`].
fn target_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let own = target
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or(crate::kurbo::Affine::IDENTITY);
    let is_x = target.tag_name().namespace() == Some(XSVG_NS);
    if !is_x && target.tag_name().name() != "g" {
        let d = shape_to_path_d(target).ok_or(RefFail::NoGeometry)?;
        return transform_d(&d, own).ok_or(RefFail::NoGeometry);
    }
    let id = target.attribute("id").map(str::to_string);
    if let Some(id) = &id {
        if let Some(hit) = ctx.resolved.borrow().get(id) {
            return hit.clone().ok_or(RefFail::NoGeometry);
        }
        if ctx.resolving.borrow().contains(id) {
            ctx.cuts.set(ctx.cuts.get() + 1);
            return Err(RefFail::Cycle);
        }
    }
    if ctx.resolving.borrow().len() >= MAX_REF_DEPTH {
        ctx.cuts.set(ctx.cuts.get() + 1);
        return Err(RefFail::Depth);
    }
    if ctx.fuel.get() == 0 {
        ctx.cuts.set(ctx.cuts.get() + 1);
        return Err(RefFail::Budget);
    }
    ctx.fuel.set(ctx.fuel.get() - 1);
    let cuts_before = ctx.cuts.get();
    if let Some(id) = &id {
        ctx.resolving.borrow_mut().push(id.clone());
    }
    let harvested = if is_x {
        x_output_geometry(target, ctx)
    } else {
        group_geometry(target, ctx)
    };
    if id.is_some() {
        ctx.resolving.borrow_mut().pop();
    }
    let result = harvested.and_then(|d| transform_d(&d, own).ok_or(RefFail::NoGeometry));
    // a cut below means this result reflects what was on the stack, not the
    // target itself — valid here, poison anywhere else
    if ctx.cuts.get() == cuts_before {
        if let Some(id) = id {
            match &result {
                Ok(d) => {
                    ctx.resolved.borrow_mut().insert(id, Some(d.clone()));
                }
                Err(RefFail::NoGeometry) => {
                    ctx.resolved.borrow_mut().insert(id, None);
                }
                Err(_) => {}
            }
        }
    }
    result
}

/// Harvest an `x:` element's compiled output as geometry: scratch-serialize
/// (with glyph outlining forced, so referenced text contributes its glyph
/// geometry), reject nested transforms the d-only harvest cannot honor, resolve
/// winding-sensitive output through the boolean engine, and join the rest.
fn x_output_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let prev = ctx.force_outline.replace(true);
    let mut buf = String::new();
    serialize(target, &mut buf, false, ctx);
    ctx.force_outline.set(prev);
    // the target's own transform reappears on its output wrapper (copy_attrs)
    // and is applied by target_geometry; any FURTHER transform nested in the
    // output would be silently lost by a d-only harvest — degrade loudly
    let expected = usize::from(target.attribute("transform").is_some());
    if buf.matches(" transform=\"").count() > expected {
        return Err(RefFail::InnerTransform);
    }
    let ranges = find_path_d_ranges(&buf);
    if ranges.is_empty() {
        return Err(RefFail::NoGeometry);
    }
    let ds: Vec<&str> = ranges.iter().map(|&(a, b)| &buf[a..b]).collect();
    let even: Vec<bool> = ranges
        .iter()
        .map(|&(a, b)| {
            let tag = buf[..a].rfind('<').unwrap_or(0);
            let end = buf[b..].find('>').map(|i| b + i).unwrap_or(buf.len());
            buf[tag..end].contains(r#"fill-rule="evenodd""#)
        })
        .collect();
    if even.iter().any(|&e| e) {
        // an evenodd path's painted region differs from its nonzero reading —
        // resolve through the boolean engine so borrowed = painted
        let ops: Vec<BoolOperand> = ds
            .iter()
            .zip(&even)
            .map(|(d, &e)| BoolOperand {
                paths: vec![d],
                even_odd: e,
            })
            .collect();
        return match boolean_svg_paths(&ops, BoolOp::Union, ctx.quality.tolerance()) {
            Some(d) if !d.is_empty() => Ok(d),
            _ => Err(RefFail::NoGeometry),
        };
    }
    Ok(ds.join(" "))
}

/// A plain `<g>` target: every shape descendant contributes, transforms compose
/// down the tree (each child applies its own inside [`target_geometry`]),
/// nested `x:` elements resolve to their compiled output. Children that carry
/// no geometry (text, defs, live `<use>`) are skipped, not fatal.
fn group_geometry(target: roxmltree::Node, ctx: &Ctx) -> Result<String, RefFail> {
    let mut ds = Vec::new();
    for child in target.children().filter(|c| c.is_element()) {
        // a passthrough <use> stays a live reference (§5), even inside a group
        if child.tag_name().name() == "use" && child.tag_name().namespace() != Some(XSVG_NS) {
            continue;
        }
        if let Ok(d) = target_geometry(child, ctx) {
            ds.push(d);
        }
    }
    if ds.is_empty() {
        return Err(RefFail::NoGeometry);
    }
    Ok(ds.join(" "))
}

/// Convert a fillable SVG shape element to a path `d` string (for rasterization).
/// `rect` is handled separately (rectangular fast path); returns `None` for shapes
/// with no fillable area (e.g. `<line>`).
fn shape_to_path_d(node: roxmltree::Node) -> Option<String> {
    match node.tag_name().name() {
        "path" => node.attribute("d").map(str::to_string),
        "rect" => {
            let (x, y, w, h) = (
                attr_num(node, "x", 0.0),
                attr_num(node, "y", 0.0),
                attr_num(node, "width", 0.0),
                attr_num(node, "height", 0.0),
            );
            if w <= 0.0 || h <= 0.0 {
                return None;
            }
            // rx/ry default to each other per SVG, clamped to the half-extent
            let rx = attr_num(node, "rx", attr_num(node, "ry", 0.0)).clamp(0.0, w / 2.0);
            let ry = attr_num(node, "ry", attr_num(node, "rx", 0.0)).clamp(0.0, h / 2.0);
            Some(if rx > 0.0 && ry > 0.0 {
                format!(
                    "M{},{y} h{} a{rx},{ry} 0 0,1 {rx},{ry} v{} a{rx},{ry} 0 0,1 -{rx},{ry} h-{} a{rx},{ry} 0 0,1 -{rx},-{ry} v-{} a{rx},{ry} 0 0,1 {rx},-{ry} Z",
                    x + rx,
                    w - 2.0 * rx,
                    h - 2.0 * ry,
                    w - 2.0 * rx,
                    h - 2.0 * ry
                )
            } else {
                format!("M{x},{y} h{w} v{h} h-{w} Z")
            })
        }
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
        "line" => {
            let (x1, y1, x2, y2) = (
                attr_num(node, "x1", 0.0),
                attr_num(node, "y1", 0.0),
                attr_num(node, "x2", 0.0),
                attr_num(node, "y2", 0.0),
            );
            Some(format!("M{x1},{y1} L{x2},{y2}"))
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
        &text_border_attrs(node),
        gx,
        m,
        &emits,
        &pos_attr(node, ctx),
        outline,
        ctx.outliner,
    );
}

/// Stroke/border paint attributes for a text element, honored on both live `<text>` and
/// outlined glyphs. The `x:border-width` / `x:border-color` convenience (§6.17) emits a
/// clean bordered-text effect: the stroke sits *behind* the fill (`paint-order="stroke"`,
/// round joins) so the border never eats into the letters, and `border-width` is the width
/// visible outside each glyph (the emitted `stroke-width` is doubled, since the fill covers
/// the inner half). Otherwise raw SVG stroke attributes pass through unchanged.
fn text_border_attrs(node: roxmltree::Node) -> String {
    let bw = node.attribute((XSVG_NS, "border-width")).and_then(parse_num);
    let bc = node.attribute((XSVG_NS, "border-color"));
    if bw.is_some() || bc.is_some() {
        let w = bw.unwrap_or(3.0).max(0.0);
        let color = resolve_var(bc.unwrap_or("#000000"));
        let mut s = format!(
            " stroke=\"{color}\" stroke-width=\"{}\" paint-order=\"stroke\" stroke-linejoin=\"round\"",
            fmt(w * 2.0)
        );
        if let Some(op) = node.attribute((XSVG_NS, "border-opacity")) {
            s.push_str(&format!(" stroke-opacity=\"{op}\""));
        }
        return s;
    }
    // Raw passthrough of hand-written stroke attributes.
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
    let (fill, stroke) = (resolve_var(fill), resolve_var(stroke));
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
        resolve_var(fill)
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
    out.push_str(&resolve_var(stroke)); // border/stroke (§6.17) — behind the fill via paint-order
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
    // A `<x:font>` token (via `x:font="name"`) supplies OVERRIDABLE defaults —
    // the element's own `font-*` attributes win over the token (§4.1).
    let tok = font_props(node).unwrap_or_default();
    let t = |k: &str| tok.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
    let s = |k: &str, d: &str| node.attribute(k).or_else(|| t(k)).unwrap_or(d).to_string();
    let num = |k: &str, d: f64| {
        node.attribute(k)
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
            .or_else(|| t(k).and_then(parse_num).filter(|n| *n > 0.0))
            .unwrap_or(d)
    };
    let spacing = |k: &str| match node.attribute(k).or_else(|| t(k)) {
        None | Some("normal") => 0.0,
        Some(v) => parse_num(v).unwrap_or(0.0),
    };
    TextStyle {
        family: s("font-family", "sans-serif"),
        size: num("font-size", 16.0),
        weight: s("font-weight", "normal"),
        style: s("font-style", "normal"),
        line_height: num("line-height", 1.2),
        letter_spacing: spacing("letter-spacing"),
        word_spacing: spacing("word-spacing"),
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

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// Copy a node's attributes, normalizing namespaces: `x:` attrs are consumed (not
/// copied), `xlink:*` modernizes to the unprefixed SVG 2 form (`xlink:href` →
/// `href` — supported by every current renderer, and we declare no xlink xmlns),
/// `xml:*` keeps its reserved prefix (`xml:space`, `xml:lang`), and other foreign
/// namespaces drop (editor metadata we can't re-emit faithfully). `skip` filters
/// by local name.
// ---- Theming (§4.1): compile-time color & type tokens ---------------------

/// A `<x:theme>`'s tokens, loaded once per compile. `colors` map a name to any
/// paint value (referenced as `var(name)`); `fonts` map a name to an ordered set
/// of font-ish properties (a CSS-`font`-shorthand-like bundle) applied as an
/// OVERRIDABLE base on text carrying `x:font="name"` (the element's own `font-*`
/// wins).
#[derive(Default)]
struct Theme {
    colors: std::collections::HashMap<String, String>,
    fonts: std::collections::HashMap<String, Vec<(String, String)>>,
}

thread_local! {
    // Per-compile theme. wasm is single-threaded and `compile_impl` reloads it at
    // the top of every compile, so tokens never leak between documents or tests.
    static THEME: std::cell::RefCell<Theme> = std::cell::RefCell::new(Theme::default());
}

/// The font-ish properties an `<x:font>` token may carry.
const FONT_PROPS: [&str; 7] = [
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
];

/// Load `<x:theme>`'s `<x:color>` / `<x:font>` tokens into [`THEME`] (clearing any
/// prior compile's). Called once before serialization.
fn load_theme(doc: &roxmltree::Document) {
    let mut theme = Theme::default();
    for th in doc
        .descendants()
        .filter(|n| n.tag_name().namespace() == Some(XSVG_NS) && n.tag_name().name() == "theme")
    {
        for c in th.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "color" => {
                    if let (Some(name), Some(val)) = (c.attribute("name"), c.attribute("value")) {
                        theme.colors.insert(name.to_string(), val.to_string());
                    }
                }
                "font" => {
                    if let Some(name) = c.attribute("name") {
                        let props = FONT_PROPS
                            .iter()
                            .filter_map(|&p| c.attribute(p).map(|v| (p.to_string(), v.to_string())))
                            .collect();
                        theme.fonts.insert(name.to_string(), props);
                    }
                }
                _ => {}
            }
        }
    }
    THEME.with(|t| *t.borrow_mut() = theme);
}

/// Resolve every `var(name)` / `var(name, fallback)` in an attribute value against
/// the theme's color tokens. An unknown token with no fallback is left verbatim
/// (renders as the property's initial value, like a dangling CSS variable).
fn resolve_var(v: &str) -> std::borrow::Cow<'_, str> {
    if !v.contains("var(") {
        return std::borrow::Cow::Borrowed(v);
    }
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find("var(") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 4..];
        let Some(close) = after.find(')') else {
            out.push_str(&rest[i..]);
            rest = "";
            break;
        };
        let inner = &after[..close];
        let mut parts = inner.splitn(2, ',');
        let name = parts.next().unwrap_or("").trim().trim_start_matches("--");
        let fallback = parts.next().map(str::trim);
        let hit = THEME.with(|t| {
            t.borrow()
                .colors
                .get(name)
                .cloned()
                .or_else(|| fallback.map(str::to_string))
        });
        match hit {
            Some(val) => out.push_str(&val),
            None => out.push_str(&rest[i..i + 4 + close + 1]), // leave var(...) verbatim
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// The `<x:font>` token's properties named by an element's `x:font`, if any.
fn font_props(node: roxmltree::Node) -> Option<Vec<(String, String)>> {
    let name = node.attribute((XSVG_NS, "font"))?;
    THEME.with(|t| t.borrow().fonts.get(name).cloned())
}

fn copy_attrs(node: roxmltree::Node, out: &mut String, skip: &[&str]) {
    for attr in node.attributes() {
        if skip.contains(&attr.name()) {
            continue;
        }
        // event handlers are outside the static subset (§9, Plan R6)
        if attr.name().starts_with("on") && attr.namespace().is_none() {
            continue;
        }
        let prefix = match attr.namespace() {
            None => "",
            Some(XLINK_NS) => "",
            Some(XML_NS) => "xml:",
            Some(_) => continue, // x: (consumed) and foreign metadata
        };
        out.push(' ');
        out.push_str(prefix);
        out.push_str(attr.name());
        out.push_str("=\"");
        push_escaped(out, &resolve_var(attr.value()), true);
        out.push('"');
    }
    // Font token (§4.1): apply the named `<x:font>`'s props the element did not
    // set itself — an overridable base. (Only fires when `x:font` is present;
    // harmless on non-text elements, which ignore font-*.)
    if let Some(props) = font_props(node) {
        for (k, v) in props {
            if node.attribute(k.as_str()).is_none() && !skip.contains(&k.as_str()) {
                out.push(' ');
                out.push_str(&k);
                out.push_str("=\"");
                push_escaped(out, &resolve_var(&v), true);
                out.push('"');
            }
        }
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
mod tests;

// ---- feature modules (split out of this file) ----
mod connector;
use connector::*;
mod table;
use table::*;
mod charts;
use charts::*;
mod list;
use list::*;
mod mesh;
use mesh::*;
mod pathops;
use pathops::*;
