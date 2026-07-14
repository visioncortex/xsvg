//! Native implementations of the compiler's platform seams, backed by `ttf-parser`
//! (metrics + glyph outlines) and `kurbo` (path inside-testing for shape flow) instead of
//! the browser. `Native` implements all three traits, so one value drives the compile.

use crate::fonts;
use ttf_parser::OutlineBuilder;
use xsvg_core::kurbo::{BezPath, Point, Shape};
use xsvg_core::{FontMetrics, GlyphOutliner, Measurer, RasterRegion, Rect, Shaper, TextStyle};

pub struct Native;

/// Sum of horizontal glyph advances for `text` in the resolved font, in user units.
fn advance(text: &str, style: &TextStyle, size: f64) -> Option<f64> {
    let r = fonts::resolve(&style.family, &style.weight, &style.style);
    let f = fonts::face(&r)?;
    let s = size / f.units_per_em() as f64;
    let mut w = 0.0;
    for c in text.chars() {
        if let Some(g) = f.glyph_index(c) {
            w += f.glyph_hor_advance(g).unwrap_or(0) as f64;
        }
    }
    Some(w * s)
}

impl Measurer for Native {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        advance(text, style, size).unwrap_or(0.0)
    }

    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        let default = FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        };
        let r = fonts::resolve(&style.family, &style.weight, &style.style);
        let Some(f) = fonts::face(&r) else {
            return default;
        };
        let s = size / f.units_per_em() as f64;
        let asc = f.ascender() as f64 * s;
        let desc = -(f.descender() as f64) * s; // descender is negative; report magnitude
        let pos = |v: f64, d: f64| if v > 0.0 { v } else { d };
        FontMetrics {
            ascent: pos(asc, default.ascent),
            descent: pos(desc, default.descent),
            cap_height: f
                .capital_height()
                .map(|v| v as f64 * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.cap_height),
            x_height: f
                .x_height()
                .map(|v| v as f64 * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.x_height),
        }
    }
}

impl GlyphOutliner for Native {
    fn outline(
        &self,
        text: &str,
        style: &TextStyle,
        size: f64,
        x: f64,
        baseline: f64,
    ) -> Option<String> {
        let r = fonts::resolve(&style.family, &style.weight, &style.style);
        let f = fonts::face(&r)?;
        let s = size / f.units_per_em() as f64;
        let mut b = SvgOutline {
            d: String::new(),
            gx: x,
            base: baseline,
            s,
        };
        let mut pen = x;
        for c in text.chars() {
            let Some(g) = f.glyph_index(c) else { continue };
            b.gx = pen;
            f.outline_glyph(g, &mut b);
            pen += f.glyph_hor_advance(g).unwrap_or(0) as f64 * s;
        }
        (!b.d.is_empty()).then_some(b.d)
    }

    fn advance_width(&self, text: &str, style: &TextStyle, size: f64) -> Option<f64> {
        advance(text, style, size)
    }
}

impl Shaper for Native {
    /// Mirror the browser rasterizer (web/src/core/compiler.ts): sample nonzero-winding
    /// inside-ness on a grid and record the leftmost/rightmost inside x per ~`row_h` row,
    /// using the SAME row/step formulas so the spans line up.
    fn rasterize(&self, path_d: &str, row_h: f64) -> Option<RasterRegion> {
        let path = BezPath::from_svg(path_d).ok()?;
        let bb = path.bounding_box();
        let (bx, by, bw, bh) = (bb.x0, bb.y0, bb.width(), bb.height());
        if !(bw > 0.0 && bh > 0.0 && row_h > 0.0) {
            return None;
        }
        let rows = ((bh / row_h).ceil() as i64).max(1);
        let rh = bh / rows as f64;
        let xsteps = (bw.ceil() as i64).clamp(24, 400);
        let dx = bw / xsteps as f64;
        let mut out: Vec<Option<(f64, f64)>> = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let y = by + (r as f64 + 0.5) * rh;
            let (mut left, mut right) = (None, None);
            for i in 0..=xsteps {
                let x = bx + i as f64 * dx;
                if path.winding(Point::new(x, y)) != 0 {
                    if left.is_none() {
                        left = Some(x);
                    }
                    right = Some(x);
                }
            }
            out.push(match (left, right) {
                (Some(l), Some(r)) => Some((l, r)),
                _ => None,
            });
        }
        Some(RasterRegion::new(
            Rect {
                x: bx,
                y: by,
                w: bw,
                h: bh,
            },
            by,
            rh,
            out,
        ))
    }
}

/// Accumulates a run's glyph outlines into one SVG `d`, mapping font units (y-up, origin at
/// the glyph pen) to user space (y-down, baseline at `base`).
struct SvgOutline {
    d: String,
    gx: f64, // current glyph's x origin (pen)
    base: f64,
    s: f64, // size / units_per_em
}

impl SvgOutline {
    fn px(&self, x: f32) -> f64 {
        self.gx + x as f64 * self.s
    }
    fn py(&self, y: f32) -> f64 {
        self.base - y as f64 * self.s
    }
}

/// Round to 2 decimals and format without a trailing `.0` (and no negative zero).
fn n(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

impl OutlineBuilder for SvgOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.d
            .push_str(&format!("M{} {}", n(self.px(x)), n(self.py(y))));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.d
            .push_str(&format!("L{} {}", n(self.px(x)), n(self.py(y))));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.d.push_str(&format!(
            "Q{} {} {} {}",
            n(self.px(x1)),
            n(self.py(y1)),
            n(self.px(x)),
            n(self.py(y))
        ));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.d.push_str(&format!(
            "C{} {} {} {} {} {}",
            n(self.px(x1)),
            n(self.py(y1)),
            n(self.px(x2)),
            n(self.py(y2)),
            n(self.px(x)),
            n(self.py(y))
        ));
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}
