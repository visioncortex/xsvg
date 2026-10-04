//! Browser adapter for the xsvg compiler. The compiler itself lives in `xsvg-core`
//! (platform-agnostic); this crate is a thin wasm-bindgen layer that backs the core's
//! platform seams with JS callbacks — `Measurer` (canvas `measureText` + font metrics),
//! `Shaper` (path rasterize for region flow), and `GlyphOutliner` (opentype.js glyph
//! outlines + advance widths) — and exposes the `compile*` entry points to JS.

use wasm_bindgen::prelude::*;
use xsvg_core::{
    compile_fragment_linked_impl, compile_fragment_platform_impl, compile_linked_impl,
    compile_platform_impl, dependents_impl, fragment_range_impl, imports_impl, node_spans_impl,
    NodeKeys, Platform,
};
use xsvg_core::{ChildMode, ElementInfo, GenRequest, GenResult, GeneratorHost, InputMode};
use xsvg_core::{
    FontMetrics, GlyphOutliner, Measurer, RasterRegion, Rect, Resolver, Shaper, TextStyle,
};

/// Runs once when the module is instantiated: route Rust panics to `console.error`.
#[wasm_bindgen(start)]
pub fn on_start() {
    console_error_panic_hook::set_once();
}

/// Browser-backed `Measurer`. `measure(text, fontCss) -> width` and
/// `metrics(fontCss) -> [ascent, descent, capHeight, xHeight]` are canvas callbacks.
pub struct JsMeasurer<'a> {
    measure: &'a js_sys::Function,
    metrics: &'a js_sys::Function,
}

