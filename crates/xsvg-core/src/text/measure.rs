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

    /// [`Measurer::measure`] for many runs at once, one result per item in order.
    /// An adapter whose single measurement is a round trip (a JS hop into canvas
    /// `measureText`) overrides this to make the trip once per batch; the default
    /// measures each item in turn, so an adapter that does not is unaffected. The
    /// results are the same either way — this is a transport seam, not a semantic one.
    fn measure_batch(&self, items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
        items
            .iter()
            .map(|(text, style, size)| self.measure(text, style, *size))
            .collect()
    }
}

impl<T: Measurer + ?Sized> Measurer for &T {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        (**self).measure(text, style, size)
    }
    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        (**self).font_metrics(style, size)
    }
    fn measure_batch(&self, items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
        (**self).measure_batch(items)
    }
}

impl<T: Measurer + ?Sized> Measurer for Box<T> {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        (**self).measure(text, style, size)
    }
    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        (**self).font_metrics(style, size)
    }
    fn measure_batch(&self, items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
        (**self).measure_batch(items)
    }
}

/// The font a measurement depends on. [`Measurer::measure`] returns the raw glyph
/// advance — `letter-spacing` and `word-spacing` are layered on by [`line_advance`]
/// — so family, weight, style and size are the whole key.
#[derive(Clone, Hash, PartialEq, Eq)]
struct FontKey {
    family: String,
    weight: String,
    style: String,
    size: u64,
}

impl FontKey {
    fn of(style: &TextStyle, size: f64) -> Self {
        Self {
            family: style.family.clone(),
            weight: style.weight.clone(),
            style: style.style.clone(),
            size: size.to_bits(),
        }
    }
}

/// A [`Measurer`] that remembers: advance widths per `(font, text)` and vertical
/// metrics per font, for as long as it lives. An editor relays out a textbox on
/// every keystroke, and nearly every word in it was measured last keystroke in the
/// same font; behind a `canvas.measureText` hop or a `ttf-parser` face walk, that
/// repeat is most of the cost of a flush (docs/Compile Session.md §5).
///
/// Transparent by construction — a hit returns what the adapter returned — so output
/// is byte-identical with or without it (pinned by `tests/session.rs`). Bounded:
/// past [`MeasureCache::CAPACITY`] entries it starts over rather than grow without
/// limit. Owned by the embedder, typically via `Platform::with_measure_cache`.
pub struct MeasureCache<M> {
    inner: M,
    advances: std::sync::Mutex<std::collections::HashMap<(FontKey, String), f64>>,
    metrics: std::sync::Mutex<std::collections::HashMap<FontKey, FontMetrics>>,
}

impl<M: Measurer> MeasureCache<M> {
    /// Advance entries kept before the cache is cleared and refilled — tens of
    /// thousands of distinct words, a few megabytes at most.
    pub const CAPACITY: usize = 1 << 16;

    pub fn new(inner: M) -> Self {
        Self {
            inner,
            advances: std::sync::Mutex::new(std::collections::HashMap::new()),
            metrics: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// The adapter behind the cache.
    pub fn inner(&self) -> &M {
        &self.inner
    }
}

impl<M: Measurer> Measurer for MeasureCache<M> {
    fn measure(&self, text: &str, style: &TextStyle, size: f64) -> f64 {
        let key = (FontKey::of(style, size), text.to_string());
        // A poisoned lock means a panic mid-insert elsewhere; the map is still a
        // valid cache, so take it rather than fail every later measurement.
        let mut map = self.advances.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&w) = map.get(&key) {
            return w;
        }
        let w = self.inner.measure(text, style, size);
        if map.len() >= Self::CAPACITY {
            map.clear();
        }
        map.insert(key, w);
        w
    }

    fn font_metrics(&self, style: &TextStyle, size: f64) -> FontMetrics {
        let key = FontKey::of(style, size);
        let mut map = self.metrics.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&fm) = map.get(&key) {
            return fm;
        }
        let fm = self.inner.font_metrics(style, size);
        if map.len() >= Self::CAPACITY {
            map.clear();
        }
        map.insert(key, fm);
        fm
    }

