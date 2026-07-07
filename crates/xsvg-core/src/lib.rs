//! Shared, pure-Rust types for the xsvg engine.
//!
//! This crate must stay free of platform/JS/web dependencies so it compiles
//! identically for native and `wasm32` targets (see Plan.md §1, "Core invariant").

pub mod boolean;
pub mod text;
pub mod warp;

pub use boolean::*;
pub use text::*;
pub use warp::*;

// Geometry re-export: kurbo is the crate-wide geometry currency (Plan.md §1), so
// downstream crates use the same types without a direct dependency.
pub use kurbo;

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
    /// curve flattening and for the adaptive subdivision of mapped segments. The
    /// single graded quality knob of the transform pipeline.
    pub fn tolerance(self) -> f64 {
        match self {
            Self::Fast => 1.0,
            Self::Balanced => 0.25,
            Self::Highest | Self::Raster => 0.05,
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
