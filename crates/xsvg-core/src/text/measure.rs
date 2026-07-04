//! The font-metrics seam ([`Measurer`]) and word measurement.

use super::style::TextStyle;

/// Vertical font metrics at a given size, in user units. All are baseline-relative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontMetrics {
    /// Em-box ascent (top of the line box above the baseline).
    pub ascent: f64,
    /// Em-box descent (bottom of the line box below the baseline).
    pub descent: f64,
    /// Capital-letter height — the reference used for vertical alignment.
    pub cap_height: f64,
    /// Lowercase x-height.
    pub x_height: f64,
}

/// Source of font metrics. The only platform-specific dependency of text layout —
/// supplied by an adapter (browser canvas `measureText`, a native shaper, or a
/// fixture in tests).
pub trait Measurer {
    /// Advance width of `text` rendered in `style` at `size`, in user units.
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64;

    /// Vertical metrics at `size`. The default uses typical proportions; adapters
    /// with real metrics (canvas font/actual bounding box, or a fixture) override it.
    fn font_metrics(&self, _style: &TextStyle, size: f64) -> FontMetrics {
        FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        }
    }
}

/// A styled slice of a word: its text and an index into the layout's style table
/// (0 = the paragraph's base style). A plain word is a single piece with style 0; a
/// word split by a `<tspan>` boundary (even mid-word) is several pieces.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub text: String,
    pub style: usize,
}

/// One wrap unit (a "word"): its total advance and grapheme count at the base size,
/// plus the styled pieces it is made of. `advance` sums each piece measured in its
/// own style, so mixed weight/family wraps correctly; trial sizes scale it linearly.
#[derive(Clone, Debug)]
pub struct Word {
    pub advance: f64,
    pub graphemes: usize,
    pub pieces: Vec<Piece>,
}

/// Word advance widths measured once at the base size. Trial sizes scale these
/// linearly (good enough for layout; avoids re-measuring per fit iteration).
/// `letter_spacing`/`word_spacing` are carried through so wrapping can add them
/// without scaling (they are absolute lengths; see [`TextStyle`]).
pub struct Measured {
    pub words: Vec<Word>,
    pub space: f64,
    pub letter_spacing: f64,
    pub word_spacing: f64,
}

/// Measure each whitespace-separated word (and a space) at `style.size`. Single
/// style: every word is one piece with style index 0.
pub fn measure_words(text: &str, style: &TextStyle, m: &dyn Measurer) -> Measured {
    let words = text
        .split_whitespace()
        .map(|w| Word {
            advance: m.measure(w, style, style.size),
            graphemes: w.chars().count(),
            pieces: vec![Piece {
                text: w.to_string(),
                style: 0,
            }],
        })
        .collect();
    Measured {
        words,
        space: m.measure(" ", style, style.size),
        letter_spacing: style.letter_spacing,
        word_spacing: style.word_spacing,
    }
}

/// Measure a run of styled segments (`(text, style_id)`, from `<tspan>` runs) into
/// wrap units. Whitespace — anywhere, in any segment — separates words; adjacent
/// non-space chunks with no whitespace between them (even across a segment boundary)
/// join into one word with multiple pieces. `styles[0]` is the base style and
/// supplies the space advance and spacing (runs share the paragraph's size/spacing).
pub fn measure_runs(
    segments: &[(String, usize)],
    styles: &[TextStyle],
    m: &dyn Measurer,
) -> Measured {
    let base = &styles[0];
    let mut words: Vec<Word> = Vec::new();
    let mut cur: Option<Word> = None;

    for (text, sid) in segments {
        let style = styles.get(*sid).unwrap_or(base);
        // walk maximal non-space / space chunks
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let ws = rest.starts_with(char::is_whitespace);
            let end = rest
                .find(|c: char| c.is_whitespace() != ws)
                .unwrap_or(rest.len());
            let (chunk, tail) = rest.split_at(end);
            rest = tail;
            if ws {
                if let Some(w) = cur.take() {
                    words.push(w); // whitespace ends the current word
                }
            } else {
                let w = cur.get_or_insert_with(|| Word {
                    advance: 0.0,
                    graphemes: 0,
                    pieces: Vec::new(),
                });
                w.advance += m.measure(chunk, style, style.size);
                w.graphemes += chunk.chars().count();
                w.pieces.push(Piece {
                    text: chunk.to_string(),
                    style: *sid,
                });
            }
        }
    }
    if let Some(w) = cur {
        words.push(w);
    }

    Measured {
        words,
        space: m.measure(" ", base, base.size),
        letter_spacing: base.letter_spacing,
        word_spacing: base.word_spacing,
    }
}

/// Rendered advance of a whole run at `size`, including `letter-spacing` (once per
/// inter-grapheme gap) and `word-spacing` (once per ASCII space), layered on top of
/// the kerned glyph advances `measure` returns. This is the width a renderer
/// produces for the emitted attributes, so layout math must use it, not the raw
/// advance. Wrapped lines join words with a single space, so counting `' '` gives
/// the inter-word gap count.
pub fn line_advance(text: &str, style: &TextStyle, size: f64, m: &dyn Measurer) -> f64 {
    let gaps = text.chars().count().saturating_sub(1) as f64;
    let spaces = text.chars().filter(|c| *c == ' ').count() as f64;
    m.measure(text, style, size) + gaps * style.letter_spacing + spaces * style.word_spacing
}
