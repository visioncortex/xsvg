//! Box ("text area") placement: wrap + fit + align text inside a rectangle,
//! producing absolutely-positioned lines ready to emit as `<tspan>`s.

use super::{
    fit::fit_size,
    measure::{measure_words, Measurer},
    style::TextStyle,
    wrap::wrap,
};

/// Horizontal alignment within the content box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Start,
    Center,
    End,
}

impl Align {
    pub fn parse(s: &str) -> Self {
        match s {
            "center" => Self::Center,
            "end" => Self::End,
            _ => Self::Start,
        }
    }
    pub fn anchor(self) -> Anchor {
        match self {
            Self::Start => Anchor::Start,
            Self::Center => Anchor::Middle,
            Self::End => Anchor::End,
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
}

/// One laid-out line: its text and the absolute coordinates of its anchor point.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    pub text: String,
    pub x: f64,
    pub baseline: f64,
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
    let scale = size / style.size;
    let line_texts = wrap(&measured, cw, scale);
    let advance = size * style.line_height;
    let block_h = line_texts.len() as f64 * advance;

    let anchor = spec.align.anchor();
    let ax = match spec.align {
        Align::Start => cx,
        Align::Center => cx + cw / 2.0,
        Align::End => cx + cw,
    };
    let top = match spec.valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - block_h) / 2.0,
        VAlign::Bottom => cy + (ch - block_h),
    };
    let first_baseline = top + m.font_metrics(style, size).ascent;

    let lines = line_texts
        .into_iter()
        .enumerate()
        .map(|(i, text)| PlacedLine {
            text,
            x: ax,
            baseline: first_baseline + i as f64 * advance,
        })
        .collect();

    AreaLayout {
        anchor,
        font_size: size,
        lines,
    }
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
                                          // single line block height = 10*1.2 = 12; top = (100-12)/2 = 44; baseline += ascent 8
        assert!((out.lines[0].baseline - 52.0).abs() < 1e-9);
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
}
