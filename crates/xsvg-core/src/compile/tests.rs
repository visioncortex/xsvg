use super::*;
use crate::{RasterRegion, Rect}; // used only by the test doubles below

/// Deterministic measurer: width = char count × 0.5 × size.
struct Mono;
impl Measurer for Mono {
    fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
        text.chars().count() as f64 * 0.5 * size
    }
}

/// No shape rasterizer (the default for tests that don't use `in=`).
struct NoShaper;
impl Shaper for NoShaper {
    fn rasterize(&self, _d: &str, _row_h: f64) -> Option<RasterRegion> {
        None
    }
}

/// Pretends every shape is a 60×60 box, so `in=`-region flow can be exercised
/// without a browser (the real raster comes from the browser / fixtures).
struct BoxShaper;
impl Shaper for BoxShaper {
    fn rasterize(&self, _d: &str, row_h: f64) -> Option<RasterRegion> {
        let n = (60.0 / row_h).ceil().max(1.0) as usize;
        Some(RasterRegion::new(
            Rect {
                x: 0.0,
                y: 0.0,
                w: 60.0,
                h: 60.0,
            },
            0.0,
            row_h,
            vec![Some((0.0, 60.0)); n],
        ))
    }
}

/// No glyph outliner (the default for tests not exercising `outline`).
struct NoOutliner;
impl GlyphOutliner for NoOutliner {
    fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, _x: f64, _b: f64) -> Option<String> {
        None
    }
}

/// Stub outliner: a deterministic 1×1 box path at the run origin (so outline
/// emit paths can be exercised without a real font) and Mono-consistent advance
/// widths (chars × 0.5 × size), matching the `Mono` measurer.
struct BoxOutliner;
impl GlyphOutliner for BoxOutliner {
    fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, x: f64, b: f64) -> Option<String> {
        Some(format!("M{x},{b} h1 v-1 h-1 Z"))
    }
    fn advance_width(&self, text: &str, _s: &TextStyle, size: f64) -> Option<f64> {
        Some(text.chars().count() as f64 * 0.5 * size)
    }
}

fn compile_test(svg: &str) -> String {
    compile_impl(svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap()
}

/// Compile with the 60×60 `BoxShaper`, for `<x:textbox in>` region-flow tests.
fn compile_shaped(svg: &str) -> String {
    compile_impl(svg, "balanced", false, &Mono, &BoxShaper, &NoOutliner).unwrap()
}

/// Compile with the stub `BoxOutliner`, for `outline` (create-outlines) tests.
fn compile_outlined(svg: &str) -> String {
    compile_impl(svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap()
}

/// Compile with the source map on (`data-xsvg-pos` attributes emitted).
fn compile_mapped(svg: &str) -> String {
    compile_impl(svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap()
}

#[test]
fn connectors_route_between_two_boxes() {
    // two rects; a straight connector's path runs between their facing edges
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b" stroke="#111"/></svg>"##
    );
    let out = compile_test(&svg);
    // the route path (fill=none) is followed by one arrowhead triangle
    assert!(out.contains("fill=\"none\""), "{out}");
    assert!(
        out[out.find("fill=\"none\"").unwrap()..].contains("<path fill=\"#111\""),
        "arrowhead after the route: {out}"
    );
    // the line stops at the arrowhead's base (one arrow height back from b's
    // edge x=120, default size 7), so the stroke never protrudes past the tip
    use crate::kurbo::Shape;
    let bb = crate::kurbo::BezPath::from_svg(route_d(&out))
        .unwrap()
        .bounding_box();
    assert!(
        (bb.x0 - 40.0).abs() < 0.5 && (bb.x1 - 113.0).abs() < 0.5,
        "{bb:?}"
    );
    // the arrowhead is a triangle whose TIP sits exactly on b's edge (x=120)
    let head = out
        .rsplit(" d=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert!(
        head.starts_with("M120,"),
        "tip at the edge, not past it: {head}"
    );

    // x-major rail: a Z of three segments (M + 3 L), elbow at the mid-x
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="80" width="40" height="40"/><x:connector from="#a" to="#b" route="x-major" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !out[out.find("fill=\"none\"").unwrap()..].contains("<path"),
        "arrow=none emits no arrowhead after the route: {out}"
    );
    let d = route_d(&out);
    assert_eq!(d.matches('L').count(), 3, "x-major is H-V-H: {d}");
    assert!(
        d.starts_with("M40,20"),
        "exits a's right edge at its center y: {d}"
    );

    // curve emits a cubic; arrow-size scales the head
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="160" y="10" width="40" height="40"/><x:connector from="#a" to="#b" route="curve" arrow-size="20"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).starts_with("M40,20 C"), "{out}");
    let head = out
        .rsplit(" d=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let hp = crate::kurbo::BezPath::from_svg(head)
        .unwrap()
        .bounding_box();
    assert!(
        hp.width().max(hp.height()) > 12.0,
        "big head for arrow-size=20: {hp:?}"
    );

    // missing endpoint degrades with a marker
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="10" height="10"/><x:connector from="#a" to="#ghost"/></svg>"##
    );
    assert!(
        compile_test(&svg).contains("endpoint not found"),
        "{}",
        compile_test(&svg)
    );
}

#[test]
fn connectors_are_baked_references() {
    // editing box #a re-emits the connector that routes from it
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b"/></svg>"##
    );
    let offs = top_level_offsets(&svg);
    let deps = dependents_impl(&svg, offs[0]); // edit #a
    assert_eq!(deps.len(), 1, "{deps:?}");
    assert_eq!(deps[0].0, offs[2], "the connector re-emits: {deps:?}");
}

#[test]
fn connectors_accept_points_and_side_anchors() {
    // to-point: the connector ends at a raw x,y — no target element needed
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><x:connector from="#a" to-point="200,150" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).ends_with("L200,150"), "ends at the given point: {}", route_d(&out));

    // side anchor: `#b:left` forces the left-edge midpoint of b (x=120..160, y=0..40 → 120,20)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:left" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).ends_with("L120,20"), "attaches at b's left-edge midpoint: {}", route_d(&out));

    // a forced side on `from` too: leaves a's bottom-edge midpoint (20,40)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a:bottom" to="#b" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).starts_with("M20,40"), "leaves a's bottom-edge midpoint: {}", route_d(&out));

    // corner anchor: b's right-bottom corner is (x1,y1) = (160,40); order-independent
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:bottom-right" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).ends_with("L160,40"), "corner anchor (either token order): {}", route_d(&out));

    // center anchor: b's center (140,20)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:center" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(route_d(&out).ends_with("L140,20"), "center anchor: {}", route_d(&out));

    // the ref wins when both a ref and a point are given
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b" to-point="500,500" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!route_d(&out).contains("500"), "the `to` ref takes precedence over to-point: {}", route_d(&out));
}

#[test]
fn offset_outsets_a_referenced_rect_and_is_baked() {
    use crate::kurbo::Shape;
    let svg = format!(
        r##"{XW}<rect id="r" x="10" y="10" width="40" height="40"/><x:offset in="#r" distance="6" join="miter" fill="#111"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<path fill=\"#111\""),
        "offset emits a plain path: {out}"
    );
    // a miter outset grows the 40×40 rect by 6 on every side → bbox [4,4]..[56,56]
    let d = out
        .rsplit(" d=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let bb = crate::kurbo::BezPath::from_svg(d).unwrap().bounding_box();
    assert!(
        (bb.x0 - 4.0).abs() < 0.7
            && (bb.y0 - 4.0).abs() < 0.7
            && (bb.x1 - 56.0).abs() < 0.7
            && (bb.y1 - 56.0).abs() < 0.7,
        "grown bbox {bb:?}"
    );
    // baked reference: editing #r re-emits the offset
    let offs = top_level_offsets(&svg);
    let deps = dependents_impl(&svg, offs[0]);
    assert_eq!(deps.len(), 1, "{deps:?}");
    assert_eq!(
        deps[0].0, offs[1],
        "the offset re-emits when #r changes: {deps:?}"
    );
}

#[test]
fn offset_inset_and_degradations() {
    // an inset larger than the region's half-thickness legitimately empties
    let svg = format!(
        r##"{XW}<rect id="r" x="0" y="0" width="40" height="40"/><x:offset in="#r" distance="-30" fill="#111"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !out.contains("<path fill=\"#111\""),
        "over-inset produces no filled offset path: {out}"
    );
    // a missing reference degrades with a marker, never a panic
    let svg = format!(r##"{XW}<x:offset in="#nope" distance="4"/></svg>"##);
    assert!(
        compile_test(&svg).contains("<!-- xsvg: <x:offset"),
        "missing ref → marker"
    );
}

#[test]
fn list_markers_and_hanging_indent() {
    // bullet list: level-0 and level-1 items get distinct glyphs, and each
    // level's text column steps right by one indent (default 1.5em)
    let svg = format!(
        r##"{XW}<x:list x="10" y="10" width="200" font-size="10" line-height="1.2"><x:li>Alpha</x:li><x:li indent="1">Beta</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    // wrapped in a <g>, with a left-anchored <text> block
    assert!(out.contains("<g>"), "{out}");
    assert!(out.contains("<text text-anchor=\"start\""), "{out}");
    // bullets are DRAWN shapes, not glyphs: level-0 filled disc, level-1 ring
    assert!(
        out.contains("<circle") && out.contains("fill=\"#111827\""),
        "disc: {out}"
    );
    assert!(
        out.contains("fill=\"none\" stroke=\"#111827\""),
        "ring: {out}"
    );
    // hanging indent: level-0 text at x=10+15=25, level-1 at x=10+30=40
    assert!(out.contains("<tspan x=\"25\""), "level-0 column: {out}");
    assert!(out.contains("<tspan x=\"40\""), "level-1 column: {out}");
    // the x: source is fully lowered
    assert!(!out.contains("x:list") && !out.contains("x:li"), "{out}");
}

#[test]
fn list_custom_markers() {
    // a named shape overrides the default cycle; a literal char is verbatim
    let svg = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10" marker="square" marker-fill="#e11d48"><x:li>a</x:li><x:li marker="▸">b</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<rect") && out.contains("fill=\"#e11d48\""),
        "square shape marker: {out}"
    );
    assert!(
        out.contains(">\u{25B8}</tspan>"),
        "literal char marker: {out}"
    );
}

#[test]
fn list_item_font_size_override() {
    // an <x:li font-size> shrinks just that item; it inherits the list size otherwise
    let svg = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="20"><x:li>big</x:li><x:li font-size="10">small</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("font-size=\"20\""), "base <text> size: {out}");
    assert!(
        out.contains("font-size=\"10\">small</tspan>"),
        "the smaller item's line overrides the size: {out}"
    );
}

#[test]
fn theme_color_tokens_resolve_in_paint() {
    let svg = format!(
        r##"{XW}<x:theme><x:color name="accent" value="#6366f1"/></x:theme><rect x="0" y="0" width="10" height="10" fill="var(accent)"/><circle cx="5" cy="5" r="3" fill="var(missing, #ff0000)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("fill=\"#6366f1\""), "token resolved: {out}");
    assert!(
        out.contains("fill=\"#ff0000\""),
        "fallback used for a missing token: {out}"
    );
    assert!(
        !out.contains("x:theme") && !out.contains("x:color") && !out.contains("var("),
        "theme defs emit nothing and no var() leaks: {out}"
    );
}

#[test]
fn theme_font_token_is_an_overridable_base() {
    let svg = format!(
        r##"{XW}<x:theme><x:font name="title" font-family="Georgia" font-size="40" font-weight="700"/></x:theme><text x="0" y="20" x:font="title">Hi</text><text x="0" y="60" x:font="title" font-size="18">Small</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("font-family=\"Georgia\"") && out.contains("font-weight=\"700\""),
        "token supplies family/weight: {out}"
    );
    assert!(
        out.contains("font-size=\"40\""),
        "token size on the first text: {out}"
    );
    assert!(
        out.contains("font-size=\"18\""),
        "element font-size overrides the token: {out}"
    );
    assert!(!out.contains("x:font"), "x:font is stripped: {out}");
}

#[test]
fn textbox_first_line_indent() {
    // Mono: char = 0.5·size. At size 10 in a width-80 box, "alpha beta gamma"
    // fills the first line; a 30-unit first-line indent narrows it and offsets
    // it 30 in, so later lines sit back at the margin.
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="80" height="200" font-size="10" text-indent="30">alpha beta gamma delta</x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<tspan x=\"30\""),
        "first line indented: {out}"
    );
    assert!(
        out.contains("<tspan x=\"0\""),
        "later lines at the margin: {out}"
    );
}

#[test]
fn textbox_paragraphs_stack_with_spacing() {
    // Mono size 10: cap 7, descent 2, advance 12. Two 1-line paragraphs with a
    // 20-unit paragraph-spacing: p0 baseline at cap (7); p1 pushed down by
    // p0 height (9) + gap (20) + cap (7) = 36.
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="200" font-size="10" line-height="1.2" paragraph-spacing="20"><x:p>one</x:p><x:p align="center">two</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        out.matches("<text ").count(),
        2,
        "one <text> per paragraph: {out}"
    );
    assert!(out.contains("y=\"7\""), "first paragraph at the top: {out}");
    assert!(
        out.contains("y=\"36\""),
        "second paragraph pushed down by the gap: {out}"
    );
    assert!(
        out.contains("text-anchor=\"middle\""),
        "per-paragraph align: {out}"
    );
}

#[test]
fn table_columns_and_content_row_heights() {
    // width 300, cols "100 * *" → 100 fixed + two flex of (200/2)=100 each.
    // Mono char = 5 at size 10: the long cell wraps to 2 lines in a 100-wide
    // column, so its row grows taller than the single-line header row.
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="300" cols="100 * *" cell-padding="0" font-size="10" line-height="1.2" stroke="#111"><x:tr><x:th>A</x:th><x:th>B</x:th><x:th>C</x:th></x:tr><x:tr><x:td>short</x:td><x:td>a b c d e f g h i j k</x:td><x:td>x</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("width=\"100\""),
        "columns resolve to 100: {out}"
    );
    // header row = 1 line (cap 7 + descent 2 = 9); content row grew to 2 lines
    // (9 + advance 12 = 21)
    assert!(
        out.contains("height=\"9\""),
        "single-line header row: {out}"
    );
    assert!(
        out.contains("height=\"21\""),
        "content row grew to fit wrapping: {out}"
    );
    assert!(
        out.contains("font-weight=\"700\""),
        "th cells are bold: {out}"
    );
    assert!(
        out.contains("fill=\"#f1f5f9\""),
        "default header-fill: {out}"
    );
}

#[test]
fn table_cell_bg_override_and_empty_degrades() {
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="100"><x:tr><x:td bg="#abcdef">hi</x:td></x:tr></x:table></svg>"##
    );
    assert!(
        compile_test(&svg).contains("fill=\"#abcdef\""),
        "per-cell bg override"
    );
    let empty = format!(r##"{XW}<x:table x="0" y="0" width="100"></x:table></svg>"##);
    assert!(
        compile_test(&empty).contains("<!-- xsvg: <x:table"),
        "an empty table degrades with a marker"
    );
    // per-cell font-* actually applies
    let f = format!(
        r##"{XW}<x:table x="0" y="0" width="80" font-weight="400"><x:tr><x:td font-weight="600">hi</x:td></x:tr></x:table></svg>"##
    );
    assert!(
        compile_test(&f).contains("font-weight=\"600\""),
        "per-cell font-weight overrides the table"
    );
}

#[test]
fn table_source_map_is_per_cell() {
    // each <x:td>/<x:th> becomes its own source node (a <g data-xsvg-pos>),
    // so a viewer resolves a click to the cell, not the whole table
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="100"><x:tr><x:td>a</x:td><x:td>b</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_impl(&svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert_eq!(
        out.matches("<g data-xsvg-pos").count(),
        2,
        "one source-mapped <g> per cell: {out}"
    );
}

#[test]
fn table_stripe_alternates_body_rows() {
    // header + 3 body rows; the stripe lands on the 2nd body row only
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="60" stripe="#eeeeee"><x:tr><x:th>H</x:th></x:tr><x:tr><x:td>r0</x:td></x:tr><x:tr><x:td>r1</x:td></x:tr><x:tr><x:td>r2</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        out.matches("fill=\"#eeeeee\"").count(),
        1,
        "exactly one striped body row: {out}"
    );
}

