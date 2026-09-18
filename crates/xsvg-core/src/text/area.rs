//! Box ("text area") placement: wrap + fit + align text inside a rectangle,
//! producing absolutely-positioned lines ready to emit as `<tspan>`s.

use super::{
    fit::{fit_size, fit_size_flows},
    measure::{measure_runs, measure_words, Measured, Measurer, Piece},
    style::TextStyle,
    truncate::{apply_ellipsis, TextOverflow},
    wrap::{fill_line, wrap_pieces},
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
    /// First-line indent (§6.6): the first line wraps to `width − indent` and is
    /// placed that far in from the start edge. Positive indents; negative outdents
    /// (a hanging first line). Applies to start/justify alignment; 0 = none.
    pub text_indent: f64,
}

/// A styled span within a placed line: contiguous text sharing one style, given as
/// an index into the caller's style table (0 = the base paragraph style). A
/// single-style line has exactly one run. The emitter maps the index back to paint
/// attributes (fill, weight, …).
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub text: String,
    pub style: usize,
}

/// One laid-out line: its text and the absolute coordinates of its anchor point.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedLine {
    /// Full line text (concatenation of `runs`) — kept for truncation and callers
    /// that don't care about styling.
    pub text: String,
    pub x: f64,
    pub baseline: f64,
    /// The line's font size at the final (fitted) scale — its LARGEST run
    /// size (the base size for a blank line). The line's advance is
    /// `size × line-height`: mixed sizes step a line by its tallest run, the
    /// word-processor convention; a caret on it is this tall.
    pub size: f64,
    /// If `Some(w)`, this line is justified: render it stretched to width `w`
    /// (via SVG `textLength`/`lengthAdjust="spacing"`). Only full, non-final,
    /// multi-word lines get this; the rest stay at natural width.
    pub justify_width: Option<f64>,
    /// The line split into styled runs (`<tspan>`s). One base run for plain text.
    pub runs: Vec<Run>,
    /// `[start, end)` into the word sequence of the [`Measured`] flow that
    /// wrapped this line (wrapping never splits a word, so a line is always a
    /// whole-word range). The editor seam (`text_layout_impl`) maps carets
    /// through it. Ellipsis truncation rewrites `text` but leaves this range —
    /// `text != words[range].join(" ")` is the truncation test. Flows that stack
    /// several paragraphs index each line against its own paragraph's words.
    ///
    /// [`Measured`]: super::measure::Measured
    pub words: (usize, usize),
}

/// Merge a line's pieces into `(full_text, runs)`, coalescing adjacent same-style
/// pieces so each style change is one run.
pub(crate) fn merge_pieces(pieces: Vec<Piece>) -> (String, Vec<Run>) {
    let mut text = String::new();
    let mut runs: Vec<Run> = Vec::new();
    for p in pieces {
        text.push_str(&p.text);
        match runs.last_mut() {
            Some(last) if last.style == p.style => last.text.push_str(&p.text),
            _ => runs.push(Run {
                text: p.text,
                style: p.style,
            }),
        }
    }
    (text, runs)
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
    layout_area_measured(&measure_words(text, style, m), std::slice::from_ref(style), spec, m)
}

/// The font size a line of `pieces` renders at (at the BASE scale): its
/// largest run size — mixed sizes step the line by its tallest run — or the
/// base size for a blank line.
pub(crate) fn line_size(pieces: &[Piece], styles: &[TextStyle]) -> f64 {
    pieces
        .iter()
        .map(|p| styles.get(p.style).unwrap_or(&styles[0]).size)
        .reduce(f64::max)
        .unwrap_or(styles[0].size)
}

/// One wrapped line awaiting placement: styled pieces, global word range, and
/// whether it ends its flow (flow-final lines stay ragged under justify).
type LineMeta = (Vec<Piece>, (usize, usize), bool);

