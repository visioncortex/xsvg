//! SVG Tiny 1.2 `textArea` layout (line-box model).
//!
//! Distinct from [`super::area`] (the xsvg `<x:textbox>`, which centres optically
//! on cap-height): this follows the spec — `text-align` for inline alignment,
//! `display-align` for block alignment, `line-increment` for line spacing, `auto`
//! width/height, and overflow clipping. Shares measurement and wrapping with the
//! rest of the engine.

use super::area::{merge_pieces, Anchor, AreaLayout, PlacedLine};
use super::measure::{measure_runs, Measurer, Piece};
use super::style::TextStyle;
use super::truncate::{apply_ellipsis, TextOverflow};
use super::wrap::wrap_pieces;

/// `text-align` — inline alignment of lines. `justify` extends the SVG Tiny 1.2
/// vocabulary with the CSS/SVG 2 value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

impl TextAlign {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            "end" => Self::End,
            "justify" => Self::Justify,
            _ => Self::Start,
        }
    }
    fn anchor(self) -> Anchor {
        match self {
            Self::Center => Anchor::Middle,
            Self::End => Anchor::End,
            // justified lines begin at the start edge; textLength fills to the right
            Self::Start | Self::Justify => Anchor::Start,
        }
    }
}

/// `display-align` — block alignment within the region (`auto` == `before`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DisplayAlign {
    #[default]
    Before,
    Center,
    After,
}

impl DisplayAlign {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            "after" => Self::After,
            _ => Self::Before, // "before" and "auto"
        }
    }
}

/// `line-increment` — line-box height (`auto` = 1.1 × font-size per the spec).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineIncrement {
    Auto,
    Fixed(f64),
}

impl LineIncrement {
    fn resolve(self, size: f64) -> f64 {
        match self {
            LineIncrement::Fixed(v) if v > 0.0 => v,
            _ => 1.1 * size, // auto, or unsupported non-positive value
        }
    }
}

/// A textArea region. `None` width/height mean `auto` (no wrap / no clip).
#[derive(Clone, Copy, Debug)]
pub struct TextAreaSpec {
    pub x: f64,
    pub y: f64,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub text_align: TextAlign,
    pub display_align: DisplayAlign,
    pub line_increment: LineIncrement,
    pub text_overflow: TextOverflow,
}

/// Lay flowed text into a textArea per SVG Tiny 1.2: wrap to the width (or not, if
/// `auto`), stack line boxes of height `line-increment`, place baselines at the
/// font ascent within each box, apply `display-align`, and clip lines whose box
/// falls outside an explicit-height region.
pub fn layout_text_area(
    text: &str,
    style: &TextStyle,
    spec: &TextAreaSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    layout_text_area_runs(&[(text.to_string(), 0)], &[style.clone()], spec, m)
}

