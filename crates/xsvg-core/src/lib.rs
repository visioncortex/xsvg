//! Shared, pure-Rust types for the xsvg engine.
//!
//! This crate must stay free of platform/JS/web dependencies so it compiles
//! identically for native and `wasm32` targets (see Plan.md §1, "Core invariant").

/// The single approximation knob threaded through every lowering pass.
/// See Plan.md §1.3. In v0 it is parsed and carried through but only lightly
/// exercised (no curve flattening yet).
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
    /// Parse a quality string (case-insensitive); unknown values fall back to `Balanced`.
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
