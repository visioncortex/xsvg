//! Box ("text area") placement: wrap + fit + align text inside a rectangle,
//! producing absolutely-positioned lines ready to emit as `<tspan>`s.

use super::{
    fit::fit_size,
    measure::{measure_words, Measurer},
    style::TextStyle,
    truncate::{apply_ellipsis, TextOverflow},
    wrap::wrap,
};

/// Horizontal alignment within the content box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
    /// Full-justify: stretch every line but the paragraph's last to the content width.
    Justify,
}

impl Align {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            "end" => Self::End,
            "justify" => Self::Justify,
            _ => Self::Start,
        }
    }
    pub fn anchor(self) -> Anchor {
        match self {
            Self::Center => Anchor::Middle,
            Self::End => Anchor::End,
            // justify lines begin at the start edge; textLength fills to the right
            Self::Start | Self::Justify => Anchor::Start,
        }
    }
}

/// Vertical alignment within the content box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

impl VAlign {
    pub fn parse(s: &str) -> Self {
        match s {
            "middle" => Self::Middle,
            "bottom" => Self::Bottom,
            _ => Self::Top,
        }
    }
}

/// Font-fitting mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    /// Use the authored size as-is (text may overflow the box).
    None,
    /// Shrink the font until the wrapped block fits, never below `min`.
    Shrink { min: f64 },
}

/// SVG `text-anchor` value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    pub fn svg(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// A rectangular text area: box geometry, padding, and alignment/fit options.
#[derive(Clone, Copy, Debug)]
pub struct AreaSpec {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub padding: f64,
    pub align: Align,
    pub valign: VAlign,
    pub fit: Fit,
    pub text_overflow: TextOverflow,
}

/// One laid-out line: its text and the absolute coordinates of its anchor point.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    pub text: String,
    pub x: f64,
    pub baseline: f64,
    /// If `Some(w)`, this line is justified: render it stretched to width `w`
    /// (via SVG `textLength`/`lengthAdjust="spacing"`). Only full, non-final,
    /// multi-word lines get this; the rest stay at natural width.
    pub justify_width: Option<f64>,
}

/// The result of laying text into an area.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaLayout {
    pub anchor: Anchor,
    pub font_size: f64,
    pub lines: Vec<PlacedLine>,
}

/// Lay `text` into `spec`'s box: shrink-to-fit (if requested), wrap to the content
/// width, then align horizontally and vertically into positioned lines.
pub fn layout_area(text: &str, style: &TextStyle, spec: &AreaSpec, m: &dyn Measurer) -> AreaLayout {
    let cx = spec.x + spec.padding;
    let cy = spec.y + spec.padding;
    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;

    let measured = measure_words(text, style, m);

    let size = match spec.fit {
        Fit::Shrink { min } => fit_size(&measured, style.size, style.line_height, cw, ch, min),
        Fit::None => style.size,
    };
    // Guard the base-size divide: a zero/degenerate `style.size` must not produce
    // NaN coordinates (scale falls back to 0 → zero-width glyphs, no NaN).
    let scale = if style.size > 0.0 {
        size / style.size
    } else {
        0.0
    };
    let line_texts = wrap(&measured, cw, scale);
    let advance = size * style.line_height;

    let anchor = spec.align.anchor();
    let ax = match spec.align {
        Align::Center => cx + cw / 2.0,
        Align::End => cx + cw,
        Align::Start | Align::Justify => cx,
    };

    // Align the cap-height band (first cap-top → last descent), not the em box, so
    // the letterforms read as centred and Top/Bottom sit flush. Ascenders/accents
    // may peek above the cap; descenders hang into the reserved descent.
    let fm = m.font_metrics(style, size);
    let lines = line_texts.len().max(1) as f64;
    let band_h = fm.cap_height + fm.descent + (lines - 1.0) * advance;
    let band_top = match spec.valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - band_h) / 2.0,
        VAlign::Bottom => cy + (ch - band_h),
    };
    let first_baseline = band_top + fm.cap_height;

    // A line is justified iff align=justify, it isn't the paragraph's last line, it
    // has something to stretch (>1 word), and the box has a positive content width.
    let last = line_texts.len().saturating_sub(1);
    let justify = spec.align == Align::Justify && cw > 0.0;

    let mut lines = Vec::new();
    let mut dropped = false;
    for (i, text) in line_texts.into_iter().enumerate() {
        let baseline = first_baseline + i as f64 * advance;
        // clip to the content height by the line's ink band [cap-top, descent]
        if baseline - fm.cap_height < cy - 1e-6 || baseline + fm.descent > cy + ch + 1e-6 {
            dropped = true;
            continue;
        }
        let justify_width = (justify && i < last && text.contains(' ')).then_some(cw);
        lines.push(PlacedLine {
            text,
            x: ax,
            baseline,
            justify_width,
        });
    }
    if spec.text_overflow == TextOverflow::Ellipsis {
        apply_ellipsis(&mut lines, dropped, cw, style, size, m);
    }

    AreaLayout {
        anchor,
        font_size: size,
        lines,
    }
}

