//! Canonical deterministic platform-seam doubles, shared by this crate's own
//! compiler tests and downstream suites (xsvg-kit's compile-parity corpus).
//! Behavior is part of the test contract — changing `Mono`'s widths or
//! `BoxShaper`'s box invalidates golden expectations downstream.

use crate::{GlyphOutliner, Measurer, RasterRegion, Rect, Shaper, TextStyle};

/// Deterministic measurer: width = char count × 0.5 × size.
pub struct Mono;
impl Measurer for Mono {
    fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
        text.chars().count() as f64 * 0.5 * size
    }
}

/// No shape rasterizer (the default for tests that don't use `in=`).
pub struct NoShaper;
impl Shaper for NoShaper {
    fn rasterize(&self, _d: &str, _row_h: f64) -> Option<RasterRegion> {
        None
    }
}

/// Pretends every shape is a 60×60 box, so `in=`-region flow can be exercised
/// without a browser (the real raster comes from the browser / fixtures).
pub struct BoxShaper;
impl Shaper for BoxShaper {
    fn rasterize(&self, _d: &str, row_h: f64) -> Option<RasterRegion> {
        let n = (60.0 / row_h).ceil().max(1.0) as usize;
        Some(RasterRegion::new(
            Rect { x: 0.0, y: 0.0, w: 60.0, h: 60.0 },
            0.0,
            row_h,
            vec![Some((0.0, 60.0)); n],
        ))
    }
}

/// No glyph outliner (the default for tests not exercising `outline`).
pub struct NoOutliner;
impl GlyphOutliner for NoOutliner {
    fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, _x: f64, _b: f64) -> Option<String> {
        None
    }
}

/// Stub outliner: a deterministic 1×1 box path at the run origin (so outline
/// emit paths can be exercised without a real font) and Mono-consistent advance
/// widths (chars × 0.5 × size), matching the `Mono` measurer.
pub struct BoxOutliner;
impl GlyphOutliner for BoxOutliner {
    fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, x: f64, b: f64) -> Option<String> {
        Some(format!("M{x},{b} h1 v-1 h-1 Z"))
    }
    fn advance_width(&self, text: &str, _s: &TextStyle, size: f64) -> Option<f64> {
        Some(text.chars().count() as f64 * 0.5 * size)
    }
}
