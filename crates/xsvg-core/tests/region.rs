//! Integration tests for region flow (`<x:textbox in="#shape">`) driven by real,
//! browser-generated shape rasters.
//!
//! Fixtures in `tests/fixtures/regions.json` are coarse per-row inside-spans produced
//! in a browser via `getBBox` + `isPointInFill` (see `web/src/fixtures.ts`).
//! `RasterRegion` replays them so the flow layout runs against realistic geometry —
//! including a true circle's curve — without a browser.

use serde_json::Value;
use xsvg_core::{
    layout_region, Align, FontMetrics, Measurer, RasterRegion, Rect, Region, RegionSpec,
    TextOverflow, TextStyle, VAlign,
};

/// Proportional test measurer: width = chars × per_char × size.
struct Mono(f64);
impl Measurer for Mono {
    fn measure(&self, text: &str, _s: &TextStyle, size: f64) -> f64 {
        text.chars().count() as f64 * self.0 * size
    }
    fn font_metrics(&self, _s: &TextStyle, size: f64) -> FontMetrics {
        FontMetrics {
            ascent: 0.8 * size,
            descent: 0.2 * size,
            cap_height: 0.7 * size,
            x_height: 0.5 * size,
        }
    }
}

fn load_region(name: &str) -> RasterRegion {
    let path = format!("{}/tests/fixtures/regions.json", env!("CARGO_MANIFEST_DIR"));
    let txt = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let v: Value = serde_json::from_str(&txt).unwrap();
    let r = v["regions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("no region {name}"));
    let num = |k: &str| r[k].as_f64().unwrap();
    let rows = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .map(|lr| (lr[0].as_f64().unwrap(), lr[1].as_f64().unwrap()))
        })
        .collect();
    RasterRegion::new(
        Rect {
            x: num("minX"),
            y: num("minY"),
            w: num("w"),
            h: num("h"),
        },
        num("minY"),
        num("rowH"),
        rows,
    )
}

fn style(size: f64) -> TextStyle {
    TextStyle {
        size,
        ..Default::default()
    }
}

fn spec(align: Align) -> RegionSpec {
    RegionSpec {
        padding: 0.0,
        align,
        valign: VAlign::Top,
        text_overflow: TextOverflow::Clip,
    }
}

/// The circle raster is short at top/bottom and widest across the middle band.
#[test]
fn circle_raster_is_widest_in_the_middle() {
    let c = load_region("circle");
    let b = c.bounds();
    let top = c
        .span(b.y + 2.0, b.y + 6.0)
        .map(|(l, r)| r - l)
        .unwrap_or(0.0);
    let mid = c
        .span(b.y + b.h / 2.0 - 2.0, b.y + b.h / 2.0 + 2.0)
        .map(|(l, r)| r - l)
        .unwrap();
    assert!(
        mid > top + 1.0,
        "circle middle {mid} not wider than top {top}"
    );
}

/// Text poured into the apex-down triangle must not widen as it descends: each line's
/// word count is ≤ the previous line's.
#[test]
fn triangle_lines_do_not_widen_downward() {
    let tri = load_region("triangle-down");
    let out = layout_region(
        "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp",
        &style(10.0),
        &tri,
        &spec(Align::Center),
        &Mono(0.5),
    );
    assert!(
        out.lines.len() >= 3,
        "expected several lines: {:?}",
        out.lines
    );
    let counts: Vec<usize> = out
        .lines
        .iter()
        .map(|l| l.text.split(' ').count())
        .collect();
    for w in counts.windows(2) {
        assert!(w[1] <= w[0], "lines widen toward apex: {counts:?}");
    }
}

/// Circle flow produces multiple centered lines that all sit within the disc's width.
#[test]
fn circle_flow_stays_within_the_disc() {
    let c = load_region("circle");
    let b = c.bounds();
    let out = layout_region(
        "one two three four five six seven eight nine ten",
        &style(9.0),
        &c,
        &spec(Align::Center),
        &Mono(0.4),
    );
    assert!(out.lines.len() >= 3);
    for l in &out.lines {
        assert!(
            l.x >= b.x - 1e-6 && l.x <= b.x + b.w + 1e-6,
            "line anchor {} outside disc bounds",
            l.x
        );
    }
}
