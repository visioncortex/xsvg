//! Native implementations of the compiler's platform seams, backed by `ttf-parser`
//! (metrics + glyph outlines) and `kurbo` (path inside-testing for shape flow) instead of
//! the browser. `Native` owns the loaded `FontDb`, so one value drives the compile.

use crate::fonts::{self, FontDb};
use ttf_parser::OutlineBuilder;
use xsvg_core::kurbo::{BezPath, Point, Shape};
use xsvg_core::{FontMetrics, GlyphOutliner, Measurer, RasterRegion, Rect, Shaper, TextStyle};

pub struct Native {
    pub fonts: FontDb,
}

impl Native {
    /// Sum of horizontal glyph advances for `text` in the resolved font, in user units.
    /// `text` shaped in the style's face at `size`: the glyphs rustybuzz lays
    /// — kerned, ligated, marks placed — each with its cluster (the byte of the
    /// first character it draws), its advance and its offset in user units. The
    /// same shaper resvg renders live text with, so what the compiler measures
    /// and traces is what that renderer draws. `None` without the font.
    fn shape(&self, text: &str, style: &TextStyle, size: f64) -> Option<Shaped<'_>> {
        let bytes = self.fonts.resolve(&style.family, &style.style)?;
        let mut face = rustybuzz::Face::from_slice(bytes, 0)?;
        face.set_variations(&[rustybuzz::Variation {
            tag: ttf_parser::Tag::from_bytes(b"wght"),
            value: fonts::parse_weight(&style.weight),
        }]);
        let s = size / face.units_per_em() as f64;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let out = rustybuzz::shape(&face, &[], buffer);
        let glyphs = out
            .glyph_infos()
            .iter()
            .zip(out.glyph_positions())
            .map(|(i, p)| ShapedGlyph {
                id: ttf_parser::GlyphId(i.glyph_id as u16),
                cluster: i.cluster as usize,
                advance: p.x_advance as f64 * s,
                dx: p.x_offset as f64 * s,
                dy: p.y_offset as f64 * s,
            })
            .collect();
        Some(Shaped { face, glyphs, s })
    }

    fn advance(&self, text: &str, style: &TextStyle, size: f64) -> Option<f64> {
        Some(
            self.shape(text, style, size)?
                .glyphs
                .iter()
                .map(|g| g.advance)
                .sum(),
        )
    }
}

/// A run shaped for one face and size (see [`Native::shape`]).
struct Shaped<'a> {
    face: rustybuzz::Face<'a>,
    glyphs: Vec<ShapedGlyph>,
    /// Font units to user units.
    s: f64,
}

struct ShapedGlyph {
    id: ttf_parser::GlyphId,
    cluster: usize,
    advance: f64,
    dx: f64,
    dy: f64,
}

impl Measurer for Native {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        self.advance(text, style, size).unwrap_or(0.0)
    }

    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        let default = FontMetrics::proportional(size);
        let Some(bytes) = self.fonts.resolve(&style.family, &style.style) else {
            return default;
        };
        let Some(f) = fonts::face(bytes, fonts::parse_weight(&style.weight)) else {
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
            // the font's own lines: `post` puts the underline's TOP at a negative
            // offset below the baseline; OS/2 puts the strikeout's top above it
            underline_position: f
                .underline_metrics()
                .map(|lm| -(lm.position as f64) * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.underline_position),
            underline_thickness: f
                .underline_metrics()
                .map(|lm| lm.thickness as f64 * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.underline_thickness),
            strikeout_position: f
                .strikeout_metrics()
                .map(|lm| (lm.position as f64 - lm.thickness as f64 / 2.0) * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.strikeout_position),
            strikeout_thickness: f
                .strikeout_metrics()
                .map(|lm| lm.thickness as f64 * s)
                .filter(|v| *v > 0.0)
                .unwrap_or(default.strikeout_thickness),
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
        let shaped = self.shape(text, style, size)?;
        let mut b = SvgOutline {
            d: String::new(),
            gx: x,
            base: baseline,
            s: shaped.s,
        };
        let mut pen = x;
        for g in &shaped.glyphs {
            b.gx = pen + g.dx;
            b.base = baseline - g.dy;
            shaped.face.outline_glyph(g.id, &mut b);
            pen += g.advance;
        }
        (!b.d.is_empty()).then_some(b.d)
    }

    /// Entry `i` is the pen where scalar `i + 1` begins, kerned; the last the
    /// run's width. A character inside a ligature begins where the ligature
    /// does — its cluster's pen — matching the browser adapter's per-prefix
    /// fallback in placement if not in principle.
    fn advances(&self, text: &str, style: &TextStyle, size: f64) -> Option<Vec<f64>> {
        let shaped = self.shape(text, style, size)?;
        // pen at each glyph's cluster start, then the total
        let mut pen_at: Vec<(usize, f64)> = Vec::with_capacity(shaped.glyphs.len());
        let mut pen = 0.0;
        for g in &shaped.glyphs {
            pen_at.push((g.cluster, pen));
            pen += g.advance;
        }
        let total = pen;
        let pen_of = |byte: usize| {
            pen_at
                .iter()
                .filter(|(c, _)| *c <= byte)
                .map(|(_, p)| *p)
                .fold(None, |acc: Option<f64>, p| {
                    Some(acc.map_or(p, |a| a.max(p)))
                })
                .unwrap_or(0.0)
        };
        let mut out: Vec<f64> = text
            .char_indices()
            .skip(1)
            .map(|(byte, _)| pen_of(byte))
            .collect();
        out.push(total);
        Some(out)
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
