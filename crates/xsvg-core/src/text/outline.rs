//! The glyph-outlining seam ([`GlyphOutliner`]): turn a text run into vector `<path>`
//! geometry ("create outlines"). Platform-free like [`super::measure::Measurer`] — a
//! browser adapter backs it with opentype.js; tests use a stub.

use super::style::TextStyle;

/// Source of glyph outlines. Given a text run, its style, and a baseline origin
/// `(x, baseline)` (the left end of the baseline), returns an SVG path `d` in user
/// units tracing the glyphs. `None` when the backend can't outline the run (e.g. the
/// font's bytes aren't available) — the caller then falls back to live `<text>`.
pub trait GlyphOutliner {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String>;

    /// Outline `text` and **warp it onto a path** (the text-on-path specialization of the
    /// geometry-transform pipeline): the run is shaped on a flat baseline, then every
    /// outline point is displaced by the field selected by `effect` — `"skew"` (1-D
    /// vertical displacement by the path's height profile) or `"rainbow"` (arc-length
    /// follow). `path_d` is the reference path's SVG `d`. Returns the warped path `d`, or
    /// `None` to fall back to live `<text>`. Default: `None` (no path-warping backend).
    fn outline_on_path(
        &self,
        _text: &str,
        _style: &TextStyle,
        _size: f64,
        _path_d: &str,
        _effect: &str,
    ) -> Option<String> {
        None
    }
}
