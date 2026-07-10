//! WASM entry point for xsvg: parse the xsvg/SVG input, run lowering passes, and emit a
//! plain-SVG-subset string. Passes wired so far:
//!   • `<rect>` (sharp-cornered) → `<path>`
//!   • `<text inline-size>` → wrapped `<tspan>` lines (§6.2)
//!   • `<textArea>` → flowed text: align / display-align / line-increment / auto sizing (§6.3)
//!   • `<x:textbox>` → wrapped + aligned + shrink-to-fit text, incl. `in="#shape"` region
//!     flow and cap-height centering (§6.4–6.5, 6.10)
//!   • styled `<tspan>` runs (§6.11); create outlines `outline="true"` → `<path>` (§6.12);
//!     text on a path `<x:textpath>` skew + rainbow + stair, with `baseline-shift` (§6.13)
//! Other `x:` extensions are recognized and skipped with a marker.
//!
//! **Platform seams.** Everything platform-specific is a trait the core calls, backed here
//! by JS callbacks: `Measurer` (canvas `measureText` + font metrics), `Shaper` (path
//! rasterize for region flow), and `GlyphOutliner` (opentype.js glyph outlines + advance
//! widths). All warp math — including the §6.13 fields — runs natively in `xsvg-core`.

use wasm_bindgen::prelude::*;
use xsvg_core::{
    boolean_svg_paths, filter_primitives, layout_area_measured, layout_flow, layout_region,
    layout_text_area_runs, line_advance, measure_runs, parse_filter_functions, run_offset,
    svg_path_bbox, warp_svg_path, warp_text_on_path, Align, Anchor, AreaLayout, AreaSpec,
    BendField, BoolOp, BoolOperand, Chain, DisplayAlign, EnvelopePreset, Field, Fit, FreeDistort,
    GlyphOutliner, Homography, LineIncrement, Measurer, PathEffect, PathFrame, PlacedLine,
    QualityProfile, RasterRegion, Rect, RegionSpec, RoughenField, Shaper, Taper, TextAlign,
    TextAreaSpec, TextOverflow, TextStyle, VAlign, WarpAxis,
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

/// Push the run's style as the shared `(family, weight, style, size)` callback arguments
/// onto `args` — the common prefix of the outliner JS calls.
fn push_style_args(args: &js_sys::Array, style: &TextStyle, size: f64) {
    args.push(&JsValue::from_str(&style.family));
    args.push(&JsValue::from_str(&style.weight));
    args.push(&JsValue::from_str(&style.style));
    args.push(&JsValue::from_f64(size));
}

/// Browser-backed [`GlyphOutliner`]. `outline_run(text, family, weight, style, size, x,
/// baseline) => d | ""` returns a glyph outline (opentype.js), or `""` when the font's
/// bytes aren't available (→ fall back to live `<text>`). `advance_width(text, family,
/// weight, style, size) => number | NaN` returns the run's advance per the same font.
/// Path-warping itself is native (§6.13 runs through the core §7.1 bake).
struct JsOutliner<'a> {
    outline_run: &'a js_sys::Function,
    advance_width: &'a js_sys::Function,
}

impl GlyphOutliner for JsOutliner<'_> {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String> {
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(text));
        push_style_args(&args, style, size);
        args.push(&JsValue::from_f64(x));
        args.push(&JsValue::from_f64(baseline));
        let d = self
            .outline_run
            .apply(&JsValue::NULL, &args)
            .ok()?
            .as_string()?;
        (!d.is_empty()).then_some(d)
    }

    fn advance_width(&self, text: &str, style: &TextStyle, size: f64) -> Option<f64> {
        let args = js_sys::Array::new();
        args.push(&JsValue::from_str(text));
        push_style_args(&args, style, size);
        self.advance_width
            .apply(&JsValue::NULL, &args)
            .ok()?
            .as_f64()
            .filter(|w| w.is_finite())
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
    sourcemap: bool,
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
}

/// `" data-xsvg-pos=\"START-END\""` (byte offsets of `node` in the original xsvg
/// source) when the source map is enabled, else empty. Attached to each emitted
/// top-level element so a viewer can project a rendered element back to its source.
fn pos_attr(node: roxmltree::Node, ctx: &Ctx) -> String {
    if !ctx.sourcemap {
        return String::new();
    }
    let r = node.range();
    format!(" data-xsvg-pos=\"{}-{}\"", r.start, r.end)
}

/// WASM entry point. `measure(text, fontCss) => number`,
/// `metrics(fontCss) => [ascent, descent, capHeight, xHeight]`, and
/// `rasterize(pathD, rowH) => Float64Array` are browser callbacks. Throws on
/// malformed XML so the JS side can surface the error.
///
/// When `sourcemap` is true, every emitted top-level element carries a
/// `data-xsvg-pos="START-END"` attribute — the byte range of the originating xsvg
/// node in `input` — so an interactive viewer can project a rendered element back
/// to its authoring source. Synthesized subtrees (e.g. `<x:textbox>` → `<text>…`)
/// tag only their root element; a viewer resolves inner nodes via the nearest
/// ancestor carrying the attribute.
#[wasm_bindgen]
pub fn compile(
    input: &str,
    quality: &str,
    sourcemap: bool,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    outline_run: &js_sys::Function,
    advance_width: &js_sys::Function,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    compile_impl(input, quality, sourcemap, &m, &shaper, &outliner).map_err(|e| JsError::new(&e))
}

/// Incremental entry (docs/Incremental.md): re-emit only the top-level element
/// containing byte `offset`. Same callbacks as [`compile`]; the returned markup is
/// byte-identical to that element's span in a full compile, so the caller can
/// replace the corresponding DOM node surgically.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: u32,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    outline_run: &js_sys::Function,
    advance_width: &js_sys::Function,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    compile_fragment_impl(
        input,
        quality,
        sourcemap,
        offset as usize,
        &m,
        &shaper,
        &outliner,
    )
    .map_err(|e| JsError::new(&e))
}

/// Source byte range `[start, end]` of the fragment unit containing `offset`, or
/// an empty array when the offset falls outside every top-level element.
#[wasm_bindgen]
pub fn fragment_range(input: &str, offset: u32) -> Vec<u32> {
    match fragment_range_impl(input, offset as usize) {
        Some((s, e)) => vec![s as u32, e as u32],
        None => Vec::new(),
    }
}

