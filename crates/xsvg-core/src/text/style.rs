//! Resolved text styling needed to measure and lay out a run.

#[derive(Clone, Debug, PartialEq)]
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
    /// Extra space added at each inter-word gap, in user units. Absolute, like
    /// [`Self::letter_spacing`]; matches CSS/SVG `word-spacing`.
    pub word_spacing: f64,
    /// `text-transform` (§6.18): the case the text renders in, whatever case it
    /// was typed in. Layout applies it before it measures, so wrapping, fitting,
    /// outlines and caret stops all see the glyphs that render.
    pub text_transform: TextTransform,
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
            word_spacing: 0.0,
            text_transform: TextTransform::None,
        }
    }
}

impl TextStyle {
    /// A CSS `font` shorthand at the given size (what canvas `measureText` wants).
    pub fn font_css(&self, size: f64) -> String {
        format!("{} {} {}px {}", self.style, self.weight, size, self.family)
    }
}

/// CSS `text-transform`, the case-mapping subset (§6.18): `uppercase`,
/// `lowercase`, or `none`.
///
/// The mapping is **one character to one character**, so a caret offset in the
/// source text is the same offset in the rendered text. A character whose full
/// Unicode case mapping is several characters (`ﬁ`, `ŉ`) keeps its form, except
/// `ß`, whose capital is `ẞ` — where CSS would render `SS`, this keeps the
/// editing model simple and the word one glyph longer instead of two.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextTransform {
    #[default]
    None,
    Uppercase,
    Lowercase,
}

impl TextTransform {
    /// The keyword, or `None` for anything this subset does not know
    /// (`capitalize`, `full-width`, a typo) — the caller then keeps what it had.
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim() {
            "none" => Some(Self::None),
            "uppercase" => Some(Self::Uppercase),
            "lowercase" => Some(Self::Lowercase),
            _ => None,
        }
    }

    /// `text` in this case, one character for each of its characters.
    pub fn apply<'a>(self, text: &'a str) -> std::borrow::Cow<'a, str> {
        fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
            let c = it.next()?;
            it.next().is_none().then_some(c)
        }
        let map = |c: char| -> char {
            match self {
                Self::None => c,
                Self::Uppercase if c == 'ß' => 'ẞ',
                Self::Uppercase => single(c.to_uppercase()).unwrap_or(c),
                Self::Lowercase => single(c.to_lowercase()).unwrap_or(c),
            }
        };
        if self == Self::None || text.chars().all(|c| map(c) == c) {
            std::borrow::Cow::Borrowed(text)
        } else {
            std::borrow::Cow::Owned(text.chars().map(map).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextTransform;

    #[test]
    fn transforms_map_one_character_to_one() {
        let up = |s: &str| TextTransform::Uppercase.apply(s).into_owned();
        let low = |s: &str| TextTransform::Lowercase.apply(s).into_owned();
        assert_eq!(up("The pitch, naïve café"), "THE PITCH, NAÏVE CAFÉ");
        assert_eq!(low("THE Pitch ÉTÉ"), "the pitch été");
        // multi-character mappings keep the character count
        assert_eq!(up("straße"), "STRAẞE");
        assert_eq!(up("ﬁne ŉ"), "ﬁNE ŉ");
        assert_eq!(low("İstanbul").chars().count(), "İstanbul".chars().count());
        for s in ["straße ﬁ ŉ ǰ ΐ", "İ ǅ Σσς", "😀 a\u{301}b"] {
            assert_eq!(up(s).chars().count(), s.chars().count(), "{s}");
            assert_eq!(low(s).chars().count(), s.chars().count(), "{s}");
        }
        assert!(matches!(
            TextTransform::None.apply("Mixed"),
            std::borrow::Cow::Borrowed("Mixed")
        ));
        assert!(matches!(
            TextTransform::Uppercase.apply("ABC 12"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(TextTransform::parse("capitalize"), None);
        assert_eq!(
            TextTransform::parse(" uppercase "),
            Some(TextTransform::Uppercase)
        );
    }
}
