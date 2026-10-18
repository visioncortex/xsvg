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

/// Every contour of an outline closed with `Z`. Outliners differ: a font
/// parser's builder closes each contour; opentype.js 2 closes none, drawing a
/// segment back to the contour's start instead. A fill closes implicitly, so
/// both render alike, but an editor's path tools see an open shape whose last
/// edge is a segment to nowhere. So the compiler closes contours itself, for
/// every outliner: an outline whose contours all end in `Z` passes through
/// untouched (byte for byte); any other is parsed, a trailing line that only
/// returns to the contour's start is dropped (`Z` is that edge), a trailing
/// curve that returns there is kept (it *is* the edge), and each contour ends
/// in `Z`.
pub fn close_contours(d: &str) -> std::borrow::Cow<'_, str> {
    let closed = d
        .split(['M', 'm'])
        .skip(1)
        .all(|sub| sub.trim_end().ends_with(['Z', 'z']));
    if closed {
        return std::borrow::Cow::Borrowed(d);
    }
    let Ok(path) = kurbo::BezPath::from_svg(d) else {
        return std::borrow::Cow::Borrowed(d);
    };
    use kurbo::PathEl;
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
    let mut out = String::with_capacity(d.len() + contours.len() * 2);
    let n = |v: f64| {
        let s = format!("{v:.3}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" {
            "0".to_string()
        } else {
            s.to_string()
        }
    };
    for mut c in contours {
        let Some(PathEl::MoveTo(start)) = c.first().copied() else {
            continue;
        };
        if c.len() == 1 {
            continue; // a lone move draws nothing
        }
        if c.last() != Some(&PathEl::ClosePath) {
            if let Some(PathEl::LineTo(p)) = c.last().copied() {
                if (p - start).hypot() < 1e-6 {
                    c.pop();
                }
            }
            c.push(PathEl::ClosePath);
        }
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
    fn closed_outlines_pass_through_untouched() {
        let d = "M3.71 0L3.71 13.76L1.85 13.76L1.85 0L3.71 0ZM15.84 8.47Q1 2 3 4Z";
        assert!(matches!(close_contours(d), std::borrow::Cow::Borrowed(_)));
        let rel = "M3.71,64.81l0,13.76-1.86,.72 0-13.76 1.86-.72zm12.13,4.54l1 1z";
        assert!(matches!(close_contours(rel), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn open_contours_close_and_lose_the_line_back_to_the_start() {
        // opentype.js 2: no Z, a segment back to the start instead
        let d = "M1.85 6.24L3.71 6.24L3.71 20L1.85 20L1.85 6.24M15.84 14.71Q15.84 17.48 14.62 18.84Q13.40 20.20 15.84 14.71";
        let c = close_contours(d);
        assert_eq!(
            &*c,
            "M1.85 6.24L3.71 6.24L3.71 20L1.85 20ZM15.84 14.71Q15.84 17.48 14.62 18.84Q13.4 20.2 15.84 14.71Z"
        );
        // a contour that simply stops is closed where it is; one already closed
        // among open ones stays as it was
        assert_eq!(
            &*close_contours("M0 0L4 0L4 3M9 9L10 9Z"),
            "M0 0L4 0L4 3ZM9 9L10 9Z"
        );
    }
}