impl<'a> JsMeasurer<'a> {
    /// For downstream wasm crates (xsvg-kit) reusing the browser seams.
    pub fn new(measure: &'a js_sys::Function, metrics: &'a js_sys::Function) -> Self {
        Self { measure, metrics }
    }
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

    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        let default = FontMetrics {
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
        FontMetrics {
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
pub struct JsShaper<'a> {
    rasterize: &'a js_sys::Function,
}

impl<'a> JsShaper<'a> {
    /// For downstream wasm crates (xsvg-kit) reusing the browser seams.
    pub fn new(rasterize: &'a js_sys::Function) -> Self {
        Self { rasterize }
    }
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
pub struct JsOutliner<'a> {
    outline_run: &'a js_sys::Function,
    advance_width: &'a js_sys::Function,
}

impl<'a> JsOutliner<'a> {
    /// For downstream wasm crates (xsvg-kit) reusing the browser seams.
    pub fn new(outline_run: &'a js_sys::Function, advance_width: &'a js_sys::Function) -> Self {
        Self { outline_run, advance_width }
    }
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

/// Browser-backed [`Resolver`] for cross-file `<use href>` links. `resolve(base, href)`
/// returns `[canonicalKey, sourceText]` for a dependency the host has already fetched
/// (same-origin), or `null`/`undefined` to degrade. The host does the URL resolution and
/// the same-origin `fetch` DAG walk *before* calling compile — the sync core just reads
/// the preloaded result here, so CORS / cross-origin simply surfaces as "not resolved".
struct JsResolver<'a> {
    resolve: &'a js_sys::Function,
}

impl Resolver for JsResolver<'_> {
    fn resolve(&self, base: &str, href: &str) -> Option<(String, String)> {
        let v = self
            .resolve
            .call2(
                &JsValue::NULL,
                &JsValue::from_str(base),
                &JsValue::from_str(href),
            )
            .ok()?;
        if v.is_null() || v.is_undefined() {
            return None;
        }
        let arr = js_sys::Array::from(&v);
        Some((arr.get(0).as_string()?, arr.get(1).as_string()?))
    }
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
#[allow(clippy::too_many_arguments)]
pub fn compile(
    input: &str,
    quality: &str,
    sourcemap: bool,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    outline_run: &js_sys::Function,
    advance_width: &js_sys::Function,
    resolve: &js_sys::Function,
    base: &str,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    let resolver = JsResolver { resolve };
    compile_linked_impl(
        input, quality, sourcemap, &m, &shaper, &outliner, &resolver, base,
    )
    .map_err(|e| JsError::new(&e))
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
    resolve: &js_sys::Function,
    base: &str,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    let resolver = JsResolver { resolve };
    compile_fragment_linked_impl(
        input,
        quality,
        sourcemap,
        offset as usize,
        &m,
        &shaper,
        &outliner,
        &resolver,
        base,
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

// ---- Loadable geometry generators (docs/Extensibility.md) -----------------

/// Browser-backed [`GeneratorHost`]. The two callbacks are the whole contract:
///
/// - `element(ns, tag)` → `null` for an unbound namespace, else
///   `{state: "ready"|"pending"|"unknown", input, children}`.
/// - `generate(ns, tag, requestJson)` → the module's reply JSON, or `null`.
///
/// The request is encoded here rather than in JS so both hosts speak the exact
/// wire format the plugin SDK decodes; JS is left with pure memory plumbing.
/// Module loading is the host's business and happens before compiling — the core
/// is synchronous, so a namespace whose module has not arrived yet answers
/// `pending` and the host recompiles once it has (Extensibility §5).
struct JsGeneratorHost<'a> {
    element: &'a js_sys::Function,
    generate: &'a js_sys::Function,
}

impl GeneratorHost for JsGeneratorHost<'_> {
    fn element(&self, ns: &str, tag: &str) -> ElementInfo {
        let Ok(v) = self.element.call2(
            &JsValue::NULL,
            &JsValue::from_str(ns),
            &JsValue::from_str(tag),
        ) else {
            return ElementInfo::Unbound;
        };
        if v.is_null() || v.is_undefined() {
            return ElementInfo::Unbound;
        }
        let field = |k: &str| {
            js_sys::Reflect::get(&v, &JsValue::from_str(k))
                .ok()
                .and_then(|x| x.as_string())
        };
        match field("state").as_deref() {
            Some("ready") => ElementInfo::Ready {
                input: match field("input").as_deref() {
                    Some("required") => InputMode::Required,
                    Some("optional") => InputMode::Optional,
                    _ => InputMode::None,
                },
                children: match field("children").as_deref() {
                    Some("container") => ChildMode::Container,
                    _ => ChildMode::None,
                },
            },
            Some("pending") => ElementInfo::Pending,
            Some("unknown") => ElementInfo::UnknownElement,
            _ => ElementInfo::Unbound,
        }
    }

    fn generate(&self, ns: &str, tag: &str, req: &GenRequest) -> GenResult {
        let kids: Vec<xsvg_plugin::host::ChildIn> = req
            .children
            .iter()
            .map(|c| (c.svg.as_str(), c.bbox))
            .collect();
        let payload = xsvg_plugin::host::encode_request(
            tag,
            &req.attrs,
            req.input_d.as_deref(),
            &kids,
            req.tolerance,
        );
        let reply = self.generate.call3(
            &JsValue::NULL,
            &JsValue::from_str(ns),
            &JsValue::from_str(tag),
            &JsValue::from_str(&payload),
        );
        match reply.ok().and_then(|v| v.as_string()) {
            Some(s) => match xsvg_plugin::host::decode_reply(&s) {
                Ok(svg) => GenResult::Svg(svg),
                Err(why) => GenResult::Error(why),
            },
            None => GenResult::Error("generator call failed".to_string()),
        }
    }
}

/// The `<x:import>` bindings a document declares, as JSON
/// `[{"ns":…,"src":…,"integrity":…|null}]`.
///
/// A host calls this **before** compiling so it can fetch and instantiate exactly
/// the modules the document asks for, instead of discovering them through a
/// throwaway compile round.
#[wasm_bindgen]
pub fn imports(input: &str) -> String {
    let mut out = String::from("[");
    for (i, im) in imports_impl(input).iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"ns\":");
        xsvg_plugin::json::push_str(&mut out, &im.ns);
        out.push_str(",\"src\":");
        xsvg_plugin::json::push_str(&mut out, &im.src);
        out.push_str(",\"integrity\":");
        match &im.integrity {
            Some(h) => xsvg_plugin::json::push_str(&mut out, h),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push(']');
    out
}

/// The compiler's **own** elements as a manifest, in the same shape a plugin
/// module answers with (docs/Extensibility.md §11). A host merges this with the
/// loaded modules' manifests so a property panel treats both tiers identically.
#[wasm_bindgen]
pub fn describe() -> String {
    xsvg_core::describe_impl()
}

/// Host-supplied node keys ([`NodeKeys`]): `key(byteStart) => string | null`. With
/// no callback it is the byte-span default, inactive like [`xsvg_core::NoKeys`].
struct JsKeys<'a> {
    key: Option<&'a js_sys::Function>,
}

impl NodeKeys for JsKeys<'_> {
    fn key(&self, byte_start: usize) -> Option<String> {
        self.key?
            .call1(&JsValue::NULL, &JsValue::from_f64(byte_start as f64))
            .ok()?
            .as_string()
    }
    fn active(&self) -> bool {
        self.key.is_some()
    }
}

