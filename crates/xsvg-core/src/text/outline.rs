//! The glyph-outlining seam ([`GlyphOutliner`]): turn a text run into vector `<path>`
//! geometry ("create outlines"). Platform-free like [`super::measure::Measurer`] — a
//! browser adapter backs it with opentype.js; tests use a stub.

use super::style::TextStyle;

/// How a text-on-path run is warped and placed (§6.13): the field-selecting `effect`
/// (`"skew"` | `"rainbow"`), the offset of the run's baseline from the path along the
/// local normal (positive = above, SVG `baseline-shift` semantics), and where the run
/// begins within the path's extent — `align` distributes the slack (`"start"` |
/// `"middle"` | `"end"`), `start` adds an absolute head-start (x units under skew,
/// arc length under rainbow).
#[derive(Clone, Copy, Debug)]
pub struct PathEffect<'a> {
    pub effect: &'a str,
    pub baseline_shift: f64,
    pub align: &'a str,
    pub start: f64,
}

/// Source of glyph outlines. Given a text run, its style, and a baseline origin
/// `(x, baseline)` (the left end of the baseline), returns an SVG path `d` in user
/// units tracing the glyphs. `None` when the backend can't outline the run (e.g. the
/// font's bytes aren't available) — the caller then falls back to live `<text>`.
pub trait GlyphOutliner {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String>;

    /// Where each next Unicode scalar of `text` begins, per the outline font's own
    /// metrics: `text.chars().count()` entries, entry `i` the pen x of scalar `i + 1`
    /// with kerning applied, the last the run's width — the width that matches the
    /// geometry [`GlyphOutliner::outline`] traces (unlike the
    /// [`super::measure::Measurer`], whose canvas metrics may differ slightly).
    /// Places text-on-path runs (§6.13 `align`) and puts each caret stop at the
    /// origin of the glyph it precedes (`compile/stops.rs`); one call shapes the run
    /// once. Default: `None` (no font bytes → the caller degrades).
    fn advances(&self, _text: &str, _style: &TextStyle, _size: f64) -> Option<Vec<f64>> {
        None
    }
}

impl<T: GlyphOutliner + ?Sized> GlyphOutliner for &T {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String> {
        (**self).outline(text, style, size, x, baseline)
    }
    fn advances(&self, text: &str, style: &TextStyle, size: f64) -> Option<Vec<f64>> {
        (**self).advances(text, style, size)
    }
}