/// Wrap one flow at `scale`, the first line narrowed by `indent` (0 = none).
fn wrap_indented(
    measured: &Measured,
    cw: f64,
    scale: f64,
    indent: f64,
) -> Vec<(Vec<Piece>, (usize, usize))> {
    if indent != 0.0 && cw > 0.0 && !measured.words.is_empty() {
        let (first, mut i) = fill_line(measured, 0, (cw - indent).max(0.0), scale);
        let mut ls = vec![(first, (0, i))];
        while i < measured.words.len() {
            let (p, n) = fill_line(measured, i, cw, scale);
            ls.push((p, (i, n)));
            i = n;
        }
        ls
    } else {
        wrap_pieces(measured, cw, scale)
    }
}

/// Place wrapped lines in the content box — the one placement pass every
/// rectangular flow shares. Each line steps by ITS OWN advance (its size ×
/// line-height, so a line holding a larger run stands taller, the
/// word-processor convention; uniform sizes give the classic constant step).
/// The cap-height band (first line's cap-top → last line's baseline + its
/// descent) is valigned, not the em box, so letterforms read as centred and
/// Top/Bottom sit flush; ascenders may peek above the cap, descenders hang
/// into the reserved descent. A line is kept by its BASELINE (its block
/// position), not its ink band: a lone line grazing an edge renders and
/// overflows rather than vanishing; a baseline past the box drops the line
/// (and triggers ellipsis). The indent inset applies to line 0 only, for
/// start-anchored text; justify stretches full, non-flow-final, multi-word
/// lines.
fn place_lines(
    line_meta: Vec<LineMeta>,
    styles: &[TextStyle],
    size: f64,
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    let style = &styles[0];
    let cx = spec.x + spec.padding;
    let cy = spec.y + spec.padding;
    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    // Guard the base-size divide: a zero/degenerate `style.size` must not produce
    // NaN coordinates (scale falls back to 0 → zero-width glyphs, no NaN).
    let scale = if style.size > 0.0 { size / style.size } else { 0.0 };
    let indent = spec.text_indent;

    let anchor = spec.align.anchor();
    let ax = match spec.align {
        Align::Center => cx + cw / 2.0,
        Align::End => cx + cw,
        Align::Start | Align::Justify => cx,
    };

    // per-line sizes at the final scale; the block band spans the first
    // line's cap, every FOLLOWING line's own advance, and the last descent
    let sizes: Vec<f64> = line_meta
        .iter()
        .map(|(p, _, _)| line_size(p, styles) * scale)
        .collect();
    let first_size = sizes.first().copied().unwrap_or(size);
    let last_size = sizes.last().copied().unwrap_or(size);
    let fm_first = m.font_metrics(style, first_size);
    let fm_last = m.font_metrics(style, last_size);
    let advances: Vec<f64> = sizes.iter().map(|s| s * style.line_height).collect();
    let band_h = fm_first.cap_height + advances.iter().skip(1).sum::<f64>() + fm_last.descent;
    let band_top = match spec.valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - band_h) / 2.0,
        VAlign::Bottom => cy + (ch - band_h),
    };
    let first_baseline = band_top + fm_first.cap_height;
    // uniform sizes keep the exact multiplicative step (no accumulated rounding)
    let uniform = sizes.iter().all(|s| (s - first_size).abs() < 1e-9);

    let justify = spec.align == Align::Justify && cw > 0.0;
    let indent_x = if anchor == Anchor::Start { indent } else { 0.0 };

    let mut lines = Vec::new();
    let mut dropped = false;
    let mut baseline = first_baseline;
    for (i, (pieces, words, flow_final)) in line_meta.into_iter().enumerate() {
        if i > 0 {
            baseline = if uniform {
                first_baseline + i as f64 * advances[0]
            } else {
                baseline + advances[i]
            };
        }
        if baseline < cy - 1e-6 || baseline > cy + ch + 1e-6 {
            dropped = true;
            continue;
        }
        let (text, runs) = merge_pieces(pieces);
        let line_w = if i == 0 { (cw - indent).max(0.0) } else { cw };
        let justify_width = (justify && !flow_final && text.contains(' ')).then_some(line_w);
        lines.push(PlacedLine {
            text,
            x: if i == 0 { ax + indent_x } else { ax },
            baseline,
            size: sizes[i],
            justify_width,
            runs,
            words,
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

/// Split styled segments at `'\n'` (`<tbreak/>` in the source) into forced
/// sub-flows. Each flow wraps independently; an EMPTY flow (consecutive
/// breaks, or a break at an edge) is a forced blank line.
pub fn split_flows(segments: &[(String, usize)]) -> Vec<Vec<(String, usize)>> {
    let mut flows: Vec<Vec<(String, usize)>> = vec![Vec::new()];
    for (text, sid) in segments {
        for (k, part) in text.split('\n').enumerate() {
            if k > 0 {
                flows.push(Vec::new());
            }
            if !part.is_empty() {
                flows.last_mut().unwrap().push((part.to_string(), *sid));
            }
        }
    }
    flows
}

/// Like [`layout_area_measured`], but takes raw styled segments and honors
/// `'\n'` forced breaks (`<tbreak/>`): every line — wrapped or forced — steps
/// by its own advance (`size × line-height`, its largest run's size), a blank
/// flow occupies one base-size line, shrink-fit sums every flow's lines, and
/// justification leaves each flow's last line ragged. A break-free stream is
/// exactly [`layout_area_measured`].
pub fn layout_area_runs(
    segments: &[(String, usize)],
    styles: &[TextStyle],
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    let style = &styles[0];
    let flows = split_flows(segments);
    let measured: Vec<Measured> = flows.iter().map(|f| measure_runs(f, styles, m)).collect();
    if measured.len() == 1 {
        return layout_area_measured(&measured[0], styles, spec, m);
    }

    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let size = match spec.fit {
        Fit::Shrink { min } => fit_size_flows(&measured, styles, cw, ch, min),
        Fit::None => style.size,
    };
    let scale = if style.size > 0.0 { size / style.size } else { 0.0 };

    // Wrap each flow; the box-wide first-line indent applies to the very
    // first line only (flow 0). A blank flow contributes one empty line. Word
    // ranges are GLOBAL — offset into the concatenation of every flow's
    // words, which is exactly the word list `measure_runs` yields for the
    // unsplit stream ('\n' is whitespace there), so seam callers keep one
    // flat word list.
    let mut line_meta: Vec<LineMeta> = Vec::new();
    let mut word_base = 0usize;
    for (fi, mw) in measured.iter().enumerate() {
        let indent = if fi == 0 { spec.text_indent } else { 0.0 };
        let wrapped = wrap_indented(mw, cw, scale, indent);
        if wrapped.is_empty() {
            line_meta.push((Vec::new(), (word_base, word_base), true)); // forced blank line
            continue;
        }
        let n = wrapped.len();
        for (j, (pieces, words)) in wrapped.into_iter().enumerate() {
            // each flow's LAST line stays ragged (the paragraph-final convention)
            line_meta.push((pieces, (words.0 + word_base, words.1 + word_base), j + 1 == n));
        }
        word_base += mw.words.len();
    }
    place_lines(line_meta, styles, size, spec, m)
}

/// Like [`layout_area`], but takes pre-[`measure`]d input so the caller can supply
/// styled runs (via [`super::measure::measure_runs`]) and their style table:
/// `styles[0]` is the base paragraph style (its size / line-height drive fit
/// and the base scale), the pieces' style indices select per-run weight,
/// family, and SIZE — a line steps by its largest run.
pub fn layout_area_measured(
    measured: &Measured,
    styles: &[TextStyle],
    spec: &AreaSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    let style = &styles[0];
    let cw = spec.width - 2.0 * spec.padding;
    let ch = spec.height - 2.0 * spec.padding;
    let size = match spec.fit {
        Fit::Shrink { min } => fit_size(measured, styles, cw, ch, min),
        Fit::None => style.size,
    };
    let scale = if style.size > 0.0 { size / style.size } else { 0.0 };
    // First-line indent (§6.6): wrap the first line to the reduced width, the rest
    // to the full content width. Only when there is text and a real indent.
    let wrapped = wrap_indented(measured, cw, scale, spec.text_indent);
    // a line is justified iff it isn't the paragraph's last (flow-final)
    let last = wrapped.len().saturating_sub(1);
    let line_meta: Vec<LineMeta> = wrapped
        .into_iter()
        .enumerate()
        .map(|(i, (pieces, words))| (pieces, words, i == last))
        .collect();
    place_lines(line_meta, styles, size, spec, m)
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
    wrap_pieces(&measured, max_width, 1.0)
        .into_iter()
        .enumerate()
        .map(|(i, (pieces, words))| {
            let (text, runs) = merge_pieces(pieces);
            PlacedLine {
                text,
                x,
                baseline: y + i as f64 * advance,
                size: style.size,
                justify_width: None,
                runs,
                words,
            }
        })
        .collect()
}

/// [`layout_flow`] over styled segments (one forced flow — split at `'\n'`
/// first with [`split_flows`]): first baseline at `y`, every following line
/// stepping by ITS OWN advance (its largest run size × the base line-height).
pub fn layout_flow_runs(
    segments: &[(String, usize)],
    styles: &[TextStyle],
    x: f64,
    y: f64,
    max_width: f64,
    m: &dyn Measurer,
) -> Vec<PlacedLine> {
    let style = &styles[0];
    let measured = measure_runs(segments, styles, m);
    let mut baseline = y;
    wrap_pieces(&measured, max_width, 1.0)
        .into_iter()
        .enumerate()
        .map(|(i, (pieces, words))| {
            let size = line_size(&pieces, styles);
            if i > 0 {
                baseline += size * style.line_height;
            }
            let (text, runs) = merge_pieces(pieces);
            PlacedLine {
                text,
                x,
                baseline,
                size,
                justify_width: None,
                runs,
                words,
            }
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
            text_indent: 0.0,
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
    fn lone_line_barely_taller_than_box_renders_not_drops() {
        // ink band = cap(0.7·20=14) + descent(0.2·20=4) = 18; box height 17 (< 18).
        // The old ink-band clip dropped this sole centered line entirely; it must
        // now render (and overflow) instead of vanishing.
        let st = TextStyle {
            size: 20.0,
            ..Default::default()
        };
        let s = spec(200.0, 17.0, Align::Center, VAlign::Middle, Fit::None);
        let out = layout_area("Hi", &st, &s, &Mono(0.1));
        assert_eq!(out.lines.len(), 1, "sole line must render: {:?}", out.lines);
        // and a line whose baseline is genuinely past the box is still dropped
        let low = spec(200.0, 4.0, Align::Start, VAlign::Top, Fit::None);
        assert!(layout_area("Hi", &st, &low, &Mono(0.1)).lines.is_empty());
    }

    #[test]
    fn text_indent_offsets_and_narrows_the_first_line() {
        // Mono(0.1) size 10: char=1, space=1. Content width 8 holds "aa bb cc"
        // (2+1+2+1+2=8) on one line — but with a 4-unit first-line indent the first
        // line only has room for "aa", so it breaks and starts 4 in.
        let st = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let s = AreaSpec {
            text_indent: 4.0,
            ..spec(8.0, 200.0, Align::Start, VAlign::Top, Fit::None)
        };
        let out = layout_area("aa bb cc", &st, &s, &Mono(0.1));
        assert_eq!(out.lines.len(), 2, "{:?}", out.lines);
        assert_eq!(out.lines[0].text, "aa");
        assert!(
            (out.lines[0].x - 4.0).abs() < 1e-9,
            "first line indented: {}",
            out.lines[0].x
        );
        assert!(
            (out.lines[1].x).abs() < 1e-9,
            "later lines sit at the margin"
        );
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
            text_indent: -8.0,
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
