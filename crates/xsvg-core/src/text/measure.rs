//! The font-metrics seam ([`Measurer`]) and word measurement.

use super::style::TextStyle;

/// Vertical font metrics at a given size, in user units. All are baseline-relative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Em-box ascent (top of the line box above the baseline).
    pub ascent: f64,
    /// Em-box descent (bottom of the line box below the baseline).
    pub descent: f64,
    /// Capital-letter height — the reference used for vertical alignment.
    pub cap_height: f64,
    /// Lowercase x-height.
    pub x_height: f64,
}

/// Source of font metrics. The only platform-specific dependency of text layout —
/// supplied by an adapter (browser canvas `measureText`, a native shaper, or a
/// fixture in tests).
pub trait Measurer {
    /// Advance width of `text` rendered in `style` at `size`, in user units.
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64;

    /// Vertical metrics at `size`. The default uses typical proportions; adapters
    /// with real metrics (canvas font/actual bounding box, or a fixture) override it.
    fn font_metrics(&self, _style: &TextStyle, size: f64) -> FontMetrics {
        FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        }
    }
}

/// Word advance widths measured once at the style's base size. Trial sizes scale
/// these linearly (good enough for layout; avoids re-measuring per fit iteration).
/// `letter_spacing` is carried through so wrapping can add it per grapheme gap
/// without scaling it (it is an absolute length; see [`TextStyle::letter_spacing`]).
pub struct Measured {
    pub words: Vec<(String, f64)>,
    pub space: f64,
    pub letter_spacing: f64,
}

/// Measure each whitespace-separated word (and a space) at `style.size`.
pub fn measure_words(text: &str, style: &TextStyle, m: &dyn Measurer) -> Measured {
    let words: Vec<(String, f64)> = text
        .split_whitespace()
        .map(|w| (w.to_string(), m.measure(w, style, style.size)))
        .collect();
    Measured {
        words,
        space: m.measure(" ", style, style.size),
        letter_spacing: style.letter_spacing,
    }
}

/// Rendered advance of a whole run at `size`, including `letter-spacing` tracking
/// (added once per inter-grapheme gap, on top of the kerned glyph advances that
/// `measure` returns). This is the width a renderer produces for the emitted
/// `letter-spacing` attribute, so layout math must use it, not the raw advance.
pub fn line_advance(text: &str, style: &TextStyle, size: f64, m: &dyn Measurer) -> f64 {
    let gaps = text.chars().count().saturating_sub(1) as f64;
    m.measure(text, style, size) + gaps * style.letter_spacing
}
