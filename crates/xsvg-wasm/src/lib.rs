//! Browser adapter for the xsvg compiler. The compiler itself lives in `xsvg-core`
//! (platform-agnostic); this crate is a thin wasm-bindgen layer that backs the core's
//! platform seams with JS callbacks — `Measurer` (canvas `measureText` + font metrics),
//! `Shaper` (path rasterize for region flow), and `GlyphOutliner` (opentype.js glyph
//! outlines + advance widths) — and exposes the `compile*` entry points to JS.

use wasm_bindgen::prelude::*;
use xsvg_core::{
    compile_fragment_linked_impl, compile_linked_impl, dependents_impl, fragment_range_impl,
};
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
