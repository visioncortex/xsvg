//! Paragraph stacking (§6.16): several blocks — `<x:p>`s and list items — each
//! laid as its own wrapped flow and stacked top-down in one box, the stack
//! valigned as a whole and clipped by baseline, with `text-overflow` applied
//! across it. The engine behind a textbox in paragraph mode and the editor's
//! caret geometry for it, so the two cannot drift.
//!
//! The compile layer resolves the blocks — attributes, list gutters and
//! markers, the gaps between them — and hands them here already at the scale
//! being tried, so a run's size resolves against its paragraph exactly as
//! emission will see it. Everything here is pure and platform-free.

use super::{
    area::{layout_area_runs, Align, Anchor, AreaSpec, Fit, PlacedLine, VAlign},
    measure::Measurer,
    style::TextStyle,
    truncate::{apply_ellipsis, mark_overflow, split_visible, TextOverflow},
};

/// One block of the stack, resolved and at the scale being tried: its styled
/// segments and style table (`styles[0]` is the block's own style — its size
/// and line-height drive the advance into it), the gaps on either side, its
/// inset from the content edge (a list item's gutter) and its alignment.
#[derive(Clone, Debug)]
pub struct ParaUnit {
    pub segments: Vec<(String, usize)>,
    pub styles: Vec<TextStyle>,
    /// The gap before this block, added to the previous block's `space_after`.
    pub space_before: f64,
    /// The gap after this block, already resolved: a paragraph's
    /// `space-after`, a list item's `item-spacing`, or the list's `space-after`
    /// after its last item.
    pub space_after: f64,
    /// The text column's inset from the content edge.
    pub inset: f64,
    pub align: Align,
    /// First-line indent, applied to this block's first line only.
    pub text_indent: f64,
    pub rtl: bool,
}

/// A stacked block: its lines in the RELATIVE frame (block 0's cap-top at 0 —
/// shift by [`block_top`] to place them), the width it wrapped to, and the
/// style and anchor emission needs.
#[derive(Clone, Debug)]
pub struct ParaLines {
    pub lines: Vec<PlacedLine>,
    /// The block's first-line baseline in the relative frame — kept outside
    /// `lines` so an EMPTY block (one that laid out to nothing) still knows its
    /// row: a list marker sits there.
    pub first_baseline: f64,
    /// The content width the block wrapped to — what a line must not exceed
    /// under `ellipsis`.
    pub width: f64,
    pub style: TextStyle,
    pub anchor: Anchor,
    pub rtl: bool,
}

/// Stack `units` top-down, each wrapped to `cw` less its inset, starting at
/// `cx`. Stacking is BASELINE-advance based (the Slides convention): a block's
/// first baseline sits one line advance (that line's own size × its
/// line-height) below the previous block's last baseline, plus the two gaps
/// between them — so pressing Enter spaces exactly like a wrapped line, never
/// tighter. An EMPTY block still owns one row (a blank line the caret can sit
/// on); it just emits no glyphs. Returns the blocks and the stack's height:
/// the first block's cap-top to the last line's descent, at their own sizes.
pub fn stack_paragraphs(
    units: &[ParaUnit],
    cx: f64,
    cw: f64,
    m: &dyn Measurer,
) -> (Vec<ParaLines>, f64) {
    let mut out: Vec<ParaLines> = Vec::with_capacity(units.len());
    let mut last_baseline: Option<f64> = None;
    let mut last_descent = 0.0f64;
    let mut prev_after = 0.0f64;
    for u in units {
        let pstyle = &u.styles[0];
        let pspec = AreaSpec {
            x: cx + u.inset,
            // lay out in the block's OWN frame (its cap-top at 0); it moves
            // onto the stack once its first line's size is known
            y: 0.0,
            width: (cw - u.inset).max(1.0),
            height: 1e5, // tall → no clip; valign=top places lines from `y`
            padding: 0.0,
            align: u.align,
            valign: VAlign::Top,
            fit: Fit::None,
            text_overflow: TextOverflow::Clip,
            text_indent: u.text_indent,
            rtl: u.rtl,
        };
        let mut layout = layout_area_runs(&u.segments, &u.styles, &pspec, m);
        // the advance INTO this block is its first line's own (that line's
        // largest run size × line-height); an empty block owns one base-size
        // row. Layout put the first baseline at that line's cap-height, so the
        // whole block shifts by the difference.
        let first_size = layout.lines.first().map(|l| l.size).unwrap_or(pstyle.size);
        let last_size = layout.lines.last().map(|l| l.size).unwrap_or(pstyle.size);
        let cap0 = m.font_metrics(pstyle, first_size).cap_height;
        let first_baseline = match last_baseline {
            None => cap0,
            Some(lb) => lb + prev_after + u.space_before + first_size * pstyle.line_height,
        };
        let shift = first_baseline - cap0;
        for l in &mut layout.lines {
            l.baseline += shift;
        }
        last_baseline = Some(
            layout
                .lines
                .last()
                .map(|l| l.baseline)
                .unwrap_or(first_baseline),
        );
        last_descent = m.font_metrics(pstyle, last_size).descent;
        prev_after = u.space_after;
        out.push(ParaLines {
            lines: layout.lines,
            first_baseline,
            width: pspec.width,
            style: pstyle.clone(),
            anchor: layout.anchor,
            rtl: layout.rtl,
        });
    }
    let block_h = last_baseline.map(|lb| lb + last_descent).unwrap_or(0.0);
    (out, block_h)
}

