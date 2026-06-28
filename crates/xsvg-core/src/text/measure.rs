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
pub struct Measured {
    pub words: Vec<(String, f64)>,
    pub space: f64,
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
    }
}