/// [`compile`] with loadable generators, and optionally host-supplied node keys
/// (docs/Incremental.md §3.1). Separate entry rather than extra parameters on
/// [`compile`] so existing embedders keep working unchanged.
///
/// With `key` supplied, identity emits as `data-xsvg-node="<key>"` and generated
/// def ids seed on the key, so an element the host keys the same way twice compiles
/// to the same bytes however far the text around it has moved. That is what lets an
/// editing host replace one element and leave the rest of the document alone — with
/// byte spans, an edit renames every span and every generated id after it.
///
/// The host must key **every** element (see `node_spans`); an unkeyed one falls back
/// to byte spans and quietly reintroduces the dependence keys remove. Pass no `key`
/// to keep plain byte-span identity.
///
/// New capabilities extend this entry as further optional trailing arguments — the
/// keyed variants used to be separate functions, and that matrix doubles per axis.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn compile_ext(
    input: &str,
    quality: &str,
    sourcemap: bool,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    outline_run: &js_sys::Function,
    advance_width: &js_sys::Function,
    resolve: &js_sys::Function,
    base: &str,
    plugin_element: &js_sys::Function,
    plugin_generate: &js_sys::Function,
    key: Option<js_sys::Function>,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    let resolver = JsResolver { resolve };
    let generators = JsGeneratorHost {
        element: plugin_element,
        generate: plugin_generate,
    };
    let platform = Platform::new(m, shaper, outliner)
        .with_resolver(resolver)
        .with_generators(generators);
    let platform = platform.with_keys(JsKeys { key: key.as_ref() });
    compile_platform_impl(input, quality, sourcemap, base, &platform).map_err(|e| JsError::new(&e))
}

/// Byte ranges `[start, end, start, end, …]` of **every** element in the document,
/// in document order. Parse-only; the host's input for keying every element.
#[wasm_bindgen]
pub fn node_spans(input: &str) -> Vec<u32> {
    node_spans_impl(input)
        .into_iter()
        .flat_map(|(s, e)| [s as u32, e as u32])
        .collect()
}

/// [`compile_fragment`] with loadable generators, and optionally host-supplied node
/// keys — the fragment counterpart of [`compile_ext`], same trailing-argument rule.
#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_ext(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: u32,
    measure: &js_sys::Function,
    metrics: &js_sys::Function,
    rasterize: &js_sys::Function,
    outline_run: &js_sys::Function,
    advance_width: &js_sys::Function,
    resolve: &js_sys::Function,
    base: &str,
    plugin_element: &js_sys::Function,
    plugin_generate: &js_sys::Function,
    key: Option<js_sys::Function>,
) -> Result<String, JsError> {
    let m = JsMeasurer { measure, metrics };
    let shaper = JsShaper { rasterize };
    let outliner = JsOutliner {
        outline_run,
        advance_width,
    };
    let resolver = JsResolver { resolve };
    let generators = JsGeneratorHost {
        element: plugin_element,
        generate: plugin_generate,
    };
    let platform = Platform::new(m, shaper, outliner)
        .with_resolver(resolver)
        .with_generators(generators);
    let platform = platform.with_keys(JsKeys { key: key.as_ref() });
    compile_fragment_platform_impl(input, quality, sourcemap, offset as usize, base, &platform)
        .map_err(|e| JsError::new(&e))
}