/// Where the stack's cap-top lands in a box of content top `cy` and height
/// `ch`: valigned as one block.
pub fn block_top(valign: VAlign, cy: f64, ch: f64, block_h: f64) -> f64 {
    match valign {
        VAlign::Top => cy,
        VAlign::Middle => cy + (ch - block_h) / 2.0,
        VAlign::Bottom => cy + (ch - block_h),
    }
}

/// Whether a stack fits its box — the shrink-to-fit predicate (§6.1 extended
/// to paragraph mode): the stacked height within the content height, plus the
/// width condition every fit shares: a lone unbreakable word is the only line
/// greedy wrap lets overflow, so it must fit the content width too.
pub fn paragraphs_fit<'a>(
    paras: impl IntoIterator<Item = &'a ParaLines>,
    block_h: f64,
    cw: f64,
    ch: f64,
    m: &dyn Measurer,
) -> bool {
    block_h <= ch
        && paras.into_iter().all(|it| {
            it.lines.iter().all(|l| {
                l.words.1 - l.words.0 != 1
                    || m.measure(&l.text, &it.style, it.style.size)
                        + it.style.letter_spacing
                            * (l.text.chars().count().saturating_sub(1)) as f64
                        <= cw + 1e-6
            })
        })
}

/// The lines the box shows, placed, with `text-overflow` applied across the
/// whole stack as §6.6 says: a line is kept by its baseline; under `ellipsis`
/// any rendered line wider than its block is cut to fit, and the stack's last
/// rendered line is marked when a line of any block was dropped (a block that
/// fell off the bottom entirely included). One `Vec` per block, empty where
/// none of it shows.
pub fn clip_paragraphs<'a>(
    paras: impl IntoIterator<Item = &'a ParaLines>,
    block_top: f64,
    cy: f64,
    ch: f64,
    overflow: TextOverflow,
    m: &dyn Measurer,
) -> Vec<Vec<PlacedLine>> {
    let paras: Vec<&ParaLines> = paras.into_iter().collect();
    let mut dropped = false;
    let mut per: Vec<Vec<PlacedLine>> = Vec::with_capacity(paras.len());
    for para in &paras {
        let placed = para
            .lines
            .iter()
            .map(|l| PlacedLine {
                baseline: l.baseline + block_top,
                ..l.clone()
            })
            .collect();
        let inside = |l: &PlacedLine| l.baseline >= cy - 1e-6 && l.baseline <= cy + ch + 1e-6;
        if overflow == TextOverflow::Visible {
            per.push(mark_overflow(placed, inside));
            continue;
        }
        let (lines, cut) = split_visible(placed, inside);
        dropped |= cut;
        per.push(lines);
    }
    if overflow == TextOverflow::Ellipsis {
        let last = per.iter().rposition(|v| !v.is_empty());
        for (i, (para, lines)) in paras.iter().zip(per.iter_mut()).enumerate() {
            if lines.is_empty() {
                continue;
            }
            apply_ellipsis(
                lines,
                dropped && Some(i) == last,
                para.width,
                &para.style,
                para.style.size,
                m,
            );
        }
    }
    per
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::test_support::Mono;

    // Mono(0.5) measures 5 per char at size 10; cap = 0.7 × size, descent = 0.2 × size.
    fn unit(text: &str, size: f64, before: f64, after: f64) -> ParaUnit {
        let style = TextStyle {
            size,
            ..TextStyle::default()
        };
        ParaUnit {
            segments: vec![(text.to_string(), 0)],
            styles: vec![style],
            space_before: before,
            space_after: after,
            inset: 0.0,
            align: Align::Start,
            text_indent: 0.0,
            rtl: false,
        }
    }

    #[test]
    fn blocks_stack_by_the_incoming_lines_own_advance_plus_the_gaps() {
        let m = Mono(0.5);
        // block 0 at size 10 (one line), block 1 at size 20 (one line), gaps 3 + 4
        let (paras, h) = stack_paragraphs(
            &[unit("aa", 10.0, 0.0, 3.0), unit("bb", 20.0, 4.0, 0.0)],
            0.0,
            100.0,
            &m,
        );
        assert_eq!(
            paras[0].lines[0].baseline, 7.0,
            "first baseline at its cap-height"
        );
        // next: 7 + after(3) + before(4) + 20 × 1.2
        assert!((paras[1].lines[0].baseline - (7.0 + 3.0 + 4.0 + 24.0)).abs() < 1e-9);
        // height: last baseline + last line's descent at ITS size
        assert!((h - (38.0 + 4.0)).abs() < 1e-9, "{h}");
    }

    #[test]
    fn an_empty_block_keeps_one_row() {
        let m = Mono(0.5);
        let (paras, _) = stack_paragraphs(
            &[
                unit("aa", 10.0, 0.0, 0.0),
                unit("", 10.0, 0.0, 0.0),
                unit("cc", 10.0, 0.0, 0.0),
            ],
            0.0,
            100.0,
            &m,
        );
        // the empty block's one row holds a blank line, one advance down
        assert_eq!(paras[1].lines.len(), 1);
        assert!(paras[1].lines[0].text.is_empty());
        assert!((paras[1].lines[0].baseline - 19.0).abs() < 1e-9);
        // the third block sits two advances down, as if the blank line were text
        assert!((paras[2].lines[0].baseline - (7.0 + 12.0 + 12.0)).abs() < 1e-9);
    }

    #[test]
    fn a_wrapped_block_steps_each_line_by_its_own_size_and_the_next_block_follows_its_last_line() {
        let m = Mono(0.5);
        // 12 chars fit a 60-wide column at size 10: "aa bb cc dd" (11) then "ee"
        let (paras, _) = stack_paragraphs(
            &[
                unit("aa bb cc dd ee", 10.0, 0.0, 0.0),
                unit("ff", 10.0, 0.0, 0.0),
            ],
            0.0,
            60.0,
            &m,
        );
        assert_eq!(paras[0].lines.len(), 2);
        assert!((paras[0].lines[1].baseline - 19.0).abs() < 1e-9);
        assert!((paras[1].lines[0].baseline - 31.0).abs() < 1e-9);
    }

    #[test]
    fn the_stack_valigns_as_one_block() {
        assert_eq!(block_top(VAlign::Top, 10.0, 100.0, 40.0), 10.0);
        assert_eq!(block_top(VAlign::Middle, 10.0, 100.0, 40.0), 40.0);
        assert_eq!(block_top(VAlign::Bottom, 10.0, 100.0, 40.0), 70.0);
    }

    #[test]
    fn the_fit_predicate_minds_the_height_and_the_widest_lone_word() {
        let m = Mono(0.5);
        let (paras, h) =
            stack_paragraphs(&[unit("abcdefghijklmnop", 10.0, 0.0, 0.0)], 0.0, 60.0, &m);
        assert!(
            !paragraphs_fit(&paras, h, 60.0, 100.0, &m),
            "an 80-wide lone word fails a 60 column"
        );
        assert!(paragraphs_fit(&paras, h, 80.0, 100.0, &m));
        assert!(!paragraphs_fit(&paras, h, 80.0, 5.0, &m), "too short");
    }

    #[test]
    fn clipping_marks_the_last_rendered_line_across_blocks() {
        let m = Mono(0.5);
        // baselines 7 (aa bb), 22.5 (cc dd ee ff), 34.5 (gg hh): a 30-high box drops the last
        let units = [
            unit("aa bb", 10.0, 0.0, 3.5),
            unit("cc dd ee ff gg hh", 10.0, 0.0, 3.5),
        ];
        let (paras, _) = stack_paragraphs(&units, 0.0, 60.0, &m);
        let shown = clip_paragraphs(&paras, 0.0, 0.0, 30.0, TextOverflow::Ellipsis, &m);
        assert_eq!(shown[0][0].text, "aa bb");
        assert_eq!(shown[1].len(), 1);
        assert_eq!(shown[1][0].text, "cc dd ee ff…");
        // under clip the same cut leaves the text alone
        let shown = clip_paragraphs(&paras, 0.0, 0.0, 30.0, TextOverflow::Clip, &m);
        assert_eq!(shown[1][0].text, "cc dd ee ff");
        // a block that falls off whole still marks the last line that shows
        let units = [
            unit("aa bb", 10.0, 0.0, 3.5),
            unit("cc dd", 10.0, 0.0, 3.5),
            unit("ee", 10.0, 0.0, 0.0),
        ];
        let (paras, _) = stack_paragraphs(&units, 0.0, 60.0, &m);
        let shown = clip_paragraphs(&paras, 0.0, 0.0, 30.0, TextOverflow::Ellipsis, &m);
        assert_eq!(shown[1][0].text, "cc dd…");
        assert!(shown[2].is_empty());
    }
}
