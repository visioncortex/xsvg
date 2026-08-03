//! Shrink-to-fit font sizing.

use super::{measure::Measured, wrap::wrap};

/// Largest font size in `[min_size, base_size]` whose wrapped block fits
/// `max_w` × `max_h`. Returns `min_size` if even that overflows.
///
/// The block's *height* drives the search — `wrap` already reflows text to
/// `max_w` at every trial, so shrinking wins back vertical space — plus one
/// width condition: the widest unbreakable word must fit `max_w`, since a
/// lone long token is the only line greedy wrap lets overflow (§6.6); a
/// shrink box shrinks it into the box instead of spilling horizontally.
/// Each trial re-wraps (cheap — base-size widths are scaled, not
/// re-measured).
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
    fit_size_flows(
        std::slice::from_ref(measured),
        base_size,
        line_height,
        max_w,
        max_h,
        min_size,
    )
}

/// [`fit_size`] over several forced sub-flows (`<tbreak/>` splits): the block
/// height sums every flow's wrapped lines — a blank flow is one forced blank
/// line — and the widest unbreakable word across all flows must fit.
pub(crate) fn fit_size_flows(
    flows: &[Measured],
    base_size: f64,
    line_height: f64,
    max_w: f64,
    max_h: f64,
    min_size: f64,
) -> f64 {
    if !base_size.is_finite() || base_size <= 0.0 {
        return 0.0;
    }
    // clamp the floor into (0, base] so the search bounds can't invert
    let min_size = min_size.clamp(0.0, base_size);

    let fits = |size: f64| {
        let scale = size / base_size;
        let lines: f64 = flows
            .iter()
            .map(|f| wrap(f, max_w, scale).len().max(1) as f64)
            .sum();
        let widest = flows
            .iter()
            .flat_map(|f| f.words.iter().map(move |w| (w, f.letter_spacing)))
            .map(|(w, ls)| w.advance * scale + ls * w.graphemes.saturating_sub(1) as f64)
            .fold(0.0f64, f64::max);
        lines * size * line_height <= max_h && widest <= max_w + 1e-6
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

    #[test]
    fn degenerate_inputs_dont_panic() {
        let m = measured("alpha beta", 0.1, 20.0);
        // min above base → clamped to base (which fits a generous box)
        assert_eq!(fit_size(&m, 20.0, 1.2, 1000.0, 1000.0, 50.0), 20.0);
        // zero / negative box dimensions → bottoms out at the (clamped) min
        assert_eq!(fit_size(&m, 20.0, 1.2, 0.0, 0.0, 6.0), 6.0);
        assert_eq!(fit_size(&m, 20.0, 1.2, -10.0, -10.0, 6.0), 6.0);
        // base size zero or non-finite → no fitting, returned as-is (>= 0)
        assert_eq!(fit_size(&m, 0.0, 1.2, 100.0, 100.0, 6.0), 0.0);
        assert_eq!(fit_size(&m, f64::INFINITY, 1.2, 100.0, 100.0, 6.0), 0.0);
        // empty text → no panic, result stays within [min, base]
        let empty = measured("", 0.1, 20.0);
        let s = fit_size(&empty, 20.0, 1.2, 10.0, 10.0, 6.0);
        assert!((6.0..=20.0).contains(&s));
    }
}
