//! Shrink-to-fit font sizing.

use super::{measure::Measured, wrap::wrap};

/// Largest font size in `[min_size, base_size]` whose wrapped block fits
/// `max_w` × `max_h`. Returns `min_size` if even that overflows the height.
///
/// Only the *height* is searched: `wrap` already reflows text to `max_w` at every
/// trial, so shrinking just needs to win back vertical space. Each trial re-wraps
/// (cheap — base-size widths are scaled, not re-measured).
///
/// Crate-internal: `area::layout_area` is the public entry point for fitting.
pub(crate) fn fit_size(
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
    use crate::text::{measure::measure_words, style::TextStyle, test_support::Mono};

    fn measured(text: &str, per_char: f64, size: f64) -> Measured {
        let style = TextStyle {
            size,
            ..Default::default()
        };
        measure_words(text, &style, &Mono(per_char))
    }

    #[test]
    fn shrinks_when_too_tall_else_keeps_base() {
        let m = measured("aaa bbb ccc ddd", 0.1, 20.0);
        // narrow + short box: at 20px this wraps to 2 lines (48 tall) > 30 → shrink
        let s = fit_size(&m, 20.0, 1.2, 20.0, 30.0, 6.0);
        assert!(s < 20.0 && s >= 6.0, "expected shrink, got {s}");
        // generous box → keep base size
        assert_eq!(fit_size(&m, 20.0, 1.2, 1000.0, 1000.0, 6.0), 20.0);
    }

    #[test]
    fn never_below_min() {
        let m = measured("supercalifragilistic", 0.5, 40.0);
        let s = fit_size(&m, 40.0, 1.2, 5.0, 5.0, 9.0);
        assert_eq!(s, 9.0);
    }
}
