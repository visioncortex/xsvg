//! SVG Tiny 1.2 `textArea` layout (line-box model).
//!
//! Distinct from [`super::area`] (the xsvg `<x:textbox>`, which centres optically
//! on cap-height): this follows the spec — `text-align` for inline alignment,
//! `display-align` for block alignment, `line-increment` for line spacing, `auto`
//! width/height, and overflow clipping. Shares measurement and wrapping with the
//! rest of the engine.

use super::area::{Anchor, AreaLayout, PlacedLine};
use super::measure::{measure_words, Measurer};
use super::style::TextStyle;
use super::truncate::{apply_ellipsis, TextOverflow};
use super::wrap::wrap;

/// `text-align` — inline alignment of lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

impl TextAlign {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            "end" => Self::End,
            _ => Self::Start,
        }
    }
    fn anchor(self) -> Anchor {
        match self {
            Self::Start => Anchor::Start,
            Self::Center => Anchor::Middle,
            Self::End => Anchor::End,
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
    let size = style.size;
    let max_w = spec.width.unwrap_or(f64::INFINITY); // auto width → no wrapping

    // `\n` marks a forced break (<tbreak/>); wrap each segment independently.
    let segments: Vec<&str> = text.split('\n').collect();
    let has_breaks = segments.len() > 1;
    let mut line_texts: Vec<String> = Vec::new();
    for seg in segments {
        let wrapped = wrap(&measure_words(seg, style, m), max_w, 1.0);
        if wrapped.is_empty() {
            if has_breaks {
                line_texts.push(String::new()); // forced blank line
            }
        } else {
            line_texts.extend(wrapped);
        }
    }

    let line_h = spec.line_increment.resolve(size);
    let fm = m.font_metrics(style, size);
    // baseline sits at the font's ascent for an assumed font-size = line-increment
    let baseline_offset = if size > 0.0 {
        fm.ascent * line_h / size
    } else {
        0.0
    };

    let block_h = line_texts.len() as f64 * line_h;
    let block_top = match (spec.height, spec.display_align) {
        (Some(h), DisplayAlign::Center) => spec.y + (h - block_h) / 2.0,
        (Some(h), DisplayAlign::After) => spec.y + (h - block_h),
        _ => spec.y, // before, or auto height
    };
    let first_baseline = block_top + baseline_offset;

    let ax = match (spec.text_align, spec.width) {
        (TextAlign::Center, Some(w)) => spec.x + w / 2.0,
        (TextAlign::End, Some(w)) => spec.x + w,
        _ => spec.x,
    };

    let mut lines = Vec::new();
    let mut dropped = false;
    for (i, text) in line_texts.into_iter().enumerate() {
        let baseline = first_baseline + i as f64 * line_h;
        if let Some(h) = spec.height {
            let box_top = baseline - baseline_offset;
            if box_top < spec.y - 1e-6 || box_top + line_h > spec.y + h + 1e-6 {
                dropped = true;
                continue; // outside the region in the block direction → not rendered
            }
        }
        lines.push(PlacedLine {
            text,
            x: ax,
            baseline,
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
        // auto line-increment = 1.1 * 10 = 11
        assert!((out.lines[1].baseline - out.lines[0].baseline - 11.0).abs() < 1e-9);
        // before-aligned baseline = ascent(0.8) * line_inc(11) / size(10) = 8.8
        assert!((out.lines[0].baseline - 8.8).abs() < 1e-9);
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
        // single line, line_inc 11, baseline_offset 8.8, region height 100
        let mut s = spec(Some(50.0), Some(100.0));
        s.display_align = DisplayAlign::Before;
        assert!((layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 8.8).abs() < 1e-9);
        s.display_align = DisplayAlign::Center;
        // block_top = (100-11)/2 = 44.5; baseline = 44.5 + 8.8 = 53.3
        assert!(
            (layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 53.3).abs() < 1e-9
        );
        s.display_align = DisplayAlign::After;
        // block_top = 100-11 = 89; baseline = 89 + 8.8 = 97.8
        assert!(
            (layout_text_area("hi", &st, &s, &Mono(0.1)).lines[0].baseline - 97.8).abs() < 1e-9
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
