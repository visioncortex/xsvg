//! The passthrough half of the degradation contract (§3/§5): anything in the SVG
//! namespace the compiler doesn't recognize must survive verbatim — elements,
//! attributes, children, recursively — with namespace normalization pinned here.

use xsvg_core::{FontMetrics, Measurer, RasterRegion, Shaper, TextStyle};

struct Mono;
impl Measurer for Mono {
    fn measure(&self, text: &str, _s: &TextStyle, size: f64) -> f64 {
        text.chars().count() as f64 * 0.5 * size
    }
    fn font_metrics(&self, _s: &TextStyle, size: f64) -> FontMetrics {
        FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        }
    }
}
struct NoShaper;
impl Shaper for NoShaper {
    fn rasterize(&self, _d: &str, _h: f64) -> Option<RasterRegion> {
        None
    }
}
struct NoOutliner;
impl xsvg_core::GlyphOutliner for NoOutliner {
    fn outline(&self, _t: &str, _s: &TextStyle, _z: f64, _x: f64, _b: f64) -> Option<String> {
        None
    }
}

fn compile(svg: &str) -> String {
    xsvg_wasm::compile_impl(svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap()
}

#[test]
fn unknown_svg_elements_pass_through_verbatim() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><defs><filter id="f"><feGaussianBlur stdDeviation="2"/></filter><linearGradient id="g"><stop offset="0" stop-color="#f00"/></linearGradient></defs><foreignObject width="10" height="10"><div>html</div></foreignObject><unknownElement custom-attr="1"><circle cx="1" cy="1" r="1"/></unknownElement></svg>"##;
    let out = compile(svg);
    for verbatim in [
        r##"<filter id="f"><feGaussianBlur stdDeviation="2"/></filter>"##,
        r##"<linearGradient id="g"><stop offset="0" stop-color="#f00"/></linearGradient>"##,
        r##"<foreignObject width="10" height="10"><div>html</div></foreignObject>"##,
        r##"<unknownElement custom-attr="1"><circle cx="1" cy="1" r="1"/></unknownElement>"##,
    ] {
        assert!(out.contains(verbatim), "missing {verbatim} in:\n{out}");
    }
}

#[test]
fn namespace_normalization_is_pinned() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:sodipodi="http://inkscape/ns" sodipodi:docname="junk.svg"><use xlink:href="#a"/><text xml:space="preserve">keep</text><sodipodi:namedview pagecolor="#fff"/></svg>"##;
    let out = compile(svg);
    // xlink modernizes to the unprefixed SVG 2 form (we declare no xlink xmlns)
    assert!(out.contains(r##"<use href="#a"/>"##), "{out}");
    assert!(!out.contains("xlink"), "{out}");
    // the reserved xml: prefix survives
    assert!(out.contains(r#"xml:space="preserve""#), "{out}");
    // foreign-namespace elements drop with a marker; foreign attributes drop silently
    assert!(
        out.contains("<!-- xsvg: foreign-namespace <namedview> dropped -->"),
        "{out}"
    );
    assert!(
        !out.contains("namedview pagecolor") && !out.contains("docname"),
        "{out}"
    );
}

#[test]
fn static_subset_gap_is_documented_behavior() {
    // §8 promises a static output subset, but enforcement (the allow/deny list) is
    // a pending deliverable — Plan.md R6. Today <script>/<animate> pass through;
    // when sanitization lands, update this test deliberately.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><script>alert(1)</script><animate attributeName="x"/></svg>"##;
    let out = compile(svg);
    assert!(out.contains("<script>alert(1)</script>"), "{out}");
    assert!(out.contains("<animate attributeName=\"x\"/>"), "{out}");
}
