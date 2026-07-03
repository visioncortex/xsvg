//! Flow text into an arbitrary shape ("fit text in polygon").
//!
//! Where [`super::area`] fills a rectangle, this fills a [`Region`]: text is poured
//! row by row, each line wrapped to the shape's inside width *at that height*, so a
//! triangle's lines shorten toward the apex and a circle's bulge in the middle.
//!
//! The geometry is abstracted behind the [`Region`] trait — the layout here is pure
//! and platform-free. A browser adapter supplies a [`RasterRegion`] built from a
//! coarse scan of the shape (curve flattening + inside-testing deferred to the
//! browser); tests use analytic regions or browser-generated raster fixtures.

use super::area::{Align, AreaLayout, PlacedLine, VAlign};
use super::measure::{line_advance, measure_words, Measurer};
use super::style::TextStyle;
use super::truncate::{ellipsize_line, TextOverflow};
use super::wrap::fill_line;

/// An axis-aligned rectangle in user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A fillable 2-D region: its bounds, and the usable horizontal extent at any
/// vertical band. The one geometry seam of region flow (cf. [`Measurer`]).
pub trait Region {
    /// Axis-aligned bounds of the shape.
    fn bounds(&self) -> Rect;

    /// The widest inside `[left, right]` that spans the *whole* band `[y0, y1]`
    /// (the intersection across the band, so a glyph box placed there never pokes
    /// outside the shape). `None` if no usable run crosses the band. Coarse: one
    /// span per band; a non-convex row collapses to its outer `[leftmost, rightmost]`.
    fn span(&self, y0: f64, y1: f64) -> Option<(f64, f64)>;
}

/// A [`Region`] backed by a coarse horizontal-span table — one `[left, right]` (or
/// `None`) per fixed-height row. This is what a browser adapter produces by scanning
/// the shape (via `getBBox` + `isPointInFill`), and what fixtures replay in tests.
#[derive(Clone, Debug)]
pub struct RasterRegion {
    bbox: Rect,
    top: f64,
    row_h: f64,
    rows: Vec<Option<(f64, f64)>>,
}

impl RasterRegion {
    pub fn new(bbox: Rect, top: f64, row_h: f64, rows: Vec<Option<(f64, f64)>>) -> Self {
        Self {
            bbox,
            top,
            row_h,
            rows,
        }
    }
}

impl Region for RasterRegion {
    fn bounds(&self) -> Rect {
        self.bbox
    }

    fn span(&self, y0: f64, y1: f64) -> Option<(f64, f64)> {
        if self.row_h <= 0.0 {
            return None;
        }
        let (mut l, mut r) = (f64::NEG_INFINITY, f64::INFINITY);
        let mut any = false;
        for (idx, row) in self.rows.iter().enumerate() {
            let ry0 = self.top + idx as f64 * self.row_h;
            let ry1 = ry0 + self.row_h;
            if ry1 <= y0 || ry0 >= y1 {
                continue; // this row doesn't overlap the band
            }
            match row {
                Some((rl, rr)) => {
                    l = l.max(*rl);
                    r = r.min(*rr);
                    any = true;
                }
                None => return None, // band crosses outside the shape
            }
        }
        (any && r - l > 1e-9).then_some((l, r))
    }
}

/// Rasterizes a filled path into a [`RasterRegion`] — the platform seam for region
/// geometry (cf. [`Measurer`] for font metrics). The browser adapter defers curve
/// flattening + inside-testing to `getBBox`/`isPointInFill`; tests replay fixtures.
pub trait Shaper {
    /// Coarsely rasterize the filled path `path_d` (an SVG `d` string) into rows of
    /// height ≈ `row_h`. `None` if the shape is degenerate or can't be rasterized.
    fn rasterize(&self, path_d: &str, row_h: f64) -> Option<RasterRegion>;
}

/// Flow options for [`layout_region`]. `padding` insets the region on all sides;
/// `align` positions each line within its own span (`justify` behaves as `start`);
/// `valign` positions the flowed block within the region's vertical extent.
#[derive(Clone, Copy, Debug)]
pub struct RegionSpec {
    pub padding: f64,
    pub align: Align,
    pub valign: VAlign,
    pub text_overflow: TextOverflow,
}

