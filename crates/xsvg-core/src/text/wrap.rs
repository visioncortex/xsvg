//! Greedy (first-fit) line breaking.

use super::measure::Measured;

/// Break `measured` into lines no wider than `max_width`. `scale` maps base-size
/// widths to the trial size (`scale = trial_size / style.size`). A word wider than
/// `max_width` is placed alone (overflow) rather than dropped.
///
/// A line's width includes `letter-spacing` (once per inter-grapheme gap) and
/// `word-spacing` (folded into each inter-word space), both *not* scaled by `scale`
/// (they are absolute lengths). Tracked separately from the natural advance so the
/// accumulators stay exact; with neither set, the result is identical to plain
/// greedy wrapping.
pub fn wrap(measured: &Measured, max_width: f64, scale: f64) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut i = 0;
    while i < measured.words.len() {
        let (line, next) = fill_line(measured, i, max_width, scale);
        lines.push(line);
        i = next; // fill_line always consumes ≥1 word, so this terminates
    }
    lines
}

/// Fill one line greedily starting at word `start`: always take that word (even if
/// it alone overflows `max_width`), then append following words while the line — with
/// `letter-spacing`/`word-spacing` folded in and *not* scaled — still fits. Returns
/// the joined line text and the index of the next unconsumed word (`> start`).
///
/// The shared inner loop of both rectangular wrapping ([`wrap`]) and region flow,
/// where each line has its own available width.
pub(crate) fn fill_line(
    measured: &Measured,
    start: usize,
    max_width: f64,
    scale: f64,
) -> (String, usize) {
    let space = measured.space * scale + measured.word_spacing;
    let ls = measured.letter_spacing;
    let gaps = |graphemes: usize| graphemes.saturating_sub(1) as f64;

    let (first, w0) = &measured.words[start];
    let mut line = first.clone();
    let mut nat = w0 * scale; // scaled natural advance (words + spaces)
    let mut graph = first.chars().count(); // grapheme count (spaces included)
    let mut i = start + 1;

    while let Some((word, w)) = measured.words.get(i) {
        let (cand_nat, cand_graph) = (nat + space + w * scale, graph + 1 + word.chars().count());
        if cand_nat + gaps(cand_graph) * ls > max_width {
            break;
        }
        line.push(' ');
        line.push_str(word);
        nat = cand_nat;
        graph = cand_graph;
        i += 1;
    }
    (line, i)
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
    fn greedy_wrap_breaks_at_width() {
        // each 3-char word = 3.0 wide, space = 1.0, at size 10 / per_char 0.1
        let m = measured("aaa bbb ccc", 0.1, 10.0);
        assert_eq!(wrap(&m, 7.0, 1.0), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap(&m, 100.0, 1.0), vec!["aaa bbb ccc"]);
        assert_eq!(wrap(&m, 2.0, 1.0), vec!["aaa", "bbb", "ccc"]); // overflow allowed
    }

    #[test]
    fn scale_changes_wrapping() {
        let m = measured("aaa bbb ccc", 0.1, 10.0);
        // at half scale, widths halve → more fits per line
        assert_eq!(wrap(&m, 4.0, 0.5), vec!["aaa bbb", "ccc"]);
    }

    #[test]
    fn letter_spacing_widens_and_wraps_earlier() {
        // Mono(0.1) at size 10: each char = 1.0, space = 1.0. "aaa bbb" = 7 (6 gaps).
        let plain = measured("aaa bbb", 0.1, 10.0);
        assert_eq!(wrap(&plain, 7.0, 1.0), vec!["aaa bbb"]); // fits exactly at ls=0

        let spaced = {
            let st = TextStyle {
                size: 10.0,
                letter_spacing: 1.0,
                ..Default::default()
            };
            measure_words("aaa bbb", &st, &Mono(0.1))
        };
        // ls=1 adds 6 → line is 13 > 7, so it breaks; "aaa" alone = 3 + 2·1 = 5 ≤ 7.
        assert_eq!(wrap(&spaced, 7.0, 1.0), vec!["aaa", "bbb"]);
        // letter-spacing is absolute: halving the scale halves the glyphs (1.5 each)
        // but not the spacing, so "aaa" = 1.5 + 2·1 = 3.5 ≤ 4 while "aaa bbb" won't fit.
        assert_eq!(wrap(&spaced, 4.0, 0.5), vec!["aaa", "bbb"]);
    }

    #[test]
    fn word_spacing_widens_and_wraps_earlier() {
        // Mono(0.1) at size 10: char = 1.0, space = 1.0. "aaa bbb" = 7 (one gap).
        let plain = measured("aaa bbb", 0.1, 10.0);
        assert_eq!(wrap(&plain, 7.0, 1.0), vec!["aaa bbb"]); // fits at word-spacing 0

        let spaced = {
            let st = TextStyle {
                size: 10.0,
                word_spacing: 3.0,
                ..Default::default()
            };
            measure_words("aaa bbb", &st, &Mono(0.1))
        };
        // word-spacing=3 widens the single inter-word gap: 7 + 3 = 10 > 7 → breaks.
        assert_eq!(wrap(&spaced, 7.0, 1.0), vec!["aaa", "bbb"]);
        // absolute: at half scale glyphs (3.0) + scaled space (0.5) halve but the
        // word-spacing (3.0) stays, so "aaa bbb" = 6.5 > 6 still breaks.
        assert_eq!(wrap(&spaced, 6.0, 0.5), vec!["aaa", "bbb"]);
    }

    #[test]
    fn negative_spacing_condenses_without_panic() {
        // Negative letter/word-spacing (tighter tracking) is valid CSS; it must not
        // underflow or panic — the saturating gap arithmetic just yields a smaller
        // (possibly ≤0) line width, so everything fits on one line.
        let st = TextStyle {
            size: 10.0,
            letter_spacing: -2.0,
            word_spacing: -3.0,
            ..Default::default()
        };
        let m = measure_words("aaa bbb ccc", &st, &Mono(0.1));
        assert_eq!(wrap(&m, 50.0, 1.0), vec!["aaa bbb ccc"]);
        // even a zero-width box can't panic (condensed width goes ≤0 → all fits)
        assert!(!wrap(&m, 0.0, 1.0).is_empty());
    }

    #[test]
    fn degenerate_wrap() {
        assert!(wrap(&measured("", 0.1, 10.0), 100.0, 1.0).is_empty());
        assert!(wrap(&measured("  \n\t ", 0.1, 10.0), 100.0, 1.0).is_empty());

        let m = measured("aaa bbb", 0.1, 10.0);
        assert_eq!(wrap(&m, 0.0, 1.0), vec!["aaa", "bbb"]);
        assert_eq!(wrap(&m, -5.0, 1.0), vec!["aaa", "bbb"]);

        let long = measured("supercalifragilistic", 0.5, 10.0);
        assert_eq!(wrap(&long, 3.0, 1.0), vec!["supercalifragilistic"]);
    }
}
