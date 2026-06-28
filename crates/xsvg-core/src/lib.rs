//! Shared, pure-Rust types for the xsvg engine.
//!
//! This crate must stay free of platform/JS/web dependencies so it compiles
//! identically for native and `wasm32` targets (see Plan.md §1, "Core invariant").

/// The single approximation knob threaded through every lowering pass.
/// See Plan.md §1.3. In v0 it is parsed and carried through but only lightly
/// exercised (no curve flattening yet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QualityProfile {
    /// Coarse tolerances; eager rasterization.
    Fast,
    /// Sensible middle ground.
    #[default]
    Balanced,
    /// Tight tolerances; vector-exact where possible.
    Highest,
    /// Rasterize hard cases to an embedded image.
    Raster,
}

impl QualityProfile {
    /// Parse a quality string (case-insensitive); unknown values fall back to `Balanced`.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" => Self::Fast,
            "highest" => Self::Highest,
            "raster" => Self::Raster,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Highest => "highest",
            Self::Raster => "raster",
        }
    }
}

// ----------------------------------------------------------------------------
// Text layout (the v0 typography POC).
//
// `Measurer` is the pure-Rust seam for font metrics (Plan.md §1.2, FontProvider).
// v0's browser adapter implements it via canvas `measureText`; native tests use a
// deterministic mock. All wrapping/fitting logic below is pure and unit-testable.
// ----------------------------------------------------------------------------

/// Resolved text styling needed to measure and lay out a run.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub family: String,
    pub size: f64,
    pub weight: String,
    pub style: String,
    /// Line advance as a multiple of `size`.
    pub line_height: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            size: 16.0,
            weight: "normal".into(),
            style: "normal".into(),
            line_height: 1.2,
        }
    }
}

impl TextStyle {
    /// A CSS `font` shorthand at the given size (what canvas `measureText` wants).
    pub fn font_css(&self, size: f64) -> String {
        format!("{} {} {}px {}", self.style, self.weight, size, self.family)
    }
}

/// Source of glyph-run advance widths. The only platform-specific dependency of
/// text layout — supplied by an adapter (browser canvas, or native shaper later).
pub trait Measurer {
    /// Advance width of `text` rendered in `style` at `size`, in user units.
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64;
}

/// Word advance widths measured once at the style's base size. Trial sizes scale
/// these linearly (good enough for layout; avoids re-measuring per fit iteration).
pub struct Measured {
    pub words: Vec<(String, f64)>,
    pub space: f64,
}

/// Measure each whitespace-separated word (and a space) at `style.size`.
pub fn measure_words(text: &str, style: &TextStyle, m: &dyn Measurer) -> Measured {
    let font_words: Vec<(String, f64)> = text
        .split_whitespace()
        .map(|w| (w.to_string(), m.measure(w, style, style.size)))
        .collect();
    Measured {
        words: font_words,
        space: m.measure(" ", style, style.size),
    }
}

/// Greedy line-breaking. `scale` maps base-size widths to the trial size
/// (`scale = trial_size / style.size`). A word wider than `max_width` is placed
/// alone (overflow) rather than dropped.
pub fn wrap(measured: &Measured, max_width: f64, scale: f64) -> Vec<String> {
    let space = measured.space * scale;
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0.0;

    for (word, w0) in &measured.words {
        let w = w0 * scale;
        if cur.is_empty() {
            cur.push_str(word);
            cur_w = w;
        } else if cur_w + space + w <= max_width {
            cur.push(' ');
            cur.push_str(word);
            cur_w += space + w;
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
            cur_w = w;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// Largest font size in `[min_size, base_size]` whose wrapped block fits
/// `max_w` × `max_h`. Returns `min_size` if even that overflows the height.
pub fn fit_size(
    measured: &Measured,
    base_size: f64,
    line_height: f64,
    max_w: f64,
    max_h: f64,
    min_size: f64,
) -> f64 {
    let fits = |size: f64| {
        let scale = size / base_size;
        let lines = wrap(measured, max_w, scale).len().max(1) as f64;
        lines * size * line_height <= max_h
    };

    if fits(base_size) {
        return base_size;
    }
    let (mut lo, mut hi) = (min_size, base_size);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo.max(min_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown() {
        assert_eq!(QualityProfile::parse("FAST"), QualityProfile::Fast);
        assert_eq!(QualityProfile::parse("  Highest "), QualityProfile::Highest);
        assert_eq!(QualityProfile::parse("nonsense"), QualityProfile::Balanced);
        assert_eq!(QualityProfile::default(), QualityProfile::Balanced);
    }

    /// Deterministic measurer: width = char count × per_char × size.
    struct Mono(f64);
    impl Measurer for Mono {
        fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
            text.chars().count() as f64 * self.0 * size
        }
    }

    fn measured(text: &str, per_char: f64, size: f64) -> Measured {
        let style = TextStyle { size, ..Default::default() };
        measure_words(text, &style, &Mono(per_char))
    }

    #[test]
    fn greedy_wrap_breaks_at_width() {
        // each 3-char word = 3.0 wide, space = 1.0, at size 10 / per_char 0.1
        let m = measured("aaa bbb ccc", 0.1, 10.0);
        // width 7 fits "aaa bbb" (3+1+3=7) then "ccc"
        assert_eq!(wrap(&m, 7.0, 1.0), vec!["aaa bbb", "ccc"]);
        // width 100 fits all on one line
        assert_eq!(wrap(&m, 100.0, 1.0), vec!["aaa bbb ccc"]);
        // width 2 forces one word per line (overflow allowed)
        assert_eq!(wrap(&m, 2.0, 1.0), vec!["aaa", "bbb", "ccc"]);
    }

    #[test]
    fn shrink_to_fit_reduces_size_when_too_tall() {
        let m = measured("aaa bbb ccc ddd", 0.1, 20.0);
        // narrow + short box: at size 20 this wraps to 2 lines (48 tall) > 30 → must shrink
        let s = fit_size(&m, 20.0, 1.2, 20.0, 30.0, 6.0);
        assert!(s < 20.0 && s >= 6.0, "expected shrink, got {s}");
        // generous box → keep base size
        let s2 = fit_size(&m, 20.0, 1.2, 1000.0, 1000.0, 6.0);
        assert_eq!(s2, 20.0);
    }
}