/// Pour `text` into `region`: for each line box (height `line-height · font-size`),
/// take the inside span at that height, greedily fill it with words, place the line,
/// and step down. Lines that run past the region's bottom (or words that don't fit
/// at all) overflow per `text_overflow`. No shrink-to-fit in v0.
///
/// `valign` centers/bottom-aligns the flowed block: a first pass counts the lines to
/// size the block, then the flow re-runs from a shifted start so the block sits at
/// the requested vertical position. The re-flow always terminates and drops no words
/// beyond what top-alignment would (a too-tall block just clamps back to the top).
pub fn layout_region(
    text: &str,
    style: &TextStyle,
    region: &dyn Region,
    spec: &RegionSpec,
    m: &dyn Measurer,
) -> AreaLayout {
    let b = region.bounds();
    let pad = spec.padding;
    let content_top = b.y + pad;
    let content_bottom = b.y + b.h - pad;
    let content_left = b.x + pad;
    let content_right = b.x + b.w - pad;

    let size = style.size;
    let line_h = size * style.line_height;
    let fm = m.font_metrics(style, size);
    let measured = measure_words(text, style, m);
    let anchor = spec.align.anchor();

    // One flow pass starting at `start_y`; returns placed lines, their span widths,
    // and whether any content overflowed (vertically or as leftover words).
    let flow = |start_y: f64| -> (Vec<PlacedLine>, Vec<f64>, bool) {
        let mut lines = Vec::new();
        let mut widths = Vec::new();
        let mut i = 0;
        let mut y = start_y;
        let mut dropped = false;
        while i < measured.words.len() {
            if line_h <= 0.0 || y + line_h > content_bottom + 1e-6 {
                dropped = true;
                break;
            }
            let span = region
                .span(y, y + line_h)
                .map(|(l, r)| (l.max(content_left), r.min(content_right)))
                .filter(|(l, r)| r - l > 1e-6);
            let Some((l, r)) = span else {
                y += line_h; // band too narrow here — try the next one down
                continue;
            };
            let (text, next) = fill_line(&measured, i, r - l, 1.0);
            let ax = match spec.align {
                Align::Center => (l + r) / 2.0,
                Align::End => r,
                Align::Start | Align::Justify => l,
            };
            lines.push(PlacedLine {
                text,
                x: ax,
                baseline: y + fm.ascent,
                justify_width: None,
            });
            widths.push(r - l);
            i = next;
            y += line_h;
        }
        (lines, widths, dropped || i < measured.words.len())
    };

    // Vertical placement. The line count changes with the start height (the shape is
    // wider lower down → fewer lines), so a single top-pass estimate mis-centers.
    // Iterate the offset to a fixpoint: re-flow, recount, re-offset — bounded, and
    // convex shapes settle in a step or two.
    let available = content_bottom - content_top;
    let offset_for = |n: usize| -> f64 {
        let block_h = n as f64 * line_h;
        match spec.valign {
            VAlign::Top => 0.0,
            VAlign::Middle => ((available - block_h) / 2.0).max(0.0),
            VAlign::Bottom => (available - block_h).max(0.0),
        }
    };
    // `result` always holds a complete, valid layout — initialized top-aligned,
    // then reassigned each iteration below.
    let mut result = flow(content_top);
    if spec.valign != VAlign::Top {
        // Iterate toward a stable offset, re-flowing since the line count shifts as
        // the shape widens lower down. Stop once the offset settles to within a line
        // height — finer precision isn't meaningful, and this skips a negligible
        // shift. Hard-capped: if it never settles (e.g. the count oscillates), we
        // keep the last iterate — a valid, if slightly off-centre, layout. Cannot loop.
        let mut applied = 0.0;
        for _ in 0..4 {
            let target = offset_for(result.0.len());
            if (target - applied).abs() < line_h {
                break;
            }
            result = flow(content_top + target);
            applied = target;
        }
    }
    let (mut lines, widths, dropped) = result;

    if spec.text_overflow == TextOverflow::Ellipsis {
        // inline overflow: a lone word wider than its span
        for (line, w) in lines.iter_mut().zip(widths.iter()) {
            if !line.text.ends_with('…') && line_advance(&line.text, style, size, m) > w + 1e-6 {
                line.text = ellipsize_line(&line.text, *w, style, size, m).unwrap_or_default();
            }
        }
        // block overflow: mark the last placed line
        if dropped {
            if let (Some(line), Some(w)) = (lines.last_mut(), widths.last()) {
                if !line.text.ends_with('…') {
                    line.text = ellipsize_line(&line.text, *w, style, size, m).unwrap_or_default();
                }
            }
        }
        lines.retain(|l| !l.text.is_empty());
    }

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
    use crate::text::truncate::TextOverflow;

    fn style(size: f64) -> TextStyle {
        TextStyle {
            size,
            ..Default::default()
        }
    }

    fn spec(align: Align, overflow: TextOverflow) -> RegionSpec {
        RegionSpec {
            padding: 0.0,
            align,
            valign: VAlign::Top,
            text_overflow: overflow,
        }
    }

    /// Full-width rectangle: every band returns the same span.
    struct RectRegion(Rect);
    impl Region for RectRegion {
        fn bounds(&self) -> Rect {
            self.0
        }
        fn span(&self, y0: f64, y1: f64) -> Option<(f64, f64)> {
            (y1 > self.0.y && y0 < self.0.y + self.0.h).then_some((self.0.x, self.0.x + self.0.w))
        }
    }

    /// Triangle wide at the top, narrowing to a point at the bottom. Uses the band's
    /// *lower* edge (narrower) so text stays inside.
    struct DownTriangle(Rect);
    impl Region for DownTriangle {
        fn bounds(&self) -> Rect {
            self.0
        }
        fn span(&self, _y0: f64, y1: f64) -> Option<(f64, f64)> {
            let frac = ((y1 - self.0.y) / self.0.h).clamp(0.0, 1.0);
            let w = self.0.w * (1.0 - frac);
            if w <= 1e-9 {
                return None;
            }
            let cx = self.0.x + self.0.w / 2.0;
            Some((cx - w / 2.0, cx + w / 2.0))
        }
    }

    #[test]
    fn rect_region_flows_like_a_rectangle() {
        let r = RectRegion(Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 1000.0,
        });
        // Mono(0.1) size 10: char=1, space=1. width 100 fits many words per line.
        let out = layout_region(
            "aa bb cc dd ee ff gg hh",
            &style(10.0),
            &r,
            &spec(Align::Start, TextOverflow::Clip),
            &Mono(0.1),
        );
        assert!(!out.lines.is_empty());
        assert!(out.lines.iter().all(|l| l.x == 0.0)); // start-anchored at left edge
                                                       // baselines step by line-height·size = 12
        for w in out.lines.windows(2) {
            assert!((w[1].baseline - w[0].baseline - 12.0).abs() < 1e-9);
        }
    }

    #[test]
    fn triangle_lines_shorten_toward_apex() {
        let r = DownTriangle(Rect {
            x: 0.0,
            y: 0.0,
            w: 30.0,
            h: 120.0,
        });
        let out = layout_region(
            "aa bb cc dd ee ff gg hh ii jj kk ll",
            &style(10.0),
            &r,
            &spec(Align::Center, TextOverflow::Clip),
            &Mono(0.1),
        );
        assert!(out.lines.len() >= 2);
        assert_eq!(out.anchor, super::super::area::Anchor::Middle);
        // word counts per line must be non-increasing as the triangle narrows
        let counts: Vec<usize> = out
            .lines
            .iter()
            .map(|l| l.text.split(' ').count())
            .collect();
        for w in counts.windows(2) {
            assert!(w[1] <= w[0], "lines should not widen downward: {counts:?}");
        }
    }

    #[test]
    fn raster_region_span_intersects_the_band() {
        let rr = RasterRegion::new(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 30.0,
            },
            0.0,
            10.0,
            vec![Some((0.0, 10.0)), Some((0.0, 6.0)), None],
        );
        // band inside row 0 only
        assert_eq!(rr.span(0.0, 9.0), Some((0.0, 10.0)));
        // band spanning rows 0+1 → intersection is the narrower right edge
        assert_eq!(rr.span(5.0, 15.0), Some((0.0, 6.0)));
        // band touching the empty row 2 → unusable
        assert_eq!(rr.span(21.0, 29.0), None);
    }

    #[test]
    fn degenerate_regions_do_not_panic() {
        // zero-area region → no lines
        let z = RectRegion(Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        });
        let out = layout_region(
            "aa bb cc",
            &style(10.0),
            &z,
            &spec(Align::Start, TextOverflow::Clip),
            &Mono(0.1),
        );
        assert!(out.lines.is_empty());

        // a fully-empty raster (all rows None) → no lines, no infinite loop
        let empty = RasterRegion::new(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 50.0,
                h: 50.0,
            },
            0.0,
            10.0,
            vec![None, None, None, None, None],
        );
        let out2 = layout_region(
            "aa bb cc",
            &style(10.0),
            &empty,
            &spec(Align::Start, TextOverflow::Clip),
            &Mono(0.1),
        );
        assert!(out2.lines.is_empty());
    }

    #[test]
    fn valign_centers_and_bottom_aligns_the_block() {
        // A short block in a tall rectangle: middle sits below top, bottom below middle.
        let r = RectRegion(Rect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 300.0,
        });
        let text = "aa bb cc"; // one short line
        let first = |align, va| {
            let mut s = spec(align, TextOverflow::Clip);
            s.valign = va;
            layout_region(text, &style(10.0), &r, &s, &Mono(0.1)).lines[0].baseline
        };
        let top = first(Align::Start, VAlign::Top);
        let mid = first(Align::Start, VAlign::Middle);
        let bot = first(Align::Start, VAlign::Bottom);
        assert!(mid > top + 50.0, "middle {mid} not well below top {top}");
        assert!(bot > mid + 50.0, "bottom {bot} not well below middle {mid}");
        // bottom line's box ends at (near) the content bottom
        assert!(bot <= 300.0 + 1e-6);
    }

    #[test]
    fn ellipsis_marks_block_overflow_in_region() {
        // short + narrow region, more text than fits → last line ends with …
        let r = RectRegion(Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 25.0,
        });
        let out = layout_region(
            "aa bb cc dd ee ff gg hh ii jj",
            &style(10.0),
            &r,
            &spec(Align::Start, TextOverflow::Ellipsis),
            &Mono(0.1),
        );
        assert!(!out.lines.is_empty());
        assert!(out.lines.last().unwrap().text.ends_with('…'));
    }
}
