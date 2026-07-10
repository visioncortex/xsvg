//! Color types and the sRGB8 <-> linear-RGB conversions.
//!
//! Ported from vtracer's `quadmesh::color`. The mesh works in **linear RGB** in
//! RAM (so bilinear blends are physically correct) and takes **8-bit sRGB** as
//! input (compact; banding is a non-issue because only the gradient *endpoints*
//! are quantized, not the blend between them).

/// 8-bit sRGB color — the on-disk / input form. Mirrors `visioncortex::Color` (RGB).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Decode sRGB8 -> linear-light RGB.
    pub fn to_linear(self) -> LinRgb {
        LinRgb {
            r: srgb8_to_linear(self.r),
            g: srgb8_to_linear(self.g),
            b: srgb8_to_linear(self.b),
        }
    }
}

/// Linear-light RGB — the in-RAM / on-GPU working form (f32 per channel).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct LinRgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl LinRgb {
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// Encode linear-light RGB -> sRGB8 (for persistence).
    pub fn to_srgb8(self) -> RgbColor {
        RgbColor {
            r: linear_to_srgb8(self.r),
            g: linear_to_srgb8(self.g),
            b: linear_to_srgb8(self.b),
        }
    }

    /// Component-wise linear interpolation (correct in linear-light space).
    pub fn lerp(self, other: LinRgb, t: f32) -> LinRgb {
        LinRgb {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
        }
    }
}

/// sRGB8 channel -> linear-light float in [0, 1] (IEC 61966-2-1 transfer fn).
pub fn srgb8_to_linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// linear-light float -> sRGB8 channel (clamped, rounded).
pub fn linear_to_srgb8(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let s = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_linear_endpoints() {
        assert_eq!(srgb8_to_linear(0), 0.0);
        assert!((srgb8_to_linear(255) - 1.0).abs() < 1e-6);
        assert_eq!(linear_to_srgb8(0.0), 0);
        assert_eq!(linear_to_srgb8(1.0), 255);
    }

    #[test]
    fn srgb_roundtrip_is_stable() {
        // Every 8-bit code must survive sRGB8 -> linear -> sRGB8 unchanged.
        for c in 0u8..=255 {
            let back = RgbColor::new(c, c, c).to_linear().to_srgb8();
            assert_eq!(back.r, c, "channel {c} drifted to {}", back.r);
        }
    }

    #[test]
    fn mid_gray_is_darker_in_linear() {
        // sRGB mid-gray (code 128 ~ 0.5 sRGB) decodes to ~0.216 linear, not 0.5 —
        // which is exactly why blends must happen in linear space.
        let lin = srgb8_to_linear(128);
        assert!(lin > 0.18 && lin < 0.25, "got {lin}");
    }
}
