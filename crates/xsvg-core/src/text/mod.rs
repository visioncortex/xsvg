//! Text layout for xsvg, decomposed by domain:
//!
//! - [`style`] — resolved text styling ([`TextStyle`]).
//! - [`measure`] — the [`Measurer`] seam (font metrics) + word measurement.
//! - [`wrap`] — greedy line breaking.
//! - [`fit`] — shrink-to-fit font sizing.
//! - [`area`] — box placement (wrap + fit + align) into positioned lines.
//! - [`region`] — flow text into an arbitrary shape (the [`Region`]/[`Shaper`] seam).
//!
//! Everything here is pure and platform-free: it depends only on a [`Measurer`]
//! implementation, which an adapter supplies (browser canvas `measureText` in
//! v0, a native shaper later, or a fixture in tests).

pub mod area;
pub mod fit;
pub mod measure;
pub mod outline;
pub mod region;
pub mod style;
pub mod text_area;
pub mod truncate;
pub mod wrap;

// Glob re-exports: the public API is governed by `pub` vs `pub(crate)` at each
// definition site — only `pub` items reach the crate's public API.
// (`fit` is intentionally absent: its `fit_size` is `pub(crate)`, reached
// directly by `area`, so it has no public surface to re-export.)
pub use area::*;
pub use measure::*;
pub use outline::*;
pub use region::*;
pub use style::*;
pub use text_area::*;
pub use truncate::*;
pub use wrap::*;

#[cfg(test)]
pub(crate) mod test_support {
    use super::{FontMetrics, Measurer, TextStyle};

    /// Deterministic measurer for unit tests: width = char count × per_char × size,
    /// with simple proportional vertical metrics.
    pub struct Mono(pub f64);

    impl Measurer for Mono {
        fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
            text.chars().count() as f64 * self.0 * size
        }
        fn font_metrics(&self, _style: &TextStyle, size: f64) -> FontMetrics {
            FontMetrics {
                ascent: 0.8 * size,
                descent: 0.2 * size,
                cap_height: 0.7 * size,
                x_height: 0.5 * size,
            }
        }
    }
}
