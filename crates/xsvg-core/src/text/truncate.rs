//! Overflow truncation shared by box text layouts (Specification.md §6.6).

use super::area::{PlacedLine, Run};
use super::measure::{line_advance, Measurer};
use super::style::TextStyle;

const ELLIPSIS: &str = "…";

/// `text-overflow` — what to do with content that doesn't fit the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextOverflow {
    /// Drop overflow lines, no marker (SVG Tiny 1.2 behavior).
    #[default]
    Clip,
    /// Mark cut content with an ellipsis on the last/over-long line.
    Ellipsis,
}

impl TextOverflow {
    pub fn parse(s: &str) -> Self {
        if s == "ellipsis" {
            Self::Ellipsis
        } else {
            Self::Clip
        }
    }
}

/// Trim `line` (by character, stripping trailing whitespace) so that `line + …`
/// fits `max_width`, then append the ellipsis. Returns `None` if the box is
/// narrower than the ellipsis itself (nothing should render).
pub fn ellipsize_line(
    line: &str,
    max_width: f64,
    style: &TextStyle,
    size: f64,
    m: &dyn Measurer,
) -> Option<String> {
    if line_advance(ELLIPSIS, style, size, m) > max_width {
        return None;
    }
    let mut s: String = line.trim_end().to_string();
    while line_advance(&format!("{s}{ELLIPSIS}"), style, size, m) > max_width {
        if s.pop().is_none() {
            break;
        }
        while s.chars().next_back().is_some_and(char::is_whitespace) {
            s.pop();
        }
    }
    Some(format!("{s}{ELLIPSIS}"))
}

/// Ellipsize a placed line in-place: truncate its text to fit `max_width` and
/// rebuild its `runs` to match (dropping/trimming trailing runs, appending the
/// marker to the last surviving run). Empties the line if nothing fits.
pub(crate) fn ellipsize_placed(
    line: &mut PlacedLine,
    max_width: f64,
    style: &TextStyle,
    size: f64,
    m: &dyn Measurer,
) {
    line.justify_width = None; // an ellipsized line renders at natural width
    let truncated = ellipsize_line(&line.text, max_width, style, size, m).unwrap_or_default();
    if truncated.is_empty() {
        line.text.clear();
        line.runs.clear();
        return;
    }
    // `truncated` = <kept prefix of the line, trailing space trimmed><…>. Take that
    // many body chars from the original runs (preserving each run's style), then
    // append the marker to the last surviving run.
    let body = truncated.strip_suffix(ELLIPSIS).unwrap_or(&truncated);
    let keep = body.chars().count();
    let mut new_runs: Vec<Run> = Vec::new();
    let mut remaining = keep;
    for run in &line.runs {
        if remaining == 0 {
            break;
        }
        let take = run.text.chars().count().min(remaining);
        let s: String = run.text.chars().take(take).collect();
        remaining -= take;
        if !s.is_empty() {
            new_runs.push(Run {
                text: s,
                style: run.style,
            });
        }
    }
    match new_runs.last_mut() {
        Some(last) => last.text.push_str(ELLIPSIS),
        None => new_runs.push(Run {
            text: ELLIPSIS.to_string(),
            style: 0,
        }),
    }
    line.runs = new_runs;
    line.text = truncated;
}

/// Mark already-clipped lines: ellipsize the last line if any were dropped
/// (block overflow), and any line wider than `max_width` (inline overflow).
/// Lines that can't fit even the ellipsis are removed.
pub fn apply_ellipsis(
    lines: &mut Vec<PlacedLine>,
    dropped: bool,
    max_width: f64,
    style: &TextStyle,
    size: f64,
    m: &dyn Measurer,
) {
    for line in lines.iter_mut() {
        if !line.text.ends_with(ELLIPSIS)
            && line_advance(&line.text, style, size, m) > max_width + 1e-6
        {
            ellipsize_placed(line, max_width, style, size, m);
        }
    }
    if dropped {
        if let Some(last) = lines.last_mut() {
            if !last.text.ends_with(ELLIPSIS) {
                ellipsize_placed(last, max_width, style, size, m);
            }
        }
    }
    lines.retain(|l| !l.text.is_empty());
}

/// The lines a box shows and whether any fell outside it: `inside` is the flow's
/// own block rule (a textbox keeps a line by its baseline, a `<textArea>` by its
/// ink band), applied after every line is placed. The split every rectangular
/// flow makes before deciding what `text-overflow` does about the remainder.
pub fn split_visible(
    lines: Vec<PlacedLine>,
    inside: impl Fn(&PlacedLine) -> bool,
) -> (Vec<PlacedLine>, bool) {
    let n = lines.len();
    let kept: Vec<PlacedLine> = lines.into_iter().filter(|l| inside(l)).collect();
    let dropped = kept.len() < n;
    (kept, dropped)
}

/// [`split_visible`] followed by the overflow rule (§6.6): under `ellipsis` the
/// last line that shows is marked when any was dropped, and any line wider than
/// `max_width` is cut to fit; under `clip` the remainder is simply gone. One
/// call, so no flow can clip and forget the mark.
#[allow(clippy::too_many_arguments)]
pub fn clip_lines(
    lines: Vec<PlacedLine>,
    inside: impl Fn(&PlacedLine) -> bool,
    overflow: TextOverflow,
    max_width: f64,
    style: &TextStyle,
    size: f64,
    m: &dyn Measurer,
) -> Vec<PlacedLine> {
    let (mut kept, dropped) = split_visible(lines, inside);
    if overflow == TextOverflow::Ellipsis {
        apply_ellipsis(&mut kept, dropped, max_width, style, size, m);
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::test_support::Mono;

    fn style() -> TextStyle {
        TextStyle {
            size: 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn trims_until_marker_fits() {
        // Mono(0.1): width = chars * 0.1 * 10 = chars. "abcdef…" must fit width 4.
        let out = ellipsize_line("abcdef", 4.0, &style(), 10.0, &Mono(0.1)).unwrap();
        assert!(out.ends_with('…'));
        assert!(Mono(0.1).measure(&out, &style(), 10.0) <= 4.0 + 1e-9);
    }

    #[test]
    fn narrow_box_renders_nothing() {
        // ellipsis itself is 1 char wide = 1.0; box 0.5 can't hold it.
        assert!(ellipsize_line("abc", 0.5, &style(), 10.0, &Mono(0.1)).is_none());
    }

    #[test]
    fn empties_to_just_ellipsis() {
        // width exactly fits the ellipsis (1.0) but no content.
        assert_eq!(
            ellipsize_line("abc", 1.0, &style(), 10.0, &Mono(0.1)).unwrap(),
            "…"
        );
    }
}
