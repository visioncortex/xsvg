//! Resolved text styling needed to measure and lay out a run.

#[derive(Clone, Debug)]
pub struct TextStyle {
    pub family: String,
    pub size: f64,
    pub weight: String,
    pub style: String,
    /// Line advance as a multiple of `size`.
    pub line_height: f64,
    /// Extra tracking added between grapheme clusters, in user units. Absolute
    /// (not em-relative): it does not scale with `size`, matching CSS/SVG
    /// `letter-spacing`, and it layers on top of the font's kerning.
    pub letter_spacing: f64,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: "sans-serif".into(),
            size: 16.0,
            weight: "normal".into(),
            style: "normal".into(),
            line_height: 1.2,
            letter_spacing: 0.0,
        }
    }
}

impl TextStyle {
    /// A CSS `font` shorthand at the given size (what canvas `measureText` wants).
    pub fn font_css(&self, size: f64) -> String {
        format!("{} {} {}px {}", self.style, self.weight, size, self.family)
    }
}
