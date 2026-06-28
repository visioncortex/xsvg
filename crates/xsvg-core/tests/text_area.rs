//! Integration tests for text layout driven by real system-font metrics.
//!
//! Fixtures in `tests/fixtures/*.json` are per-glyph advance widths + vertical
//! metrics measured in a browser via canvas `measureText` (see
//! `web/fixtures.html`). `FixtureMeasurer` replays them so wrap/fit/placement run
//! against realistic, deterministic numbers without a browser.

use std::collections::HashMap;
use xsvg_core::{
    layout_area, layout_flow, measure_words, wrap, Align, Anchor, AreaSpec, Fit, FontMetrics,
    Measurer, TextStyle, VAlign,
};

/// A font fixture: glyph advances and vertical metrics at `base_size`.
struct FixtureFont {
    base_size: f64,
    ascent: f64,
    descent: f64,
    cap_height: f64,
    x_height: f64,
    space: f64,
    chars: HashMap<char, f64>,
}

impl FixtureFont {
    fn load(slug: &str) -> Self {
        let path = format!("{}/tests/fixtures/{slug}.json", env!("CARGO_MANIFEST_DIR"));
        let txt = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
        let num = |k: &str| v[k].as_f64().unwrap();
        let chars = v["chars"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, val)| (k.chars().next().unwrap(), val.as_f64().unwrap()))
            .collect();
        FixtureFont {
            base_size: num("baseSize"),
            ascent: num("ascent"),
            descent: num("descent"),
            cap_height: num("capHeight"),
            x_height: num("xHeight"),
            space: num("space"),
            chars,
        }
    }
}

impl Measurer for FixtureFont {
    fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
        let base: f64 = text
            .chars()
            .map(|c| *self.chars.get(&c).unwrap_or(&self.space))
            .sum();
        base * size / self.base_size
    }
    fn font_metrics(&self, _style: &TextStyle, size: f64) -> FontMetrics {
        let k = size / self.base_size;
        FontMetrics {
            ascent: self.ascent * k,
            descent: self.descent * k,
            cap_height: self.cap_height * k,
            x_height: self.x_height * k,
        }
    }
}

const FONTS: &[&str] = &["arial", "times-new-roman", "courier-new", "georgia"];
const TOL: f64 = 1e-6;

fn style(size: f64) -> TextStyle {
    TextStyle {
        size,
        ..Default::default()
    }
}

/// Every multi-word wrapped line must fit the width (lone overflow words may not).
#[test]
fn wrapped_lines_fit_width_in_every_font() {
    let text = "The quick brown fox jumps over the lazy dog again and again today";
    let width = 220.0;
    for slug in FONTS {
        let font = FixtureFont::load(slug);
        let st = style(16.0);
        let measured = measure_words(text, &st, &font);
        let lines = wrap(&measured, width, 1.0);
        assert!(lines.len() > 1, "{slug}: expected multiple lines");
        for line in &lines {
            if line.split(' ').count() > 1 {
                let w = font.measure(line, &st, 16.0);
                assert!(
                    w <= width + TOL,
                    "{slug}: line {line:?} width {w} > {width}"
                );
            }
        }
    }
}

/// shrink-to-fit lands within [min, base] and the block fits the box height.
#[test]
fn shrink_to_fit_respects_box_in_every_font() {
    let text = "This caption is far too long to fit the box at its original size";
    for slug in FONTS {
        let font = FixtureFont::load(slug);
        let st = style(40.0);
        let spec = AreaSpec {
            x: 0.0,
            y: 0.0,
            width: 160.0,
            height: 80.0,
            padding: 8.0,
            align: Align::Center,
            valign: VAlign::Middle,
            fit: Fit::Shrink { min: 6.0 },
        };
        let out = layout_area(text, &st, &spec, &font);
        assert!(
            out.font_size <= 40.0 + TOL,
            "{slug}: not shrunk: {}",
            out.font_size
        );
        assert!(
            out.font_size >= 6.0 - TOL,
            "{slug}: below min: {}",
            out.font_size
        );

        let (cw, ch) = (160.0 - 16.0, 80.0 - 16.0);
        let advance = out.font_size * st.line_height;
        let block_h = out.lines.len() as f64 * advance;
        // either it fits, or we bottomed out at the min size
        assert!(
            block_h <= ch + TOL || (out.font_size - 6.0).abs() < TOL,
            "{slug}: block {block_h} > {ch} at size {}",
            out.font_size
        );
        for l in &out.lines {
            if l.text.split(' ').count() > 1 {
                let w = font.measure(&l.text, &st, out.font_size);
                assert!(w <= cw + TOL, "{slug}: line overflows: {w} > {cw}");
            }
        }
    }
}