/// Every contour of an outline closed with `Z`, and no node repeated at the
/// seam. Outliners differ: a font parser's builder closes each contour but
/// draws a line back to the start first; opentype.js 2 closes none, drawing
/// that line instead; an adapter may append `Z` after it. A fill hides all of
/// it, but an editor's path tools see an open shape, or a closed one whose
/// last node sits on its first. So the compiler normalises every outline: a
/// trailing line that only returns to the contour's start is dropped (`Z` is
/// that edge), a trailing curve that returns there is kept (it *is* the edge),
/// and each contour ends in `Z`. An outline that already has that shape
/// passes through byte for byte; anything else is re-serialised whole.
pub fn close_contours(d: &str) -> std::borrow::Cow<'_, str> {
    use kurbo::PathEl;
    let Ok(path) = kurbo::BezPath::from_svg(d) else {
        return std::borrow::Cow::Borrowed(d);
    };
    let mut contours: Vec<Vec<PathEl>> = Vec::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => contours.push(vec![*el]),
            _ => {
                if let Some(c) = contours.last_mut() {
                    c.push(*el);
                }
            }
        }
    }
    let returns_to = |el: Option<&PathEl>, start: kurbo::Point| matches!(el, Some(PathEl::LineTo(p)) if (*p - start).hypot() < 1e-6);
    let mut changed = false;
    let mut fixed: Vec<Vec<PathEl>> = Vec::with_capacity(contours.len());
    for mut c in contours {
        let Some(PathEl::MoveTo(start)) = c.first().copied() else {
            continue;
        };
        if c.last() == Some(&PathEl::ClosePath) {
            if returns_to(c.get(c.len() - 2), start) {
                c.remove(c.len() - 2);
                changed = true;
            }
        } else {
            if returns_to(c.last(), start) {
                c.pop();
            }
            c.push(PathEl::ClosePath);
            changed = true;
        }
        if c.len() <= 2 {
            changed = true; // a move and its close draw nothing
            continue;
        }
        fixed.push(c);
    }
    if !changed {
        return std::borrow::Cow::Borrowed(d);
    }
    let n = |v: f64| {
        let s = format!("{v:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".to_string()
        } else {
            s.to_string()
        }
    };
    let mut out = String::with_capacity(d.len());
    for c in fixed {
        for el in c {
            match el {
                PathEl::MoveTo(p) => out.push_str(&format!("M{} {}", n(p.x), n(p.y))),
                PathEl::LineTo(p) => out.push_str(&format!("L{} {}", n(p.x), n(p.y))),
                PathEl::QuadTo(a, p) => {
                    out.push_str(&format!("Q{} {} {} {}", n(a.x), n(a.y), n(p.x), n(p.y)))
                }
                PathEl::CurveTo(a, b, p) => out.push_str(&format!(
                    "C{} {} {} {} {} {}",
                    n(a.x),
                    n(a.y),
                    n(b.x),
                    n(b.y),
                    n(p.x),
                    n(p.y)
                )),
                PathEl::ClosePath => out.push('Z'),
            }
        }
    }
    std::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod close_tests {
    use super::close_contours;

    #[test]
    fn well_formed_outlines_pass_through_untouched() {
        // closed, no node repeated at the seam: a curve back to the start is the edge
        let d = "M3.71 0L3.71 13.76L1.85 13.76L1.85 0ZM15.84 8.47Q1 2 15.84 8.47Z";
        assert!(matches!(close_contours(d), std::borrow::Cow::Borrowed(_)));
        let rel = "M3.71,64.81l0,13.76-1.86,.72 0-13.76zm12.13,4.54l1 1z";
        assert!(matches!(close_contours(rel), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn open_contours_close_and_lose_the_line_back_to_the_start() {
        // opentype.js 2: no Z, a segment back to the start instead
        let d = "M1.85 6.24L3.71 6.24L3.71 20L1.85 20L1.85 6.24M15.84 14.71Q15.84 17.48 14.62 18.84Q13.40 20.20 15.84 14.71";
        assert_eq!(
            &*close_contours(d),
            "M1.85 6.24L3.71 6.24L3.71 20L1.85 20ZM15.84 14.71Q15.84 17.48 14.62 18.84Q13.4 20.2 15.84 14.71Z"
        );
        // a contour that simply stops is closed where it is; one already closed
        // among open ones stays as it was
        assert_eq!(
            &*close_contours("M0 0L4 0L4 3M9 9L10 9Z"),
            "M0 0L4 0L4 3ZM9 9L10 9Z"
        );
    }

    #[test]
    fn a_closed_contour_loses_a_node_repeated_at_its_seam() {
        // a font parser's builder, or an adapter's appended Z: the line back to
        // the start and then Z put the last node on the first
        let d = "M3.71 0L3.71 13.76L1.85 13.76L1.85 0L3.71 0ZM1 1L2 1L2 2Z";
        assert_eq!(
            &*close_contours(d),
            "M3.71 0L3.71 13.76L1.85 13.76L1.85 0ZM1 1L2 1L2 2Z"
        );
        // relative spelling, same seam
        assert_eq!(
            &*close_contours("M1 1l1 0 0 1 -1 0 0 -1z"),
            "M1 1L2 1L2 2L1 2Z"
        );
        // a move, a line home and a close draw nothing: dropped
        assert_eq!(&*close_contours("M5 5L5 5ZM0 0L1 0L1 1Z"), "M0 0L1 0L1 1Z");
    }
}
