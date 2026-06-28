//! Greedy (first-fit) line breaking.

use super::measure::Measured;

/// Break `measured` into lines no wider than `max_width`. `scale` maps base-size
/// widths to the trial size (`scale = trial_size / style.size`). A word wider than
/// `max_width` is placed alone (overflow) rather than dropped.
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
