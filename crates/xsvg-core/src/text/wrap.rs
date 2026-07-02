//! Greedy (first-fit) line breaking.

use super::measure::Measured;

/// Break `measured` into lines no wider than `max_width`. `scale` maps base-size
/// widths to the trial size (`scale = trial_size / style.size`). A word wider than
/// `max_width` is placed alone (overflow) rather than dropped.
///
/// A line's width includes `letter-spacing`, added once per inter-grapheme gap and
/// *not* scaled by `scale` (it is an absolute length). Tracked separately from the
/// natural advance so the two accumulators stay exact; with no letter-spacing the
/// result is identical to plain greedy wrapping.
pub fn wrap(measured: &Measured, max_width: f64, scale: f64) -> Vec<String> {
    let space = measured.space * scale;
    let ls = measured.letter_spacing;
    let gaps = |graphemes: usize| graphemes.saturating_sub(1) as f64;

    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut nat = 0.0; // scaled natural advance of the current line (words + spaces)
    let mut graph = 0usize; // grapheme count of the current line (spaces included)

    for (word, w0) in &measured.words {
        let w = w0 * scale;
        let g = word.chars().count();
        if cur.is_empty() {
            cur.push_str(word);
            nat = w;
            graph = g;
        } else {
            let (cand_nat, cand_graph) = (nat + space + w, graph + 1 + g);
            if cand_nat + gaps(cand_graph) * ls <= max_width {
                cur.push(' ');
                cur.push_str(word);
                nat = cand_nat;
                graph = cand_graph;
            } else {
                lines.push(std::mem::take(&mut cur));
                cur.push_str(word);
                nat = w;
                graph = g;
            }
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
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