/// Inline-size flow anchored at `(x, y)`: wrap to `max_width`, first baseline at
/// `y` (the SVG `<text>` convention, unlike a box where `y` is the top edge).
pub fn layout_flow(
    text: &str,
    style: &TextStyle,
    x: f64,
    y: f64,
    max_width: f64,
    m: &dyn Measurer,
) -> Vec<PlacedLine> {
    let measured = measure_words(text, style, m);
    let advance = style.size * style.line_height;
    wrap(&measured, max_width, 1.0)
        .into_iter()
        .enumerate()
        .map(|(i, t)| PlacedLine {
            text: t,
            x,
            baseline: y + i as f64 * advance,
            justify_width: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::test_support::Mono;

    fn spec(width: f64, height: f64, align: Align, valign: VAlign, fit: Fit) -> AreaSpec {
        AreaSpec {
            x: 0.0,
            y: 0.0,
            width,
            height,
            padding: 0.0,
            align,
            valign,
            fit,
            text_overflow: TextOverflow::Clip,
        }
    }

    #[test]
    fn centers_and_anchors() {
        let style = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // one short word, centered both ways in a 100×100 box
        let s = spec(100.0, 100.0, Align::Center, VAlign::Middle, Fit::None);
        let out = layout_area("hi", &style, &s, &Mono(0.1));
        assert_eq!(out.anchor, Anchor::Middle);
        assert_eq!(out.lines.len(), 1);
        assert_eq!(out.lines[0].x, 50.0); // content center
                                          // cap band = cap_height 7 + descent 2 = 9; top = (100-9)/2 = 45.5; baseline = +cap 7
        assert!((out.lines[0].baseline - 52.5).abs() < 1e-9);
    }

    #[test]
    fn middle_centering_is_symmetric() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        for text in ["Hi", "alpha beta gamma delta epsilon zeta"] {
            let s = spec(80.0, 130.0, Align::Center, VAlign::Middle, Fit::None);
            let out = layout_area(text, &st, &s, &Mono(0.2));
            let fm = Mono(0.2).font_metrics(&st, out.font_size);
            let cap_top = out.lines.first().unwrap().baseline - fm.cap_height;
            let descent_bottom = out.lines.last().unwrap().baseline + fm.descent;
            let above = cap_top; // cy = 0
            let below = 130.0 - descent_bottom; // ch = 130
            assert!(
                (above - below).abs() < 1e-9,
                "{text}: above {above} below {below}"
            );
        }
    }

    #[test]
    fn shrink_reduces_font_and_baselines_step_by_advance() {
        let style = TextStyle {
            size: 40.0,
            ..Default::default()
        };
        let s = spec(
            40.0,
            30.0,
            Align::Start,
            VAlign::Top,
            Fit::Shrink { min: 5.0 },
        );
        let out = layout_area("alpha beta gamma delta", &style, &s, &Mono(0.2));
        assert!(out.font_size < 40.0);
        let adv = out.font_size * style.line_height;
        for w in out.lines.windows(2) {
            assert!((w[1].baseline - w[0].baseline - adv).abs() < 1e-9);
        }
    }

    #[test]
    fn empty_text_yields_no_lines() {
        let st = TextStyle {
            size: 12.0,
            ..Default::default()
        };
        let s = spec(100.0, 100.0, Align::Start, VAlign::Top, Fit::None);
        assert!(layout_area("   ", &st, &s, &Mono(0.1)).lines.is_empty());
    }

    #[test]
    fn degenerate_box_does_not_panic() {
        let st = TextStyle {
            size: 20.0,
            ..Default::default()
        };
        let z = spec(
            0.0,
            0.0,
            Align::Center,
            VAlign::Middle,
            Fit::Shrink { min: 5.0 },
        );
        let out = layout_area("alpha beta gamma", &st, &z, &Mono(0.2));
        assert_eq!(out.font_size, 5.0); // fit still bottoms out at the floor
        assert!(out.lines.is_empty()); // zero height clips every line away

        let p = AreaSpec {
            padding: 80.0,
            ..spec(100.0, 100.0, Align::Start, VAlign::Top, Fit::None)
        };
        assert!(layout_area("a b c", &st, &p, &Mono(0.2)).lines.is_empty());
    }

    #[test]
    fn align_end_anchors_at_content_right() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let s = AreaSpec {
            padding: 5.0,
            ..spec(100.0, 40.0, Align::End, VAlign::Top, Fit::None)
        };
        let out = layout_area("hi", &st, &s, &Mono(0.1));
        assert_eq!(out.anchor, Anchor::End);
        assert_eq!(out.lines[0].x, 95.0);
    }

    #[test]
    fn flow_baseline_starts_at_y() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let line = layout_flow("aaa bbb", &st, 7.0, 30.0, 100.0, &Mono(0.1));
        assert_eq!(line.len(), 1);
        assert_eq!(line[0].x, 7.0);
        assert_eq!(line[0].baseline, 30.0);

        let wrapped = layout_flow("aaa bbb", &st, 0.0, 0.0, 4.0, &Mono(0.1));
        assert_eq!(wrapped.len(), 2);
        assert!((wrapped[1].baseline - 12.0).abs() < 1e-9);

        assert!(layout_flow("", &st, 0.0, 0.0, 50.0, &Mono(0.1)).is_empty());
        assert_eq!(
            layout_flow("a b c", &st, 0.0, 0.0, 0.0, &Mono(0.1)).len(),
            3
        );
    }

    #[test]
    fn ellipsis_marks_block_overflow() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let s = AreaSpec {
            text_overflow: TextOverflow::Ellipsis,
            ..spec(30.0, 14.0, Align::Start, VAlign::Top, Fit::None)
        };
        let out = layout_area("alpha beta gamma delta epsilon zeta", &st, &s, &Mono(0.2));
        assert!(!out.lines.is_empty());
        assert!(
            out.lines.last().unwrap().text.ends_with('…'),
            "{:?}",
            out.lines
        );
    }

    /// A degenerate measurer returning a fixed advance for every string — used to
    /// prove pathological metrics (NaN/inf/negative) never crash or leak NaN coords.
    struct Const(f64);
    impl Measurer for Const {
        fn measure(&self, _t: &str, _s: &TextStyle, _z: f64) -> f64 {
            self.0
        }
    }

    #[test]
    fn justify_marks_full_lines_in_textbox() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // content width 8 (no padding); Mono char=1, space=1 → wraps to 2 full lines
        let s = spec(8.0, 200.0, Align::Justify, VAlign::Top, Fit::None);
        let out = layout_area("aa bb cc dd ee ff", &st, &s, &Mono(0.1));
        assert_eq!(out.anchor, Anchor::Start);
        assert!(out.lines.len() >= 2);
        let n = out.lines.len();
        for (i, l) in out.lines.iter().enumerate() {
            if i + 1 < n {
                assert_eq!(l.justify_width, Some(8.0), "full line not justified: {l:?}");
            } else {
                assert_eq!(l.justify_width, None, "last line must stay ragged");
            }
        }
    }

    #[test]
    fn justify_skips_single_word_lines() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        // each word alone per line (tiny width) → nothing to stretch between words
        let s = spec(1.0, 200.0, Align::Justify, VAlign::Top, Fit::None);
        let out = layout_area("aaa bbb ccc", &st, &s, &Mono(0.1));
        assert!(out.lines.iter().all(|l| l.justify_width.is_none()));
    }

    #[test]
    fn zero_font_size_produces_no_nan() {
        let st = TextStyle {
            size: 0.0,
            ..Default::default()
        };
        for fit in [Fit::None, Fit::Shrink { min: 0.0 }] {
            let s = spec(100.0, 100.0, Align::Center, VAlign::Middle, fit);
            let out = layout_area("hi there world", &st, &s, &Mono(0.1));
            for l in &out.lines {
                assert!(
                    l.x.is_finite() && l.baseline.is_finite(),
                    "size=0 leaked NaN: {l:?}"
                );
            }
        }
    }

    #[test]
    fn negative_dimensions_do_not_panic() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let neg = AreaSpec {
            x: 0.0,
            y: 0.0,
            width: -50.0,
            height: -20.0,
            padding: -5.0,
            align: Align::Center,
            valign: VAlign::Middle,
            fit: Fit::Shrink { min: 5.0 },
            text_overflow: TextOverflow::Ellipsis,
        };
        let out = layout_area("alpha beta gamma", &st, &neg, &Mono(0.2));
        for l in &out.lines {
            assert!(l.x.is_finite() && l.baseline.is_finite());
        }
    }

    #[test]
    fn pathological_measurer_never_panics_or_leaks_nan() {
        let st = TextStyle {
            size: 12.0,
            ..Default::default()
        };
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e9, 0.0] {
            let s = spec(
                80.0,
                40.0,
                Align::Center,
                VAlign::Middle,
                Fit::Shrink { min: 5.0 },
            );
            let out = layout_area("some words placed here now", &st, &s, &Const(bad));
            // Coordinates come from size + vertical metrics, never from `measure`,
            // so they must stay finite even when advances are garbage.
            for l in &out.lines {
                assert!(
                    l.x.is_finite() && l.baseline.is_finite(),
                    "measurer={bad} leaked NaN coord: {l:?}"
                );
            }
        }
    }

    #[test]
    fn ellipsis_marks_inline_overflow() {
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let s = AreaSpec {
            text_overflow: TextOverflow::Ellipsis,
            ..spec(20.0, 100.0, Align::Start, VAlign::Top, Fit::None)
        };
        let out = layout_area("supercalifragilistic", &st, &s, &Mono(0.2));
        assert!(out.lines[0].text.ends_with('…'));
        assert!(Mono(0.2).measure(&out.lines[0].text, &st, 10.0) <= 20.0 + 1e-9);
    }
}