    /// Hits are served from the map; only the misses go to the adapter, in one
    /// batch, so a cold paragraph costs the adapter one trip and a warm one none.
    fn measure_batch(&self, items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
        let keys: Vec<(FontKey, String)> = items
            .iter()
            .map(|(text, style, size)| (FontKey::of(style, *size), text.to_string()))
            .collect();
        let mut out = vec![0.0; items.len()];
        let misses: Vec<usize> = {
            let map = self.advances.lock().unwrap_or_else(|e| e.into_inner());
            (0..items.len())
                .filter(|&i| match map.get(&keys[i]) {
                    Some(&w) => {
                        out[i] = w;
                        false
                    }
                    None => true,
                })
                .collect()
        };
        if misses.is_empty() {
            return out;
        }
        // The lock is not held across the adapter call: it may be slow, and the
        // same word twice in one batch is simply measured twice.
        let asked: Vec<(&str, &TextStyle, f64)> = misses.iter().map(|&i| items[i]).collect();
        let widths = self.inner.measure_batch(&asked);
        let mut map = self.advances.lock().unwrap_or_else(|e| e.into_inner());
        for (k, &i) in misses.iter().enumerate() {
            // an adapter that returns too few results leaves those items at the
            // single-call path's answer rather than at zero
            let w = widths
                .get(k)
                .copied()
                .unwrap_or_else(|| self.inner.measure(items[i].0, items[i].1, items[i].2));
            out[i] = w;
            if map.len() >= Self::CAPACITY {
                map.clear();
            }
            map.insert(keys[i].clone(), w);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Width = chars × size; counts single calls and batch calls separately.
    struct Counting {
        singles: AtomicUsize,
        batches: AtomicUsize,
    }
    impl Measurer for Counting {
        fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
            self.singles.fetch_add(1, Ordering::Relaxed);
            text.chars().count() as f64 * size
        }
        fn measure_batch(&self, items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
            self.batches.fetch_add(1, Ordering::Relaxed);
            items.iter().map(|(t, _, s)| t.chars().count() as f64 * s).collect()
        }
    }

    fn counting() -> Counting {
        Counting {
            singles: AtomicUsize::new(0),
            batches: AtomicUsize::new(0),
        }
    }

    #[test]
    fn the_default_batch_is_the_single_calls_in_order() {
        struct Single;
        impl Measurer for Single {
            fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
                text.len() as f64 * size
            }
        }
        let st = TextStyle::default();
        let items = [("ab", &st, 10.0), ("", &st, 10.0), ("abcd", &st, 2.0)];
        assert_eq!(Single.measure_batch(&items), vec![20.0, 0.0, 8.0]);
    }

    #[test]
    fn the_cache_batches_only_its_misses() {
        let cache = MeasureCache::new(counting());
        let st = TextStyle::default();
        let big = TextStyle {
            size: 32.0,
            ..TextStyle::default()
        };
        // a cold batch: one trip, every item a miss
        let items = [("alpha", &st, 16.0), ("beta", &st, 16.0), ("alpha", &big, 32.0)];
        assert_eq!(cache.measure_batch(&items), vec![80.0, 64.0, 160.0]);
        assert_eq!(cache.inner().batches.load(Ordering::Relaxed), 1);
        assert_eq!(cache.inner().singles.load(Ordering::Relaxed), 0);

        // a warm batch with one new word: one trip carrying only that word
        let items = [("alpha", &st, 16.0), ("gamma", &st, 16.0), ("beta", &st, 16.0)];
        assert_eq!(cache.measure_batch(&items), vec![80.0, 80.0, 64.0]);
        assert_eq!(cache.inner().batches.load(Ordering::Relaxed), 2);
        assert_eq!(cache.inner().singles.load(Ordering::Relaxed), 0);

        // everything known: no trip at all, and single calls see the batch's entries
        assert_eq!(cache.measure_batch(&items), vec![80.0, 80.0, 64.0]);
        assert_eq!(cache.measure("gamma", &st, 16.0), 80.0);
        assert_eq!(cache.inner().batches.load(Ordering::Relaxed), 2);
        assert_eq!(cache.inner().singles.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_short_answer_from_the_adapter_falls_back_per_item() {
        struct Short;
        impl Measurer for Short {
            fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
                text.len() as f64 * size
            }
            fn measure_batch(&self, _items: &[(&str, &TextStyle, f64)]) -> Vec<f64> {
                Vec::new() // a broken adapter
            }
        }
        let cache = MeasureCache::new(Short);
        let st = TextStyle::default();
        assert_eq!(cache.measure_batch(&[("ab", &st, 3.0), ("abc", &st, 3.0)]), vec![6.0, 9.0]);
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
    /// Each run style's size as a multiple of the base size, indexed like the
    /// styles — empty for a single-style flow. A space drawn inside a larger run is
    /// that much wider on screen, so wrapping scales it (see `wrap::fill_line`).
    pub space_scale: Vec<f64>,
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
        space_scale: Vec::new(),
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
        space_scale: styles
            .iter()
            .map(|st| if base.size > 0.0 { st.size / base.size } else { 1.0 })
            .collect(),
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