/// Like [`layout_text_area`], but takes styled segments (`(text, style_id)`, from
/// `<tspan>` runs) plus the style table (`styles[0]` is the base). `'\n'` — anywhere
/// in the segment stream — is a `<tbreak/>` forced break, splitting into paragraphs
/// that wrap independently.
pub fn layout_text_area_runs(
    segments: &[(String, usize)],
    styles: &[TextStyle],
    spec: &TextAreaSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    let style = &styles[0];
    let size = style.size;
    let max_w = spec.width.unwrap_or(f64::INFINITY); // auto width → no wrapping

    // Split the styled segment stream at `'\n'` into paragraphs (each a list of
    // styled sub-segments), then wrap each independently. Track whether each line is
    // its paragraph's last, so justification leaves the paragraph-final line ragged.
    let mut paragraphs: Vec<Vec<(String, usize)>> = vec![Vec::new()];
    let mut breaks = 0usize;
    for (text, sid) in segments {
        for (k, part) in text.split('\n').enumerate() {
            if k > 0 {
                paragraphs.push(Vec::new());
                breaks += 1;
            }
            if !part.is_empty() {
                paragraphs
                    .last_mut()
                    .unwrap()
                    .push((part.to_string(), *sid));
            }
        }
    }
    let has_breaks = breaks > 0;

    let mut line_metas: Vec<(Vec<Piece>, bool)> = Vec::new(); // (pieces, is_paragraph_final)
    for para in &paragraphs {
        if para.is_empty() {
            if has_breaks {
                line_metas.push((Vec::new(), true)); // forced blank line
            }
            continue;
        }
        let wrapped = wrap_pieces(&measure_runs(para, styles, m), max_w, 1.0);
        let n = wrapped.len();
        for (j, pieces) in wrapped.into_iter().enumerate() {
            line_metas.push((pieces, j + 1 == n));
        }
    }

    let line_h = spec.line_increment.resolve(size);
    let fm = m.font_metrics(style, size);

    // Align the cap-height ink band (first line's cap-top → last line's baseline +
    // descent) optically, like <x:textbox> (§6.5) — not the em/line boxes, whose
    // ascent/descent asymmetry biases centred text low. Lines still step by
    // line-increment.
    let n = line_metas.len().max(1) as f64;
    let band_h = fm.cap_height + fm.descent + (n - 1.0) * line_h;
    let block_top = match (spec.height, spec.display_align) {
        (Some(h), DisplayAlign::Center) => spec.y + (h - band_h) / 2.0,
        (Some(h), DisplayAlign::After) => spec.y + (h - band_h),
        _ => spec.y, // before, or auto height
    };
    let first_baseline = block_top + fm.cap_height;

    let ax = match (spec.text_align, spec.width) {
        (TextAlign::Center, Some(w)) => spec.x + w / 2.0,
        (TextAlign::End, Some(w)) => spec.x + w,
        _ => spec.x,
    };

    // Justify needs a known, positive width; auto-width falls back to `start`.
    let justify_w = match (spec.text_align, spec.width) {
        (TextAlign::Justify, Some(w)) if w > 0.0 => Some(w),
        _ => None,
    };

    let mut lines = Vec::new();
    let mut dropped = false;
    for (i, (pieces, is_final)) in line_metas.into_iter().enumerate() {
        let baseline = first_baseline + i as f64 * line_h;
        // clip by the line's ink band [cap-top, baseline + descent]
        if let Some(h) = spec.height {
            if baseline - fm.cap_height < spec.y - 1e-6 || baseline + fm.descent > spec.y + h + 1e-6
            {
                dropped = true;
                continue; // outside the region in the block direction → not rendered
            }
        }
        let (text, runs) = merge_pieces(pieces);
        // stretch full, non-paragraph-final, multi-word lines to the content width
        let justify_width = justify_w.filter(|_| !is_final && text.contains(' '));
        lines.push(PlacedLine {
            text,
            x: ax,
            baseline,
            justify_width,
            runs,
        });
    }
    if spec.text_overflow == TextOverflow::Ellipsis {
        apply_ellipsis(&mut lines, dropped, max_w, style, size, m);
    }

    AreaLayout {
        anchor: spec.text_align.anchor(),
        font_size: size,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::test_support::Mono;

    fn spec(width: Option<f64>, height: Option<f64>) -> TextAreaSpec {
        TextAreaSpec {
            x: 0.0,
            y: 0.0,
            width,
            height,
            text_align: TextAlign::Start,
            display_align: DisplayAlign::Before,
            line_increment: LineIncrement::Auto,
            text_overflow: TextOverflow::Clip,
        }
    }

    #[test]
    fn auto_width_does_not_wrap() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let out = layout_text_area(
            "one two three four five",
            &st,
            &spec(None, None),
            &Mono(0.2),
        );
        assert_eq!(out.lines.len(), 1);
    }

    #[test]
    fn explicit_width_wraps() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let out = layout_text_area("aaa bbb ccc", &st, &spec(Some(7.0), None), &Mono(0.1));
        assert!(out.lines.len() > 1);
    }

    #[test]
    fn line_increment_auto_is_1_1_em() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let out = layout_text_area("aa bb", &st, &spec(Some(2.0), None), &Mono(0.1));
        assert_eq!(out.lines.len(), 2);
        // auto line-increment = 1.1 * 10 = 11 (baselines still step by it)
        assert!((out.lines[1].baseline - out.lines[0].baseline - 11.0).abs() < 1e-9);
        // before-aligned: first cap-top at the top edge → baseline = cap_height (0.7·10)
        assert!((out.lines[0].baseline - 7.0).abs() < 1e-9);
    }

    #[test]
    fn fixed_line_increment() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let mut s = spec(Some(2.0), None);
        s.line_increment = LineIncrement::Fixed(20.0);
        let out = layout_text_area("aa bb", &st, &s, &Mono(0.1));
        assert!((out.lines[1].baseline - out.lines[0].baseline - 20.0).abs() < 1e-9);
    }

    #[test]
    fn text_align_sets_anchor_and_x() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let mut s = spec(Some(100.0), None);
        s.text_align = TextAlign::Center;
        let out = layout_text_area("hi", &st, &s, &Mono(0.1));
        assert_eq!(out.anchor, Anchor::Middle);
        assert_eq!(out.lines[0].x, 50.0);
        s.text_align = TextAlign::End;
        assert_eq!(
            layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].x,
            100.0
        );
    }

    #[test]
    fn display_align_positions_block() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // one line, cap-height band = cap(7) + descent(2) = 9, region height 100
        let mut s = spec(Some(50.0), Some(100.0));
        s.display_align = DisplayAlign::Before;
        // cap-top at the top edge → baseline = cap_height = 7
        assert!((layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 7.0).abs() < 1e-9);
        s.display_align = DisplayAlign::Center;
        // band centred: top = (100-9)/2 = 45.5; baseline = 45.5 + cap(7) = 52.5
        assert!(
            (layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 52.5).abs() < 1e-9
        );
        s.display_align = DisplayAlign::After;
        // band bottom at the edge: top = 100-9 = 91; baseline = 91 + cap(7) = 98
        assert!(
            (layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 98.0).abs() < 1e-9
        );
    }

    #[test]
    fn explicit_height_clips_overflow() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // 6 short words, each its own line at width 2 → line_inc 11 each.
        // region height 30 fits floor(30/11) = 2 line boxes (top-aligned).
        let out = layout_text_area(
            "aa bb cc dd ee ff",
            &st,
            &spec(Some(2.0), Some(30.0)),
            &Mono(0.1),
        );
        assert_eq!(out.lines.len(), 2);
    }

    #[test]
    fn justify_stretches_full_lines_only() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // width 8 (Mono: char=1, space=1) → "aa bb cc" | "dd ee ff", two full lines
        let mut s = spec(Some(8.0), None);
        s.text_align = TextAlign::Justify;
        let out = layout_text_area("aa bb cc dd ee ff", &st, &s, &Mono(0.1));
        assert!(out.lines.len() >= 2);
        assert_eq!(out.anchor, Anchor::Start);
        let (all_but_last, last) = out.lines.split_at(out.lines.len() - 1);
        for l in all_but_last {
            assert_eq!(l.justify_width, Some(8.0), "full line not justified: {l:?}");
        }
        // paragraph-final line stays ragged
        assert_eq!(last[0].justify_width, None);
    }

    #[test]
    fn justify_leaves_paragraph_final_lines_ragged_across_tbreak() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // width 8 → each paragraph wraps to "xx yy zz" | "ww": a justified full line
        // then a ragged segment-final line. Two paragraphs → 2 justified, 2 ragged.
        let mut s = spec(Some(8.0), None);
        s.text_align = TextAlign::Justify;
        let out = layout_text_area("aa bb cc dd\nee ff gg hh", &st, &s, &Mono(0.1));
        let justified = out
            .lines
            .iter()
            .filter(|l| l.justify_width.is_some())
            .count();
        let ragged = out.lines.len() - justified;
        assert!(
            justified >= 2 && ragged >= 2,
            "each paragraph should have a justified line + a ragged final line: {:?}",
            out.lines
        );
    }

    #[test]
    fn justify_needs_a_width() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // auto width → nothing to justify against → all lines ragged (degrades to start)
        let mut s = spec(None, None);
        s.text_align = TextAlign::Justify;
        let out = layout_text_area("aa bb cc dd", &st, &s, &Mono(0.1));
        assert!(out.lines.iter().all(|l| l.justify_width.is_none()));
        assert_eq!(out.anchor, Anchor::Start);
    }

    #[test]
    fn styled_runs_split_a_line_into_runs() {
        let base = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let bold = TextStyle {
            weight: "bold".into(),
            size: 10.0,
            ..Default::default()
        };
        let styles = [base, bold];
        // "Ship " (base) + "fast" (bold) + " and safe" (base), wide box → one line
        let segs = [
            ("Ship ".to_string(), 0),
            ("fast".to_string(), 1),
            (" and safe".to_string(), 0),
        ];
        let out = layout_text_area_runs(&segs, &styles, &spec(Some(1000.0), None), &Mono(0.1));
        assert_eq!(out.lines.len(), 1);
        let line = &out.lines[0];
        assert_eq!(line.text, "Ship fast and safe");
        assert_eq!(
            line.runs.iter().map(|r| r.style).collect::<Vec<_>>(),
            vec![0, 1, 0]
        );
        assert!(line.runs[1].text.contains("fast"));
    }

    #[test]
    fn styled_runs_handle_mid_word_boundaries() {
        let styles = [
            TextStyle {
                size: 10.0,
                ..Default::default()
            },
            TextStyle {
                weight: "bold".into(),
                size: 10.0,
                ..Default::default()
            },
        ];
        // "un" + bold "real", no space between → one word, two runs
        let segs = [("un".to_string(), 0), ("real".to_string(), 1)];
        let out = layout_text_area_runs(&segs, &styles, &spec(Some(1000.0), None), &Mono(0.1));
        assert_eq!(out.lines.len(), 1);
        assert_eq!(out.lines[0].text, "unreal");
        assert_eq!(
            out.lines[0]
                .runs
                .iter()
                .map(|r| r.style)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn tbreak_forces_breaks() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // auto width would be one line; \n (from <tbreak/>) forces breaks
        let out = layout_text_area("one\ntwo\nthree", &st, &spec(None, None), &Mono(0.2));
        assert_eq!(out.lines.len(), 3);
        // consecutive breaks → a blank line between
        let out2 = layout_text_area("a\n\nb", &st, &spec(None, None), &Mono(0.2));
        assert_eq!(out2.lines.len(), 3);
        assert_eq!(out2.lines[1].text, "");
    }

    #[test]
    fn ellipsis_marks_clipped_textarea() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let mut s = spec(Some(2.0), Some(30.0));
        s.text_overflow = TextOverflow::Ellipsis;
        let out = layout_text_area("aa bb cc dd ee ff", &st, &s, &Mono(0.1));
        assert!(!out.lines.is_empty());
        assert!(out.lines.last().unwrap().text.ends_with('…'));
    }
}
