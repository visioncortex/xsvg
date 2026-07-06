//! The glyph-outlining seam ([`GlyphOutliner`]): turn a text run into vector `<path>`
//! geometry ("create outlines"). Platform-free like [`super::measure::Measurer`] — a
//! browser adapter backs it with opentype.js; tests use a stub.

use super::style::TextStyle;

/// How a text-on-path run is warped and placed (§6.13): the field-selecting `effect`
/// (`"skew"` | `"rainbow"`), the offset of the run's baseline from the path along the
/// local normal (positive = above, SVG `baseline-shift` semantics), and where the run
/// begins within the path's extent — `align` distributes the slack (`"start"` |
/// `"middle"` | `"end"`), `start` adds an absolute head-start (x units under skew,
/// arc length under rainbow).
#[derive(Clone, Copy, Debug)]
pub struct PathEffect<'a> {
    pub effect: &'a str,
    pub baseline_shift: f64,
    pub align: &'a str,
    pub start: f64,
}

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

    /// Advance width of `text` per the outline font's own metrics — the run width
    /// that matches the geometry [`GlyphOutliner::outline`] traces (unlike the
    /// [`super::measure::Measurer`], whose canvas metrics may differ slightly).
    /// Used to place text-on-path runs (§6.13 `align`). Default: `None` (no font
    /// bytes → the caller degrades).
    fn advance_width(&self, _text: &str, _style: &TextStyle, _size: f64) -> Option<f64> {
        None
    }
}
