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
}