/// Flat `[start, end, start, end, …]` byte ranges of the top-level elements whose
/// baked `in="#id"` references point into the fragment at `offset` — they must be
/// re-emitted alongside it.
#[wasm_bindgen]
pub fn dependents(input: &str, offset: u32) -> Vec<u32> {
    dependents_impl(input, offset as usize)
        .into_iter()
        .flat_map(|(s, e)| [s as u32, e as u32])
        .collect()
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
pub fn compile_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    let q = xsvg_core::QualityProfile::parse(quality);
    check_nesting_depth(input, MAX_NESTING_DEPTH)?;
    let doc = roxmltree::Document::parse(input).map_err(|e| format!("xsvg parse error: {e}"))?;

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
            sourcemap,
            resolving: std::cell::RefCell::new(Vec::new()),
            resolved: std::cell::RefCell::new(std::collections::HashMap::new()),
            cuts: std::cell::Cell::new(0),
            fuel: std::cell::Cell::new(REF_FUEL),
            force_outline: std::cell::Cell::new(false),
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
    let q = xsvg_core::QualityProfile::parse(quality);
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
            sourcemap,
            resolving: std::cell::RefCell::new(Vec::new()),
            resolved: std::cell::RefCell::new(std::collections::HashMap::new()),
            cuts: std::cell::Cell::new(0),
            fuel: std::cell::Cell::new(REF_FUEL),
            force_outline: std::cell::Cell::new(false),
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
/// The reference this node **bakes** at compile time, if any: an `in="#id"`
/// attribute (textbox / textpath / warp bend), or the `href` of a `<use>` that
/// is a direct child of `<x:boolean>` (an operand by reference — §7.4). A
/// passthrough `<use>` anywhere else is a live reference the browser resolves,
/// so it is deliberately not reported.
fn baked_ref<'a>(n: roxmltree::Node<'a, 'a>) -> Option<&'a str> {
    if let Some(r) = n.attribute("in") {
        return Some(r);
    }
    if n.tag_name().name() == "use"
        && n.tag_name().namespace() != Some(XSVG_NS)
        && n.parent().is_some_and(|p| {
            p.tag_name().namespace() == Some(XSVG_NS) && p.tag_name().name() == "boolean"
        })
    {
        return n
            .attribute("href")
            .or_else(|| n.attribute((XLINK_NS, "href")));
    }
    None
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
                baked_ref(n)
                    .map(|r| live.contains(&r.strip_prefix('#').unwrap_or(r)))
                    .unwrap_or(false)
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
    out.push_str(&format!(
        "<filter id=\"{id}\" color-interpolation-filters=\"sRGB\" x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"120%\">{}</filter>",
        filter_primitives(&fns)
    ));
    Some(format!(" filter=\"url(#{id})\""))
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

    // xsvg extension elements.
    if node.tag_name().namespace() == Some(XSVG_NS) {
        match node.tag_name().name() {
            "textbox" => emit_textbox(node, out, ctx),
            "textpath" => emit_textpath(node, out, ctx),
            "warp" => emit_warp(node, out, ctx),
            "boolean" => emit_boolean(node, out, ctx),
            "mesh" => emit_mesh(node, out, ctx),
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

    // <xsvg> root is just an alias for <svg>.
    let name = match node.tag_name().name() {
        "xsvg" => "svg",
        other => other,
    };

    if name == "text" && node.attribute("inline-size").is_some() {
        emit_inline_size_text(node, out, ctx);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, ctx);
        return;
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
    let stroke = outline_stroke_attrs(node);
    let pos = pos_attr(node, ctx);

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
    let stroke = outline_stroke_attrs(node);
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

/// `<x:warp field="…" bend="…" axis="h|v">` (§7.3): the generic geometry-warp
/// front-end. Children lower to pure `<path>` geometry, their union bbox builds the
/// field's envelope frame, and every path bakes through the §7.1 pipeline at the
/// quality tolerance. Children that cannot become path geometry (live text, rounded
/// rects, lines, images) are skipped with a marker — a warp never *silently* emits
/// unwarped content; an unknown/absent field or empty geometry emits the children
/// unwarped behind a marker. The element's own paint / `transform` ride on the
/// emitted `<g>` (an affine `transform` composes after the bake for free).
fn emit_warp(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let mut inner = String::new();
    for child in node.children().filter(|c| c.is_element()) {
        match warp_child_markup(child, ctx) {
            Ok(s) => inner.push_str(&s),
            Err(why) => inner.push_str(&format!(
                "<!-- xsvg: <x:warp> skipped <{}>: {why} -->",
                child.tag_name().name()
            )),
        }
    }

    // the pre-warp union bbox of all path geometry = the envelope frame (§7.2)
    let ranges = find_path_d_ranges(&inner);
    let bbox = ranges
        .iter()
        .fold(None, |acc: Option<xsvg_core::kurbo::Rect>, &(a, b)| match (
            acc,
            svg_path_bbox(&inner[a..b]),
        ) {
            (Some(r), Some(n)) => Some(r.union(n)),
            (acc, n) => acc.or(n),
        });

    let field_name = node.attribute("field").unwrap_or("");
    let bend = attr_num(node, "bend", 0.0) / 100.0; // authored as −100…100 %
    let axis = WarpAxis::parse(node.attribute("axis").unwrap_or("h"));
    // the base field: an envelope preset, a corner-driven map, a spine follow, or
    // seeded noise (§7.3)
    let base: Option<Box<dyn Field>> = bbox.and_then(|b| match field_name {
        "perspective" => parse_corners(node)
            .and_then(|t| Homography::new(b, t))
            .map(|h| Box::new(h) as Box<dyn Field>),
        "free" => parse_corners(node)
            .and_then(|t| FreeDistort::new(b, t))
            .map(|f| Box::new(f) as Box<dyn Field>),
        "bend" => {
            // flow the children along a referenced spine (Inkscape's LPE Bend): the
            // envelope's left edge starts on the spine (placed by align/start) and
            // its vertical midline rides it
            let frame = node
                .attribute("in")
                .and_then(|r| ref_geometry(node, r, ctx).ok())
                .and_then(|d| PathFrame::new(&d, ctx.quality.tolerance()))?;
            let s0 = run_offset(
                frame.len(),
                b.width(),
                node.attribute("align").unwrap_or("start"),
                attr_num(node, "start", 0.0),
            );
            let anchor = xsvg_core::kurbo::Point::new(b.min_x(), b.center().y);
            Some(Box::new(BendField { frame, s0, anchor }) as Box<dyn Field>)
        }
        "roughen" => Some(
            Box::new(RoughenField::new(bend, attr_num(node, "detail", 10.0), b)) as Box<dyn Field>,
        ),
        name => EnvelopePreset::new(name, bend, axis, b).map(|p| Box::new(p) as Box<dyn Field>),
    });
    // the Warp-Options distortion sliders compose a projective taper after the field
    let dh = attr_num(node, "distort-h", 0.0) / 100.0;
    let dv = attr_num(node, "distort-v", 0.0) / 100.0;
    let field: Option<Box<dyn Field>> = match (base, bbox) {
        (Some(f), Some(b)) if dh != 0.0 || dv != 0.0 => {
            Some(Box::new(Chain(vec![f, Box::new(Taper::new(b, dh, dv))])))
        }
        (f, _) => f,
    };

    out.push_str("<g");
    copy_attrs(
        node,
        out,
        &[
            "field",
            "bend",
            "axis",
            "corners",
            "distort-h",
            "distort-v",
            "in",
            "detail",
            "align",
            "start",
        ],
    );
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    match field {
        Some(f) => {
            let tol = ctx.quality.tolerance();
            let mut last = 0;
            for (a, b) in ranges {
                out.push_str(&inner[last..a]);
                // a path that fails to bake keeps its original geometry (§4
                // totality). Refit is DISABLED (see §7.1): kurbo's fitter
                // overshoots on dense glyph outlines and dominates compile time.
                match warp_svg_path(&inner[a..b], f.as_ref(), tol, false) {
                    Some(d) => out.push_str(&d),
                    None => out.push_str(&inner[a..b]),
                }
                last = b;
            }
            out.push_str(&inner[last..]);
        }
        None => {
            out.push_str(&format!(
                "<!-- xsvg: <x:warp field=\"{field_name}\"> unknown field, bad corners, or no geometry — children unwarped -->"
            ));
            out.push_str(&inner);
        }
    }
    out.push_str("</g>");
}

/// `corners="x0,y0 x1,y1 x2,y2 x3,y3"` — the four target corners (**TL TR BR BL**)
/// for `field="perspective"` / `"free"`. `None` unless exactly 8 finite numbers.
fn parse_corners(node: roxmltree::Node) -> Option<[xsvg_core::kurbo::Point; 4]> {
    let nums: Vec<f64> = node
        .attribute("corners")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .filter_map(parse_num)
        .collect();
    if nums.len() != 8 {
        return None;
    }
    let p = |i: usize| xsvg_core::kurbo::Point::new(nums[2 * i], nums[2 * i + 1]);
    Some([p(0), p(1), p(2), p(3)])
}

/// Lower one `<x:warp>` child to pre-warp markup whose geometry is all `<path d>`:
/// basic shapes convert directly (the sharp-`rect` pass and `shape_to_path_d`),
/// everything else runs through the normal pipeline (so `outline="true"` text and
/// nested `<x:warp>`s compose). `Err(reason)` when the result still contains
/// geometry the bake cannot warp.
fn warp_child_markup(child: roxmltree::Node, ctx: &Ctx) -> Result<String, String> {
    let name = child.tag_name().name();
    if child.tag_name().namespace() != Some(XSVG_NS)
        && matches!(name, "circle" | "ellipse" | "polygon" | "polyline")
    {
        let d = shape_to_path_d(child).ok_or("degenerate shape")?;
        let mut s = String::from("<path");
        copy_attrs(child, &mut s, &["cx", "cy", "r", "rx", "ry", "points"]);
        s.push_str(&pos_attr(child, ctx));
        s.push_str(&format!(" d=\"{d}\"/>"));
        return Ok(s);
    }
    let mut buf = String::new();
    serialize(child, &mut buf, false, ctx);
    for tag in [
        "text", "rect", "circle", "ellipse", "line", "image", "use", "polygon", "polyline",
    ] {
        if has_tag(&buf, tag) {
            return Err(format!("lowers to <{tag}> — needs path/outline form"));
        }
    }
    Ok(buf)
}

/// Whether `s` contains an opening `<tag …>` element (tag-boundary-aware, so
/// `"text"` does not match `<textArea>` and `"line"` not `<linearGradient>`).
fn has_tag(s: &str, tag: &str) -> bool {
    let needle = format!("<{tag}");
    let mut i = 0;
    while let Some(pos) = s[i..].find(&needle) {
        let after = i + pos + needle.len();
        match s.as_bytes().get(after) {
            Some(b' ') | Some(b'>') | Some(b'/') | None => return true,
            _ => i = after,
        }
    }
    false
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

/// `<x:boolean op="union|intersect|subtract|exclude">` (§7.5): Pathfinder-style
/// path algebra. Each element child is one **operand** (lowered to path geometry
/// exactly like `<x:warp>` children — shapes convert, text participates outlined,
/// nested `x:` elements compose); `subtract` removes every later operand from the
/// first (*Minus Front*), the other ops fold symmetrically. Operands flatten at
/// the profile tolerance; the ops are integer-exact and deterministic. The result
/// is one region: paint comes from the element itself (per-child paint is
/// ignored), a legitimately empty result emits an empty `<g>`, and an unknown
/// `op` degrades behind a marker with the children un-operated.
fn emit_boolean(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    // lower each child to path markup; one child = one operand. A <use href>
    // child is an operand **by reference** (§7.4): the target's geometry (§4
    // reference resolution — compiled output for x: targets) joins the algebra
    // while the target itself keeps rendering wherever it is.
    let mut markups: Vec<(String, bool)> = Vec::new(); // (markup, even_odd)
    let mut markers = String::new();
    for child in node.children().filter(|c| c.is_element()) {
        let even_odd = child.attribute("fill-rule") == Some("evenodd");
        if child.tag_name().name() == "use" && child.tag_name().namespace() != Some(XSVG_NS) {
            let href = child
                .attribute("href")
                .or_else(|| child.attribute((XLINK_NS, "href")));
            let placed = href
                .ok_or(RefFail::NotFound)
                .and_then(|r| ref_geometry(child, r, ctx))
                .and_then(|d| transform_d(&d, use_placement(child)).ok_or(RefFail::NoGeometry));
            match placed {
                Ok(d) => markups.push((format!("<path d=\"{d}\"/>"), even_odd)),
                Err(f) => markers.push_str(&format!(
                    "<!-- xsvg: <x:boolean> skipped <use> ({}) -->",
                    f.reason()
                )),
            }
            continue;
        }
        match warp_child_markup(child, ctx) {
            Ok(m) => markups.push((m, even_odd)),
            Err(why) => markers.push_str(&format!(
                "<!-- xsvg: <x:boolean> skipped <{}>: {why} -->",
                child.tag_name().name()
            )),
        }
    }

    let op = BoolOp::parse(node.attribute("op").unwrap_or("union"));
    let Some(op) = op else {
        // unknown op → children un-operated behind a marker (never silent)
        out.push_str(&format!(
            "<!-- xsvg: <x:boolean op=\"{}\"> unknown op — children un-combined -->",
            node.attribute("op").unwrap_or("")
        ));
        out.push_str(&markers);
        out.push_str("<g");
        copy_attrs(node, out, &["op"]);
        out.push_str(&pos_attr(node, ctx));
        out.push('>');
        for (m, _) in &markups {
            out.push_str(m);
        }
        out.push_str("</g>");
        return;
    };

    let operands: Vec<BoolOperand> = markups
        .iter()
        .map(|(m, even_odd)| BoolOperand {
            paths: find_path_d_ranges(m)
                .into_iter()
                .map(|(a, b)| &m[a..b])
                .collect(),
            even_odd: *even_odd,
        })
        .filter(|o| !o.paths.is_empty())
        .collect();

    out.push_str(&markers);
    match boolean_svg_paths(&operands, op, ctx.quality.tolerance()) {
        Some(d) if !d.is_empty() => {
            out.push_str("<path");
            copy_attrs(node, out, &["op"]);
            out.push_str(&pos_attr(node, ctx));
            out.push_str(&format!(" d=\"{d}\"/>"));
        }
        Some(_) => {
            // a legitimately empty result (e.g. disjoint intersect)
            out.push_str("<g");
            copy_attrs(node, out, &["op"]);
            out.push_str(&pos_attr(node, ctx));
            out.push_str("/>");
        }
        None => {
            out.push_str("<!-- xsvg: <x:boolean> no usable geometry -->");
        }
    }
}

/// `<x:mesh>` (§8.2) — a quad-dominant mesh gradient with per-corner colors and
/// cracks, lowered by the two-stage pipeline: (1) rasterize the mesh in memory
/// (linear-light, crack-respecting region labels), (2) refit each region with a
/// seam-free shared-vertex GridField grown until the residual passes the
/// profile tolerance, then serialize each region as a **tiny PNG** placed so
/// its texel centers land on the grid vertices — the renderer's own bilinear
/// image filter reconstructs the field (a single patch is exactly a stretched
/// 2×2). Regions are clipped by the exact union of their face polygons
/// (nonzero), so cracks stay geometry-sharp regardless of raster resolution.
fn emit_mesh(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    use xsvg_core::gradient;
    use xsvg_core::gradient::{fit_field, fit_grid, texel_placement, Dof, Mesh};

    // ---- parse: <x:verts> + <x:face v=".." fill="..">
    let mut mesh = Mesh::default();
    let mut markers = String::new();
    for child in node.children().filter(|c| c.is_element()) {
        if child.tag_name().name() == "verts" {
            let text = child.text().unwrap_or("");
            let nums: Vec<f32> = text
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|t| !t.is_empty())
                .filter_map(|t| t.parse::<f32>().ok().filter(|v| v.is_finite()))
                .collect();
            for pair in nums.chunks_exact(2) {
                mesh.add_vertex(pair[0], pair[1]);
            }
        }
    }
    let nv = mesh.verts.len() as u32;
    for child in node.children().filter(|c| c.is_element()) {
        if child.tag_name().name() != "face" {
            continue;
        }
        let idx: Vec<u32> = child
            .attribute("v")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        let cols: Vec<xsvg_core::gradient::LinRgb> = child
            .attribute("fill")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(parse_hex_color)
            .collect();
        let ok_idx = (idx.len() == 3 || idx.len() == 4) && idx.iter().all(|&i| i < nv);
        let ok_col = cols.len() == idx.len() || cols.len() == 1;
        if !ok_idx || !ok_col || cols.is_empty() {
            markers.push_str("<!-- xsvg: <x:face> skipped (bad indices or colors) -->");
            continue;
        }
        let col = |k: usize| if cols.len() == 1 { cols[0] } else { cols[k] };
        if idx.len() == 4 {
            mesh.add_quad(
                [idx[0], idx[1], idx[2], idx[3]],
                [col(0), col(1), col(2), col(3)],
            );
        } else {
            mesh.add_tri([idx[0], idx[1], idx[2]], [col(0), col(1), col(2)]);
        }
    }
    out.push_str(&markers);
    if mesh.faces.is_empty() {
        out.push_str("<!-- xsvg: <x:mesh> no usable faces -->");
        return;
    }

    // ---- stage 1: rasterize over the mesh bbox at a profile-graded resolution
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &mesh.verts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if !(x1 > x0 && y1 > y0) {
        out.push_str("<!-- xsvg: <x:mesh> degenerate extent -->");
        return;
    }
    let max_px = match ctx.quality {
        QualityProfile::Fast => 64.0f32,
        QualityProfile::Balanced => 128.0,
        QualityProfile::Highest | QualityProfile::Raster => 256.0,
    };
    // resolution from the LONG axis, but never starve the short one below
    // ~32px — a thin strip still needs rows for its cross-axis fit
    let (dim_max, dim_min) = ((x1 - x0).max(y1 - y0), (x1 - x0).min(y1 - y0));
    let scale = (dim_max / max_px).min(dim_min / 32.0).max(1e-6);
    let (w, h) = (
        (((x1 - x0) / scale).ceil() as usize).max(1),
        (((y1 - y0) / scale).ceil() as usize).max(1),
    );
    let raster = mesh.rasterize(w, h, (x0, y0), scale, 1e-3);
    let srgb = raster.to_srgb8();

    // per-region pixel index lists + pixel bboxes
    let mut region_px: Vec<Vec<u32>> = vec![Vec::new(); raster.regions];
    let mut bbox: Vec<(u32, u32, u32, u32)> = vec![(u32::MAX, u32::MAX, 0, 0); raster.regions];
    for (i, &l) in raster.labels.iter().enumerate() {
        if l == gradient::mesh::NONE {
            continue;
        }
        region_px[l as usize].push(i as u32);
        let (px, py) = ((i % w) as u32, (i / w) as u32);
        let b = &mut bbox[l as usize];
        b.0 = b.0.min(px);
        b.1 = b.1.min(py);
        b.2 = b.2.max(px);
        b.3 = b.3.max(py);
    }

    // rmse tolerance (sRGB units) and grid cap by profile
    let (tol, cap) = match ctx.quality {
        QualityProfile::Fast => (4.0f32, 10usize),
        QualityProfile::Balanced => (1.5, 24),
        QualityProfile::Highest | QualityProfile::Raster => (0.75, 32),
    };

    out.push_str("<g");
    copy_attrs(node, out, &[]);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    let mesh_pos = node.range().start;
    for r in 0..raster.regions {
        if region_px[r].is_empty() {
            continue;
        }
        // exact clip geometry: union of the region's face polygons (nonzero)
        let mut clip_d = String::new();
        for (f, face) in mesh.faces.iter().enumerate() {
            if raster.face_regions[f] != r as u32 {
                continue;
            }
            let n = face.arity();
            for c in 0..n {
                let (px, py) = mesh.verts[face.v[c] as usize];
                clip_d.push(if c == 0 { 'M' } else { 'L' });
                clip_d.push_str(&format!("{},{}", fmt(px as f64), fmt(py as f64)));
            }
            clip_d.push('Z');
        }

        let single = fit_field(&region_px[r], w, &srgb, 2.0);
        if single.dof == Dof::Solid {
            let c = single.corners[0];
            out.push_str(&format!(
                "<path fill=\"#{:02x}{:02x}{:02x}\" d=\"{clip_d}\"/>",
                c[0].round().clamp(0.0, 255.0) as u8,
                c[1].round().clamp(0.0, 255.0) as u8,
                c[2].round().clamp(0.0, 255.0) as u8
            ));
            continue;
        }

        // grow the shared-vertex grid until the residual passes the tolerance
        let (bx0, by0, bx1, by1) = bbox[r];
        let ar = ((bx1 - bx0).max(1) as f32) / ((by1 - by0).max(1) as f32);
        let mut best = None;
        for g in [1usize, 2, 3, 4, 6, 8, 12, 16, 24, 32] {
            let gx = ((g as f32 * ar.sqrt()).round() as usize).clamp(1, cap);
            let gy = ((g as f32 / ar.sqrt()).round() as usize).clamp(1, cap);
            let grid = fit_grid(&region_px[r], w, &srgb, gx, gy);
            let done = grid.rmse <= tol || g >= cap;
            best = Some(grid);
            if done {
                break;
            }
        }
        let grid = best.unwrap();

        // tiny PNG: (gx+1)×(gy+1) texels, one per grid vertex
        let (tw, th) = (grid.gx + 1, grid.gy + 1);
        let mut rgb = Vec::with_capacity(tw * th * 3);
        for vert in &grid.verts {
            for c in 0..3 {
                rgb.push(vert[c].round().clamp(0.0, 255.0) as u8);
            }
        }
        let png = gradient::png::encode_rgb_png(tw as u32, th as u32, &rgb);
        let (ix, iy, iw, ih) = texel_placement(bx0, by0, bx1, by1, tw, th);
        // raster pixel space -> user units
        let (ux, uy) = (x0 as f64 + ix * scale as f64, y0 as f64 + iy * scale as f64);
        let (uw, uh) = (iw * scale as f64, ih * scale as f64);
        let cid = format!("x-mesh-{mesh_pos}-{r}");
        out.push_str(&format!(
            "<clipPath id=\"{cid}\"><path d=\"{clip_d}\"/></clipPath><image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" clip-path=\"url(#{cid})\" href=\"data:image/png;base64,{}\"/>",
            fmt(ux),
            fmt(uy),
            fmt(uw),
            fmt(uh),
            gradient::base64::encode(&png)
        ));
    }
    out.push_str("</g>");
}

/// `#rgb` / `#rrggbb` → linear-light RGB.
fn parse_hex_color(s: &str) -> Option<xsvg_core::gradient::LinRgb> {
    let hex = s.strip_prefix('#')?;
    let byte = |a: u8, b: u8| {
        let hi = (a as char).to_digit(16)?;
        let lo = (b as char).to_digit(16)?;
        Some((hi * 16 + lo) as u8)
    };
    let b = hex.as_bytes();
    let (r, g, bl) = match b.len() {
        3 => (byte(b[0], b[0])?, byte(b[1], b[1])?, byte(b[2], b[2])?),
        6 => (byte(b[0], b[1])?, byte(b[2], b[3])?, byte(b[4], b[5])?),
        _ => return None,
    };
    Some(xsvg_core::gradient::RgbColor::new(r, g, bl).to_linear())
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

/// Parse an SVG `transform` list into a kurbo [`Affine`] (§4 reference
/// resolution honors the target's own transform). Mirrors browser behavior for
/// invalid input: the whole list is ignored (identity).
fn parse_transform(s: &str) -> xsvg_core::kurbo::Affine {
    use xsvg_core::kurbo::Affine;
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

/// Apply an affine to path data. `None` if `d` fails to parse.
fn transform_d(d: &str, a: xsvg_core::kurbo::Affine) -> Option<String> {
    if a == xsvg_core::kurbo::Affine::IDENTITY {
        return Some(d.to_string());
    }
    let mut p = xsvg_core::kurbo::BezPath::from_svg(d).ok()?;
    p.apply_affine(a);
    Some(p.to_svg())
}

/// A `<use>` operand's placement: its `transform` composed with the extra
/// `x`/`y` translation (per SVG, x/y append a translate after the transform).
fn use_placement(use_el: roxmltree::Node) -> xsvg_core::kurbo::Affine {
    let t = use_el
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or(xsvg_core::kurbo::Affine::IDENTITY);
    t * xsvg_core::kurbo::Affine::translate((
        attr_num(use_el, "x", 0.0),
        attr_num(use_el, "y", 0.0),
    ))
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
        .unwrap_or(xsvg_core::kurbo::Affine::IDENTITY);
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
        &outline_stroke_attrs(node),
        gx,
        m,
        &emits,
        &pos_attr(node, ctx),
        outline,
        ctx.outliner,
    );
}

/// Paint attributes carried onto an outlined `<g>` beyond `fill` — stroke and
/// paint-order — so create-outlines can render a stroked / keyline outline. Empty
/// for the common fill-only case; only meaningful when `outline="true"`.
fn outline_stroke_attrs(node: roxmltree::Node) -> String {
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
        fill
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
fn copy_attrs(node: roxmltree::Node, out: &mut String, skip: &[&str]) {
    for attr in node.attributes() {
        if skip.contains(&attr.name()) {
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

    /// No glyph outliner (the default for tests not exercising `outline`).
    struct NoOutliner;
    impl GlyphOutliner for NoOutliner {
        fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, _x: f64, _b: f64) -> Option<String> {
            None
        }
    }

    /// Stub outliner: a deterministic 1×1 box path at the run origin (so outline
    /// emit paths can be exercised without a real font) and Mono-consistent advance
    /// widths (chars × 0.5 × size), matching the `Mono` measurer.
    struct BoxOutliner;
    impl GlyphOutliner for BoxOutliner {
        fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, x: f64, b: f64) -> Option<String> {
            Some(format!("M{x},{b} h1 v-1 h-1 Z"))
        }
        fn advance_width(&self, text: &str, _s: &TextStyle, size: f64) -> Option<f64> {
            Some(text.chars().count() as f64 * 0.5 * size)
        }
    }

    fn compile_test(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap()
    }

    /// Compile with the 60×60 `BoxShaper`, for `<x:textbox in>` region-flow tests.
    fn compile_shaped(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &BoxShaper, &NoOutliner).unwrap()
    }

    /// Compile with the stub `BoxOutliner`, for `outline` (create-outlines) tests.
    fn compile_outlined(svg: &str) -> String {
        compile_impl(svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap()
    }

    /// Compile with the source map on (`data-xsvg-pos` attributes emitted).
    fn compile_mapped(svg: &str) -> String {
        compile_impl(svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap()
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
        assert!(compile_impl(
            "<svg><unclosed></svg>",
            "balanced",
            false,
            &Mono,
            &NoShaper,
            &NoOutliner
        )
        .is_err());
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
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:sparkle/></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<!-- xsvg: <x:sparkle> not yet lowered -->"),
            "{out}"
        );
        // an empty mesh is KNOWN but has nothing to lower — its own marker
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:mesh/></svg>"#;
        let out = compile_test(svg);
        assert!(
            out.contains("<!-- xsvg: <x:mesh> no usable faces -->"),
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
        let err = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap_err();
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
        assert!(compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
        // and modest legitimate nesting is fine
        let nested = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
            "<g>".repeat(64),
            "</g>".repeat(64)
        );
        assert!(compile_impl(&nested, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
    }

    #[test]
    fn glyph_x_scale_non_positive_is_ignored() {
        // 0 and negative scales must not emit a zero/negative textLength or NaN.
        for v in ["0", "-1.5"] {
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"https://xsvg.visioncortex.org\"><textArea x=\"0\" y=\"10\" font-size=\"10\" x:glyph-x-scale=\"{v}\">hello</textArea></svg>"
            );
            let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
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
    fn styled_tspan_runs_emit_nested_tspans() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10" fill="#111">Ship <tspan font-weight="bold" fill="#e11">fast</tspan> today</textArea></svg>"##;
        let out = compile_test(svg);
        // base attrs live on the <text>; the bold run is a nested <tspan> with overrides
        assert!(out.contains(r##"fill="#111""##), "base fill missing: {out}");
        assert_eq!(out.matches("font-weight=\"bold\"").count(), 1, "{out}");
        assert!(out.contains(r##"fill="#e11""##), "run fill missing: {out}");
        assert!(out.contains("fast"), "{out}");
        // base-styled parts stay bare text (not wrapped)
        assert!(out.contains("Ship ") && out.contains("today"));
    }

    #[test]
    fn plain_textarea_has_no_inner_tspans() {
        // no runs → one outer <tspan> per line, nothing nested
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="500" font-size="10">hello world</textArea></svg>"#;
        assert_eq!(compile_test(svg).matches("<tspan").count(), 1);
    }

    #[test]
    fn outline_emits_paths_not_text() {
        // outline="true" with a working outliner → <g><path>, no <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="#111">Hi</x:textbox></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"#111\""), "{out}");
        assert!(out.contains("<path d=\""), "{out}");
        assert!(!out.contains("<text") && !out.contains("<x:textbox"));
    }

    #[test]
    fn textarea_x_outline_emits_paths() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="200" font-size="10" x:outline="true">one two</textArea></svg>"##;
        let out = compile_outlined(svg);
        assert!(
            out.contains("<path d=\"") && !out.contains("<text"),
            "{out}"
        );
    }

    #[test]
    fn outline_carries_stroke_onto_the_group() {
        // stroke/stroke-width on an outlined box propagate to the outline <g> (a
        // keyline outline), and fill="none" is honored — the live <text> branch is
        // unaffected (it has no stroke path).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="none" stroke="#fff" stroke-width="1.5" stroke-linejoin="round">Hi</x:textbox></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"none\""), "{out}");
        assert!(out.contains("stroke=\"#fff\""), "{out}");
        assert!(out.contains("stroke-width=\"1.5\""), "{out}");
        assert!(out.contains("stroke-linejoin=\"round\""), "{out}");
        assert!(out.contains("<path d=\""), "{out}");
    }

    #[test]
    fn outline_falls_back_to_text_without_a_font() {
        // outline requested but the outliner has no font (returns None) → live <text>
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true">Hi</x:textbox></svg>"##;
        let out = compile_test(svg); // NoOutliner
        assert!(
            out.contains("<text") && !out.contains("<path d=\""),
            "{out}"
        );
    }

    #[test]
    fn textpath_emits_warped_path() {
        // <x:textpath in="#p"> with an outline font → <g><path>, no <text>; the
        // native skew field lands the run's baseline on the path (f(0) = 20)
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 C40,0 80,40 120,20" fill="none"/><x:textpath in="#p" effect="skew" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"#111\""), "{out}");
        assert!(out.contains("<path d=\"M0,20"), "{out}");
        assert!(!out.contains("<text"), "{out}");
    }

    #[test]
    fn textpath_carries_stroke() {
        // fill="none" + stroke → a keyline outline on the warped path
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="none" stroke="#0af" stroke-width="1.5">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<g fill=\"none\""), "{out}");
        assert!(out.contains("stroke=\"#0af\""), "{out}");
    }

    #[test]
    fn textpath_without_a_font_degrades_to_stepped_text() {
        // No outline font, but the native height profile still works → the skew run
        // degrades to stepped live <text> (§6.13.3), not a flat line. Mono at size
        // 20: "wave" prefix advances 0/10/20/30; the flat path pins y at 20.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20">wave</x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains(">wave</text>"), "{out}");
        assert!(out.contains(r#"x="0 10 20 30""#), "{out}");
        assert!(out.contains(r#"y="20 20 20 20""#), "{out}");
    }

    #[test]
    fn textpath_missing_target_is_a_comment() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textpath in="#nope" font-size="20">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(
            out.contains("<!-- xsvg: <x:textpath in> target not found"),
            "{out}"
        );
    }

    #[test]
    fn textpath_baseline_shift_lifts_the_run() {
        // rainbow + baseline-shift 8 on a flat path at y = 20 → baseline at y = 12
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="8" font-size="20" fill="#111">arc</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<path d=\"M0,12"), "{out}");
    }

    #[test]
    fn textpath_defaults_to_skew_on_the_path() {
        // no effect / baseline-shift attributes → skew at the path's start
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<path d=\"M0,20"), "{out}");
    }

    #[test]
    fn textpath_align_and_start_place_the_run() {
        // extent 120, advance("wave") = 40 → middle slack 40, +12 head-start → x = 52
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" align="middle" start="12" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<path d=\"M52,20"), "{out}");
    }

    #[test]
    fn textpath_skew_degrades_to_stepped_baseline() {
        // No outline font → the native height profile places each glyph (Stair Step,
        // §6.13.1). Mono(0.5·size): "abc" prefix advances 0/5/10; start=10 offsets;
        // the flat path pins y at 50, baseline-shift 2 lifts to 48.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" baseline-shift="2" start="10" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains(r#"x="10 15 20""#), "{out}");
        assert!(out.contains(r#"y="48 48 48""#), "{out}");
        assert!(out.contains(">abc</text>"), "{out}");

        // letter-spacing widens the per-glyph gaps in the baked positions (§6.8)
        let ls = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" letter-spacing="2" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_test(ls);
        assert!(out.contains(r#"x="0 7 14""#), "{out}");
    }

    #[test]
    fn textpath_stair_is_an_authored_effect() {
        // effect="stair" chooses the stepped live-<text> lowering even when an
        // outline font IS available — it never consults the outliner (§6.13.3).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="stair" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains(r#"x="0 5 10""#), "{out}");
        assert!(out.contains(r#"y="50 50 50""#), "{out}");
        assert!(!out.contains("<path d=\""), "outliner ran: {out}");

        // a degenerate (zero-length) target path → straight <text>, never a panic
        let degen = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="stair" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_outlined(degen);
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
    }

    #[test]
    fn textpath_rainbow_without_font_stays_straight() {
        // stepped degradation is skew/stair-only; rainbow with no outliner → straight
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="10" fill="#111">abc</x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
        assert!(!out.contains("<path d=\""), "{out}");
    }

    #[test]
    fn textpath_stepped_skips_empty_and_whitespace_text() {
        // nothing to place → the straight fallback, never a panic or an empty x list
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" font-size="10">   </x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains(r#"x="0" y="0""#), "{out}");
    }

    #[test]
    fn textpath_degenerate_input_never_panics_or_leaks_nan() {
        // Degenerate baseline-shift values (garbage, ±inf, NaN) collapse to 0 —
        // the output matches an unshifted run and never leaks NaN (§4 totality).
        let plain = compile_outlined(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20">wave</x:textpath></svg>"##,
        );
        for bad in ["garbage", "1e999", "-1e999", "NaN", "inf"] {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="{bad}" font-size="20">wave</x:textpath></svg>"##
            );
            let out = compile_outlined(&svg);
            assert_eq!(out, plain, "shift={bad}");
            assert!(!out.contains("NaN"), "shift={bad}: {out}");
        }
        // A unit suffix parses its numeric prefix (13px → 13), like every length attr.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="13px" font-size="20">wave</x:textpath></svg>"##;
        assert!(compile_outlined(svg).contains("<path d=\"M0,7"));
    }

    #[test]
    fn textpath_empty_and_degenerate_targets_do_not_panic() {
        // empty run → still total (the stub warps ""; a real backend may fall back)
        let empty = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20"></x:textpath></svg>"##;
        compile_outlined(empty);
        // zero-length path data still reaches the seam verbatim (degeneracy is the
        // backend's to detect — it returns None and the element falls back)
        let point = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20">dot</x:textpath></svg>"##;
        compile_outlined(point);
        // a <line> target is a valid reference path per §6.13 (was a gap until
        // shape_to_path_d grew a line arm): text warps onto it, no marker
        let line = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><line id="p" x1="0" y1="20" x2="120" y2="20"/><x:textpath in="#p" effect="rainbow" font-size="20">hi</x:textpath></svg>"##;
        let out = compile_outlined(line);
        assert!(!out.contains("not found or not a path"), "{out}");
        assert!(out.contains("<path"), "{out}");
    }

    // ---- incremental compilation (docs/Incremental.md) ----

    /// A document exercising every emitter family: passthrough, textbox, a
    /// referenced path + its textpath, a warp, and a boolean.
    const INC: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 400 300">
  <rect x="5" y="5" width="40" height="20" fill="#eee"/>
  <x:textbox x="0" y="0" width="80" height="30" font-size="10" fill="#111">hello world</x:textbox>
  <path id="wave" d="M0,100 C40,60 80,140 120,100" fill="none"/>
  <x:textpath in="#wave" font-size="12" fill="#222">on the wave</x:textpath>
  <x:warp field="arch" bend="40"><rect x="0" y="200" width="100" height="30" fill="#333"/></x:warp>
  <x:boolean op="subtract" fill="#444"><rect x="200" y="200" width="60" height="30"/><rect x="230" y="200" width="60" height="30"/></x:boolean>
  <x:boolean id="blob" op="union" fill="#555"><rect x="300" y="240" width="40" height="50"/><rect x="320" y="240" width="40" height="50"/></x:boolean>
  <x:textbox in="#blob" font-size="10" fill="#666">into the blob</x:textbox>
  <x:boolean op="intersect" fill="#777"><use href="#blob"/><rect x="310" y="250" width="40" height="30"/></x:boolean>
  <rect x="5" y="270" width="30" height="20" fill="#48a" filter="brightness(1.2) saturate(0.8)"/>
  <x:mesh><x:verts>340,240 380,240 380,280 340,280</x:verts><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #ff0"/></x:mesh>
</svg>"##;

    /// Byte offsets of every top-level element in `INC`, via the parser itself.
    fn top_level_offsets(input: &str) -> Vec<usize> {
        let doc = roxmltree::Document::parse(input).unwrap();
        doc.root_element()
            .children()
            .filter(|c| c.is_element())
            .map(|c| c.range().start)
            .collect()
    }

    #[test]
    fn fragments_are_verbatim_slices_of_the_full_compile() {
        // THE incremental invariant: emission is a pure function of the subtree
        // (plus its references), so each fragment must appear byte-identically —
        // and in order — inside the full compile. If a future emitter introduces
        // cross-sibling state, this test is the canary.
        for sourcemap in [false, true] {
            let full =
                compile_impl(INC, "balanced", sourcemap, &Mono, &BoxShaper, &BoxOutliner).unwrap();
            let mut cursor = 0;
            for off in top_level_offsets(INC) {
                let frag = compile_fragment_impl(
                    INC,
                    "balanced",
                    sourcemap,
                    off,
                    &Mono,
                    &BoxShaper,
                    &BoxOutliner,
                )
                .unwrap();
                assert!(!frag.is_empty());
                let at = full[cursor..]
                    .find(&frag)
                    .unwrap_or_else(|| panic!("fragment not found in order: {frag}\n{full}"));
                cursor += at + frag.len();
            }
        }
    }

    #[test]
    fn fragment_range_and_offsets_resolve() {
        let offs = top_level_offsets(INC);
        for &off in &offs {
            let (s, e) = fragment_range_impl(INC, off).unwrap();
            assert_eq!(s, off);
            assert!(e > s);
            // any offset inside the element resolves to the same fragment
            assert_eq!(fragment_range_impl(INC, off + 1), Some((s, e)));
        }
        // offset in inter-element whitespace or the root tag → no fragment
        assert_eq!(fragment_range_impl(INC, 0), None);
        assert!(
            compile_fragment_impl(INC, "balanced", false, 0, &Mono, &NoShaper, &NoOutliner)
                .is_err()
        );
    }

    #[test]
    fn dependents_of_nothing_is_nothing() {
        assert!(dependents_impl("<svg", 2).is_empty()); // malformed input
        assert!(dependents_impl(INC, 0).is_empty()); // offset in the root tag
    }

    #[test]
    fn dependents_track_baked_in_references() {
        let offs = top_level_offsets(INC);
        // editing the referenced #wave path invalidates the textpath that bakes it
        let wave_off = offs[2];
        let deps = dependents_impl(INC, wave_off);
        assert_eq!(deps.len(), 1, "{deps:?}");
        assert_eq!(deps[0].0, offs[3], "expected the textpath: {deps:?}");
        // an unreferenced element invalidates nothing
        assert!(dependents_impl(INC, offs[0]).is_empty());
        // the dependent itself has no dependents
        assert!(dependents_impl(INC, offs[3]).is_empty());
        // an x: element can be a target: editing the #blob boolean invalidates
        // the textbox that flows inside its compiled output AND the boolean
        // holding #blob as a <use> operand (a baked href, unlike passthrough <use>)
        let deps = dependents_impl(INC, offs[6]);
        assert_eq!(deps.len(), 2, "{deps:?}");
        assert_eq!(deps[0].0, offs[7], "expected the blob textbox: {deps:?}");
        assert_eq!(
            deps[1].0, offs[8],
            "expected the use-operand boolean: {deps:?}"
        );
    }

    #[test]
    fn dependents_closure_is_transitive() {
        // #spine ← warp#bent(in=#spine) ← textpath(in=#bent): editing the spine
        // must re-emit BOTH downstream elements, since the warp's compiled output
        // is itself baked geometry for the textpath.
        let svg = format!(
            r##"{XW}<path id="spine" d="M0,80 C40,20 60,20 100,80" fill="none"/><x:warp id="bent" field="bend" in="#spine" bend="30"><rect x="0" y="40" width="100" height="20" fill="#333"/></x:warp><x:textpath in="#bent" effect="stair" font-size="10" fill="#111">chain</x:textpath></svg>"##
        );
        let offs = top_level_offsets(&svg);
        let starts = |deps: Vec<(usize, usize)>| deps.into_iter().map(|d| d.0).collect::<Vec<_>>();
        assert_eq!(
            starts(dependents_impl(&svg, offs[0])),
            vec![offs[1], offs[2]]
        );
        assert_eq!(starts(dependents_impl(&svg, offs[1])), vec![offs[2]]);
        assert!(dependents_impl(&svg, offs[2]).is_empty());
    }

    #[test]
    fn textbox_flows_inside_a_boolean_union() {
        // the union of two overlapping rects is ONE region; the textbox flows
        // inside the boolean's compiled output, not any single source shape
        let svg = format!(
            r##"{XW}<x:boolean id="blob" op="union" fill="#eee"><rect x="0" y="0" width="40" height="60"/><rect x="20" y="0" width="40" height="60"/></x:boolean><x:textbox in="#blob" font-size="10" fill="#111">alpha beta gamma</x:textbox></svg>"##
        );
        let out = compile_shaped(&svg);
        assert!(!out.contains("not rasterizable"), "{out}");
        assert!(!out.contains("target not found"), "{out}");
        assert!(out.contains("alpha"), "{out}");
    }

    #[test]
    fn textpath_rides_a_warped_spine_not_its_source() {
        // in= on an x:warp must sample the WARPED spine: the stair steps differ
        // from the same textpath bound to the raw source path
        let spine = r##"<path d="M0,80 C40,20 60,20 100,80" fill="none"/>"##;
        let text = r##"effect="stair" font-size="10" fill="#111">abcdef</x:textpath>"##;
        let warped = format!(
            r##"{XW}<x:warp id="s" field="arch" bend="60">{spine}</x:warp><x:textpath in="#s" {text}</svg>"##
        );
        let raw = format!(
            r##"{XW}<path id="s" d="M0,80 C40,20 60,20 100,80" fill="none"/><x:textpath in="#s" {text}</svg>"##
        );
        let w = compile_test(&warped);
        let r = compile_test(&raw);
        assert!(!w.contains("target not found"), "{w}");
        let ys = |out: &str| {
            let text = &out[out.find("<text").unwrap()..];
            let y = &text[text.find(" y=\"").unwrap() + 4..];
            y[..y.find('"').unwrap()].to_string()
        };
        assert_ne!(ys(&w), ys(&r), "stair must step along the warped spine");
    }

    #[test]
    fn reference_cycles_degrade_instead_of_hanging() {
        // mutual references: both degrade with the standard marker, no recursion
        let svg = format!(
            r##"{XW}<x:textpath id="a" in="#b" effect="stair" font-size="10">one</x:textpath><x:textpath id="b" in="#a" effect="stair" font-size="10">two</x:textpath></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("target not found or not a path"), "{out}");
        // self-reference from inside: the cyclic contribution drops out and the
        // textbox flows in the rest of the boolean (the rect)
        let svg = format!(
            r##"{XW}<x:boolean id="a" op="union" fill="#eee"><rect x="0" y="0" width="60" height="60"/><x:textbox in="#a" font-size="10">loop</x:textbox></x:boolean></svg>"##
        );
        let out = compile_shaped(&svg);
        assert!(out.contains("<path"), "{out}");
    }

    #[test]
    fn boolean_use_children_are_operands_by_reference() {
        // venn lens: two circles referenced by <use> — both keep rendering,
        // and the intersect gets their geometry without consuming them
        let svg = format!(
            r##"{XW}<circle id="va" cx="100" cy="100" r="60" fill="#fecaca"/><circle id="vb" cx="150" cy="100" r="60" fill="#bbf7d0"/><x:boolean op="intersect" fill="#818cf8"><use href="#va"/><use href="#vb"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert_eq!(out.matches("<circle").count(), 2, "{out}");
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&out).bounding_box();
        // lens spans [90,160] × [100 ± √(60²−25²) ≈ 54.5]
        assert!(
            (bb.x0 - 90.0).abs() < 1.0 && (bb.x1 - 160.0).abs() < 1.0,
            "{bb:?}"
        );
        assert!(
            (bb.y0 - 45.5).abs() < 1.0 && (bb.y1 - 154.5).abs() < 1.0,
            "{bb:?}"
        );
    }

    #[test]
    fn boolean_use_mixes_with_literals_and_takes_xy_offsets() {
        // literal subject minus a referenced cutter stamped at a <use x y> offset
        let svg = format!(
            r##"{XW}<rect id="cut" x="0" y="0" width="40" height="40"/><x:boolean op="subtract" fill="#123"><rect x="0" y="100" width="100" height="40"/><use href="#cut" x="60" y="100"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        use xsvg_core::kurbo::Shape;
        let p = first_path(&out);
        let bb = p.bounding_box();
        assert!(
            (bb.x1 - 60.0).abs() < 0.5,
            "cutter offset not applied: {bb:?}"
        );
        assert!((p.area().abs() - 2400.0).abs() < 5.0, "{}", p.area());
    }

    #[test]
    fn boolean_use_can_reference_a_compiled_x_target() {
        // operand = another boolean's compiled output (union spans x 0..100)
        let svg = format!(
            r##"{XW}<x:boolean id="u" op="union" fill="#eee"><rect x="0" y="0" width="60" height="40"/><rect x="40" y="0" width="60" height="40"/></x:boolean><x:boolean op="intersect" fill="#345"><use href="#u"/><rect x="80" y="0" width="80" height="40"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        use xsvg_core::kurbo::Shape;
        let p = first_path(&out);
        let bb = p.bounding_box();
        assert!(
            (bb.x0 - 80.0).abs() < 0.5 && (bb.x1 - 100.0).abs() < 0.5,
            "{bb:?}"
        );
        assert!((p.area().abs() - 800.0).abs() < 5.0, "{}", p.area());
    }

    #[test]
    fn boolean_use_missing_target_and_self_reference_degrade() {
        let svg = format!(
            r##"{XW}<x:boolean op="union" fill="#123"><use href="#ghost"/><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("skipped <use>"), "{out}");
        assert!(out.contains("<path"), "{out}"); // the surviving operand still emits
        let svg = format!(
            r##"{XW}<x:boolean id="s" op="union" fill="#123"><use href="#s"/><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg); // terminates; cyclic operand drops out
        assert!(out.contains("<path"), "{out}");
    }

    #[test]
    fn grim_corpus_is_total_and_never_silent() {
        // §3/§4 as a property, swept across every emitter: a degenerate
        // document must COMPILE (no panic), the output must never be empty,
        // must never leak NaN/inf coordinates, and an x: element that cannot
        // do its job must leave a marker — degradation is always acknowledged,
        // never silent.
        let cases: &[&str] = &[
            // x: elements with nothing to work with
            r##"<x:textbox in="#nope">text</x:textbox>"##,
            r##"<x:textpath font-size="10">no in</x:textpath>"##,
            r##"<x:textpath in="#nope" font-size="10">x</x:textpath>"##,
            r##"<x:warp><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="nope" bend="40"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="bend"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:boolean op="union"><g/></x:boolean>"##,
            r##"<x:boolean op="nope"><rect x="0" y="0" width="10" height="10"/></x:boolean>"##,
            r##"<x:boolean op="union"><use/></x:boolean>"##,
            // degenerate numerics
            r##"<x:warp field="arch" bend="NaN"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="arch" bend="1e999"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="roughen" bend="-40" detail="-8"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="perspective" corners="a b c d"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:warp field="arch" bend="40"><rect x="0" y="0" width="0" height="0"/></x:warp>"##,
            // degenerate reference targets
            r##"<path id="t" d=""/><x:textpath in="#t" font-size="10">x</x:textpath>"##,
            r##"<path id="t" d="garbage"/><x:textpath in="#t" font-size="10">x</x:textpath>"##,
            r##"<circle id="t" cx="5" cy="5" r="0"/><x:textbox in="#t">x</x:textbox>"##,
            r##"<text id="t">words</text><x:textpath in="#t" font-size="10">x</x:textpath>"##,
            r##"<g id="t"/><x:boolean op="union"><use href="#t"/></x:boolean>"##,
            r##"<g id="t"><text>only text</text></g><x:textbox in="#t">x</x:textbox>"##,
            // hostile transforms
            r##"<rect id="t" x="0" y="0" width="10" height="10" transform="matrix(0 0 0 0 0 0)"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
            r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="scale(0)"/></x:boolean>"##,
            r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="translate(1e300)"/></x:boolean>"##,
            r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="rotate(1e9)"/></x:boolean>"##,
            r##"<rect id="t" x="0" y="0" width="10" height="10" transform="skewX(90)"/><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
            // self-reference in every consumer
            r##"<x:textbox id="s" in="#s">x</x:textbox>"##,
            r##"<x:warp id="s" field="bend" in="#s"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
            r##"<x:boolean id="s" op="subtract"><use href="#s"/></x:boolean>"##,
            // group walk: live <use> children skipped, not fatal
            r##"<g id="t"><use href="#s"/><rect x="0" y="0" width="10" height="10"/></g><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
            // zero-extent rect as a reference target
            r##"<rect id="t" x="0" y="0" width="0" height="10"/><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
            // unparseable d inside a warp child: segment passes through unwarped
            r##"<x:warp field="arch" bend="40"><path d="garbage" fill="#000"/></x:warp>"##,
            // evenodd output that cancels to nothing
            r##"<x:warp id="t" field="arch" bend="0"><path d="M0,0 h10 v10 h-10 Z M0,0 h10 v10 h-10 Z" fill-rule="evenodd" fill="#000"/></x:warp><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
            // hostile meshes: bad indices, color-count mismatch, degenerate extent
            r##"<x:mesh><x:verts>0,0 10,0</x:verts><x:face v="0 1 9" fill="#f00"/></x:mesh>"##,
            r##"<x:mesh><x:verts>0,0 10,0 10,10 0,10</x:verts><x:face v="0 1 2 3" fill="#f00 #0f0"/></x:mesh>"##,
            r##"<x:mesh><x:verts>5,5 5,5 5,5</x:verts><x:face v="0 1 2" fill="#f00"/></x:mesh>"##,
            r##"<x:mesh><x:verts>garbage</x:verts><x:face v="0 1 2" fill="#f00 #0f0 #00f"/></x:mesh>"##,
            // hostile filter attributes: unparseable lists stay as authored
            r##"<rect x="0" y="0" width="10" height="10" filter="brightness("/>"##,
            r##"<rect x="0" y="0" width="10" height="10" filter="brightness(NaN)"/>"##,
            r##"<rect x="0" y="0" width="10" height="10" filter="-x-curve(9 9)"/>"##,
            r##"<g filter="brightness(1e999)"><rect x="0" y="0" width="10" height="10"/></g>"##,
        ];
        for (i, body) in cases.iter().enumerate() {
            let svg = format!("{XW}{body}</svg>");
            let out = compile_shaped(&svg); // a panic here fails the test: totality
            assert!(!out.is_empty(), "case {i} produced empty output: {body}");
            // non-finite text is only allowed where the AUTHOR wrote it (a
            // passthrough attribute value); generated coordinates never carry it
            for bad in ["NaN", "inf"] {
                assert!(
                    !out.contains(bad) || body.contains(bad),
                    "case {i} leaked non-finite coords: {body}\n{out}"
                );
            }
            assert!(
                out.contains("<path") || out.contains("<text") || out.contains("<!-- xsvg:"),
                "case {i} degraded SILENTLY (no geometry, no live text, no marker): {body}\n{out}"
            );
        }
    }

    #[test]
    fn textbox_region_without_a_shaper_markers() {
        // geometry resolves but the host cannot rasterize: the marker names it
        let svg = format!(
            r##"{XW}<circle id="t" cx="20" cy="20" r="15"/><x:textbox in="#t" font-size="10">x</x:textbox></svg>"##
        );
        let out = compile_test(&svg); // NoShaper
        assert!(out.contains("not rasterizable"), "{out}");
    }

    #[test]
    fn every_plain_shape_kind_resolves_as_a_reference() {
        use xsvg_core::kurbo::Shape;
        let bb =
            |body: &str| first_path(&compile_test(&format!("{XW}{body}</svg>"))).bounding_box();
        let e = bb(
            r##"<ellipse id="t" cx="20" cy="10" rx="20" ry="10"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
        );
        assert!(
            (e.x1 - 40.0).abs() < 0.5 && (e.y1 - 20.0).abs() < 0.5,
            "{e:?}"
        );
        let p = bb(
            r##"<polygon id="t" points="0,0 20,0 10,18"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
        );
        assert!(
            (p.x1 - 20.0).abs() < 0.5 && (p.y1 - 18.0).abs() < 0.5,
            "{p:?}"
        );
        let l = bb(
            r##"<polyline id="t" points="0,0 10,5 20,0"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
        );
        assert!(
            (l.x1 - 20.0).abs() < 0.5 && (l.y1 - 5.0).abs() < 0.5,
            "{l:?}"
        );
    }

    #[test]
    fn warp_unions_the_bbox_of_multiple_children() {
        // the envelope frame is the union of all child geometry, so both rects
        // share one field and bend together
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="40"><rect x="0" y="0" width="40" height="10" fill="#000"/><rect x="60" y="0" width="40" height="10" fill="#000"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert_eq!(out.matches("<path").count(), 2, "{out}");
        assert!(!out.contains("<!-- xsvg:"), "{out}");
    }

    #[test]
    fn filter_functions_lower_to_a_filter_definition() {
        // §8: CSS filter functions on any graphics element become a real
        // <filter> (sRGB, ±10% region) referenced by url() — one per element,
        // deterministic id, self-contained in the element's own fragment
        let svg = format!(
            r##"{XW}<rect x="0" y="0" width="40" height="30" fill="#48a" filter="brightness(1.2) contrast(1.1)"/></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<filter id=\"x-flt-"), "{out}");
        assert!(
            out.contains("color-interpolation-filters=\"sRGB\""),
            "{out}"
        );
        assert!(out.contains("filter=\"url(#x-flt-"), "{out}");
        assert!(out.contains("slope=\"1.2\""), "{out}");
        assert!(!out.contains("filter=\"brightness"), "{out}");
        // passthrough elements get the same treatment, children intact
        let svg = format!(
            r##"{XW}<g filter="saturate(0)"><circle cx="5" cy="5" r="4" fill="#c00"/></g></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("type=\"saturate\" values=\"0\""), "{out}");
        assert!(out.contains("<circle"), "{out}");
        // a tone curve samples into a lookup table
        let svg = format!(
            r##"{XW}<g filter="-x-curve(0 0, 0.4 0.6, 1 1)"><rect x="0" y="0" width="10" height="10"/></g></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("type=\"table\" tableValues=\"0 "), "{out}");
    }

    #[test]
    fn filter_references_and_unknown_functions_pass_through() {
        // url() references and functions we do not lower (blur, for now) stay
        // exactly as authored — browsers still honor them live
        for f in ["url(#soft)", "blur(3px)", "none"] {
            let svg =
                format!(r##"{XW}<rect x="0" y="0" width="10" height="10" filter="{f}"/></svg>"##);
            let out = compile_test(&svg);
            assert!(out.contains(&format!("filter=\"{f}\"")), "{f}: {out}");
            assert!(!out.contains("<filter id="), "{f}: {out}");
        }
    }

    #[test]
    fn mesh_lowers_to_a_texel_aligned_image() {
        // one smooth quad -> one region -> one clipped tiny-PNG image whose
        // placement OVERHANGS the mesh bbox (the texel-center construction)
        let svg = format!(
            r##"{XW}<x:mesh><x:verts>0,0 200,0 200,100 0,100</x:verts><x:face v="0 1 2 3" fill="#e11 #fa0 #3b7 #06c"/></x:mesh></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("<!-- xsvg:"), "{out}");
        assert_eq!(out.matches("<image").count(), 1, "{out}");
        assert!(out.contains("data:image/png;base64,"), "{out}");
        assert!(out.contains("preserveAspectRatio=\"none\""), "{out}");
        assert!(out.contains("<clipPath id=\"x-mesh-"), "{out}");
        let attr = |name: &str| -> f64 {
            let k = out.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
            out[k..k + out[k..].find('"').unwrap()].parse().unwrap()
        };
        assert!(attr("width") > 200.0, "image must overhang the bbox");
        assert!(attr("x") < 0.5, "offset before the bbox");
    }

    #[test]
    fn mesh_solid_region_is_a_plain_path() {
        let svg = format!(
            r##"{XW}<x:mesh><x:verts>0,0 40,0 40,40 0,40</x:verts><x:face v="0 1 2 3" fill="#3b82f6"/></x:mesh></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("<image"), "{out}");
        assert!(out.contains("<path fill=\"#3b82f6\""), "{out}");
    }

    #[test]
    fn mesh_cracks_split_into_separately_clipped_regions() {
        // two quads sharing an edge with DISAGREEING colors: a crack -> two
        // regions, each with its own image + clip
        let svg = format!(
            r##"{XW}<x:mesh><x:verts>0,0 100,0 200,0 0,100 100,100 200,100</x:verts><x:face v="0 1 4 3" fill="#e11 #fa0 #fa0 #e11"/><x:face v="1 2 5 4" fill="#06c #3b7 #3b7 #06c"/></x:mesh></svg>"##
        );
        let out = compile_test(&svg);
        assert_eq!(out.matches("<clipPath").count(), 2, "{out}");
        assert_eq!(out.matches("<image").count(), 2, "{out}");
        // smooth version (shared edge agrees) -> ONE region
        let svg = format!(
            r##"{XW}<x:mesh><x:verts>0,0 100,0 200,0 0,100 100,100 200,100</x:verts><x:face v="0 1 4 3" fill="#e11 #fa0 #fa0 #e11"/><x:face v="1 2 5 4" fill="#fa0 #3b7 #3b7 #fa0"/></x:mesh></svg>"##
        );
        let out = compile_test(&svg);
        assert_eq!(out.matches("<image").count(), 1, "{out}");
    }

    #[test]
    fn mesh_triangles_and_single_color_faces_work() {
        let svg = format!(
            r##"{XW}<x:mesh><x:verts>0,0 80,0 40,60</x:verts><x:face v="0 1 2" fill="#e11 #3b7 #06c"/></x:mesh></svg>"##
        );
        let out = compile_test(&svg);
        assert_eq!(out.matches("<image").count(), 1, "{out}");
        // triangle clip path has three points
        let k = out.find("<clipPath").unwrap();
        let d = &out[k..k + out[k..].find("/>").unwrap()];
        assert_eq!(d.matches('L').count(), 2, "{d}");
    }

    #[test]
    fn parse_transform_matches_svg_semantics() {
        use xsvg_core::kurbo::Point;
        let p = |x, y| Point::new(x, y);
        assert_eq!(
            parse_transform("translate(10,20)") * p(0.0, 0.0),
            p(10.0, 20.0)
        );
        assert_eq!(
            parse_transform("matrix(1 0 0 1 5 6)") * p(1.0, 1.0),
            p(6.0, 7.0)
        );
        // list applies left-to-right: scale first in point terms, then translate
        assert_eq!(
            parse_transform("translate(10) scale(2)") * p(1.0, 0.0),
            p(12.0, 0.0)
        );
        let r = parse_transform("rotate(90 10 10)") * p(10.0, 0.0);
        assert!(
            (r.x - 20.0).abs() < 1e-9 && (r.y - 10.0).abs() < 1e-9,
            "{r:?}"
        );
        assert_eq!(parse_transform("scale(2,3)") * p(1.0, 1.0), p(2.0, 3.0));
        let sk = parse_transform("skewY(45)") * p(10.0, 0.0);
        assert!((sk.y - 10.0).abs() < 1e-9, "{sk:?}");
        // invalid input → the whole list is ignored, like a browser
        assert_eq!(parse_transform("rotate(nope)") * p(3.0, 4.0), p(3.0, 4.0));
        assert_eq!(parse_transform("translate(3") * p(0.0, 0.0), p(0.0, 0.0));
        // a finite list whose product overflows is ignored too
        assert_eq!(
            parse_transform("scale(1e308) scale(1e308)") * p(1.0, 1.0),
            p(1.0, 1.0)
        );
    }

    #[test]
    fn referenced_targets_honor_their_own_transform() {
        // plain shape: a rect rotated about (20,20) — the borrowed geometry is
        // where the user SEES it, not the untransformed source
        let svg = format!(
            r##"{XW}<rect id="t" x="0" y="0" width="40" height="10" transform="rotate(90 20 20)"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean></svg>"##
        );
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&compile_test(&svg)).bounding_box();
        assert!(
            (bb.x0 - 30.0).abs() < 0.5 && (bb.x1 - 40.0).abs() < 0.5,
            "{bb:?}"
        );
        assert!(
            (bb.y0 - 0.0).abs() < 0.5 && (bb.y1 - 40.0).abs() < 0.5,
            "{bb:?}"
        );
        // x: target: the transform on the boolean applies to its borrowed output
        let svg = format!(
            r##"{XW}<x:boolean id="b" op="union" fill="#000" transform="translate(100,0)"><rect x="0" y="0" width="40" height="30"/></x:boolean><x:boolean op="union" fill="#000"><use href="#b"/></x:boolean></svg>"##
        );
        let bb = first_path(&compile_test(&svg)).bounding_box();
        assert!(
            (bb.x0 - 100.0).abs() < 0.5 && (bb.x1 - 140.0).abs() < 0.5,
            "{bb:?}"
        );
        // a <use> operand's own transform composes with x/y
        let svg = format!(
            r##"{XW}<rect id="st" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#st" transform="scale(2)"/></x:boolean></svg>"##
        );
        let bb = first_path(&compile_test(&svg)).bounding_box();
        assert!(
            (bb.x1 - 20.0).abs() < 0.5 && (bb.y1 - 20.0).abs() < 0.5,
            "{bb:?}"
        );
    }

    #[test]
    fn nested_transform_in_referenced_output_degrades_loudly() {
        // the warp's CHILD carries a transform that survives into its output;
        // a d-only harvest cannot honor it, so the reference degrades
        let svg = format!(
            r##"{XW}<x:warp id="t" field="arch" bend="10"><path d="M0,20 h40 v10 h-40 Z" transform="translate(3,4)" fill="#000"/></x:warp><x:textpath in="#t" effect="stair" font-size="10">hi</x:textpath></svg>"##
        );
        let out = compile_test(&svg);
        assert!(
            out.contains("(referenced output nests a transform)"),
            "{out}"
        );
    }

    #[test]
    fn referenced_evenodd_output_resolves_to_its_painted_region() {
        // a ring drawn with two same-winding subpaths under evenodd: the painted
        // region is the frame (1200), not the nonzero solid (1600)
        let svg = format!(
            r##"{XW}<x:warp id="ring" field="arch" bend="0"><path d="M0,0 h40 v40 h-40 Z M10,10 h20 v20 h-20 Z" fill-rule="evenodd" fill="#000"/></x:warp><x:boolean op="union" fill="#000"><use href="#ring"/></x:boolean></svg>"##
        );
        use xsvg_core::kurbo::Shape;
        let p = first_path(&compile_test(&svg));
        assert!((p.area().abs() - 1200.0).abs() < 5.0, "{}", p.area());
    }

    #[test]
    fn group_targets_contribute_their_shape_descendants() {
        // transforms compose down the tree: group translate + nested-group translate
        let svg = format!(
            r##"{XW}<g id="grp" transform="translate(10,0)"><circle cx="20" cy="20" r="10"/><g transform="translate(0,40)"><rect x="0" y="0" width="10" height="10"/></g></g><x:boolean op="union" fill="#000"><use href="#grp"/></x:boolean></svg>"##
        );
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&compile_test(&svg)).bounding_box();
        assert!(
            (bb.x0 - 10.0).abs() < 0.5 && (bb.x1 - 40.0).abs() < 0.5,
            "{bb:?}"
        );
        assert!(
            (bb.y0 - 10.0).abs() < 0.5 && (bb.y1 - 50.0).abs() < 0.5,
            "{bb:?}"
        );
    }

    #[test]
    fn cycle_poisoned_fanout_exhausts_fuel_not_time() {
        // a self-cycle at the bottom poisons the memo (cut results are never
        // cached), so 26 double-referencing levels would branch ~2^26 without
        // the fuel bound — this compiling promptly IS the assertion
        let mut body = String::from(
            r##"<x:boolean id="p0" op="union" fill="#000"><use href="#p0"/><rect x="0" y="0" width="10" height="10"/></x:boolean>"##,
        );
        for i in 1..=26 {
            body.push_str(&format!(
                r##"<x:boolean id="p{i}" op="union" fill="#000"><use href="#p{}"/><use href="#p{}" x="5"/></x:boolean>"##,
                i - 1,
                i - 1
            ));
        }
        let out = compile_test(&format!("{XW}{body}</svg>"));
        assert!(
            out.contains("(reference budget exhausted)"),
            "expected a fuel cut"
        );
    }

    #[test]
    fn referenced_text_is_outlined_for_geometry() {
        // punching a textbox by reference: the scratch resolution forces
        // outline="true" semantics so glyphs contribute geometry — while the
        // textbox itself still renders as live <text>
        let svg = format!(
            r##"{XW}<x:textbox id="w" x="10" y="10" width="80" height="20" font-size="10">hi</x:textbox><x:boolean op="subtract" fill="#000"><rect x="0" y="0" width="100" height="40"/><use href="#w"/></x:boolean></svg>"##
        );
        let out = compile_outlined(&svg);
        assert!(
            out.contains("<text"),
            "textbox itself must stay live: {out}"
        );
        use xsvg_core::kurbo::Shape;
        let a = first_path(&out).area().abs();
        assert!(a < 3999.5 && a > 3990.0, "glyph geometry not punched: {a}");
    }

    #[test]
    fn reference_chains_deeper_than_the_tree_bound_degrade() {
        // 80 SIBLING elements chained by reference sidestep the XML nesting
        // bound entirely; the resolving stack's own MAX_REF_DEPTH cap keeps §4
        // totality — a marker, never a stack overflow. (Reversed document order
        // defeats memo pre-warming, forcing the deep descents.)
        let mut body = String::new();
        for i in (1..=80).rev() {
            body.push_str(&format!(
                r##"<x:boolean id="c{i}" op="union" fill="#000"><use href="#c{}"/></x:boolean>"##,
                i - 1
            ));
        }
        body.push_str(r##"<rect id="c0" x="0" y="0" width="10" height="10"/>"##);
        let out = compile_test(&format!("{XW}{body}</svg>"));
        assert!(out.contains("skipped <use>"), "expected a depth-cut marker");
        assert!(out.contains("<path"));
    }

    #[test]
    fn diamond_reference_fanout_is_memoized_not_exponential() {
        // each level references the previous level TWICE (once offset by 5);
        // 24 levels would be 2^24 lowerings without the per-compile memo —
        // this test finishing at all is the assertion that the memo works
        let mut body = String::from(r##"<rect id="g0" x="0" y="0" width="40" height="30"/>"##);
        for i in 1..=24 {
            body.push_str(&format!(
                r##"<x:boolean id="g{i}" op="union" fill="#000"><use href="#g{}"/><use href="#g{}" x="5"/></x:boolean>"##,
                i - 1,
                i - 1
            ));
        }
        let out = compile_test(&format!("{XW}{body}</svg>"));
        use xsvg_core::kurbo::Shape;
        // width grows by 5 per level: 40 + 24·5 = 160
        let bb = first_path(&out).bounding_box();
        assert!(
            (bb.x1 - 160.0).abs() < 0.5 && (bb.y1 - 30.0).abs() < 0.5,
            "{bb:?}"
        );
    }

    #[test]
    fn fragment_recompile_reflects_a_local_edit() {
        // same-length edit inside the textbox keeps all offsets stable
        let edited = INC.replace("hello world", "HELLO WORLD");
        let off = top_level_offsets(INC)[1];
        let before =
            compile_fragment_impl(INC, "balanced", false, off, &Mono, &NoShaper, &NoOutliner)
                .unwrap();
        let after = compile_fragment_impl(
            &edited,
            "balanced",
            false,
            off,
            &Mono,
            &NoShaper,
            &NoOutliner,
        )
        .unwrap();
        assert_ne!(before, after);
        assert!(after.contains("HELLO"), "{after}");
        // and the unrelated warp fragment is byte-identical across the edit
        let woff = top_level_offsets(INC)[4];
        let w0 = compile_fragment_impl(INC, "balanced", false, woff, &Mono, &NoShaper, &NoOutliner)
            .unwrap();
        let w1 = compile_fragment_impl(
            &edited,
            "balanced",
            false,
            woff,
            &Mono,
            &NoShaper,
            &NoOutliner,
        )
        .unwrap();
        assert_eq!(w0, w1);
    }

    // ---- <x:boolean> (§7.5) ----

    #[test]
    fn boolean_subtract_takes_paint_from_the_element() {
        // rect minus rect → one <path>, element paint, no child paint, L-shape bbox
        let svg = format!(
            r##"{XW}<x:boolean op="subtract" fill="#1d4ed8"><rect x="0" y="0" width="100" height="40" fill="#fff"/><rect x="60" y="0" width="60" height="40" fill="#000"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains(r##"<path fill="#1d4ed8""##), "{out}");
        assert!(!out.contains("#fff") && !out.contains("#000"), "{out}");
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&out).bounding_box();
        assert!(bb.x0.abs() < 0.1 && (bb.x1 - 60.0).abs() < 0.1, "{bb:?}");
    }

    #[test]
    fn boolean_defaults_to_union() {
        // two overlapping rects, no op attr → one merged outline (5 distinct
        // corners on the silhouette → more than one rect's 4)
        let svg = format!(
            r##"{XW}<x:boolean fill="#111"><rect x="0" y="0" width="100" height="40"/><rect x="50" y="20" width="100" height="40"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&out).bounding_box();
        assert!(
            (bb.x1 - 150.0).abs() < 0.1 && (bb.y1 - 60.0).abs() < 0.1,
            "{bb:?}"
        );
        assert_eq!(out.matches("<path").count(), 1, "{out}");
    }

    #[test]
    fn boolean_punches_outlined_text() {
        // the flagship: text (BoxOutliner's 1×1 box at the line origin) punched
        // out of a plate — the result is a single path with a hole
        let svg = format!(
            r##"{XW}<x:boolean op="subtract" fill="#333"><rect x="0" y="0" width="80" height="20"/><x:textbox x="10" y="5" width="60" height="10" outline="true" font-size="8">x</x:textbox></x:boolean></svg>"##
        );
        let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
        assert!(out.contains("<path fill=\"#333\""), "{out}");
        // two contours: the plate outline + the punched hole
        let d = out
            .rsplit(" d=\"")
            .next()
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert_eq!(d.matches('M').count() + d.matches('m').count(), 2, "{out}");
    }

    #[test]
    fn boolean_empty_and_degenerate_results() {
        // disjoint intersect → legitimately empty <g/>
        let svg = format!(
            r##"{XW}<x:boolean op="intersect" fill="#111"><rect x="0" y="0" width="10" height="10"/><rect x="100" y="0" width="10" height="10"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("<path"), "{out}");
        assert!(out.contains("<g fill=\"#111\"/>"), "{out}");
        // unknown op → marker + children un-combined
        let svg = format!(
            r##"{XW}<x:boolean op="divide"><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("unknown op"), "{out}");
        assert!(out.contains("h10"), "child lost: {out}");
        // live-text child skipped with a marker; empty element → no-geometry marker
        let svg = format!(
            r##"{XW}<x:boolean><x:textbox x="0" y="0" width="40" height="10" font-size="8">hi</x:textbox></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<x:boolean> skipped <textbox>"), "{out}");
        assert!(out.contains("no usable geometry"), "{out}");
    }

    #[test]
    fn boolean_composes_with_warp_both_ways() {
        // boolean inside warp: the combined path bakes like any geometry
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="50"><x:boolean op="union" fill="#111"><rect x="0" y="0" width="60" height="20"/><rect x="40" y="0" width="60" height="20"/></x:boolean></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<path"), "{out}");
        assert!(!out.contains("skipped"), "{out}");
        // warp inside boolean: warped output is an operand
        let svg = format!(
            r##"{XW}<x:boolean op="intersect" fill="#111"><x:warp field="arch" bend="50"><rect x="0" y="0" width="100" height="30"/></x:warp><rect x="0" y="-20" width="100" height="30"/></x:boolean></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<path fill=\"#111\""), "{out}");
        assert!(!out.contains("NaN"), "{out}");
    }

    // ---- <x:warp> (§7.3) ----

    const XW: &str =
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">"##;

    /// Compile at `fast` quality — raw polyline output, exact vertex assertions.
    fn compile_fast(svg: &str) -> String {
        compile_impl(svg, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap()
    }

    /// Reparse the last emitted `d` attribute — the warped output (reference paths
    /// pass through first). The output is compact/relative, so geometry assertions
    /// go through kurbo rather than string matching.
    fn first_path(out: &str) -> xsvg_core::kurbo::BezPath {
        let d = out
            .rsplit(" d=\"")
            .next()
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        xsvg_core::kurbo::BezPath::from_svg(d).unwrap()
    }

    #[test]
    fn warp_arch_bends_a_rect() {
        // bbox 100×40, bend 100% → A = 25: the apex reaches y = −25, the corners
        // stay pinned (u = ±1 → Δ = 0), and the rect is now a subdivided polyline.
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="100" height="40" fill="#f00"/></x:warp></svg>"##
        );
        let out = compile_fast(&svg);
        assert!(out.contains("<g"), "{out}");
        assert!(!out.contains("<rect"), "{out}");
        assert!(out.contains(r##"fill="#f00""##), "{out}");
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&out).bounding_box();
        assert!((bb.y0 + 25.0).abs() < 0.1, "apex {}: {out}", bb.y0);
        assert!((bb.x0.abs()) < 0.1 && (bb.x1 - 100.0).abs() < 0.1, "{out}");
        assert!(
            first_path(&out).elements().len() > 10,
            "no subdivision: {out}"
        );
    }

    #[test]
    fn warp_skips_live_text_child_with_a_marker() {
        // no outline font → the textbox lowers to live <text>, which a warp must
        // not silently emit unwarped
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi</x:textbox></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("<x:warp> skipped <textbox>"), "{out}");
        assert!(!out.contains("<text "), "{out}");
    }

    #[test]
    fn warp_warps_outlined_text_child() {
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">hi</x:textbox></x:warp></svg>"##
        );
        let out = compile_outlined(&svg);
        assert!(!out.contains("<text"), "{out}");
        assert!(!out.contains("skipped"), "{out}");
        assert!(out.contains("<path d=\"M"), "{out}");
    }

    #[test]
    fn warp_circle_child_becomes_a_warped_path() {
        let svg = format!(
            r##"{XW}<x:warp field="flag" bend="60"><circle cx="50" cy="50" r="40" fill="#0af"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("<circle"), "{out}");
        assert!(out.contains(r##"fill="#0af""##), "{out}");
        assert!(out.contains("<path"), "{out}");
    }

    #[test]
    fn warp_radial_and_rotational_presets_parse() {
        for name in [
            "fisheye",
            "inflate",
            "squeeze",
            "twist",
            "arc",
            "arc-lower",
            "arc-upper",
            "bulge",
            "shell-lower",
            "shell-upper",
            "fish",
        ] {
            let svg = format!(
                r##"{XW}<x:warp field="{name}" bend="80"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
            );
            let out = compile_test(&svg);
            assert!(!out.contains("unknown field"), "{name}: {out}");
            assert!(out.contains("<path"), "{name}: {out}");
            assert!(!out.contains("NaN"), "{name}: {out}");
        }
    }

    #[test]
    fn warp_perspective_pins_authored_corners_without_subdividing() {
        // bbox (0,0)-(200,120) → authored quad; projective keeps edges straight, so
        // the rect bakes to exactly its 4 corner vertices (M + 4 L, incl. the
        // explicit closing edge) — no wasted subdivision
        let svg = format!(
            r##"{XW}<x:warp field="perspective" corners="20,10 180,10 200,120 0,120"><rect x="0" y="0" width="200" height="120" fill="#f0f"/></x:warp></svg>"##
        );
        let out = compile_fast(&svg);
        assert!(out.contains("M20,10"), "{out}");
        let path = first_path(&out);
        use xsvg_core::kurbo::PathEl;
        let lines = path
            .elements()
            .iter()
            .filter(|e| matches!(e, PathEl::LineTo(_)))
            .count();
        assert_eq!(lines, 4, "{out}");
        // the authored corners are pinned exactly (within the fast grid)
        for corner in [(20.0, 10.0), (180.0, 10.0), (200.0, 120.0), (0.0, 120.0)] {
            let hit = path.elements().iter().any(|e| match e {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => {
                    (p.x - corner.0).abs() < 0.06 && (p.y - corner.1).abs() < 0.06
                }
                _ => false,
            });
            assert!(hit, "corner {corner:?} missing: {out}");
        }
    }

    #[test]
    fn warp_perspective_without_corners_degrades_with_a_marker() {
        let svg = format!(
            r##"{XW}<x:warp field="perspective"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("bad corners"), "{out}");
        assert!(out.contains("h100"), "child not passed through: {out}");
    }

    #[test]
    fn warp_free_distort_parses_and_bakes() {
        let svg = format!(
            r##"{XW}<x:warp field="free" corners="0,20 100,-10 110,50 -10,30"><rect x="0" y="0" width="100" height="40" fill="#0f0"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("unknown field"), "{out}");
        assert!(out.contains("M0,20"), "{out}");
    }

    #[test]
    fn warp_distortion_sliders_compose_after_the_preset() {
        let plain = format!(
            r##"{XW}<x:warp field="arch" bend="60"><rect x="0" y="0" width="200" height="80"/></x:warp></svg>"##
        );
        let tapered = format!(
            r##"{XW}<x:warp field="arch" bend="60" distort-h="60"><rect x="0" y="0" width="200" height="80"/></x:warp></svg>"##
        );
        let a = compile_test(&plain);
        let b = compile_test(&tapered);
        assert_ne!(a, b);
        assert!(!b.contains("distort-h"), "slider attr leaked onto <g>: {b}");
        assert!(!b.contains("NaN"), "{b}");
    }

    #[test]
    fn warp_bend_flows_children_along_a_spine() {
        // envelope (0,0)-(100,40), flat spine at y = 100: the envelope midline
        // (y = 20) rides the spine, so the rect maps to a band y = 80…120 starting
        // at the spine's start
        let svg = format!(
            r##"{XW}<path id="spine" d="M0,100 L300,100" fill="none"/><x:warp field="bend" in="#spine"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
        );
        let out = compile_fast(&svg);
        assert!(out.contains("M0,80"), "{out}");
        use xsvg_core::kurbo::Shape;
        let bb = first_path(&out).bounding_box();
        assert!(
            bb.x0.abs() < 0.1
                && (bb.x1 - 100.0).abs() < 0.1
                && (bb.y0 - 80.0).abs() < 0.1
                && (bb.y1 - 120.0).abs() < 0.1,
            "band {bb:?}: {out}"
        );
        // a vertical spine rotates the band onto it
        let svg = format!(
            r##"{XW}<path id="spine" d="M50,0 L50,300" fill="none"/><x:warp field="bend" in="#spine"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
        );
        let out = compile_fast(&svg);
        assert!(out.contains("M70,0"), "{out}"); // top edge (20 above mid) → x = 50+20
        assert!(!out.contains("NaN"), "{out}");
        // missing/degenerate spine degrades behind the usual marker
        let svg = format!(
            r##"{XW}<x:warp field="bend" in="#nope"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
        );
        let out = compile_fast(&svg);
        assert!(out.contains("unknown field"), "{out}");
        assert!(out.contains("h100"), "{out}");
    }

    #[test]
    fn warp_roughen_is_deterministic_and_bounded() {
        let svg = format!(
            r##"{XW}<x:warp field="roughen" bend="60" detail="12"><rect x="0" y="0" width="200" height="80" fill="#f00"/></x:warp></svg>"##
        );
        let a = compile_fast(&svg);
        let b = compile_fast(&svg);
        assert_eq!(a, b, "roughen must be deterministic");
        assert!(a.contains("<path"), "{a}");
        assert!(!a.contains("NaN"), "{a}");
        // it actually roughens: the output differs from the unwarped rect
        assert!(!a.contains("h200"), "{a}");
    }

    #[test]
    fn textpath_ribbon_warps_and_degrades_like_skew() {
        // flat path → ribbon behaves like skew: baseline lands on y = 20
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="ribbon" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_outlined(svg);
        assert!(out.contains("<path d=\"M0,20"), "{out}");
        // without a font it degrades to the stepped baseline, like skew
        let out = compile_test(svg);
        assert!(out.contains(r#"x="0 10 20 30""#), "{out}");
        assert!(out.contains(r#"y="20 20 20 20""#), "{out}");
    }

    #[test]
    fn textpath_follow_lowers_to_native_textpath() {
        // live SVG <textPath> — no font needed; align=middle places startOffset at
        // (arclen 120 − advance 40)/2 = 40; baseline-shift rides along
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="wavy" d="M0,20 L120,20" fill="none"/><x:textpath in="#wavy" effect="follow" align="middle" baseline-shift="5" font-size="20" fill="#111">wave</x:textpath></svg>"##;
        let out = compile_test(svg);
        assert!(out.contains("<textPath href=\"#wavy\""), "{out}");
        assert!(out.contains("startOffset=\"40\""), "{out}");
        assert!(out.contains("baseline-shift=\"5\""), "{out}");
        assert!(out.contains(">wave</textPath></text>"), "{out}");
        // a degenerate path → straight fallback, no <textPath>
        let degen = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="follow" font-size="20">hi</x:textpath></svg>"##;
        let out = compile_test(degen);
        assert!(!out.contains("<textPath"), "{out}");
        assert!(out.contains(">hi</text>"), "{out}");
    }

    #[test]
    fn warp_unknown_field_marks_and_passes_through() {
        let svg = format!(
            r##"{XW}<x:warp field="bogus" bend="50"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.contains("unknown field"), "{out}");
        assert!(out.contains("h100"), "child not passed through: {out}");
    }

    #[test]
    fn warp_quality_grades_output_form() {
        // every profile emits the tolerance-graded polyline (refit is disabled —
        // §7.1): tighter tolerance → more segments, and never any cubics
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="200" height="60"/></x:warp></svg>"##
        );
        let fast = compile_impl(&svg, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        let hi = compile_impl(&svg, "highest", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        assert!(!fast.contains('C') && !hi.contains('C'), "{hi}");
        let (nf, nh) = (
            first_path(&fast).elements().len(),
            first_path(&hi).elements().len(),
        );
        assert!(nh > nf, "highest ({nh}) !> fast ({nf})");
    }

    #[test]
    fn warp_nested_composes() {
        // the inner warp bakes first (recursive serialize), the outer re-bakes its
        // emitted paths — nesting composes innermost-first
        let svg = format!(
            r##"{XW}<x:warp field="arch" bend="40"><x:warp field="flag" bend="40"><rect x="0" y="0" width="100" height="30"/></x:warp></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(out.matches("<g").count() >= 2, "{out}");
        assert!(out.contains("<path"), "{out}");
        assert!(!out.contains("skipped"), "{out}");
    }

    #[test]
    fn warp_degenerate_input_never_panics() {
        // no children / no geometry / garbage bend / rounded rect (unwarpable)
        for svg in [
            format!(r##"{XW}<x:warp field="arch" bend="50"></x:warp></svg>"##),
            format!(
                r##"{XW}<x:warp field="arch" bend="garbage"><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##
            ),
            format!(
                r##"{XW}<x:warp field="wave" bend="100"><rect x="0" y="0" width="10" height="10" rx="3"/></x:warp></svg>"##
            ),
            format!(r##"{XW}<x:warp><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##),
        ] {
            let out = compile_test(&svg);
            assert!(!out.contains("NaN"), "{out}");
        }
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

    // ---- source map (data-xsvg-pos) ----

    #[test]
    fn sourcemap_off_by_default() {
        // The default compile (sourcemap=false) must not pollute the output.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="10" y="20" width="30" height="40"/></svg>"#;
        assert!(!compile_test(svg).contains("data-xsvg-pos"));
    }

    #[test]
    fn sourcemap_tags_elements_with_valid_source_ranges() {
        // A passthrough <rect>→<path>, a synthesized <x:textbox>→<text>, plus the
        // <svg> root all get tagged; every range is well-formed and in-bounds, and
        // maps back to the authoring node's markup.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><rect id="r" x="10" y="20" width="30" height="40"/><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi there</x:textbox></svg>"##;
        let out = compile_mapped(svg);
        let n = svg.len();

        let ranges: Vec<(usize, usize)> = out
            .split("data-xsvg-pos=\"")
            .skip(1)
            .map(|s| {
                let r = s.split('"').next().unwrap();
                let (a, b) = r.split_once('-').unwrap();
                (a.parse().unwrap(), b.parse().unwrap())
            })
            .collect();

        // svg root + rect + textbox
        assert!(ranges.len() >= 3, "expected >=3 tagged elements: {out}");
        for &(a, b) in &ranges {
            assert!(a < b && b <= n, "bad range {a}-{b} (len {n}): {out}");
        }
        // ranges point back at the authoring nodes
        let slices: Vec<&str> = ranges.iter().map(|&(a, b)| &svg[a..b]).collect();
        assert!(
            slices.iter().any(|s| s.contains("rect")),
            "no range covers the <rect>: {slices:?}"
        );
        assert!(
            slices.iter().any(|s| s.contains("x:textbox")),
            "no range covers the <x:textbox>: {slices:?}"
        );
    }
}