/// Placement uses the fixture's real cap-height and centers correctly.
#[test]
fn placement_uses_cap_height_and_centers() {
    let font = FixtureFont::load("times-new-roman");
    let st = style(20.0);
    let spec = AreaSpec {
        x: 10.0,
        y: 10.0,
        width: 120.0,
        height: 120.0,
        padding: 10.0,
        align: Align::Center,
        valign: VAlign::Top,
        fit: Fit::None,
    };
    let out = layout_area("hi there", &st, &spec, &font);
    assert_eq!(out.anchor, Anchor::Middle);

    // horizontal: anchor x at content centre
    let (cx, cw) = (10.0 + 10.0, 120.0 - 20.0);
    assert!((out.lines[0].x - (cx + cw / 2.0)).abs() < TOL);

    // vertical: top-aligned → first baseline = content top + cap_height(20px).
    let expected = cx + font.cap_height * 20.0 / 100.0; // cy == cx here (both 20)
    assert!(
        (out.lines[0].baseline - expected).abs() < 1e-3,
        "baseline {} != {expected}",
        out.lines[0].baseline
    );
}

/// The fixtures reflect font shape: Courier is monospace, Arial is proportional.
#[test]
fn fixtures_capture_font_shape() {
    let st = style(10.0);

    let courier = FixtureFont::load("courier-new");
    let m = courier.measure("M", &st, 10.0);
    let i = courier.measure("i", &st, 10.0);
    assert!(
        (m - i).abs() < TOL,
        "courier should be monospace: M={m} i={i}"
    );

    let arial = FixtureFont::load("arial");
    assert!(
        arial.measure("M", &st, 10.0) > arial.measure("i", &st, 10.0),
        "arial should be proportional"
    );
}

/// Different fonts can wrap the same text/box differently — both stay valid.
#[test]
fn fonts_can_wrap_differently() {
    let text = "Wrapping depends on the font's own glyph advance widths";
    let width = 180.0;
    let st = style(15.0);
    let counts: Vec<usize> = ["courier-new", "arial"]
        .iter()
        .map(|slug| {
            let font = FixtureFont::load(slug);
            wrap(&measure_words(text, &st, &font), width, 1.0).len()
        })
        .collect();
    // Courier is much wider than Arial at the same size → at least as many lines.
    assert!(counts.iter().all(|&n| n >= 1));
    assert!(
        counts[0] >= counts[1],
        "courier {} should wrap >= arial {}",
        counts[0],
        counts[1]
    );
    // sanity: descent is positive in a loaded fixture
    assert!(FixtureFont::load("georgia").descent > 0.0);
}

/// Degenerate geometry must not panic and must produce sane output, every font.
#[test]
fn degenerate_layouts_hold_across_fonts() {
    for slug in FONTS {
        let font = FixtureFont::load(slug);
        let st = style(20.0);
        let base = AreaSpec {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
            padding: 8.0,
            align: Align::Center,
            valign: VAlign::Middle,
            fit: Fit::None,
        };

        assert!(
            layout_area("", &st, &base, &font).lines.is_empty(),
            "{slug}: empty text should yield no lines"
        );

        let zero = AreaSpec {
            width: 0.0,
            height: 0.0,
            padding: 0.0,
            fit: Fit::Shrink { min: 5.0 },
            ..base
        };
        let out = layout_area("alpha beta gamma", &st, &zero, &font);
        assert_eq!(out.font_size, 5.0, "{slug}");
        assert_eq!(out.lines.len(), 3, "{slug}: zero width → one word per line");

        let long = layout_area("supercalifragilisticexpialidocious", &st, &base, &font);
        assert_eq!(
            long.lines.len(),
            1,
            "{slug}: long word is one overflow line"
        );
    }
}

/// `layout_flow` puts the first baseline at `y` (the SVG `<text>` convention).
#[test]
fn flow_first_baseline_at_y_real_font() {
    let font = FixtureFont::load("arial");
    let st = style(16.0);
    let lines = layout_flow("hello world", &st, 5.0, 40.0, 1000.0, &font);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].x, 5.0);
    assert_eq!(lines[0].baseline, 40.0);
}

/// Descenders must not move the baseline: alignment reserves the *font's* descent
/// (constant), not the per-string ink, so a descender-free and a descender-heavy
/// label land on the exact same baseline for every valign.
#[test]
fn descenders_do_not_shift_alignment() {
    let font = FixtureFont::load("arial");
    let st = style(20.0);
    for valign in [VAlign::Top, VAlign::Middle, VAlign::Bottom] {
        let spec = AreaSpec {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 80.0,
            padding: 8.0,
            align: Align::Start,
            valign,
            fit: Fit::None,
        };
        let plain = layout_area("Aa Bb", &st, &spec, &font);
        let desc = layout_area("Aa Gg", &st, &spec, &font);
        assert_eq!(plain.lines.len(), 1);
        assert_eq!(desc.lines.len(), 1);
        assert!(
            (plain.lines[0].baseline - desc.lines[0].baseline).abs() < TOL,
            "{valign:?}: descender shifted baseline {} vs {}",
            plain.lines[0].baseline,
            desc.lines[0].baseline
        );
    }
}
