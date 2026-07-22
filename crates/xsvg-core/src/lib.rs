//! Shared, pure-Rust types for the xsvg engine, plus the document compiler.
//!
//! This crate must stay free of platform/JS/web dependencies so it compiles
//! identically for native and `wasm32` targets (see Plan.md §1, "Core invariant").
//! The `compile` module is the xsvg → plain-SVG compiler over these primitives; its
//! platform seams (`Measurer`/`Shaper`/`GlyphOutliner`) are backed by `xsvg-wasm`
//! (browser) or `xsvg-cli` (native).

pub mod boolean;
pub mod filter;
pub mod offset;
pub mod text;
pub mod warp;

// The document compiler: parse xsvg/SVG, run the lowering passes, emit plain SVG. Behind
// the `compile` feature (on by default) since it's the only thing that needs an XML parser;
// without it, xsvg-core is the geometry/text/gradient primitives alone. Kept a private
// module — only its entry points are re-exported (the emit_* internals stay here).
#[cfg(feature = "compile")]
mod compile;

pub use boolean::*;
#[cfg(feature = "compile")]
pub use compile::{
    compile_fragment_impl, compile_fragment_linked_impl, compile_impl, compile_linked_impl,
    dependents_impl, fragment_range_impl, NoResolver, Resolver,
};
pub use filter::*;
pub use offset::*;
pub use text::*;
pub use warp::*;

// Geometry re-export: kurbo is the crate-wide geometry currency (Plan.md §1), so
// downstream crates use the same types without a direct dependency.
pub use kurbo;

// Mesh-gradient engine (Pillar 3): mesh rasterization, color-field fitting, and
// the texel-aligned tiny-PNG serialization — extracted from vtracer into the
// workspace `xsvg-gradient` crate and re-exported as `gradient`, the same way as kurbo.
pub use xsvg_gradient as gradient;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QualityProfile {
    /// Coarse tolerances; eager rasterization.
    Fast,
    /// Sensible middle ground.
    #[default]
    Balanced,
    /// Tight tolerances; vector-exact where possible.
    Highest,
    /// Rasterize hard cases to an embedded image.
    Raster,
}

impl QualityProfile {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "fast" => Self::Fast,
            "highest" => Self::Highest,
            "raster" => Self::Raster,
            _ => Self::Balanced,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Balanced => "balanced",
            Self::Highest => "highest",
            Self::Raster => "raster",
        }
    }

    /// The geometry-bake tolerance (§7.1) in user units: the Hausdorff bound for
    /// curve flattening and for the adaptive subdivision of mapped segments — the
    /// graded quality knob for **shape** geometry. (Balanced sits at 0.1: a
    /// stroked circle edge at 0.25 showed faint visible faceting.)
    pub fn tolerance(self) -> f64 {
        match self {
            Self::Fast => 1.0,
            Self::Balanced => 0.1,
            Self::Highest | Self::Raster => 0.02,
        }
    }

    /// The bake tolerance for **glyph** geometry (§6.13): letterforms are judged
    /// at reading distance, so text bakes much tighter than shapes. These values
    /// are visually validated (pixel parity with dense uniform sampling) — they
    /// are pinned independently of [`Self::tolerance`] so shape tuning cannot
    /// silently change text quality.
    pub fn text_tolerance(self) -> f64 {
        match self {
            Self::Fast => 0.1,
            Self::Balanced => 0.025,
            Self::Highest | Self::Raster => 0.005,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_and_unknown() {
        assert_eq!(QualityProfile::parse("FAST"), QualityProfile::Fast);
        assert_eq!(QualityProfile::parse("  Highest "), QualityProfile::Highest);
        assert_eq!(QualityProfile::parse("nonsense"), QualityProfile::Balanced);
        assert_eq!(QualityProfile::default(), QualityProfile::Balanced);
    }
}
