//! Shrink-to-fit font sizing.

use super::{area::line_size, measure::Measured, style::TextStyle, wrap::wrap_pieces};

/// Largest font size in `[min_size, base_size]` whose wrapped block fits
/// `max_w` × `max_h`. Returns `min_size` if even that overflows. `styles[0]`
/// is the base style (its `size` is the search ceiling and its `line_height`
/// the advance factor); the other entries are the runs' styles, whose sizes
/// scale with the trial.
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
    styles: &[TextStyle],
    max_w: f64,
    max_h: f64,
    min_size: f64,
) -> f64 {
    fit_size_flows(
        std::slice::from_ref(measured),
        styles,
        max_w,
        max_h,
        min_size,
    )
}

/// [`fit_size`] over several forced sub-flows (`<tbreak/>` splits): the block
/// height sums every line's OWN advance (its largest run size × line-height)
/// across all flows — a blank flow is one base-size line — and the widest
/// unbreakable word across all flows must fit.
pub(crate) fn fit_size_flows(
    flows: &[Measured],
    styles: &[TextStyle],
    max_w: f64,
    max_h: f64,
    min_size: f64,
) -> f64 {
    let base_size = styles[0].size;
    let line_height = styles[0].line_height;
    if !base_size.is_finite() || base_size <= 0.0 {
        return 0.0;
    }
    // clamp the floor into (0, base] so the search bounds can't invert
    let min_size = min_size.clamp(0.0, base_size);

    let fits = |size: f64| {
        let scale = size / base_size;
        let height: f64 = flows
            .iter()
            .map(|f| {
                let lines = wrap_pieces(f, max_w, scale);
                if lines.is_empty() {
                    base_size * scale * line_height
                } else {
                    lines
                        .iter()
                        .map(|(p, _)| line_size(p, styles) * scale * line_height)
                        .sum()
                }
            })
            .sum();
        let widest = flows
            .iter()
            .flat_map(|f| f.words.iter().map(move |w| (w, f.letter_spacing)))
            .map(|(w, ls)| w.advance * scale + ls * w.graphemes.saturating_sub(1) as f64)
            .fold(0.0f64, f64::max);
        height <= max_h && widest <= max_w + 1e-6
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
    use crate::text::{
        measure::{measure_runs, measure_words},
        style::TextStyle,
        test_support::Mono,
    };

    fn styles(size: f64) -> [TextStyle; 1] {
        [TextStyle {
            size,
            line_height: 1.2,
            ..Default::default()
        }]
    }

    fn measured(text: &str, per_char: f64, size: f64) -> Measured {
        measure_words(text, &styles(size)[0], &Mono(per_char))
    }

    #[test]
    fn shrinks_when_too_tall_else_keeps_base() {
        let m = measured("aaa bbb ccc ddd", 0.1, 20.0);
        // narrow + short box: at 20px this wraps to 2 lines (48 tall) > 30 → shrink
        let s = fit_size(&m, &styles(20.0), 20.0, 30.0, 6.0);
        assert!((6.0..20.0).contains(&s), "expected shrink, got {s}");
        // generous box → keep base size
        assert_eq!(fit_size(&m, &styles(20.0), 1000.0, 1000.0, 6.0), 20.0);
    }

    #[test]
    fn never_below_min() {
        let m = measured("supercalifragilistic", 0.5, 40.0);
        let s = fit_size(&m, &styles(40.0), 5.0, 5.0, 9.0);
        assert_eq!(s, 9.0);
    }

    #[test]
    fn mixed_run_sizes_count_each_lines_own_height() {
        // base 10 / line-height 1.2, a 20px run on its own line: the block is
        // 12 + 24 = 36 tall, not 2 × 12 — so a 30-tall box forces a shrink that
        // a uniform-size reading would have skipped
        let big = TextStyle {
            size: 20.0,
            ..styles(10.0)[0].clone()
        };
        let st = [styles(10.0)[0].clone(), big];
        let segs = [("aaaa ".to_string(), 0), ("bbbb".to_string(), 1)];
        // Mono(0.1): "aaaa" = 4, "bbbb" at 20 = 8 → width 6 breaks them apart
        let m = measure_runs(&segs, &st, &Mono(0.1));
        assert_eq!(fit_size(&m, &st, 8.5, 40.0, 2.0), 10.0, "36 fits 40");
        let s = fit_size(&m, &st, 8.5, 30.0, 2.0);
        assert!(s < 10.0 && s > 2.0, "36 > 30 must shrink, got {s}");
    }

    #[test]
    fn degenerate_inputs_dont_panic() {
        let m = measured("alpha beta", 0.1, 20.0);
        // min above base → clamped to base (which fits a generous box)
        assert_eq!(fit_size(&m, &styles(20.0), 1000.0, 1000.0, 50.0), 20.0);
        // zero / negative box dimensions → bottoms out at the (clamped) min
        assert_eq!(fit_size(&m, &styles(20.0), 0.0, 0.0, 6.0), 6.0);
        assert_eq!(fit_size(&m, &styles(20.0), -10.0, -10.0, 6.0), 6.0);
        // base size zero or non-finite → no fitting, returned as-is (>= 0)
        assert_eq!(fit_size(&m, &styles(0.0), 100.0, 100.0, 6.0), 0.0);
        assert_eq!(fit_size(&m, &styles(f64::INFINITY), 100.0, 100.0, 6.0), 0.0);
        // empty text → no panic, result stays within [min, base]
        let empty = measured("", 0.1, 20.0);
        let s = fit_size(&empty, &styles(20.0), 10.0, 10.0, 6.0);
        assert!((6.0..=20.0).contains(&s));
    }
}