#[test]
fn pie_geometry_and_per_slice() {
    // three slices → three sector <path>s, each its own source node
    let svg = format!(
        r##"{XW}<x:pie cx="100" cy="100" r="80"><x:slice value="1"/><x:slice value="1"/><x:slice value="1"/></x:pie></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        out.matches("<path fill=").count(),
        3,
        "one path per slice: {out}"
    );
    let sm = compile_impl(&svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert_eq!(
        sm.matches("<g data-xsvg-pos").count(),
        3,
        "per-slice source map: {sm}"
    );

    // per-slice radius: explicit `r` and a `grow` factor both set the arc radius
    let svg = format!(
        r##"{XW}<x:pie cx="0" cy="0" r="100"><x:slice value="1" r="50"/><x:slice value="1" grow="1.5"/></x:pie></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("A50,50"), "explicit slice radius: {out}");
    assert!(
        out.contains("A150,150"),
        "grow scales the pie radius: {out}"
    );

    // donut: inner-radius → annular sectors (two arcs per slice)
    let svg = format!(
        r##"{XW}<x:pie cx="0" cy="0" r="100" inner-radius="40"><x:slice value="1"/><x:slice value="1"/></x:pie></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        out.matches('A').count(),
        4,
        "two arcs per donut slice: {out}"
    );
    assert!(out.contains("40,40"), "inner-radius arc: {out}");

    // empty pie degrades with a marker
    assert!(
        compile_test(&format!(
            r##"{XW}<x:pie cx="0" cy="0" r="50"></x:pie></svg>"##
        ))
        .contains("<!-- xsvg: <x:pie"),
        "empty pie marker"
    );
}

#[test]
fn plot_maps_bars_and_lines() {
    // plot (0,0) 100×200, y-domain 0..100 → a value-50 bar is bottom-aligned
    // with its top at mapy(50)=100 and height 100
    let svg = format!(
        r##"{XW}<x:plot x="0" y="0" width="100" height="200" y-domain="0 100"><x:bars><x:bar value="50"/></x:bars></x:plot></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"100\"") && out.contains("height=\"100\""),
        "bar height maps from the domain: {out}"
    );

    // a line point (5,100) with x-domain 0..10, y-domain 0..100 maps to
    // (mid-width 50, top 0)
    let svg = format!(
        r##"{XW}<x:plot x="0" y="0" width="100" height="200" x-domain="0 10" y-domain="0 100"><x:line points="5,100 10,0"/></x:plot></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("points=\"50,0 100,200\""),
        "line points map: {out}"
    );

    // y-ticks draws N+1 gridlines
    let svg = format!(
        r##"{XW}<x:plot x="0" y="0" width="100" height="200" y-domain="0 100" y-ticks="4"><x:bars><x:bar value="50"/></x:bars></x:plot></svg>"##
    );
    assert_eq!(
        compile_test(&svg).matches("<line ").count(),
        5,
        "y-ticks=4 → 5 gridlines"
    );

    // grid-width, and a line's marker-fill overriding the dot color
    let svg = format!(
        r##"{XW}<x:plot x="0" y="0" width="100" height="200" y-domain="0 100" y-ticks="1" grid-width="2"><x:line points="0,50" stroke="#111" marker="dot" marker-fill="#f00"/></x:plot></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<line ") && out.contains("stroke-width=\"2\""),
        "grid-width: {out}"
    );
    assert!(
        out.contains("<circle") && out.contains("fill=\"#f00\""),
        "marker-fill: {out}"
    );
}

#[test]
fn list_number_outline_counters_restart_on_pop() {
    // decimal at level 0, lower-alpha at level 1; the sublist restarts and the
    // outer counter resumes when nesting pops back
    let svg = format!(
        r##"{XW}<x:list list="number" x="0" y="0" width="200" font-size="10"><x:li>a</x:li><x:li>b</x:li><x:li indent="1">c</x:li><x:li indent="1">d</x:li><x:li>e</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    for want in [
        ">1.</tspan>",
        ">2.</tspan>",
        ">a.</tspan>",
        ">b.</tspan>",
        ">3.</tspan>",
    ] {
        assert!(out.contains(want), "missing {want}: {out}");
    }
}

#[test]
fn list_marker_none_still_indents() {
    let svg = format!(
        r##"{XW}<x:list list="none" x="0" y="0" width="200" font-size="10"><x:li>solo</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("text-anchor=\"end\""), "no marker: {out}");
    assert!(
        out.contains("<tspan x=\"15\""),
        "still indented one step: {out}"
    );
}

#[test]
fn list_number_formats() {
    assert_eq!(alpha_lower(1), "a");
    assert_eq!(alpha_lower(26), "z");
    assert_eq!(alpha_lower(27), "aa");
    assert_eq!(roman_lower(1), "i");
    assert_eq!(roman_lower(4), "iv");
    assert_eq!(roman_lower(58), "lviii");
    assert_eq!(number_marker(0, 3), "3.");
    assert_eq!(number_marker(1, 2), "b.");
    assert_eq!(number_marker(2, 4), "iv.");
    assert_eq!(bullet_shape(0), "disc");
    assert_eq!(bullet_shape(1), "ring");
    assert_eq!(bullet_shape(2), "square");
    assert_eq!(bullet_shape(3), "disc"); // cycles every 3 levels
    assert_eq!(shape_token("circle"), Some("ring"));
    assert_eq!(shape_token("square"), Some("square"));
    assert_eq!(shape_token("\u{2605}"), None); // ★ → literal text marker
}

#[test]
fn artboards_carry_data_attributes_and_stay_plain_groups() {
    let svg = format!(
        r##"{XW}<g x:artboard="Slide 1" x:frame="0 0 720 405"><rect x="0" y="0" width="10" height="10" fill="#111"/></g><g x:artboard="Slide 2"><rect x="800" y="0" width="10" height="10" fill="#222"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("data-xsvg-artboard").count(), 2, "{out}");
    assert!(out.contains(r#"data-xsvg-artboard="Slide 1""#), "{out}");
    assert!(out.contains(r#"data-xsvg-frame="0 0 720 405""#), "{out}");
    // no explicit frame → no data-xsvg-frame (tools fall back to bbox)
    assert_eq!(out.matches("data-xsvg-frame").count(), 1, "{out}");
    // the x: metadata is stripped; artboards emit as plain <g>
    assert!(
        !out.contains("x:artboard") && !out.contains("x:frame"),
        "{out}"
    );
    // a malformed frame is dropped, the artboard survives
    let svg = format!(
        r##"{XW}<g x:artboard="A" x:frame="0 0 -5 10"><rect x="0" y="0" width="4" height="4"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r#"data-xsvg-artboard="A""#) && !out.contains("data-xsvg-frame"),
        "{out}"
    );
}

#[test]
fn layers_restack_by_band_and_order_and_strip_metadata() {
    // authored order: foreground, loose, background — compiled paint order
    // must be background → loose → foreground (each rect keeps its fill)
    let svg = format!(
        r##"{XW}<g x:layer="foreground" x:label="top"><rect x="0" y="0" width="9" height="9" fill="#ff0000"/></g><rect x="0" y="0" width="9" height="9" fill="#00cc00"/><g x:layer="background"><rect x="0" y="0" width="9" height="9" fill="#0000ff"/></g></svg>"##
    );
    let out = compile_test(&svg);
    let pos = |hex: &str| out.find(hex).unwrap();
    assert!(
        pos("#0000ff") < pos("#00cc00") && pos("#00cc00") < pos("#ff0000"),
        "expected bg → loose → fg: {out}"
    );
    // the x: metadata is stripped; the layers emit as plain <g>
    assert!(
        !out.contains("x:layer") && !out.contains("x:label"),
        "{out}"
    );
    assert!(out.contains("<g>"), "{out}");
}

#[test]
fn layer_order_breaks_ties_within_a_band() {
    // two backgrounds, authored 5 then 1: order 1 paints first
    let svg = format!(
        r##"{XW}<g x:layer="background" x:order="5"><rect x="0" y="0" width="9" height="9" fill="#aa0000"/></g><g x:layer="background" x:order="1"><rect x="0" y="0" width="9" height="9" fill="#00aa00"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.find("#00aa00").unwrap() < out.find("#aa0000").unwrap(),
        "{out}"
    );
    // x:order alone (no x:layer) also restacks — a plain z-index
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="9" height="9" fill="#111" x:order="3"/><rect x="0" y="0" width="9" height="9" fill="#222" x:order="1"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.find("#222").unwrap() < out.find("#111").unwrap(),
        "{out}"
    );
    assert!(!out.contains("x:order"), "{out}");
}

#[test]
fn hidden_layers_compile_to_nothing() {
    let svg = format!(
        r##"{XW}<g x:layer="foreground" x:hidden="true"><rect x="0" y="0" width="9" height="9" fill="#ff0000"/></g><rect x="0" y="0" width="9" height="9" fill="#00cc00"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !out.contains("#ff0000"),
        "hidden subtree must vanish: {out}"
    );
    assert!(out.contains("#00cc00"), "{out}");
    assert!(!out.contains("x:hidden"), "{out}");
    // x:hidden="false" is NOT hidden
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="9" height="9" fill="#ff0000" x:hidden="false"/></svg>"##
    );
    assert!(compile_test(&svg).contains("#ff0000"));
}

#[test]
fn rect_becomes_path() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect x="10" y="20" width="30" height="40" fill="#f00"/></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains("<path"));
    assert!(out.contains(r#"d="M10,20 h30 v40 h-30 Z""#));
    assert!(out.contains(r##"fill="#f00""##));
    assert!(!out.contains("<rect"));
}

#[test]
fn inline_size_wraps_into_tspans() {
    // 6 words, each 3 chars → 1.5 wide at size 1; force several lines at width 5.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="5" y="10" font-size="10" inline-size="40">one two six ten cat dog</text></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("<tspan"));
    // more than one line emitted
    assert!(out.matches("<tspan").count() >= 2, "expected wrap: {out}");
    assert!(out.contains("<text"));
}

#[test]
fn textbox_shrinks_to_fit() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="40" height="20" font-size="40" fit="shrink" fit-min="5" align="center" valign="middle">long label that must shrink</x:textbox></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains(r#"text-anchor="middle""#));
    // font-size must have been reduced from the authored 40
    let size = out
        .split("font-size=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap();
    assert!(size < 40.0 && size >= 5.0, "expected shrink, got {size}");
}

#[test]
fn malformed_errors() {
    assert!(compile_impl(
        "<svg><unclosed></svg>",
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &NoOutliner
    )
    .is_err());
}

#[test]
fn text_area_wraps_and_uses_text_align() {
    // SVG Tiny 1.2: text-align (not text-anchor); explicit width wraps.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="40" height="100" font-size="10" text-align="center">one two three four five</textArea></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("<text"));
    assert!(out.contains(r#"text-anchor="middle""#)); // text-align:center → anchor middle
    assert!(out.matches("<tspan").count() >= 2);
    assert!(!out.contains("<textArea"));
}

#[test]
fn text_area_auto_width_does_not_wrap() {
    // lacuna width = auto ⇒ single line, no wrapping
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="10" font-size="10">one two three four five</textArea></svg>"#;
    let out = compile_test(svg);
    assert_eq!(
        out.matches("<tspan").count(),
        1,
        "auto width must not wrap: {out}"
    );
}

#[test]
fn degenerate_text_does_not_panic() {
    // empty textArea → empty <text>, no tspans
    let a = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="50" height="50"></textArea></svg>"#,
    );
    assert!(a.contains("<text") && !a.contains("<tspan"));

    // textArea with no width/height (lacuna = auto) → single unwrapped line
    let b = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea>hello world</textArea></svg>"#,
    );
    assert_eq!(b.matches("<tspan").count(), 1);

    // inline-size 0 → one word per line
    let c = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="0">a b c</text></svg>"#,
    );
    assert_eq!(c.matches("<tspan").count(), 3);

    // font-size 0 → falls back to default, no NaN from a 0/0 scale
    let d = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="50" height="50" font-size="0" fit="shrink">hi there friend</x:textbox></svg>"#,
    );
    assert!(d.contains("<text") && !d.contains("NaN"));

    // non-finite numeric attribute → default
    let e = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="1e999" y="10" font-size="10" inline-size="40">x y z</text></svg>"#,
    );
    assert!(!e.contains("inf") && !e.contains("NaN"));
}

#[test]
fn text_overflow_ellipsis_emits_marker() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="80" height="30" font-size="12" text-overflow="ellipsis">this paragraph is far too tall to fit inside the short box provided here</textArea></svg>"#;
    assert!(compile_test(svg).contains('…'), "expected ellipsis marker");
    // default (clip) emits no marker
    let clip = svg.replace(" text-overflow=\"ellipsis\"", "");
    assert!(!compile_test(&clip).contains('…'));
}

#[test]
fn tbreak_forces_lines() {
    // auto width would be a single line; each <tbreak/> forces a new one.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" font-size="10">one<tbreak/>two<tbreak/>three</textArea></svg>"#;
    let out = compile_test(svg);
    assert_eq!(out.matches("<tspan").count(), 3, "tbreak lines: {out}");
}

#[test]
fn glyph_x_scale_emits_text_length() {
    // x:glyph-x-scale on a plain textArea → textLength/lengthAdjust per line.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="10" font-size="10" x:glyph-x-scale="1.5">hello</textArea></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("lengthAdjust=\"spacingAndGlyphs\""), "{out}");
    // "hello" = 5 chars × 0.5 × 10 = 25; scaled ×1.5 = 37.5
    assert!(out.contains("textLength=\"37.5\""), "{out}");

    // no scale attribute → no textLength emitted
    let plain = compile_test(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="10" font-size="10">hello</textArea></svg>"#,
    );
    assert!(!plain.contains("textLength"));
}

#[test]
fn glyph_x_scale_on_textbox_and_inline_size() {
    // <x:textbox> takes it unprefixed; "hi" = 2 × 0.5 × 10 = 10, ×1.5 = 15.
    let box_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" glyph-x-scale="1.5">hi</x:textbox></svg>"#;
    assert!(compile_test(box_svg).contains("textLength=\"15\""));

    // <text inline-size> takes it x:-prefixed (reused SVG element).
    let flow_svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><text x="0" y="10" font-size="10" inline-size="500" x:glyph-x-scale="1.5">hi</text></svg>"#;
    assert!(compile_test(flow_svg).contains("textLength=\"15\""));
}

// ---- degradation / passthrough contract ----

#[test]
fn rounded_rect_passes_through() {
    // rx/ry present → not lowered to a path; emitted as <rect> unchanged.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="1" y="2" width="3" height="4" rx="2"/></svg>"#;
    let out = compile_test(svg);
    assert!(
        out.contains("<rect"),
        "rounded rect should pass through: {out}"
    );
    assert!(out.contains("rx=\"2\""));
    assert!(!out.contains("<path"));
}

#[test]
fn rounded_rect_is_an_operand_not_skipped() {
    // Regression: a rounded <rect> inside <x:boolean>/<x:warp> used to fall
    // through to a <rect> the bake can't take and get dropped behind a skip
    // marker. shape_to_path_d lowers its rx corners to an arc path, so it now
    // participates like any other operand.
    let svg = format!(
        r##"{XW}<x:boolean op="union" fill="#111"><rect x="0" y="0" width="60" height="40" rx="10"/><circle cx="70" cy="20" r="20"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !out.contains("skipped"),
        "rounded rect operand must not be skipped: {out}"
    );
    assert!(
        out.contains("<path") && out.contains("fill=\"#111\""),
        "boolean of a rounded rect + circle bakes to a filled path: {out}"
    );

    // and the same rect warps rather than being skipped
    let svg = format!(
        r##"{XW}<x:warp field="arc" bend="20" fill="#222"><rect x="0" y="0" width="80" height="30" rx="8"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !out.contains("skipped") && out.contains("<path"),
        "rounded rect warps to a path: {out}"
    );
}

#[test]
fn xsvg_root_aliases_to_svg() {
    let svg = r#"<xsvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><g/></xsvg>"#;
    let out = compile_test(svg);
    assert!(
        out.contains("<svg"),
        "root <xsvg> should become <svg>: {out}"
    );
    assert!(!out.contains("<xsvg"));
    assert!(out.contains(&format!("xmlns=\"{SVG_NS}\"")));
}

#[test]
fn unknown_extension_degrades_to_marker() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:sparkle/></svg>"#;
    let out = compile_test(svg);
    assert!(
        out.contains("<!-- xsvg: <x:sparkle> not yet lowered -->"),
        "{out}"
    );
    // an empty mesh is KNOWN but has nothing to lower — its own marker
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:mesh/></svg>"#;
    let out = compile_test(svg);
    assert!(
        out.contains("<!-- xsvg: <x:mesh> no usable faces -->"),
        "{out}"
    );
}

#[test]
fn xsvg_attributes_are_stripped() {
    // an x:-namespaced attribute on a passed-through element is dropped.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><circle cx="5" cy="5" r="3" x:note="hint"/></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("<circle"));
    assert!(out.contains("r=\"3\""));
    assert!(!out.contains("note"), "x: attr should be stripped: {out}");
}

#[test]
fn letter_spacing_affects_layout_and_is_emitted() {
    // Mono: char = 0.5 × size. At size 10, "aa bb" = 10 + 5 + 10 = 25 wide (4 gaps).
    let base = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10">aa bb</textArea></svg>"#;
    assert_eq!(compile_test(base).matches("<tspan").count(), 1);

    // letter-spacing=3 adds 4·3 = 12 → 37 > 25, so it must wrap to two lines,
    // and the attribute is written onto the synthesized <text>.
    let spaced = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10" letter-spacing="3">aa bb</textArea></svg>"#;
    let out = compile_test(spaced);
    assert_eq!(out.matches("<tspan").count(), 2, "{out}");
    assert!(out.contains("letter-spacing=\"3\""), "{out}");

    // On <text inline-size> the attribute is forwarded verbatim (passthrough).
    let flow = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="500" letter-spacing="2">hi there</text></svg>"#;
    assert!(compile_test(flow).contains("letter-spacing=\"2\""));
}

#[test]
fn word_spacing_affects_layout_and_is_emitted() {
    // Mono: char = 0.5 × size. At size 10, "aa bb" = 10 + 5 + 10 = 25 (one gap).
    let base = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10">aa bb</textArea></svg>"#;
    assert_eq!(compile_test(base).matches("<tspan").count(), 1);

    // word-spacing=6 widens the inter-word gap: 25 + 6 = 31 > 25 → wraps, and
    // the attribute lands on the synthesized <text>.
    let spaced = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="25" font-size="10" word-spacing="6">aa bb</textArea></svg>"#;
    let out = compile_test(spaced);
    assert_eq!(out.matches("<tspan").count(), 2, "{out}");
    assert!(out.contains("word-spacing=\"6\""), "{out}");

    // Forwarded on <text inline-size> too.
    let flow = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" font-size="10" inline-size="500" word-spacing="4">hi there</text></svg>"#;
    assert!(compile_test(flow).contains("word-spacing=\"4\""));
}

#[test]
fn justify_emits_textlength_spacing_on_full_lines() {
    // Mono: char = 0.5 × size. Wraps to several lines at width 40, size 10.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="40" font-size="10" text-align="justify">aa bb cc dd ee ff</textArea></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains(r#"text-anchor="start""#), "{out}");
    assert!(
        out.contains(r#"textLength="40" lengthAdjust="spacing""#),
        "expected justified full lines: {out}"
    );
    // start alignment (no justify) emits no textLength
    let plain = svg.replace(" text-align=\"justify\"", "");
    assert!(!compile_test(&plain).contains("textLength"));
}

#[test]
fn justify_on_textbox_uses_content_width() {
    // width 60, padding 10 → content width 40 is the justify target.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="60" height="200" padding="10" align="justify" font-size="10">aa bb cc dd ee ff</x:textbox></svg>"#;
    let out = compile_test(svg);
    assert!(
        out.contains(r#"textLength="40" lengthAdjust="spacing""#),
        "{out}"
    );
}

#[test]
fn textbox_in_rect_binds_to_referenced_geometry() {
    // The textbox has no geometry of its own; it takes the rect's box.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><rect id="card" x="10" y="10" width="200" height="80"/><x:textbox in="#card" align="center" valign="middle">label</x:textbox></svg>"##;
    let out = compile_test(svg); // rect fast path — no shaper needed
    assert!(out.contains("<text") && !out.contains("<x:textbox"));
    assert!(out.contains(r#"text-anchor="middle""#));
    // centered in content box: x=10, width 200 → anchor x = 10 + 100 = 110
    assert!(out.contains(r#"x="110""#), "{out}");
}

#[test]
fn textbox_in_shape_flows_region() {
    // Non-rect target → region flow via the (test) shaper; emits <text>/<tspan>.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><circle id="blob" cx="30" cy="30" r="30"/><x:textbox in="#blob" align="start" font-size="10">one two three four five six seven</x:textbox></svg>"##;
    let out = compile_shaped(svg);
    assert!(out.contains("<text") && !out.contains("<x:textbox"));
    assert!(out.contains(r#"text-anchor="start""#));
    assert!(
        out.matches("<tspan").count() >= 2,
        "expected flowed lines: {out}"
    );
}

#[test]
fn textbox_in_missing_target_degrades_to_marker() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox in="#ghost">hi</x:textbox></svg>"##;
    let out = compile_shaped(svg);
    assert!(out.contains("target not found"), "{out}");
    assert!(!out.contains("<text"));
}

#[test]
fn deep_nesting_errors_instead_of_aborting() {
    // Far past the stack-overflow threshold; the depth guard must turn this into
    // a clean Err rather than a hard abort (see check_nesting_depth).
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
        "<g>".repeat(5000),
        "</g>".repeat(5000)
    );
    let err = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap_err();
    assert!(err.contains("nesting depth"), "{err}");
}

#[test]
fn many_siblings_and_self_closing_are_not_rejected() {
    // Depth guard must count nesting, not element count: thousands of siblings
    // (net depth 1) stay well under the limit.
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
        "<rect/>".repeat(3000)
    );
    assert!(compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
    // and modest legitimate nesting is fine
    let nested = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\">{}{}</svg>",
        "<g>".repeat(64),
        "</g>".repeat(64)
    );
    assert!(compile_impl(&nested, "balanced", false, &Mono, &NoShaper, &NoOutliner).is_ok());
}

#[test]
fn glyph_x_scale_non_positive_is_ignored() {
    // 0 and negative scales must not emit a zero/negative textLength or NaN.
    for v in ["0", "-1.5"] {
        let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:x=\"https://xsvg.visioncortex.org\"><textArea x=\"0\" y=\"10\" font-size=\"10\" x:glyph-x-scale=\"{v}\">hello</textArea></svg>"
            );
        let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
        assert!(out.contains("<text") && !out.contains("<textArea"));
        assert!(
            !out.contains("textLength"),
            "scale {v} still emitted: {out}"
        );
        assert!(!out.contains("NaN"));
    }
}

#[test]
fn tbreak_only_textarea_does_not_panic() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="50" font-size="10"><tbreak/></textArea></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("<text") && !out.contains("NaN"));
}

#[test]
fn negative_geometry_does_not_panic_or_leak_nan() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="-40" height="-20" padding="-5" fit="shrink" align="center" valign="middle" font-size="10">hi there friend</x:textbox></svg>"#;
    assert!(!compile_test(svg).contains("NaN"));
}

#[test]
fn styled_tspan_runs_emit_nested_tspans() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10" fill="#111">Ship <tspan font-weight="bold" fill="#e11">fast</tspan> today</textArea></svg>"##;
    let out = compile_test(svg);
    // base attrs live on the <text>; the bold run is a nested <tspan> with overrides
    assert!(out.contains(r##"fill="#111""##), "base fill missing: {out}");
    assert_eq!(out.matches("font-weight=\"bold\"").count(), 1, "{out}");
    assert!(out.contains(r##"fill="#e11""##), "run fill missing: {out}");
    assert!(out.contains("fast"), "{out}");
    // base-styled parts stay bare text (not wrapped)
    assert!(out.contains("Ship ") && out.contains("today"));
}

#[test]
fn plain_textarea_has_no_inner_tspans() {
    // no runs → one outer <tspan> per line, nothing nested
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="500" font-size="10">hello world</textArea></svg>"#;
    assert_eq!(compile_test(svg).matches("<tspan").count(), 1);
}

#[test]
fn outline_emits_paths_not_text() {
    // outline="true" with a working outliner → <g><path>, no <text>
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="#111">Hi</x:textbox></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<g fill=\"#111\""), "{out}");
    assert!(out.contains("<path d=\""), "{out}");
    assert!(!out.contains("<text") && !out.contains("<x:textbox"));
}

#[test]
fn textarea_x_outline_emits_paths() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="200" font-size="10" x:outline="true">one two</textArea></svg>"##;
    let out = compile_outlined(svg);
    assert!(
        out.contains("<path d=\"") && !out.contains("<text"),
        "{out}"
    );
}

#[test]
fn outline_carries_stroke_onto_the_group() {
    // stroke/stroke-width on an outlined box propagate to the outline <g> (a
    // keyline outline), and fill="none" is honored — the live <text> branch is
    // unaffected (it has no stroke path).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" fill="none" stroke="#fff" stroke-width="1.5" stroke-linejoin="round">Hi</x:textbox></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<g fill=\"none\""), "{out}");
    assert!(out.contains("stroke=\"#fff\""), "{out}");
    assert!(out.contains("stroke-width=\"1.5\""), "{out}");
    assert!(out.contains("stroke-linejoin=\"round\""), "{out}");
    assert!(out.contains("<path d=\""), "{out}");
}

#[test]
fn outline_falls_back_to_text_without_a_font() {
    // outline requested but the outliner has no font (returns None) → live <text>
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true">Hi</x:textbox></svg>"##;
    let out = compile_test(svg); // NoOutliner
    assert!(
        out.contains("<text") && !out.contains("<path d=\""),
        "{out}"
    );
}

#[test]
fn textpath_emits_warped_path() {
    // <x:textpath in="#p"> with an outline font → <g><path>, no <text>; the
    // native skew field lands the run's baseline on the path (f(0) = 20)
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 C40,0 80,40 120,20" fill="none"/><x:textpath in="#p" effect="skew" font-size="20" fill="#111">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<g fill=\"#111\""), "{out}");
    assert!(out.contains("<path d=\"M0,20"), "{out}");
    assert!(!out.contains("<text"), "{out}");
}

#[test]
fn textpath_carries_stroke() {
    // fill="none" + stroke → a keyline outline on the warped path
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="none" stroke="#0af" stroke-width="1.5">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<g fill=\"none\""), "{out}");
    assert!(out.contains("stroke=\"#0af\""), "{out}");
}

#[test]
fn textpath_without_a_font_degrades_to_stepped_text() {
    // No outline font, but the native height profile still works → the skew run
    // degrades to stepped live <text> (§6.13.3), not a flat line. Mono at size
    // 20: "wave" prefix advances 0/10/20/30; the flat path pins y at 20.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20">wave</x:textpath></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(">wave</text>"), "{out}");
    assert!(out.contains(r#"x="0 10 20 30""#), "{out}");
    assert!(out.contains(r#"y="20 20 20 20""#), "{out}");
}

#[test]
fn textpath_missing_target_is_a_comment() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textpath in="#nope" font-size="20">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(
        out.contains("<!-- xsvg: <x:textpath in> target not found"),
        "{out}"
    );
}

#[test]
fn textpath_baseline_shift_lifts_the_run() {
    // rainbow + baseline-shift 8 on a flat path at y = 20 → baseline at y = 12
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="8" font-size="20" fill="#111">arc</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<path d=\"M0,12"), "{out}");
}

#[test]
fn textpath_defaults_to_skew_on_the_path() {
    // no effect / baseline-shift attributes → skew at the path's start
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" font-size="20" fill="#111">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<path d=\"M0,20"), "{out}");
}

#[test]
fn textpath_align_and_start_place_the_run() {
    // extent 120, advance("wave") = 40 → middle slack 40, +12 head-start → x = 52
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" align="middle" start="12" font-size="20" fill="#111">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<path d=\"M52,20"), "{out}");
}

#[test]
fn textpath_skew_degrades_to_stepped_baseline() {
    // No outline font → the native height profile places each glyph (Stair Step,
    // §6.13.1). Mono(0.5·size): "abc" prefix advances 0/5/10; start=10 offsets;
    // the flat path pins y at 50, baseline-shift 2 lifts to 48.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" baseline-shift="2" start="10" font-size="10" fill="#111">abc</x:textpath></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(r#"x="10 15 20""#), "{out}");
    assert!(out.contains(r#"y="48 48 48""#), "{out}");
    assert!(out.contains(">abc</text>"), "{out}");

    // letter-spacing widens the per-glyph gaps in the baked positions (§6.8)
    let ls = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" letter-spacing="2" font-size="10" fill="#111">abc</x:textpath></svg>"##;
    let out = compile_test(ls);
    assert!(out.contains(r#"x="0 7 14""#), "{out}");
}

#[test]
fn textpath_stair_is_an_authored_effect() {
    // effect="stair" chooses the stepped live-<text> lowering even when an
    // outline font IS available — it never consults the outliner (§6.13.3).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="stair" font-size="10" fill="#111">abc</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains(r#"x="0 5 10""#), "{out}");
    assert!(out.contains(r#"y="50 50 50""#), "{out}");
    assert!(!out.contains("<path d=\""), "outliner ran: {out}");

    // a degenerate (zero-length) target path → straight <text>, never a panic
    let degen = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="stair" font-size="10" fill="#111">abc</x:textpath></svg>"##;
    let out = compile_outlined(degen);
    assert!(out.contains(r#"x="0" y="0""#), "{out}");
}

#[test]
fn textpath_rainbow_without_font_stays_straight() {
    // stepped degradation is skew/stair-only; rainbow with no outliner → straight
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 C40,20 80,80 120,50" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="10" fill="#111">abc</x:textpath></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(r#"x="0" y="0""#), "{out}");
    assert!(!out.contains("<path d=\""), "{out}");
}

#[test]
fn textpath_stepped_skips_empty_and_whitespace_text() {
    // nothing to place → the straight fallback, never a panic or an empty x list
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,50 L120,50" fill="none"/><x:textpath in="#p" effect="skew" font-size="10">   </x:textpath></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(r#"x="0" y="0""#), "{out}");
}

#[test]
fn textpath_degenerate_input_never_panics_or_leaks_nan() {
    // Degenerate baseline-shift values (garbage, ±inf, NaN) collapse to 0 —
    // the output matches an unshifted run and never leaks NaN (§4 totality).
    let plain = compile_outlined(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20">wave</x:textpath></svg>"##,
    );
    for bad in ["garbage", "1e999", "-1e999", "NaN", "inf"] {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="{bad}" font-size="20">wave</x:textpath></svg>"##
        );
        let out = compile_outlined(&svg);
        assert_eq!(out, plain, "shift={bad}");
        assert!(!out.contains("NaN"), "shift={bad}: {out}");
    }
    // A unit suffix parses its numeric prefix (13px → 13), like every length attr.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" baseline-shift="13px" font-size="20">wave</x:textpath></svg>"##;
    assert!(compile_outlined(svg).contains("<path d=\"M0,7"));
}

#[test]
fn textpath_empty_and_degenerate_targets_do_not_panic() {
    // empty run → still total (the stub warps ""; a real backend may fall back)
    let empty = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20"></x:textpath></svg>"##;
    compile_outlined(empty);
    // zero-length path data still reaches the seam verbatim (degeneracy is the
    // backend's to detect — it returns None and the element falls back)
    let point = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="rainbow" font-size="20">dot</x:textpath></svg>"##;
    compile_outlined(point);
    // a <line> target is a valid reference path per §6.13 (was a gap until
    // shape_to_path_d grew a line arm): text warps onto it, no marker
    let line = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><line id="p" x1="0" y1="20" x2="120" y2="20"/><x:textpath in="#p" effect="rainbow" font-size="20">hi</x:textpath></svg>"##;
    let out = compile_outlined(line);
    assert!(!out.contains("not found or not a path"), "{out}");
    assert!(out.contains("<path"), "{out}");
}

// ---- incremental compilation (docs/Incremental.md) ----

/// A document exercising every emitter family: passthrough, textbox, a
/// referenced path + its textpath, a warp, and a boolean.
const INC: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 400 300">
  <rect x="5" y="5" width="40" height="20" fill="#eee"/>
  <x:textbox x="0" y="0" width="80" height="30" font-size="10" fill="#111">hello world</x:textbox>
  <path id="wave" d="M0,100 C40,60 80,140 120,100" fill="none"/>
  <x:textpath in="#wave" font-size="12" fill="#222">on the wave</x:textpath>
  <x:warp field="arch" bend="40"><rect x="0" y="200" width="100" height="30" fill="#333"/></x:warp>
  <x:boolean op="subtract" fill="#444"><rect x="200" y="200" width="60" height="30"/><rect x="230" y="200" width="60" height="30"/></x:boolean>
  <x:boolean id="blob" op="union" fill="#555"><rect x="300" y="240" width="40" height="50"/><rect x="320" y="240" width="40" height="50"/></x:boolean>
  <x:textbox in="#blob" font-size="10" fill="#666">into the blob</x:textbox>
  <x:boolean op="intersect" fill="#777"><use href="#blob"/><rect x="310" y="250" width="40" height="30"/></x:boolean>
  <rect x="5" y="270" width="30" height="20" fill="#48a" filter="brightness(1.2) saturate(0.8)"/>
  <x:mesh points="340,240 380,240 380,280 340,280"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #ff0"/></x:mesh>
  <meshgradient id="mg" x="200" y="240"><meshrow><meshpatch><stop path="l 40,0" stop-color="#e11"/><stop path="l 0,30" stop-color="#fa0"/><stop path="l -40,0" stop-color="#3b7"/><stop path="l 0,-30" stop-color="#06c"/></meshpatch></meshrow></meshgradient>
  <rect x="200" y="240" width="40" height="30" fill="url(#mg)"/>
</svg>"##;

/// Byte offsets of every top-level element in `INC`, via the parser itself.
fn top_level_offsets(input: &str) -> Vec<usize> {
    let doc = roxmltree::Document::parse(input).unwrap();
    doc.root_element()
        .children()
        .filter(|c| c.is_element())
        .map(|c| c.range().start)
        .collect()
}

#[test]
fn fragments_are_verbatim_slices_of_the_full_compile() {
    // THE incremental invariant: emission is a pure function of the subtree
    // (plus its references), so each fragment must appear byte-identically —
    // and in order — inside the full compile. If a future emitter introduces
    // cross-sibling state, this test is the canary.
    for sourcemap in [false, true] {
        let full =
            compile_impl(INC, "balanced", sourcemap, &Mono, &BoxShaper, &BoxOutliner).unwrap();
        let mut cursor = 0;
        for off in top_level_offsets(INC) {
            let frag = compile_fragment_impl(
                INC,
                "balanced",
                sourcemap,
                off,
                &Mono,
                &BoxShaper,
                &BoxOutliner,
            )
            .unwrap();
            assert!(!frag.is_empty());
            let at = full[cursor..]
                .find(&frag)
                .unwrap_or_else(|| panic!("fragment not found in order: {frag}\n{full}"));
            cursor += at + frag.len();
        }
    }
}

#[test]
fn fragment_range_and_offsets_resolve() {
    let offs = top_level_offsets(INC);
    for &off in &offs {
        let (s, e) = fragment_range_impl(INC, off).unwrap();
        assert_eq!(s, off);
        assert!(e > s);
        // any offset inside the element resolves to the same fragment
        assert_eq!(fragment_range_impl(INC, off + 1), Some((s, e)));
    }
    // offset in inter-element whitespace or the root tag → no fragment
    assert_eq!(fragment_range_impl(INC, 0), None);
    assert!(
        compile_fragment_impl(INC, "balanced", false, 0, &Mono, &NoShaper, &NoOutliner).is_err()
    );
}

#[test]
fn dependents_of_nothing_is_nothing() {
    assert!(dependents_impl("<svg", 2).is_empty()); // malformed input
    assert!(dependents_impl(INC, 0).is_empty()); // offset in the root tag
}

#[test]
fn dependents_track_baked_in_references() {
    let offs = top_level_offsets(INC);
    // editing the referenced #wave path invalidates the textpath that bakes it
    let wave_off = offs[2];
    let deps = dependents_impl(INC, wave_off);
    assert_eq!(deps.len(), 1, "{deps:?}");
    assert_eq!(deps[0].0, offs[3], "expected the textpath: {deps:?}");
    // an unreferenced element invalidates nothing
    assert!(dependents_impl(INC, offs[0]).is_empty());
    // the dependent itself has no dependents
    assert!(dependents_impl(INC, offs[3]).is_empty());
    // an x: element can be a target: editing the #blob boolean invalidates
    // the textbox that flows inside its compiled output AND the boolean
    // holding #blob as a <use> operand (a baked href, unlike passthrough <use>)
    let deps = dependents_impl(INC, offs[6]);
    assert_eq!(deps.len(), 2, "{deps:?}");
    assert_eq!(deps[0].0, offs[7], "expected the blob textbox: {deps:?}");
    assert_eq!(
        deps[1].0, offs[8],
        "expected the use-operand boolean: {deps:?}"
    );
    // a meshgradient fill is baked: editing the gradient re-emits the shape
    let deps = dependents_impl(INC, offs[11]);
    assert_eq!(deps.len(), 1, "{deps:?}");
    assert_eq!(
        deps[0].0, offs[12],
        "expected the mesh-filled rect: {deps:?}"
    );
}

#[test]
fn dependents_closure_is_transitive() {
    // #spine ← warp#bent(in=#spine) ← textpath(in=#bent): editing the spine
    // must re-emit BOTH downstream elements, since the warp's compiled output
    // is itself baked geometry for the textpath.
    let svg = format!(
        r##"{XW}<path id="spine" d="M0,80 C40,20 60,20 100,80" fill="none"/><x:warp id="bent" field="bend" in="#spine" bend="30"><rect x="0" y="40" width="100" height="20" fill="#333"/></x:warp><x:textpath in="#bent" effect="stair" font-size="10" fill="#111">chain</x:textpath></svg>"##
    );
    let offs = top_level_offsets(&svg);
    let starts = |deps: Vec<(usize, usize)>| deps.into_iter().map(|d| d.0).collect::<Vec<_>>();
    assert_eq!(
        starts(dependents_impl(&svg, offs[0])),
        vec![offs[1], offs[2]]
    );
    assert_eq!(starts(dependents_impl(&svg, offs[1])), vec![offs[2]]);
    assert!(dependents_impl(&svg, offs[2]).is_empty());
}

#[test]
fn textbox_flows_inside_a_boolean_union() {
    // the union of two overlapping rects is ONE region; the textbox flows
    // inside the boolean's compiled output, not any single source shape
    let svg = format!(
        r##"{XW}<x:boolean id="blob" op="union" fill="#eee"><rect x="0" y="0" width="40" height="60"/><rect x="20" y="0" width="40" height="60"/></x:boolean><x:textbox in="#blob" font-size="10" fill="#111">alpha beta gamma</x:textbox></svg>"##
    );
    let out = compile_shaped(&svg);
    assert!(!out.contains("not rasterizable"), "{out}");
    assert!(!out.contains("target not found"), "{out}");
    assert!(out.contains("alpha"), "{out}");
}

#[test]
fn textpath_rides_a_warped_spine_not_its_source() {
    // in= on an x:warp must sample the WARPED spine: the stair steps differ
    // from the same textpath bound to the raw source path
    let spine = r##"<path d="M0,80 C40,20 60,20 100,80" fill="none"/>"##;
    let text = r##"effect="stair" font-size="10" fill="#111">abcdef</x:textpath>"##;
    let warped = format!(
        r##"{XW}<x:warp id="s" field="arch" bend="60">{spine}</x:warp><x:textpath in="#s" {text}</svg>"##
    );
    let raw = format!(
        r##"{XW}<path id="s" d="M0,80 C40,20 60,20 100,80" fill="none"/><x:textpath in="#s" {text}</svg>"##
    );
    let w = compile_test(&warped);
    let r = compile_test(&raw);
    assert!(!w.contains("target not found"), "{w}");
    let ys = |out: &str| {
        let text = &out[out.find("<text").unwrap()..];
        let y = &text[text.find(" y=\"").unwrap() + 4..];
        y[..y.find('"').unwrap()].to_string()
    };
    assert_ne!(ys(&w), ys(&r), "stair must step along the warped spine");
}

#[test]
fn reference_cycles_degrade_instead_of_hanging() {
    // mutual references: both degrade with the standard marker, no recursion
    let svg = format!(
        r##"{XW}<x:textpath id="a" in="#b" effect="stair" font-size="10">one</x:textpath><x:textpath id="b" in="#a" effect="stair" font-size="10">two</x:textpath></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("target not found or not a path"), "{out}");
    // self-reference from inside: the cyclic contribution drops out and the
    // textbox flows in the rest of the boolean (the rect)
    let svg = format!(
        r##"{XW}<x:boolean id="a" op="union" fill="#eee"><rect x="0" y="0" width="60" height="60"/><x:textbox in="#a" font-size="10">loop</x:textbox></x:boolean></svg>"##
    );
    let out = compile_shaped(&svg);
    assert!(out.contains("<path"), "{out}");
}

#[test]
fn boolean_operands_include_their_stroke_ink() {
    // a stroked rect operand: the union covers fill + expanded stroke,
    // so the bbox inflates by half the stroke width on every side
    let svg = format!(
        r##"{XW}<x:boolean op="union" fill="#123"><rect x="10" y="10" width="40" height="20" stroke="#000" stroke-width="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    assert!(
        (bb.x0 - 5.0).abs() < 0.5 && (bb.x1 - 55.0).abs() < 0.5,
        "{bb:?}"
    );
    assert!(
        (bb.y0 - 5.0).abs() < 0.5 && (bb.y1 - 35.0).abs() < 0.5,
        "{bb:?}"
    );
    // evenodd operands resolve fill ∪ stroke instead of skipping (see
    // evenodd_operands_resolve_their_stroke_via_union)
    let svg = format!(
        r##"{XW}<x:boolean op="union" fill="#123"><rect x="10" y="10" width="40" height="20" fill-rule="evenodd" stroke="#000" stroke-width="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<!-- xsvg:"), "{out}");
    let bb2 = first_path(&out).bounding_box();
    assert!(
        (bb2.x0 - 5.0).abs() < 0.5 && (bb2.x1 - 55.0).abs() < 0.5,
        "{bb2:?}"
    );
}

#[test]
fn dashed_strokes_expand_as_dashes() {
    // a dashed thick stroke covers roughly half the solid ring's area
    let mk = |dash: &str| {
        format!(
            r##"{XW}<x:boolean op="union" fill="#123"><path d="M10,20 L110,20" fill="none" stroke="#000" stroke-width="8"{dash}/></x:boolean></svg>"##
        )
    };
    use crate::kurbo::Shape;
    let solid = first_path(&compile_test(&mk(""))).area().abs();
    let dashed = first_path(&compile_test(&mk(r#" stroke-dasharray="10 10""#)))
        .area()
        .abs();
    assert!((solid - 800.0).abs() < 20.0, "solid ~100x8: {solid}");
    assert!(
        dashed > 0.3 * solid && dashed < 0.7 * solid,
        "dashes remove about half the ink: {dashed} vs {solid}"
    );
}

#[test]
fn evenodd_operands_resolve_their_stroke_via_union() {
    // an evenodd ring (two same-winding squares) with a stroke: the
    // operand becomes fill-region ∪ stroke ink — no marker, and the
    // stroke inflates the outer bound
    let svg = format!(
        r##"{XW}<x:boolean op="union" fill="#123"><path d="M10,10 h40 v40 h-40 Z M20,20 h20 v20 h-20 Z" fill-rule="evenodd" stroke="#000" stroke-width="4"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("evenodd operand ignored"), "{out}");
    use crate::kurbo::Shape;
    let p = first_path(&out);
    let bb = p.bounding_box();
    assert!(
        (bb.x0 - 8.0).abs() < 0.5 && (bb.x1 - 52.0).abs() < 0.5,
        "{bb:?}"
    );
    // ring fill (1600-400) + stroke ink area; must exceed the bare ring
    assert!(p.area().abs() > 1250.0, "{}", p.area());
}

#[test]
fn boolean_use_children_are_operands_by_reference() {
    // venn lens: two circles referenced by <use> — both keep rendering,
    // and the intersect gets their geometry without consuming them
    let svg = format!(
        r##"{XW}<circle id="va" cx="100" cy="100" r="60" fill="#fecaca"/><circle id="vb" cx="150" cy="100" r="60" fill="#bbf7d0"/><x:boolean op="intersect" fill="#818cf8"><use href="#va"/><use href="#vb"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<circle").count(), 2, "{out}");
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    // lens spans [90,160] × [100 ± √(60²−25²) ≈ 54.5]
    assert!(
        (bb.x0 - 90.0).abs() < 1.0 && (bb.x1 - 160.0).abs() < 1.0,
        "{bb:?}"
    );
    assert!(
        (bb.y0 - 45.5).abs() < 1.0 && (bb.y1 - 154.5).abs() < 1.0,
        "{bb:?}"
    );
}

#[test]
fn boolean_use_mixes_with_literals_and_takes_xy_offsets() {
    // literal subject minus a referenced cutter stamped at a <use x y> offset
    let svg = format!(
        r##"{XW}<rect id="cut" x="0" y="0" width="40" height="40"/><x:boolean op="subtract" fill="#123"><rect x="0" y="100" width="100" height="40"/><use href="#cut" x="60" y="100"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    use crate::kurbo::Shape;
    let p = first_path(&out);
    let bb = p.bounding_box();
    assert!(
        (bb.x1 - 60.0).abs() < 0.5,
        "cutter offset not applied: {bb:?}"
    );
    assert!((p.area().abs() - 2400.0).abs() < 5.0, "{}", p.area());
}

#[test]
fn boolean_use_can_reference_a_compiled_x_target() {
    // operand = another boolean's compiled output (union spans x 0..100)
    let svg = format!(
        r##"{XW}<x:boolean id="u" op="union" fill="#eee"><rect x="0" y="0" width="60" height="40"/><rect x="40" y="0" width="60" height="40"/></x:boolean><x:boolean op="intersect" fill="#345"><use href="#u"/><rect x="80" y="0" width="80" height="40"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    use crate::kurbo::Shape;
    let p = first_path(&out);
    let bb = p.bounding_box();
    assert!(
        (bb.x0 - 80.0).abs() < 0.5 && (bb.x1 - 100.0).abs() < 0.5,
        "{bb:?}"
    );
    assert!((p.area().abs() - 800.0).abs() < 5.0, "{}", p.area());
}

#[test]
fn boolean_use_missing_target_and_self_reference_degrade() {
    let svg = format!(
        r##"{XW}<x:boolean op="union" fill="#123"><use href="#ghost"/><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("skipped <use>"), "{out}");
    assert!(out.contains("<path"), "{out}"); // the surviving operand still emits
    let svg = format!(
        r##"{XW}<x:boolean id="s" op="union" fill="#123"><use href="#s"/><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg); // terminates; cyclic operand drops out
    assert!(out.contains("<path"), "{out}");
}

#[test]
fn grim_corpus_is_total_and_never_silent() {
    // §3/§4 as a property, swept across every emitter: a degenerate
    // document must COMPILE (no panic), the output must never be empty,
    // must never leak NaN/inf coordinates, and an x: element that cannot
    // do its job must leave a marker — degradation is always acknowledged,
    // never silent.
    let cases: &[&str] = &[
        // x: elements with nothing to work with
        r##"<x:textbox in="#nope">text</x:textbox>"##,
        r##"<x:textpath font-size="10">no in</x:textpath>"##,
        r##"<x:textpath in="#nope" font-size="10">x</x:textpath>"##,
        r##"<x:warp><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="nope" bend="40"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="bend"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:boolean op="union"><g/></x:boolean>"##,
        r##"<x:boolean op="nope"><rect x="0" y="0" width="10" height="10"/></x:boolean>"##,
        r##"<x:boolean op="union"><use/></x:boolean>"##,
        // degenerate numerics
        r##"<x:warp field="arch" bend="NaN"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="arch" bend="1e999"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="roughen" bend="-40" detail="-8"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="perspective" corners="a b c d"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:warp field="arch" bend="40"><rect x="0" y="0" width="0" height="0"/></x:warp>"##,
        // degenerate reference targets
        r##"<path id="t" d=""/><x:textpath in="#t" font-size="10">x</x:textpath>"##,
        r##"<path id="t" d="garbage"/><x:textpath in="#t" font-size="10">x</x:textpath>"##,
        r##"<circle id="t" cx="5" cy="5" r="0"/><x:textbox in="#t">x</x:textbox>"##,
        r##"<text id="t">words</text><x:textpath in="#t" font-size="10">x</x:textpath>"##,
        r##"<g id="t"/><x:boolean op="union"><use href="#t"/></x:boolean>"##,
        r##"<g id="t"><text>only text</text></g><x:textbox in="#t">x</x:textbox>"##,
        // hostile transforms
        r##"<rect id="t" x="0" y="0" width="10" height="10" transform="matrix(0 0 0 0 0 0)"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
        r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="scale(0)"/></x:boolean>"##,
        r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="translate(1e300)"/></x:boolean>"##,
        r##"<rect id="t" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#t" transform="rotate(1e9)"/></x:boolean>"##,
        r##"<rect id="t" x="0" y="0" width="10" height="10" transform="skewX(90)"/><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
        // self-reference in every consumer
        r##"<x:textbox id="s" in="#s">x</x:textbox>"##,
        r##"<x:warp id="s" field="bend" in="#s"><rect x="0" y="0" width="10" height="10"/></x:warp>"##,
        r##"<x:boolean id="s" op="subtract"><use href="#s"/></x:boolean>"##,
        // group walk: live <use> children skipped, not fatal
        r##"<g id="t"><use href="#s"/><rect x="0" y="0" width="10" height="10"/></g><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
        // zero-extent rect as a reference target
        r##"<rect id="t" x="0" y="0" width="0" height="10"/><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
        // unparseable d inside a warp child: segment passes through unwarped
        r##"<x:warp field="arch" bend="40"><path d="garbage" fill="#000"/></x:warp>"##,
        // evenodd output that cancels to nothing
        r##"<x:warp id="t" field="arch" bend="0"><path d="M0,0 h10 v10 h-10 Z M0,0 h10 v10 h-10 Z" fill-rule="evenodd" fill="#000"/></x:warp><x:textpath in="#t" effect="stair" font-size="10">x</x:textpath>"##,
        // hostile connectors: missing endpoints, self, bad route
        r##"<x:connector/>"##,
        r##"<rect id="c" x="0" y="0" width="9" height="9"/><x:connector from="#c" to="#c" route="curve"/>"##,
        r##"<rect id="c1" x="0" y="0" width="9" height="9"/><rect id="c2" x="9" y="0" width="9" height="9"/><x:connector from="#c1" to="#c2" route="nonsense"/>"##,
        // degenerate meshgradient extent + non-finite stop-opacity
        r##"<meshgradient id="gz" x="5" y="5"><meshrow><meshpatch><stop path="l 0,0" stop-color="#e11"/><stop path="l 0,0" stop-color="#fa0"/><stop path="l 0,0" stop-color="#3b7"/><stop path="l 0,0" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="10" height="10" fill="url(#gz)"/>"##,
        r##"<meshgradient id="gi" x="0" y="0"><meshrow><meshpatch><stop path="l 9,0" stop-color="#e11" stop-opacity="inf"/><stop path="l 0,9" stop-color="#fa0"/><stop path="l -9,0" stop-color="#3b7"/><stop path="l 0,-9" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="9" height="9" fill="url(#gi)"/>"##,
        // hostile alpha: 5-digit hex, garbage stop-opacity
        r##"<x:mesh points="0,0 10,0 10,10 0,10"><x:face v="0 1 2 3" fill="#ff000"/></x:mesh>"##,
        r##"<meshgradient id="ga" x="0" y="0"><meshrow><meshpatch><stop path="l 9,0" stop-color="#e11" stop-opacity="soon"/><stop path="l 0,9" stop-color="#fa0"/><stop path="l -9,0" stop-color="#3b7"/><stop path="l 0,-9" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="9" height="9" fill="url(#ga)"/>"##,
        // hostile meshgradient: bare stop, empty gradient, referenced fills
        r##"<meshgradient id="gm"><meshrow><meshpatch><stop path="l 5,0"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="10" height="10" fill="url(#gm)"/>"##,
        r##"<meshgradient id="gm2" x="0" y="0"/><circle cx="5" cy="5" r="5" fill="url(#gm2)"/>"##,
        // hostile grid sugar: wrong color count, zero cells
        r##"<x:mesh x="0" y="0" width="10" height="10" cols="2" rows="2" fill="#f00 #0f0"/>"##,
        r##"<x:mesh x="0" y="0" width="10" height="10" cols="0" rows="2" fill="#f00"/>"##,
        // hostile meshes: bad indices, color-count mismatch, degenerate extent
        r##"<x:mesh points="0,0 10,0"><x:face v="0 1 9" fill="#f00"/></x:mesh>"##,
        r##"<x:mesh points="0,0 10,0 10,10 0,10"><x:face v="0 1 2 3" fill="#f00 #0f0"/></x:mesh>"##,
        r##"<x:mesh points="5,5 5,5 5,5"><x:face v="0 1 2" fill="#f00"/></x:mesh>"##,
        r##"<x:mesh points="garbage"><x:face v="0 1 2" fill="#f00 #0f0 #00f"/></x:mesh>"##,
        // hostile filter attributes: unparseable lists stay as authored
        r##"<rect x="0" y="0" width="10" height="10" filter="brightness("/>"##,
        r##"<rect x="0" y="0" width="10" height="10" filter="brightness(NaN)"/>"##,
        r##"<rect x="0" y="0" width="10" height="10" filter="-x-curve(9 9)"/>"##,
        r##"<g filter="brightness(1e999)"><rect x="0" y="0" width="10" height="10"/></g>"##,
    ];
    for (i, body) in cases.iter().enumerate() {
        let svg = format!("{XW}{body}</svg>");
        let out = compile_shaped(&svg); // a panic here fails the test: totality
        assert!(!out.is_empty(), "case {i} produced empty output: {body}");
        // non-finite text is only allowed where the AUTHOR wrote it (a
        // passthrough attribute value); generated coordinates never carry it
        for bad in ["NaN", "inf"] {
            assert!(
                !out.contains(bad) || body.contains(bad),
                "case {i} leaked non-finite coords: {body}\n{out}"
            );
        }
        assert!(
            out.contains("<path") || out.contains("<text") || out.contains("<!-- xsvg:"),
            "case {i} degraded SILENTLY (no geometry, no live text, no marker): {body}\n{out}"
        );
    }
}

#[test]
fn textbox_region_without_a_shaper_markers() {
    // geometry resolves but the host cannot rasterize: the marker names it
    let svg = format!(
        r##"{XW}<circle id="t" cx="20" cy="20" r="15"/><x:textbox in="#t" font-size="10">x</x:textbox></svg>"##
    );
    let out = compile_test(&svg); // NoShaper
    assert!(out.contains("not rasterizable"), "{out}");
}

#[test]
fn every_plain_shape_kind_resolves_as_a_reference() {
    use crate::kurbo::Shape;
    let bb = |body: &str| first_path(&compile_test(&format!("{XW}{body}</svg>"))).bounding_box();
    let e = bb(
        r##"<ellipse id="t" cx="20" cy="10" rx="20" ry="10"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
    );
    assert!(
        (e.x1 - 40.0).abs() < 0.5 && (e.y1 - 20.0).abs() < 0.5,
        "{e:?}"
    );
    let p = bb(
        r##"<polygon id="t" points="0,0 20,0 10,18"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
    );
    assert!(
        (p.x1 - 20.0).abs() < 0.5 && (p.y1 - 18.0).abs() < 0.5,
        "{p:?}"
    );
    let l = bb(
        r##"<polyline id="t" points="0,0 10,5 20,0"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean>"##,
    );
    assert!(
        (l.x1 - 20.0).abs() < 0.5 && (l.y1 - 5.0).abs() < 0.5,
        "{l:?}"
    );
}

#[test]
fn warp_unions_the_bbox_of_multiple_children() {
    // the envelope frame is the union of all child geometry, so both rects
    // share one field and bend together
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="40"><rect x="0" y="0" width="40" height="10" fill="#000"/><rect x="60" y="0" width="40" height="10" fill="#000"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<path").count(), 2, "{out}");
    assert!(!out.contains("<!-- xsvg:"), "{out}");
}

#[test]
fn filter_functions_lower_to_a_filter_definition() {
    // §8: CSS filter functions on any graphics element become a real
    // <filter> (sRGB, ±10% region) referenced by url() — one per element,
    // deterministic id, self-contained in the element's own fragment
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="40" height="30" fill="#48a" filter="brightness(1.2) contrast(1.1)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<filter id=\"x-flt-"), "{out}");
    assert!(
        out.contains("color-interpolation-filters=\"sRGB\""),
        "{out}"
    );
    assert!(out.contains("filter=\"url(#x-flt-"), "{out}");
    assert!(out.contains("slope=\"1.2\""), "{out}");
    assert!(!out.contains("filter=\"brightness"), "{out}");
    // passthrough elements get the same treatment, children intact
    let svg = format!(
        r##"{XW}<g filter="saturate(0)"><circle cx="5" cy="5" r="4" fill="#c00"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("type=\"saturate\" values=\"0\""), "{out}");
    assert!(out.contains("<circle"), "{out}");
    // a tone curve samples into a lookup table
    let svg = format!(
        r##"{XW}<g filter="-x-curve(0 0, 0.4 0.6, 1 1)"><rect x="0" y="0" width="10" height="10"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("type=\"table\" tableValues=\"0 "), "{out}");
}

#[test]
fn bleeding_filters_get_an_exact_or_inflated_region() {
    // a plain shape: the region is EXACT userSpaceOnUse — bbox grown by
    // 3σ + the shadow offset + half the (default) stroke width
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="40" height="30" fill="#48a" filter="drop-shadow(2 3 4 #123456)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("filterUnits=\"userSpaceOnUse\""), "{out}");
    // margin = 0.5 (stroke) + 3*4 + max(2,3) = 15.5 → x = -15.5, w = 71
    assert!(out.contains("x=\"-15.5\""), "{out}");
    assert!(out.contains("width=\"71\""), "{out}");
    assert!(out.contains("<feDropShadow"), "{out}");
    // unmeasurable content (a group) falls back to the ±50% region
    let svg = format!(
        r##"{XW}<g filter="blur(3)"><rect x="0" y="0" width="40" height="30" fill="#48a"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("x=\"-50%\""), "{out}");
    // pointwise lists keep the slim margin
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="40" height="30" fill="#48a" filter="sepia(0.5)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("x=\"-10%\""), "{out}");
}

#[test]
fn filter_references_and_unknown_functions_pass_through() {
    // url() references and unknown lists stay exactly as authored —
    // browsers still honor them live
    for f in ["url(#soft)", "none", "backdrop-blur(2)"] {
        let svg = format!(r##"{XW}<rect x="0" y="0" width="10" height="10" filter="{f}"/></svg>"##);
        let out = compile_test(&svg);
        assert!(out.contains(&format!("filter=\"{f}\"")), "{f}: {out}");
        assert!(!out.contains("<filter id="), "{f}: {out}");
    }
}

#[test]
fn mesh_lowers_to_a_texel_aligned_image() {
    // one smooth quad -> one region -> one clipped tiny-PNG image whose
    // placement OVERHANGS the mesh bbox (the texel-center construction)
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 200,0 200,100 0,100"><x:face v="0 1 2 3" fill="#e11 #fa0 #3b7 #06c"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<!-- xsvg:"), "{out}");
    assert_eq!(out.matches("<image").count(), 1, "{out}");
    assert!(out.contains("data:image/png;base64,"), "{out}");
    assert!(out.contains("preserveAspectRatio=\"none\""), "{out}");
    assert!(out.contains("<clipPath id=\"x-mesh-"), "{out}");
    let attr = |name: &str| -> f64 {
        let k = out.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        out[k..k + out[k..].find('"').unwrap()].parse().unwrap()
    };
    assert!(attr("width") > 200.0, "image must overhang the bbox");
    assert!(attr("x") < 0.5, "offset before the bbox");
}

const INKMESH: &str = r##"<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="c 30,-20 70,20 100,0" style="stop-color:#e11"/><stop path="c 10,20 -10,60 0,80" style="stop-color:#fa0"/><stop path="c -30,20 -70,-20 -100,0" style="stop-color:#3b7"/><stop path="c -10,-20 10,-60 0,-80" style="stop-color:#06c"/></meshpatch><meshpatch><stop path="c 30,20 70,-20 100,0" stop-color="#ff5"/><stop path="c -10,20 10,60 0,80" stop-color="#09f"/><stop path="c -30,-20 -70,20 -100,0"/></meshpatch></meshrow></meshgradient>"##;

#[test]
fn meshgradient_fill_compiles_the_inkscape_dialect() {
    // two Coons patches with edge/corner inheritance filling a rect: the
    // patches agree at the shared edge, so ONE smooth region, one image
    let svg = format!(
        r##"{XW}{INKMESH}<rect x="0" y="0" width="200" height="80" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<clipPath id=\"x-mgc-"), "{out}");
    assert_eq!(out.matches("<image").count(), 1, "{out}");
    assert!(out.contains("data:image/png;base64,"), "{out}");
    // the curved top edge bows ABOVE the anchor line: the mesh raster (and
    // so the image placement) must start above y=0
    let k = out.find("<image").unwrap();
    let y = {
        let j = out[k..].find(" y=\"").unwrap() + k + 4;
        out[j..j + out[j..].find('"').unwrap()]
            .parse::<f64>()
            .unwrap()
    };
    assert!(y < 0.0, "curved bulge must lift the raster: y={y}");
}

#[test]
fn meshgradient_object_bounding_box_units_scale_to_the_shape() {
    // unit-square patch + oBB units on a 200x80 rect: the image placement
    // must span (most of) the rect, not the unit square
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0" gradientUnits="objectBoundingBox"><meshrow><meshpatch><stop path="l 1,0" stop-color="#e11"/><stop path="l 0,1" stop-color="#fa0"/><stop path="l -1,0" stop-color="#3b7"/><stop path="l 0,-1" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="200" height="80" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<image"), "{out}");
    let k = out.find("<image").unwrap();
    let attr = |name: &str| -> f64 {
        let j = out[k..].find(&format!(" {name}=\"")).unwrap() + k + name.len() + 3;
        out[j..j + out[j..].find('"').unwrap()].parse().unwrap()
    };
    assert!(attr("width") > 150.0, "oBB mesh must span the shape: {out}");
}

#[test]
fn bicubic_meshgradients_differ_from_bilinear() {
    let mk = |ty: &str| {
        format!(
            r##"{XW}<meshgradient id="m" x="0" y="0"{ty}><meshrow><meshpatch><stop path="l 100,0" stop-color="#000"/><stop path="l 0,50" stop-color="#000"/><stop path="l -100,0" stop-color="#fff"/><stop path="l 0,-50" stop-color="#fff"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="100" height="50" fill="url(#m)"/></svg>"##
        )
    };
    let bilinear = compile_test(&mk(""));
    let bicubic = compile_test(&mk(r##" type="bicubic""##));
    assert!(bicubic.contains("<image"), "{bicubic}");
    let uri = |o: &str| {
        let k = o.find("base64,").unwrap();
        o[k..k + o[k..].find('"').unwrap()].to_string()
    };
    assert_ne!(uri(&bilinear), uri(&bicubic), "easing must change the fit");
}

#[test]
fn meshgradient_multi_row_grids_inherit_top_edges() {
    // a 2x2 patch grid exercising every inheritance case: (0,0) 4 stops,
    // (0,1) 3 stops (left inherited), (1,0) 3 stops (top inherited),
    // (1,1) 2 stops (top + left inherited). Corner colors agree at every
    // shared corner, so the whole grid is ONE smooth region.
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="l 50,0" stop-color="#e11"/><stop path="l 0,40" stop-color="#fa0"/><stop path="l -50,0" stop-color="#3b7"/><stop path="l 0,-40" stop-color="#06c"/></meshpatch><meshpatch><stop path="l 50,0" stop-color="#ff5"/><stop path="l 0,40" stop-color="#09f"/><stop path="l -50,0"/></meshpatch></meshrow><meshrow><meshpatch><stop path="l 0,40" stop-color="#a3f"/><stop path="l -50,0" stop-color="#0aa"/><stop path="l 0,-40"/></meshpatch><meshpatch><stop path="l 0,40" stop-color="#333"/><stop path="l -50,0"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="100" height="80" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("left live"), "{out}");
    assert_eq!(out.matches("<image").count(), 1, "one smooth region: {out}");
    // ragged rows (row 2 wider than row 1) are not a mesh
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="l 50,0" stop-color="#e11"/><stop path="l 0,40" stop-color="#fa0"/><stop path="l -50,0" stop-color="#3b7"/><stop path="l 0,-40" stop-color="#06c"/></meshpatch></meshrow><meshrow><meshpatch><stop path="l 0,40" stop-color="#a3f"/><stop path="l -50,0" stop-color="#0aa"/><stop path="l 0,-40"/></meshpatch><meshpatch><stop path="l 50,0" stop-color="#333"/><stop path="l 0,40" stop-color="#333"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="100" height="80" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("left live"), "{out}");
}

#[test]
fn meshgradient_absolute_edge_commands_and_shape_fallbacks() {
    // absolute C/L stop edges parse like their relative forms
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="C 20,-10 60,10 80,0" stop-color="#e11"/><stop path="L 80,50" stop-color="#fa0"/><stop path="C 60,60 20,40 0,50" stop-color="#3b7"/><stop path="L 0,0" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="80" height="50" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<image").count(), 1, "{out}");
    // a mesh fill on an element with no shape geometry falls through live
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="l 9,0" stop-color="#e11"/><stop path="l 0,9" stop-color="#fa0"/><stop path="l -9,0" stop-color="#3b7"/><stop path="l 0,-9" stop-color="#06c"/></meshpatch></meshrow></meshgradient><text x="0" y="10" fill="url(#m)">hi</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("fill=\"url(#m)\""), "{out}");
    assert!(!out.contains("<image"), "{out}");
    // a transformed mesh-filled shape keeps its transform on the wrapper
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="l 9,0" stop-color="#e11"/><stop path="l 0,9" stop-color="#fa0"/><stop path="l -9,0" stop-color="#3b7"/><stop path="l 0,-9" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="9" height="9" fill="url(#m)" transform="translate(30,0)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("transform=\"translate(30,0)\""), "{out}");
}

#[test]
fn mesh_lowers_at_every_quality_profile() {
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#e11 #fa0 #3b7 #06c"/></x:mesh></svg>"##
    );
    for q in ["fast", "balanced", "highest"] {
        let out = compile_impl(&svg, q, false, &Mono, &NoShaper, &NoOutliner).unwrap();
        assert_eq!(out.matches("<image").count(), 1, "{q}: {out}");
    }
}

#[test]
fn short_hex_alpha_and_exact_blur_regions() {
    // #rgba short form parses (alpha nibble doubled)
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 40,0 40,40 0,40"><x:face v="0 1 2 3" fill="#f00a"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("fill-opacity=\"0.667"), "{out}");
    // blur's exact region: margin = 0.5 + 3·4 = 12.5 on a 40x30 rect
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="40" height="30" fill="#48a" filter="blur(4)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("x=\"-12.5\""), "{out}");
    assert!(out.contains("width=\"65\""), "{out}");
}

#[test]
fn meshgradient_fill_keeps_the_stroke_on_top() {
    let svg = format!(
        r##"{XW}{INKMESH}<rect x="0" y="0" width="200" height="80" fill="url(#m)" stroke="#111" stroke-width="3"/></svg>"##
    );
    let out = compile_test(&svg);
    let img = out.find("<image").unwrap();
    let stroke = out.rfind("stroke=\"#111\"").unwrap();
    assert!(stroke > img, "stroke overlay paints after the mesh: {out}");
    assert!(out.contains("fill=\"none\""), "{out}");
}

#[test]
fn malformed_meshgradient_fill_stays_live_with_a_marker() {
    // a stop with garbage path: the dialect fails to parse, the element
    // passes through with its url() fill (unrendered, as in any browser)
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="q 1 2" stop-color="#e11"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="20" height="20" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("meshgradient fill left live"), "{out}");
    assert!(out.contains("fill=\"url(#m)\""), "{out}");
    assert!(!out.contains("<image"), "{out}");
}

/// Decode enough base64 to inspect a data-URI PNG's header.
fn b64_prefix(out: &str, n: usize) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let k = out.find("base64,").unwrap() + 7;
    let mut bytes = Vec::new();
    let mut buf = 0u32;
    let mut bits = 0;
    for &c in out[k..].as_bytes() {
        if c == b'"' || bytes.len() >= n {
            break;
        }
        let v = T.iter().position(|&t| t == c).unwrap() as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((buf >> bits) as u8);
        }
    }
    bytes
}

#[test]
fn feathered_meshes_carry_alpha() {
    // a fade-to-transparent mesh (feathering): the tiny PNG must be RGBA
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#ff3300ff #ff330000 #ff330000 #ff3300ff"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<image").count(), 1, "{out}");
    let png = b64_prefix(&out, 26);
    assert_eq!(png[25], 6, "feathered mesh must emit an RGBA PNG");
    // an opaque mesh stays RGB (color type 2)
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#e11 #fa0 #3b7 #06c"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    let png = b64_prefix(&out, 26);
    assert_eq!(png[25], 2, "opaque mesh stays RGB");
    // a constant semi-transparent region collapses to fill-opacity
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 40,0 40,40 0,40"><x:face v="0 1 2 3" fill="#3b82f680"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("fill-opacity=\"0.50"), "{out}");
    assert!(!out.contains("<image"), "{out}");
}

#[test]
fn grid_growth_follows_the_fields_anisotropy() {
    // a WIDE mesh whose color varies only vertically: the fitted grid must
    // spend its texels on rows, not aspect-locked columns — the PNG comes
    // out tall-and-narrow, not stretched wide
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 400,0 400,50 0,50" cols="1" rows="1"></x:mesh><x:mesh points="0,0 200,0 400,0 0,25 200,25 400,25 0,50 200,50 400,50"><x:face v="0 1 4 3" fill="#000 #000 #777 #777"/><x:face v="1 2 5 4" fill="#000 #000 #777 #777"/><x:face v="3 4 7 6" fill="#777 #777 #fff #fff"/><x:face v="4 5 8 7" fill="#777 #777 #fff #fff"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    let png = b64_prefix(&out, 26);
    let tw = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let th = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert!(
        th > tw,
        "vertical-only structure must yield a tall grid, got {tw}x{th}: {out}"
    );
    assert!(tw <= 4, "no wasted columns: {tw}x{th}");
}

#[test]
fn meshgradient_stop_opacity_feathers() {
    let svg = format!(
        r##"{XW}<meshgradient id="m" x="0" y="0"><meshrow><meshpatch><stop path="l 40,0" stop-color="#e11"/><stop path="l 0,30" stop-color="#fa0" stop-opacity="0"/><stop path="l -40,0" style="stop-color:#3b7;stop-opacity:0.25"/><stop path="l 0,-30" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="0" width="40" height="30" fill="url(#m)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<image"), "{out}");
    let png = b64_prefix(&out, 26);
    assert_eq!(png[25], 6, "stop-opacity must produce an RGBA PNG");
}

#[test]
fn mesh_grid_sugar_desugars_to_the_indexed_mesh() {
    // the sugar and its hand-written indexed equivalent must emit the SAME
    // fitted PNG and placement (ids differ by source position only)
    let sugar = format!(
        r##"{XW}<x:mesh x="0" y="0" width="200" height="100" cols="2" rows="1" fill="#e11 #fa0 #ff5 #06c #3b7 #09f"/></svg>"##
    );
    let indexed = format!(
        r##"{XW}<x:mesh points="0,0 100,0 200,0 0,100 100,100 200,100"><x:face v="0 1 4 3" fill="#e11 #fa0 #3b7 #06c"/><x:face v="1 2 5 4" fill="#fa0 #ff5 #09f #3b7"/></x:mesh></svg>"##
    );
    let a = compile_test(&sugar);
    let b = compile_test(&indexed);
    assert!(!a.contains("<!-- xsvg:"), "{a}");
    let uri = |o: &str| {
        let k = o.find("base64,").unwrap();
        o[k..k + o[k..].find('"').unwrap()].to_string()
    };
    assert_eq!(uri(&a), uri(&b), "same mesh, same fitted PNG");
    for attr in [" width=", " height=", " x=", " y="] {
        let val = |o: &str| {
            let img = o.find("<image").unwrap();
            let k = o[img..].find(attr).unwrap() + img + attr.len() + 1;
            o[k..k + o[k..].find('"').unwrap()].to_string()
        };
        assert_eq!(val(&a), val(&b), "{attr}");
    }
}

#[test]
fn mesh_solid_region_is_a_plain_path() {
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 40,0 40,40 0,40"><x:face v="0 1 2 3" fill="#3b82f6"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<image"), "{out}");
    assert!(out.contains("<path fill=\"#3b82f6\""), "{out}");
}

#[test]
fn mesh_cracks_split_into_separately_clipped_regions() {
    // two quads sharing an edge with DISAGREEING colors: a crack -> two
    // regions, each with its own image + clip
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 100,0 200,0 0,100 100,100 200,100"><x:face v="0 1 4 3" fill="#e11 #fa0 #fa0 #e11"/><x:face v="1 2 5 4" fill="#06c #3b7 #3b7 #06c"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<clipPath").count(), 2, "{out}");
    assert_eq!(out.matches("<image").count(), 2, "{out}");
    // smooth version (shared edge agrees) -> ONE region
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 100,0 200,0 0,100 100,100 200,100"><x:face v="0 1 4 3" fill="#e11 #fa0 #fa0 #e11"/><x:face v="1 2 5 4" fill="#fa0 #3b7 #3b7 #fa0"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<image").count(), 1, "{out}");
}

#[test]
fn mesh_triangles_and_single_color_faces_work() {
    let svg = format!(
        r##"{XW}<x:mesh points="0,0 80,0 40,60"><x:face v="0 1 2" fill="#e11 #3b7 #06c"/></x:mesh></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<image").count(), 1, "{out}");
    // triangle clip path has three points
    let k = out.find("<clipPath").unwrap();
    let d = &out[k..k + out[k..].find("/>").unwrap()];
    assert_eq!(d.matches('L').count(), 2, "{d}");
}

#[test]
fn parse_transform_matches_svg_semantics() {
    use crate::kurbo::Point;
    let p = |x, y| Point::new(x, y);
    assert_eq!(
        parse_transform("translate(10,20)") * p(0.0, 0.0),
        p(10.0, 20.0)
    );
    assert_eq!(
        parse_transform("matrix(1 0 0 1 5 6)") * p(1.0, 1.0),
        p(6.0, 7.0)
    );
    // list applies left-to-right: scale first in point terms, then translate
    assert_eq!(
        parse_transform("translate(10) scale(2)") * p(1.0, 0.0),
        p(12.0, 0.0)
    );
    let r = parse_transform("rotate(90 10 10)") * p(10.0, 0.0);
    assert!(
        (r.x - 20.0).abs() < 1e-9 && (r.y - 10.0).abs() < 1e-9,
        "{r:?}"
    );
    assert_eq!(parse_transform("scale(2,3)") * p(1.0, 1.0), p(2.0, 3.0));
    let sk = parse_transform("skewY(45)") * p(10.0, 0.0);
    assert!((sk.y - 10.0).abs() < 1e-9, "{sk:?}");
    // invalid input → the whole list is ignored, like a browser
    assert_eq!(parse_transform("rotate(nope)") * p(3.0, 4.0), p(3.0, 4.0));
    assert_eq!(parse_transform("translate(3") * p(0.0, 0.0), p(0.0, 0.0));
    // a finite list whose product overflows is ignored too
    assert_eq!(
        parse_transform("scale(1e308) scale(1e308)") * p(1.0, 1.0),
        p(1.0, 1.0)
    );
}

#[test]
fn referenced_targets_honor_their_own_transform() {
    // plain shape: a rect rotated about (20,20) — the borrowed geometry is
    // where the user SEES it, not the untransformed source
    let svg = format!(
        r##"{XW}<rect id="t" x="0" y="0" width="40" height="10" transform="rotate(90 20 20)"/><x:boolean op="union" fill="#000"><use href="#t"/></x:boolean></svg>"##
    );
    use crate::kurbo::Shape;
    let bb = first_path(&compile_test(&svg)).bounding_box();
    assert!(
        (bb.x0 - 30.0).abs() < 0.5 && (bb.x1 - 40.0).abs() < 0.5,
        "{bb:?}"
    );
    assert!(
        (bb.y0 - 0.0).abs() < 0.5 && (bb.y1 - 40.0).abs() < 0.5,
        "{bb:?}"
    );
    // x: target: the transform on the boolean applies to its borrowed output
    let svg = format!(
        r##"{XW}<x:boolean id="b" op="union" fill="#000" transform="translate(100,0)"><rect x="0" y="0" width="40" height="30"/></x:boolean><x:boolean op="union" fill="#000"><use href="#b"/></x:boolean></svg>"##
    );
    let bb = first_path(&compile_test(&svg)).bounding_box();
    assert!(
        (bb.x0 - 100.0).abs() < 0.5 && (bb.x1 - 140.0).abs() < 0.5,
        "{bb:?}"
    );
    // a <use> operand's own transform composes with x/y
    let svg = format!(
        r##"{XW}<rect id="st" x="0" y="0" width="10" height="10"/><x:boolean op="union" fill="#000"><use href="#st" transform="scale(2)"/></x:boolean></svg>"##
    );
    let bb = first_path(&compile_test(&svg)).bounding_box();
    assert!(
        (bb.x1 - 20.0).abs() < 0.5 && (bb.y1 - 20.0).abs() < 0.5,
        "{bb:?}"
    );
}

#[test]
fn nested_transform_in_referenced_output_degrades_loudly() {
    // the warp's CHILD carries a transform that survives into its output;
    // a d-only harvest cannot honor it, so the reference degrades
    let svg = format!(
        r##"{XW}<x:warp id="t" field="arch" bend="10"><path d="M0,20 h40 v10 h-40 Z" transform="translate(3,4)" fill="#000"/></x:warp><x:textpath in="#t" effect="stair" font-size="10">hi</x:textpath></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("(referenced output nests a transform)"),
        "{out}"
    );
}

#[test]
fn referenced_evenodd_output_resolves_to_its_painted_region() {
    // a ring drawn with two same-winding subpaths under evenodd: the painted
    // region is the frame (1200), not the nonzero solid (1600)
    let svg = format!(
        r##"{XW}<x:warp id="ring" field="arch" bend="0"><path d="M0,0 h40 v40 h-40 Z M10,10 h20 v20 h-20 Z" fill-rule="evenodd" fill="#000"/></x:warp><x:boolean op="union" fill="#000"><use href="#ring"/></x:boolean></svg>"##
    );
    use crate::kurbo::Shape;
    let p = first_path(&compile_test(&svg));
    assert!((p.area().abs() - 1200.0).abs() < 5.0, "{}", p.area());
}

#[test]
fn group_targets_contribute_their_shape_descendants() {
    // transforms compose down the tree: group translate + nested-group translate
    let svg = format!(
        r##"{XW}<g id="grp" transform="translate(10,0)"><circle cx="20" cy="20" r="10"/><g transform="translate(0,40)"><rect x="0" y="0" width="10" height="10"/></g></g><x:boolean op="union" fill="#000"><use href="#grp"/></x:boolean></svg>"##
    );
    use crate::kurbo::Shape;
    let bb = first_path(&compile_test(&svg)).bounding_box();
    assert!(
        (bb.x0 - 10.0).abs() < 0.5 && (bb.x1 - 40.0).abs() < 0.5,
        "{bb:?}"
    );
    assert!(
        (bb.y0 - 10.0).abs() < 0.5 && (bb.y1 - 50.0).abs() < 0.5,
        "{bb:?}"
    );
}

#[test]
fn cycle_poisoned_fanout_exhausts_fuel_not_time() {
    // a self-cycle at the bottom poisons the memo (cut results are never
    // cached), so 26 double-referencing levels would branch ~2^26 without
    // the fuel bound — this compiling promptly IS the assertion
    let mut body = String::from(
        r##"<x:boolean id="p0" op="union" fill="#000"><use href="#p0"/><rect x="0" y="0" width="10" height="10"/></x:boolean>"##,
    );
    for i in 1..=26 {
        body.push_str(&format!(
                r##"<x:boolean id="p{i}" op="union" fill="#000"><use href="#p{}"/><use href="#p{}" x="5"/></x:boolean>"##,
                i - 1,
                i - 1
            ));
    }
    let out = compile_test(&format!("{XW}{body}</svg>"));
    assert!(
        out.contains("(reference budget exhausted)"),
        "expected a fuel cut"
    );
}

#[test]
fn referenced_text_is_outlined_for_geometry() {
    // punching a textbox by reference: the scratch resolution forces
    // outline="true" semantics so glyphs contribute geometry — while the
    // textbox itself still renders as live <text>
    let svg = format!(
        r##"{XW}<x:textbox id="w" x="10" y="10" width="80" height="20" font-size="10">hi</x:textbox><x:boolean op="subtract" fill="#000"><rect x="0" y="0" width="100" height="40"/><use href="#w"/></x:boolean></svg>"##
    );
    let out = compile_outlined(&svg);
    assert!(
        out.contains("<text"),
        "textbox itself must stay live: {out}"
    );
    use crate::kurbo::Shape;
    let a = first_path(&out).area().abs();
    assert!(a < 3999.5 && a > 3990.0, "glyph geometry not punched: {a}");
}

#[test]
fn reference_chains_deeper_than_the_tree_bound_degrade() {
    // 80 SIBLING elements chained by reference sidestep the XML nesting
    // bound entirely; the resolving stack's own MAX_REF_DEPTH cap keeps §4
    // totality — a marker, never a stack overflow. (Reversed document order
    // defeats memo pre-warming, forcing the deep descents.)
    let mut body = String::new();
    for i in (1..=80).rev() {
        body.push_str(&format!(
            r##"<x:boolean id="c{i}" op="union" fill="#000"><use href="#c{}"/></x:boolean>"##,
            i - 1
        ));
    }
    body.push_str(r##"<rect id="c0" x="0" y="0" width="10" height="10"/>"##);
    let out = compile_test(&format!("{XW}{body}</svg>"));
    assert!(out.contains("skipped <use>"), "expected a depth-cut marker");
    assert!(out.contains("<path"));
}

#[test]
fn diamond_reference_fanout_is_memoized_not_exponential() {
    // each level references the previous level TWICE (once offset by 5);
    // 24 levels would be 2^24 lowerings without the per-compile memo —
    // this test finishing at all is the assertion that the memo works
    let mut body = String::from(r##"<rect id="g0" x="0" y="0" width="40" height="30"/>"##);
    for i in 1..=24 {
        body.push_str(&format!(
                r##"<x:boolean id="g{i}" op="union" fill="#000"><use href="#g{}"/><use href="#g{}" x="5"/></x:boolean>"##,
                i - 1,
                i - 1
            ));
    }
    let out = compile_test(&format!("{XW}{body}</svg>"));
    use crate::kurbo::Shape;
    // width grows by 5 per level: 40 + 24·5 = 160
    let bb = first_path(&out).bounding_box();
    assert!(
        (bb.x1 - 160.0).abs() < 0.5 && (bb.y1 - 30.0).abs() < 0.5,
        "{bb:?}"
    );
}

#[test]
fn fragment_recompile_reflects_a_local_edit() {
    // same-length edit inside the textbox keeps all offsets stable
    let edited = INC.replace("hello world", "HELLO WORLD");
    let off = top_level_offsets(INC)[1];
    let before =
        compile_fragment_impl(INC, "balanced", false, off, &Mono, &NoShaper, &NoOutliner).unwrap();
    let after = compile_fragment_impl(
        &edited,
        "balanced",
        false,
        off,
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert_ne!(before, after);
    assert!(after.contains("HELLO"), "{after}");
    // and the unrelated warp fragment is byte-identical across the edit
    let woff = top_level_offsets(INC)[4];
    let w0 =
        compile_fragment_impl(INC, "balanced", false, woff, &Mono, &NoShaper, &NoOutliner).unwrap();
    let w1 = compile_fragment_impl(
        &edited,
        "balanced",
        false,
        woff,
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert_eq!(w0, w1);
}

// ---- <x:boolean> (§7.5) ----

#[test]
fn boolean_subtract_takes_paint_from_the_element() {
    // rect minus rect → one <path>, element paint, no child paint, L-shape bbox
    let svg = format!(
        r##"{XW}<x:boolean op="subtract" fill="#1d4ed8"><rect x="0" y="0" width="100" height="40" fill="#fff"/><rect x="60" y="0" width="60" height="40" fill="#000"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains(r##"<path fill="#1d4ed8""##), "{out}");
    assert!(!out.contains("#fff") && !out.contains("#000"), "{out}");
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    assert!(bb.x0.abs() < 0.1 && (bb.x1 - 60.0).abs() < 0.1, "{bb:?}");
}

#[test]
fn boolean_defaults_to_union() {
    // two overlapping rects, no op attr → one merged outline (5 distinct
    // corners on the silhouette → more than one rect's 4)
    let svg = format!(
        r##"{XW}<x:boolean fill="#111"><rect x="0" y="0" width="100" height="40"/><rect x="50" y="20" width="100" height="40"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    assert!(
        (bb.x1 - 150.0).abs() < 0.1 && (bb.y1 - 60.0).abs() < 0.1,
        "{bb:?}"
    );
    assert_eq!(out.matches("<path").count(), 1, "{out}");
}

#[test]
fn boolean_punches_outlined_text() {
    // the flagship: text (BoxOutliner's 1×1 box at the line origin) punched
    // out of a plate — the result is a single path with a hole
    let svg = format!(
        r##"{XW}<x:boolean op="subtract" fill="#333"><rect x="0" y="0" width="80" height="20"/><x:textbox x="10" y="5" width="60" height="10" outline="true" font-size="8">x</x:textbox></x:boolean></svg>"##
    );
    let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
    assert!(out.contains("<path fill=\"#333\""), "{out}");
    // two contours: the plate outline + the punched hole
    let d = out
        .rsplit(" d=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(d.matches('M').count() + d.matches('m').count(), 2, "{out}");
}

#[test]
fn boolean_empty_and_degenerate_results() {
    // disjoint intersect → legitimately empty <g/>
    let svg = format!(
        r##"{XW}<x:boolean op="intersect" fill="#111"><rect x="0" y="0" width="10" height="10"/><rect x="100" y="0" width="10" height="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<path"), "{out}");
    assert!(out.contains("<g fill=\"#111\"/>"), "{out}");
    // unknown op → marker + children un-combined
    let svg = format!(
        r##"{XW}<x:boolean op="divide"><rect x="0" y="0" width="10" height="10"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("unknown op"), "{out}");
    assert!(out.contains("h10"), "child lost: {out}");
    // live-text child skipped with a marker; empty element → no-geometry marker
    let svg = format!(
        r##"{XW}<x:boolean><x:textbox x="0" y="0" width="40" height="10" font-size="8">hi</x:textbox></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<x:boolean> skipped <textbox>"), "{out}");
    assert!(out.contains("no usable geometry"), "{out}");
}

#[test]
fn boolean_composes_with_warp_both_ways() {
    // boolean inside warp: the combined path bakes like any geometry
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="50"><x:boolean op="union" fill="#111"><rect x="0" y="0" width="60" height="20"/><rect x="40" y="0" width="60" height="20"/></x:boolean></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<path"), "{out}");
    assert!(!out.contains("skipped"), "{out}");
    // warp inside boolean: warped output is an operand
    let svg = format!(
        r##"{XW}<x:boolean op="intersect" fill="#111"><x:warp field="arch" bend="50"><rect x="0" y="0" width="100" height="30"/></x:warp><rect x="0" y="-20" width="100" height="30"/></x:boolean></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<path fill=\"#111\""), "{out}");
    assert!(!out.contains("NaN"), "{out}");
}

// ---- <x:warp> (§7.3) ----

// In-memory resolver for cross-file link tests: flat namespace, href IS the key
// (relative-path resolution is the host's job; the core just links what it's handed).
struct MapResolver(std::collections::HashMap<String, String>);
impl Resolver for MapResolver {
    fn resolve(&self, _base: &str, href: &str) -> Option<(String, String)> {
        self.0.get(href).map(|s| (href.to_string(), s.clone()))
    }
}
fn compile_linked(main: &str, files: &[(&str, &str)]) -> String {
    let map = MapResolver(files.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
    compile_linked_impl(main, "balanced", false, &Mono, &NoShaper, &NoOutliner, &map, "main")
        .unwrap()
}

#[test]
fn use_links_external_files() {
    let logo = format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);

    // whole-file link → the dependency inlined inside a nested <svg> viewport, baked
    let main = format!(r##"{XW}<use href="logo.svg" x="20" y="30" width="40" height="40"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(out.contains(r#"<svg x="20" y="30" width="40" height="40""#), "nested svg viewport: {out}");
    assert!(out.contains("#f00"), "dep content inlined (rect lowers to a path): {out}");
    assert!(!out.contains("<use"), "the <use> is baked, not a live ref: {out}");

    // by-id link → just that element, placed with a translate
    let main = format!(r##"{XW}<use href="logo.svg#mark" x="5" y="5"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(out.contains("translate(5,5)") && out.contains(r#"id="mark""#), "by-id placed: {out}");

    // by-id with a width sizes to the *element's own* extent (10×10 rect → scale 2), not the file
    let main = format!(r##"{XW}<use href="logo.svg#mark" x="0" y="0" width="20"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(out.contains("scale(2)"), "by-id sizes to the target's own extent: {out}");

    // a group target with no declared size measures its geometry (a 10×10 rect inside → scale 3)
    let grp = format!(r##"{XW}<g id="mark"><rect x="0" y="0" width="10" height="10" fill="#f00"/></g></svg>"##);
    let main = format!(r##"{XW}<use href="g.svg#mark" x="0" y="0" width="30"/></svg>"##);
    let out = compile_linked(&main, &[("g.svg", &grp)]);
    assert!(out.contains("scale(3)"), "group by-id sizes to its geometry bbox: {out}");

    // missing dependency degrades with a marker
    let out = compile_linked(&main, &[]);
    assert!(out.contains("not resolved"), "missing dep degrades: {out}");

    // a same-document <use href="#id"> is NOT linked — it stays a live reference
    let main = format!(r##"{XW}<rect id="r" x="0" y="0" width="4" height="4"/><use href="#r"/></svg>"##);
    let out = compile_linked(&main, &[]);
    assert!(out.contains(r##"<use href="#r""##), "same-doc use passes through: {out}");

    // cycle a → b → a is refused
    let a = format!(r##"{XW}<use href="b.svg"/></svg>"##);
    let b = format!(r##"{XW}<use href="a.svg"/></svg>"##);
    let main = format!(r##"{XW}<use href="a.svg"/></svg>"##);
    let out = compile_linked(&main, &[("a.svg", &a), ("b.svg", &b)]);
    assert!(out.contains("cyclic"), "cycle refused: {out}");
}

#[test]
fn linked_fragment_is_a_verbatim_slice_of_the_full_compile() {
    // The incremental invariant must hold for linked elements too: re-emitting the
    // <use>'s fragment must match its span in the full linked compile (not degrade).
    let logo = format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);
    let main =
        format!(r##"{XW}<rect id="bg" x="0" y="0" width="100" height="100"/><use href="logo.svg" x="20" y="20" width="40"/></svg>"##);
    let map = MapResolver([("logo.svg".to_string(), logo.clone())].into_iter().collect());
    let full =
        compile_linked_impl(&main, "balanced", false, &Mono, &NoShaper, &NoOutliner, &map, "main")
            .unwrap();
    let off = main.find("<use").unwrap();
    let frag = compile_fragment_linked_impl(
        &main, "balanced", false, off, &Mono, &NoShaper, &NoOutliner, &map, "main",
    )
    .unwrap();
    assert!(frag.contains(r#"<svg x="20""#) && frag.contains("#f00"), "fragment links: {frag}");
    assert!(full.contains(&frag), "fragment is a verbatim slice of the full compile:\nfrag={frag}\nfull={full}");
}

#[test]
fn text_border_strokes_behind_the_fill() {
    // x:border-* → a bordered-text effect: stroke sits behind the fill (paint-order),
    // round joins, and border-width is what shows outside the glyph (stroke-width doubled).
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="50" font-size="30" fill="#fff" x:border-width="2" x:border-color="#000">Hi</x:textbox></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains(r##"stroke="#000""##), "border color: {out}");
    assert!(out.contains(r#"stroke-width="4""#), "border-width doubled: {out}");
    assert!(out.contains(r#"paint-order="stroke""#), "stroke behind fill: {out}");

    // Raw stroke on *live* text now passes through (was silently dropped before).
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="50" stroke="#f00" stroke-width="1">Hi</x:textbox></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("<text") && out.contains(r##"stroke="#f00""##), "raw stroke on live text: {out}");
}

#[test]
fn external_use_source_map_points_at_the_use_not_the_dependency() {
    // A linked dependency's element ranges index the *dependency* file, not the entry
    // document the viewer maps against — so baked content must carry no data-xsvg-pos,
    // and the block resolves (up the DOM) to the <use>'s range in the entry source.
    let logo = format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);
    let main = format!(r##"{XW}<use href="logo.svg" x="20" y="30" width="40" height="40"/></svg>"##);
    let map = MapResolver([("logo.svg".to_string(), logo)].into_iter().collect());
    let out =
        compile_linked_impl(&main, "balanced", true, &Mono, &NoShaper, &NoOutliner, &map, "main")
            .unwrap();

    // only the root and the <use> wrapper are tagged — nothing baked in from the dep
    assert_eq!(
        out.matches("data-xsvg-pos").count(),
        2,
        "no source ranges on baked dependency content: {out}"
    );
    // and the wrapper carries the <use> element's own byte range in the entry source
    let us = main.find("<use").unwrap();
    let ue = main[us..].find("/>").unwrap() + us + 2;
    assert!(
        out.contains(&format!(r#"data-xsvg-pos="{us}-{ue}""#)),
        "linked block maps to the <use> in the entry source: {out}"
    );
}

#[test]
fn by_id_bbox_ignores_non_rendered_hidden_and_clips_viewports() {
    // In every case #mark's *visible* geometry is 10 wide, so a <use> at width=20 must
    // scale by 2. Anything that wrongly inflates the measured box breaks that.
    let cases: &[(&str, &str)] = &[
        // definition-only subtrees are referenced, never drawn in place
        ("defs", r##"<g id="mark"><rect width="10" height="10"/><defs><rect width="1000" height="1000"/></defs></g>"##),
        ("clipPath", r##"<g id="mark"><rect width="10" height="10"/><clipPath id="c"><rect width="1000" height="1000"/></clipPath></g>"##),
        // explicitly hidden, by attribute and by inline style
        ("display-attr", r##"<g id="mark"><rect width="10" height="10"/><rect width="1000" height="1000" display="none"/></g>"##),
        ("display-style", r##"<g id="mark"><rect width="10" height="10"/><rect width="1000" height="1000" style="display: none"/></g>"##),
        // a nested <svg> clips to its viewport — 10 wide, not its 1000-unit viewBox content
        ("nested-svg", r##"<g id="mark"><svg x="0" y="0" width="10" height="10" viewBox="0 0 1000 1000"><rect width="1000" height="1000"/></svg></g>"##),
        // <image> is box-shaped: it has no path geometry but does have an extent
        ("image", r##"<g id="mark"><image x="0" y="0" width="10" height="10" href="p.png"/></g>"##),
    ];
    for (name, body) in cases {
        let dep = format!("{XW}{body}</svg>");
        let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="20"/></svg>"##);
        let out = compile_linked(&main, &[("d.svg", &dep)]);
        assert!(out.contains("scale(2)"), "{name}: expected scale(2) — got {out}");
    }
}

#[test]
fn by_id_bbox_follows_same_document_use() {
    // A group whose content is a live <use href="#part"> must measure the target, offset
    // by the <use>'s x/y — otherwise it contributes nothing and the box is wrong.
    // #part is 10 wide at x=0; the <use> shifts it to 10..20, so #mark spans 0..20.
    let dep = format!(
        r##"{XW}<defs><rect id="part" width="10" height="10"/></defs><g id="mark"><rect width="10" height="10"/><use href="#part" x="10" y="0"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="40"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(out.contains("scale(2)"), "measured through the <use>: {out}");

    // a <use> chain that loops back must stop, not recurse forever
    let dep = format!(
        r##"{XW}<g id="mark"><rect width="10" height="10"/><use href="#loop"/></g><g id="loop"><use href="#mark"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="20"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(out.contains("scale(2)"), "cyclic <use> terminates: {out}");
}

#[test]
fn by_id_bbox_counts_the_stroke_outset() {
    // A stroke straddles the outline, so a 10-wide rect with stroke-width 4 paints from
    // -2 to 12 — 14 wide. At width=28 that is scale 2 (without the outset it'd be 2.8).
    let dep = format!(
        r##"{XW}<g id="mark"><rect width="10" height="10" stroke="#000" stroke-width="4"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="28"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(out.contains("scale(2)"), "stroke half-width counted: {out}");

    // stroke="none" paints nothing, so the box stays the 10-wide fill (scale 2 at 20)
    let dep = format!(
        r##"{XW}<g id="mark"><rect width="10" height="10" stroke="none" stroke-width="4"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="20"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(out.contains("scale(2)"), "stroke=none adds no outset: {out}");

    // stroke inherits from an ancestor group
    let dep = format!(
        r##"{XW}<g id="mark" stroke="#000" stroke-width="4"><rect width="10" height="10"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="28"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(out.contains("scale(2)"), "inherited stroke counted: {out}");
}

#[test]
fn by_id_bbox_honors_a_css_style_transform() {
    // style="transform: translate(10px,0)" puts the 10-wide rect at x=10..20 — still 10
    // wide (scale 2 at width=20), but the box origin re-anchors onto the <use>'s x/y.
    let dep = format!(
        r##"{XW}<g id="mark"><rect width="10" height="10" style="transform: translate(10px, 0px)"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="20"/></svg>"##);
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    assert!(
        out.contains("translate(-20,0) scale(2)"),
        "css style transform folded into the measured box: {out}"
    );
}

#[test]
fn linked_files_do_not_share_the_ref_memo() {
    // `#id` refs never cross a file boundary, but the memo and cycle stack are keyed by
    // bare id — so a dependency resolving its own `#a` must not poison the referrer's.
    // The id sits on a <g>: only groups and x: elements go through the memo (a plain
    // shape resolves directly), so that is where a cross-file collision can bite.
    let dep = format!(
        r##"{XW}<g id="a"><rect x="500" y="500" width="4" height="4"/></g><x:textbox in="#a" font-size="2">d</x:textbox></svg>"##
    );
    let main = format!(
        r##"{XW}<g id="a"><rect x="0" y="0" width="100" height="100"/></g><rect id="b" x="200" y="0" width="100" height="100"/><use href="d.svg"/><x:connector from="#a" to="#b"/></svg>"##
    );
    let out = compile_linked(&main, &[("d.svg", &dep)]);
    let d = route_d(&out);
    assert!(
        d.starts_with("M100,50"),
        "entry `#a` must resolve to the entry's own rect, not the dependency's: d={d}"
    );
}

const XW: &str =
    r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">"##;

/// Compile at `fast` quality — raw polyline output, exact vertex assertions.
fn compile_fast(svg: &str) -> String {
    compile_impl(svg, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap()
}

/// Reparse the last emitted `d` attribute — the warped output (reference paths
/// pass through first). The output is compact/relative, so geometry assertions
/// go through kurbo rather than string matching.
/// The `d` of a connector's route path — the one with fill="none".
fn route_d(out: &str) -> &str {
    let k = out.find("fill=\"none\"").unwrap();
    let j = out[k..].find(" d=\"").unwrap() + k + 4;
    &out[j..j + out[j..].find('"').unwrap()]
}

fn first_path(out: &str) -> crate::kurbo::BezPath {
    let d = out
        .rsplit(" d=\"")
        .next()
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    crate::kurbo::BezPath::from_svg(d).unwrap()
}

#[test]
fn warp_arch_bends_a_rect() {
    // bbox 100×40, bend 100% → A = 25: the apex reaches y = −25, the corners
    // stay pinned (u = ±1 → Δ = 0), and the rect is now a subdivided polyline.
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="100" height="40" fill="#f00"/></x:warp></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("<g"), "{out}");
    assert!(!out.contains("<rect"), "{out}");
    assert!(out.contains(r##"fill="#f00""##), "{out}");
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    assert!((bb.y0 + 25.0).abs() < 0.1, "apex {}: {out}", bb.y0);
    assert!((bb.x0.abs()) < 0.1 && (bb.x1 - 100.0).abs() < 0.1, "{out}");
    assert!(
        first_path(&out).elements().len() > 10,
        "no subdivision: {out}"
    );
}

#[test]
fn warp_skips_live_text_child_with_a_marker() {
    // no outline font → the textbox lowers to live <text>, which a warp must
    // not silently emit unwarped
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi</x:textbox></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<x:warp> skipped <textbox>"), "{out}");
    assert!(!out.contains("<text "), "{out}");
}

#[test]
fn warp_warps_outlined_text_child() {
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="50"><x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">hi</x:textbox></x:warp></svg>"##
    );
    let out = compile_outlined(&svg);
    assert!(!out.contains("<text"), "{out}");
    assert!(!out.contains("skipped"), "{out}");
    assert!(out.contains("<path d=\"M"), "{out}");
}

#[test]
fn warp_circle_child_becomes_a_warped_path() {
    let svg = format!(
        r##"{XW}<x:warp field="flag" bend="60"><circle cx="50" cy="50" r="40" fill="#0af"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<circle"), "{out}");
    assert!(out.contains(r##"fill="#0af""##), "{out}");
    assert!(out.contains("<path"), "{out}");
}

#[test]
fn warp_radial_and_rotational_presets_parse() {
    for name in [
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
        "arc",
        "arc-lower",
        "arc-upper",
        "bulge",
        "shell-lower",
        "shell-upper",
        "fish",
    ] {
        let svg = format!(
            r##"{XW}<x:warp field="{name}" bend="80"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
        );
        let out = compile_test(&svg);
        assert!(!out.contains("unknown field"), "{name}: {out}");
        assert!(out.contains("<path"), "{name}: {out}");
        assert!(!out.contains("NaN"), "{name}: {out}");
    }
}

#[test]
fn warp_perspective_pins_authored_corners_without_subdividing() {
    // bbox (0,0)-(200,120) → authored quad; projective keeps edges straight, so
    // the rect bakes to exactly its 4 corner vertices (M + 4 L, incl. the
    // explicit closing edge) — no wasted subdivision
    let svg = format!(
        r##"{XW}<x:warp field="perspective" corners="20,10 180,10 200,120 0,120"><rect x="0" y="0" width="200" height="120" fill="#f0f"/></x:warp></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("M20,10"), "{out}");
    let path = first_path(&out);
    use crate::kurbo::PathEl;
    let lines = path
        .elements()
        .iter()
        .filter(|e| matches!(e, PathEl::LineTo(_)))
        .count();
    assert_eq!(lines, 4, "{out}");
    // the authored corners are pinned exactly (within the fast grid)
    for corner in [(20.0, 10.0), (180.0, 10.0), (200.0, 120.0), (0.0, 120.0)] {
        let hit = path.elements().iter().any(|e| match e {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => {
                (p.x - corner.0).abs() < 0.06 && (p.y - corner.1).abs() < 0.06
            }
            _ => false,
        });
        assert!(hit, "corner {corner:?} missing: {out}");
    }
}

#[test]
fn warp_perspective_without_corners_degrades_with_a_marker() {
    let svg = format!(
        r##"{XW}<x:warp field="perspective"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("bad corners"), "{out}");
    assert!(out.contains("h100"), "child not passed through: {out}");
}

#[test]
fn warp_free_distort_parses_and_bakes() {
    let svg = format!(
        r##"{XW}<x:warp field="free" corners="0,20 100,-10 110,50 -10,30"><rect x="0" y="0" width="100" height="40" fill="#0f0"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("unknown field"), "{out}");
    assert!(out.contains("M0,20"), "{out}");
}

#[test]
fn warp_distortion_sliders_compose_after_the_preset() {
    let plain = format!(
        r##"{XW}<x:warp field="arch" bend="60"><rect x="0" y="0" width="200" height="80"/></x:warp></svg>"##
    );
    let tapered = format!(
        r##"{XW}<x:warp field="arch" bend="60" distort-h="60"><rect x="0" y="0" width="200" height="80"/></x:warp></svg>"##
    );
    let a = compile_test(&plain);
    let b = compile_test(&tapered);
    assert_ne!(a, b);
    assert!(!b.contains("distort-h"), "slider attr leaked onto <g>: {b}");
    assert!(!b.contains("NaN"), "{b}");
}

#[test]
fn warp_bend_flows_children_along_a_spine() {
    // envelope (0,0)-(100,40), flat spine at y = 100: the envelope midline
    // (y = 20) rides the spine, so the rect maps to a band y = 80…120 starting
    // at the spine's start
    let svg = format!(
        r##"{XW}<path id="spine" d="M0,100 L300,100" fill="none"/><x:warp field="bend" in="#spine"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("M0,80"), "{out}");
    use crate::kurbo::Shape;
    let bb = first_path(&out).bounding_box();
    assert!(
        bb.x0.abs() < 0.1
            && (bb.x1 - 100.0).abs() < 0.1
            && (bb.y0 - 80.0).abs() < 0.1
            && (bb.y1 - 120.0).abs() < 0.1,
        "band {bb:?}: {out}"
    );
    // a vertical spine rotates the band onto it
    let svg = format!(
        r##"{XW}<path id="spine" d="M50,0 L50,300" fill="none"/><x:warp field="bend" in="#spine"><rect x="0" y="0" width="100" height="40" fill="#0af"/></x:warp></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("M70,0"), "{out}"); // top edge (20 above mid) → x = 50+20
    assert!(!out.contains("NaN"), "{out}");
    // missing/degenerate spine degrades behind the usual marker
    let svg = format!(
        r##"{XW}<x:warp field="bend" in="#nope"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(out.contains("unknown field"), "{out}");
    assert!(out.contains("h100"), "{out}");
}

#[test]
fn warp_roughen_is_deterministic_and_bounded() {
    let svg = format!(
        r##"{XW}<x:warp field="roughen" bend="60" detail="12"><rect x="0" y="0" width="200" height="80" fill="#f00"/></x:warp></svg>"##
    );
    let a = compile_fast(&svg);
    let b = compile_fast(&svg);
    assert_eq!(a, b, "roughen must be deterministic");
    assert!(a.contains("<path"), "{a}");
    assert!(!a.contains("NaN"), "{a}");
    // it actually roughens: the output differs from the unwarped rect
    assert!(!a.contains("h200"), "{a}");
}

#[test]
fn textpath_ribbon_warps_and_degrades_like_skew() {
    // flat path → ribbon behaves like skew: baseline lands on y = 20
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="ribbon" font-size="20" fill="#111">wave</x:textpath></svg>"##;
    let out = compile_outlined(svg);
    assert!(out.contains("<path d=\"M0,20"), "{out}");
    // without a font it degrades to the stepped baseline, like skew
    let out = compile_test(svg);
    assert!(out.contains(r#"x="0 10 20 30""#), "{out}");
    assert!(out.contains(r#"y="20 20 20 20""#), "{out}");
}

#[test]
fn textpath_follow_lowers_to_native_textpath() {
    // live SVG <textPath> — no font needed; align=middle places startOffset at
    // (arclen 120 − advance 40)/2 = 40; baseline-shift rides along
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="wavy" d="M0,20 L120,20" fill="none"/><x:textpath in="#wavy" effect="follow" align="middle" baseline-shift="5" font-size="20" fill="#111">wave</x:textpath></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains("<textPath href=\"#wavy\""), "{out}");
    assert!(out.contains("startOffset=\"40\""), "{out}");
    assert!(out.contains("baseline-shift=\"5\""), "{out}");
    assert!(out.contains(">wave</textPath></text>"), "{out}");
    // a degenerate path → straight fallback, no <textPath>
    let degen = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><path id="p" d="M5,5 L5,5" fill="none"/><x:textpath in="#p" effect="follow" font-size="20">hi</x:textpath></svg>"##;
    let out = compile_test(degen);
    assert!(!out.contains("<textPath"), "{out}");
    assert!(out.contains(">hi</text>"), "{out}");
}

#[test]
fn warp_unknown_field_marks_and_passes_through() {
    let svg = format!(
        r##"{XW}<x:warp field="bogus" bend="50"><rect x="0" y="0" width="100" height="40"/></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("unknown field"), "{out}");
    assert!(out.contains("h100"), "child not passed through: {out}");
}

#[test]
fn warp_quality_grades_output_form() {
    // every profile emits the tolerance-graded polyline (refit is disabled —
    // §7.1): tighter tolerance → more segments, and never any cubics
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="100"><rect x="0" y="0" width="200" height="60"/></x:warp></svg>"##
    );
    let fast = compile_impl(&svg, "fast", false, &Mono, &NoShaper, &NoOutliner).unwrap();
    let hi = compile_impl(&svg, "highest", false, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(!fast.contains('C') && !hi.contains('C'), "{hi}");
    let (nf, nh) = (
        first_path(&fast).elements().len(),
        first_path(&hi).elements().len(),
    );
    assert!(nh > nf, "highest ({nh}) !> fast ({nf})");
}

#[test]
fn warp_nested_composes() {
    // the inner warp bakes first (recursive serialize), the outer re-bakes its
    // emitted paths — nesting composes innermost-first
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="40"><x:warp field="flag" bend="40"><rect x="0" y="0" width="100" height="30"/></x:warp></x:warp></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.matches("<g").count() >= 2, "{out}");
    assert!(out.contains("<path"), "{out}");
    assert!(!out.contains("skipped"), "{out}");
}

#[test]
fn warp_degenerate_input_never_panics() {
    // no children / no geometry / garbage bend / rounded rect (unwarpable)
    for svg in [
        format!(r##"{XW}<x:warp field="arch" bend="50"></x:warp></svg>"##),
        format!(
            r##"{XW}<x:warp field="arch" bend="garbage"><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##
        ),
        format!(
            r##"{XW}<x:warp field="wave" bend="100"><rect x="0" y="0" width="10" height="10" rx="3"/></x:warp></svg>"##
        ),
        format!(r##"{XW}<x:warp><rect x="0" y="0" width="10" height="10"/></x:warp></svg>"##),
    ] {
        let out = compile_test(&svg);
        assert!(!out.contains("NaN"), "{out}");
    }
}

#[test]
fn xml_special_chars_are_escaped() {
    // text content and attribute values must be entity-escaped.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><text x="0" y="10" aria-label="a &quot;b&quot; &amp; c">x &lt; y &amp; z</text></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("x &lt; y &amp; z"), "text not escaped: {out}");
    assert!(
        out.contains("&quot;b&quot;") && out.contains("&amp; c"),
        "attr not escaped: {out}"
    );
}

// ---- source map (data-xsvg-pos) ----

#[test]
fn sourcemap_off_by_default() {
    // The default compile (sourcemap=false) must not pollute the output.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect x="10" y="20" width="30" height="40"/></svg>"#;
    assert!(!compile_test(svg).contains("data-xsvg-pos"));
}

#[test]
fn sourcemap_tags_elements_with_valid_source_ranges() {
    // A passthrough <rect>→<path>, a synthesized <x:textbox>→<text>, plus the
    // <svg> root all get tagged; every range is well-formed and in-bounds, and
    // maps back to the authoring node's markup.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><rect id="r" x="10" y="20" width="30" height="40"/><x:textbox x="0" y="0" width="80" height="20" font-size="10">hi there</x:textbox></svg>"##;
    let out = compile_mapped(svg);
    let n = svg.len();

    let ranges: Vec<(usize, usize)> = out
        .split("data-xsvg-pos=\"")
        .skip(1)
        .map(|s| {
            let r = s.split('"').next().unwrap();
            let (a, b) = r.split_once('-').unwrap();
            (a.parse().unwrap(), b.parse().unwrap())
        })
        .collect();

    // svg root + rect + textbox
    assert!(ranges.len() >= 3, "expected >=3 tagged elements: {out}");
    for &(a, b) in &ranges {
        assert!(a < b && b <= n, "bad range {a}-{b} (len {n}): {out}");
    }
    // ranges point back at the authoring nodes
    let slices: Vec<&str> = ranges.iter().map(|&(a, b)| &svg[a..b]).collect();
    assert!(
        slices.iter().any(|s| s.contains("rect")),
        "no range covers the <rect>: {slices:?}"
    );
    assert!(
        slices.iter().any(|s| s.contains("x:textbox")),
        "no range covers the <x:textbox>: {slices:?}"
    );
}
