use super::*;
use crate::test_doubles::{BoxOutliner, BoxShaper, Mono, NoOutliner, NoShaper};

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
fn connector_anchors_turn_with_a_rotated_target() {
    // b is a 40×40 square rotated 90° about its centre (140, 20): its own
    // top-edge midpoint (140, 0) turns to (160, 20) — the right side of the
    // axis-aligned bounds, where the shape's top edge now lies. An anchored
    // end attaches there, not at the bounds' top midpoint (140, 0).
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40" transform="rotate(90 140 20)"/><x:connector from="#a" to="#b:top" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L160,20"),
        "attaches at the turned top midpoint: {}",
        route_d(&out)
    );
    // a half turn: b and c are zero, yet the shape's top is at the bottom of
    // its bounds — rotate(180 140 20) maps (140, 0) to (140, 40)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40" transform="rotate(180 140 20)"/><x:connector from="#a" to="#b:top" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L140,40"),
        "a half turn puts top at the bottom: {}",
        route_d(&out)
    );
    // without an anchor the bounds still decide (the centre ray clips at them)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40" transform="rotate(90 140 20)"/><x:connector from="#a" to="#b" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L120,20"),
        "unanchored: the bounds' edge: {}",
        route_d(&out)
    );
}

#[test]
fn connector_anchors_on_an_ellipse_lie_on_its_circumference() {
    // a circle of radius 20 at (140, 20): its top anchor is (140, 0), as on a
    // box; its left-top anchor is on the circumference at 45° — (140 − 20/√2,
    // 20 − 20/√2) ≈ (125.86, 5.86), not the box corner (120, 0)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><circle id="b" cx="140" cy="20" r="20"/><x:connector from="#a" to="#b:left-top" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    let d = route_d(&out);
    let end = d.rsplit('L').next().unwrap_or("");
    let (x, y) = end
        .split_once(',')
        .map(|(a, b)| (a.parse::<f64>().unwrap(), b.parse::<f64>().unwrap()))
        .unwrap();
    assert!(
        (x - 125.858).abs() < 0.01 && (y - 5.858).abs() < 0.01,
        "on the circumference at 45°: {d}"
    );
}

#[test]
fn connectors_accept_points_and_side_anchors() {
    // to-point: the connector ends at a raw x,y — no target element needed
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><x:connector from="#a" to-point="200,150" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L200,150"),
        "ends at the given point: {}",
        route_d(&out)
    );

    // side anchor: `#b:left` forces the left-edge midpoint of b (x=120..160, y=0..40 → 120,20)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:left" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L120,20"),
        "attaches at b's left-edge midpoint: {}",
        route_d(&out)
    );

    // a forced side on `from` too: leaves a's bottom-edge midpoint (20,40)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a:bottom" to="#b" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).starts_with("M20,40"),
        "leaves a's bottom-edge midpoint: {}",
        route_d(&out)
    );

    // corner anchor: b's right-bottom corner is (x1,y1) = (160,40); order-independent
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:bottom-right" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L160,40"),
        "corner anchor (either token order): {}",
        route_d(&out)
    );

    // center anchor: b's center (140,20)
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b:center" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        route_d(&out).ends_with("L140,20"),
        "center anchor: {}",
        route_d(&out)
    );

    // the ref wins when both a ref and a point are given
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:connector from="#a" to="#b" to-point="500,500" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        !route_d(&out).contains("500"),
        "the `to` ref takes precedence over to-point: {}",
        route_d(&out)
    );
}

#[test]
fn x_major_rail_flips_to_a_4_turn_detour_when_the_target_sits_back() {
    use crate::kurbo::Shape;
    // a:right → b:left with b to the RIGHT: the exits face each other with room, so a
    // 2-turn Z (M + 3 L) — and the mid-x crossing lies between the edges.
    let svg = format!(
        r##"{XW}<rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="60" width="40" height="40"/><x:connector from="#a:right" to="#b:left" route="x-major" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        route_d(&out).matches('L').count(),
        3,
        "2-turn Z when facing with room: {}",
        route_d(&out)
    );

    // b's left edge NOT to the right of a's right edge (here: directly behind, overlapping
    // rows) → the rail must flip to a 4-turn detour (M + 5 L) and route AROUND the boxes,
    // never doubling back through them. a:right=40, b at x=0..40 → b:left=0 ≤ 40.
    let svg = format!(
        r##"{XW}<rect id="a" x="120" y="0" width="40" height="40"/><rect id="b" x="0" y="0" width="40" height="40"/><x:connector from="#a:right" to="#b:left" route="x-major" arrow="none"/></svg>"##
    );
    let out = compile_test(&svg);
    let d = route_d(&out);
    assert_eq!(
        d.matches('L').count(),
        5,
        "4-turn detour when the target sits back: {d}"
    );
    assert!(d.starts_with("M160,20"), "still exits a's right edge: {d}");
    assert!(d.ends_with("L0,20"), "still enters b's left edge: {d}");
    // routes around: the path clears the boxes' shared row (y 0..40) on one side
    let bb = crate::kurbo::BezPath::from_svg(d).unwrap().bounding_box();
    assert!(
        bb.y0 < 0.0 || bb.y1 > 40.0,
        "detour clears the boxes vertically: {bb:?}"
    );
    // and the first move out of a is rightward (the +stub), not back toward b
    assert!(
        d.contains("M160,20 L178,20"),
        "stubs OUT of a before turning: {d}"
    );
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
    assert!(
        out.contains("font-size=\"20\""),
        "the list's size on the big item's <text>: {out}"
    );
    // each item is its own <text> at ITS size — the smaller item's lines are
    // bare positioned tspans under a font-size="10" <text>
    assert!(
        out.contains("font-size=\"10\"") && out.contains(">small</tspan>"),
        "the smaller item's <text> carries the size: {out}"
    );
    assert!(
        !out.contains("font-size=\"10\">small"),
        "no per-line size override: {out}"
    );
}

#[test]
fn list_items_render_their_runs() {
    // a bold run inside a bullet renders styled — items are run leaves, not
    // flattened text — and the plain neighbours stay bare
    let svg = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10"><x:li>keep <tspan font-weight="700">bold</tspan> tail</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r#">keep <tspan font-weight="700">bold</tspan> tail</tspan>"#),
        "{out}"
    );
}

#[test]
fn table_cells_render_runs_and_forced_breaks() {
    // Mono size 10, line-height 1.2, padding 0: a cell with a <tbreak/> is two
    // rows tall (cap 7 + advance 12 + descent 2 = 21) and its bold run renders
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="200" cell-padding="0" font-size="10" line-height="1.2"><x:tr><x:td>one<tbreak/><tspan font-weight="700">two</tspan></x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("height=\"21\""),
        "forced break grows the row: {out}"
    );
    assert!(
        out.contains(r#"<tspan font-weight="700">two</tspan>"#),
        "{out}"
    );
    assert!(
        out.contains("y=\"7\">one</tspan>") && out.contains("y=\"19\">"),
        "{out}"
    );
}

#[test]
fn tbreak_in_a_list_item_breaks_within_the_bullet() {
    // Mono size 10, line-height 1.2 → advance 12, item-spacing 3.5, offset 7:
    // the forced line stays inside item 1 (one keyed <text>, rows 7 and 19),
    // and item 2 stacks off item 1's LAST row (19 + 12 + 3.5 + 7−7 = 34.5)
    let svg = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10" line-height="1.2"><x:li>alpha<tbreak/>beta</x:li><x:li>gamma</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"7\">alpha</tspan>") && out.contains("y=\"19\">beta</tspan>"),
        "forced break inside the item, one advance apart: {out}"
    );
    assert!(
        out.contains("y=\"34.5\">gamma</tspan>"),
        "the next item stacks off the LAST forced row: {out}"
    );
    assert_eq!(
        out.matches("<circle").count(),
        2,
        "one marker per ITEM: {out}"
    );
}

#[test]
fn empty_list_item_keeps_its_row_and_marker() {
    // an empty <x:li> is a real row: it advances the flow by its own line
    // height, and its marker sits on THAT row (an editor's Enter-at-end
    // creates one; the bullet must appear on the new line, not at the top)
    let svg = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10" line-height="1.2"><x:li>a</x:li><x:li></x:li><x:li>b</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    // rows: baseline 0 / 15.5 / 31 (advance 12 + item-spacing 3.5), offset 7
    // (cap height); marker cy = baseline + 7 − x-height/2
    assert!(out.contains("cy=\"4.5\""), "first item's marker row: {out}");
    assert!(
        out.contains("cy=\"20\""),
        "the EMPTY item's marker on its own row, between its neighbors: {out}"
    );
    assert!(
        out.contains("cy=\"35.5\""),
        "third item's marker row: {out}"
    );
    // the empty item emits no <text>: two keyed texts, three markers
    assert_eq!(out.matches("<circle").count(), 3, "{out}");
    assert!(
        out.contains("y=\"7\">a</tspan>") && out.contains("y=\"38\">b</tspan>"),
        "{out}"
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
    // 20-unit paragraph-spacing: p0 baseline at cap (7); p1's baseline one
    // LINE ADVANCE below it plus the gap (the Slides convention — Enter never
    // spaces tighter than a wrapped line): 7 + 20 + 12 = 39.
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
        out.contains("y=\"39\""),
        "second paragraph one advance + gap down: {out}"
    );
    assert!(
        out.contains("text-anchor=\"middle\""),
        "per-paragraph align: {out}"
    );
    // the textbox keeps its OWN rendered identity: one <g> wraps the paragraph
    // <text>s (otherwise a multi-paragraph textbox has no element — no bbox —
    // and its paragraphs get promoted to free-standing objects)
    assert!(
        out.contains("<g>") && out.contains("</text></g>"),
        "a <g> wraps the paragraphs: {out}"
    );
    let sm = compile_mapped(&svg);
    // root + textbox wrapper + one per paragraph = 4 mapped elements
    assert!(
        sm.contains("<g data-xsvg-pos=") && sm.matches("data-xsvg-pos").count() == 4,
        "the wrapper carries the TEXTBOX's source map, the texts their <x:p>s': {sm}"
    );
}

#[test]
fn tbreak_in_a_textbox_breaks_at_the_line_advance() {
    // Mono size 10, line-height 1.5 → advance 15: a wrapped line and a
    // <tbreak/> line must step by the SAME advance (the break used to degrade
    // to a word space in textboxes — no break at all)
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="10" line-height="1.5">alpha<tbreak/>beta</x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"7\">alpha</tspan>") && out.contains("y=\"22\">beta</tspan>"),
        "forced break, one line advance apart: {out}"
    );
    // consecutive breaks open a blank line: gamma lands two advances down
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="10" line-height="1.5">alpha<tbreak/><tbreak/>gamma</x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"37\">gamma</tspan>"),
        "blank forced line occupies its row: {out}"
    );
}

#[test]
fn tbreak_inside_a_paragraph_breaks_within_it() {
    // paragraph mode used to DROP <tbreak/> entirely (collect_text skipped
    // elements); now it forces a line within the paragraph, and the next
    // paragraph still stacks one advance below the LAST line
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="10" line-height="1.2"><x:p>alpha<tbreak/>beta</x:p><x:p>gamma</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"7\">alpha</tspan>") && out.contains("y=\"19\">beta</tspan>"),
        "forced line inside the paragraph: {out}"
    );
    assert!(
        out.contains("y=\"34.5\">gamma</tspan>"),
        "next paragraph one advance + default paragraph gap (0.35em) below: {out}"
    );
}

#[test]
fn empty_paragraph_owns_a_blank_row() {
    // Enter Enter: the empty <x:p> renders nothing but takes one line advance,
    // so the following paragraph sits two advances down (a visible blank line)
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="10" line-height="1.2"><x:p>alpha</x:p><x:p></x:p><x:p>beta</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(
        out.matches("<text ").count(),
        3,
        "the empty para's <text> holds its blank line: {out}"
    );
    assert!(
        out.contains("y=\"7\">alpha</tspan>") && out.contains("y=\"38\">beta</tspan>"),
        "the blank row: two advances + two default gaps (7 + 2·12 + 2·3.5): {out}"
    );
    // the blank line sits on its row, one advance and a gap below alpha — the
    // caret's home, and where an empty text measures
    assert!(out.contains("<tspan x=\"0\" y=\"22.5\"></tspan>"), "{out}");
}

#[test]
fn an_empty_box_lays_one_blank_line_where_typing_starts() {
    // an empty textbox — its own box, a referenced rect, an empty paragraph,
    // `<text inline-size>` — emits its <text> with one blank line at the first
    // line's position, placed by align/valign like text would be; the layout
    // seam reports the same line, so a caret has a home and the empty element
    // measures where typing starts instead of at the origin
    let svg = format!(
        r##"{XW}<rect id="r" x="20" y="200" width="100" height="50"/><x:textbox x="10" y="10" width="200" height="60" font-size="10"></x:textbox><x:textbox in="#r" font-size="10"></x:textbox><x:textbox x="10" y="10" width="200" height="60" font-size="10" align="center" valign="middle"></x:textbox><x:textbox x="10" y="10" width="200" height="60" font-size="10"><x:p></x:p></x:textbox><text x="5" y="40" inline-size="100" font-size="10"></text></svg>"##
    );
    let out = compile_test(&svg);
    // cap height 7 below the top at size 10 (Mono); the rect box at its own top
    assert!(out.contains("<tspan x=\"10\" y=\"17\"></tspan>"), "{out}");
    assert!(out.contains("<tspan x=\"20\" y=\"207\"></tspan>"), "{out}");
    // centred: x at the middle, the cap band (7 + 2) centred in 60 → cap-top at 35.5
    assert!(
        out.contains("<tspan x=\"110\" y=\"42.5\"></tspan>"),
        "{out}"
    );
    // inline-size text: first baseline at y, as always
    assert!(out.contains("<tspan x=\"5\" y=\"40\"></tspan>"), "{out}");
    assert_eq!(out.matches("<tspan").count(), 5, "{out}");
    for (i, (x, y)) in [(10.0, 17.0), (20.0, 207.0), (110.0, 42.5), (10.0, 17.0)]
        .into_iter()
        .enumerate()
    {
        let l = layout_at(&svg, nth_at(&svg, "<x:textbox", i)).unwrap();
        let lines = &l.paras[0].lines;
        assert_eq!(lines.len(), 1, "case {i}: {l:?}");
        assert!(
            lines[0].text.is_empty()
                && (lines[0].x - x).abs() < 1e-9
                && (lines[0].baseline - y).abs() < 1e-9,
            "case {i}: {:?}",
            lines[0]
        );
    }
}

#[test]
fn paragraph_styled_runs_render_styled() {
    // <tspan> runs inside an <x:p> keep their overrides in the output (they
    // used to be flattened to the base style in paragraph mode)
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="10"><x:p>plain <tspan font-weight="800">bold</tspan></x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r#"<tspan font-weight="800">bold</tspan>"#),
        "the run's override survives into the paragraph's <text>: {out}"
    );
}

#[test]
fn textarea_line_height_drives_auto_line_increment() {
    // authored line-height (em) wins over the SVG Tiny 1.1×size default when
    // line-increment is auto — <tbreak/> finally respects our line spacing
    let svg = format!(
        r##"{XW}<textArea x="0" y="0" width="200" height="100" font-size="10" line-height="1.5">alpha<tbreak/>beta</textArea></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"7\">alpha</tspan>") && out.contains("y=\"22\">beta</tspan>"),
        "step = size × line-height: {out}"
    );
    // an explicit line-increment keeps SVG Tiny semantics (wins over both)
    let svg = format!(
        r##"{XW}<textArea x="0" y="0" width="200" height="100" font-size="10" line-height="1.5" line-increment="20">alpha<tbreak/>beta</textArea></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("y=\"27\">beta</tspan>"),
        "explicit line-increment wins: {out}"
    );
}

#[test]
fn textbox_paragraphs_shrink_to_fit() {
    // Mono size 20: cap 14, descent 4 → two 1-line paragraphs stack to
    // 18+18=36, over the 30-high box → fit="shrink" scales the block down
    // (found live: pasting newlines into a shrink box exploded the text,
    // because paragraph mode ignored fit entirely).
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="30" font-size="20" line-height="1" fit="shrink" fit-min="10"><x:p>aa</x:p><x:p>bb</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    let sizes: Vec<f64> = out
        .match_indices("font-size=\"")
        .map(|(i, _)| {
            let rest = &out[i + 11..];
            rest[..rest.find('"').unwrap()].parse().unwrap()
        })
        .collect();
    assert_eq!(sizes.len(), 2, "one <text> per paragraph: {out}");
    for s in &sizes {
        assert!(
            *s < 20.0 && *s >= 10.0,
            "shrunk within [fit-min, base): {sizes:?} {out}"
        );
    }
    assert!(
        (sizes[0] - sizes[1]).abs() < 1e-6,
        "the block scales as one: {sizes:?}"
    );

    // a block that already fits stays at the base size
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="100" font-size="20" line-height="1" fit="shrink" fit-min="10"><x:p>aa</x:p><x:p>bb</x:p></x:textbox></svg>"##
    );
    let fits = compile_test(&svg);
    assert!(
        fits.contains("font-size=\"20\""),
        "no shrink when it fits: {fits}"
    );

    // a lone unbreakable word wider than the box shrinks too (the §6.6
    // overflow is for non-fit boxes): Mono 20 → "wwwwwwwwww" = 100 wide in a
    // 60-wide box; plenty of height, so only the WIDTH forces the shrink
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="200" font-size="20" line-height="1" fit="shrink" fit-min="5"><x:p>wwwwwwwwww</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    let size: f64 = {
        let i = out.find("font-size=\"").unwrap() + 11;
        out[i..i + out[i..].find('"').unwrap()].parse().unwrap()
    };
    assert!(
        size <= 12.0 + 1e-6 && size > 5.0,
        "10 mono chars at size s are s/2·10 wide → fits 60 at s ≤ 12: {size} {out}"
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
    // so a viewer resolves a click to the cell, not the whole table — while the
    // wrapper still carries the table's own span, which is the range an incremental
    // re-emit replaces (docs/Incremental.md §4)
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="100"><x:tr><x:td>a</x:td><x:td>b</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_impl(&svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert_eq!(
        out.matches("<g data-xsvg-pos").count(),
        3,
        "the table's wrapper plus one source-mapped <g> per cell: {out}"
    );
    let table = top_level_offsets(&svg)[0];
    let range = fragment_range_impl(&svg, table).unwrap();
    assert!(
        out.contains(&format!(r#"<g data-xsvg-pos="{}-{}">"#, range.0, range.1)),
        "the wrapper carries the very range `fragment_range` reports: {out}"
    );
}

#[test]
fn table_column_and_row_styles_resolve_cell_row_column_table() {
    // <x:col> styles a column, <x:tr> attrs style a row, a cell wins over both
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="200" cols="* *"><x:col font-weight="600" fill="#111111"/><x:col align="end"/><x:tr><x:td>a</x:td><x:td>b</x:td></x:tr><x:tr font-weight="300" fill="#222222"><x:td>c</x:td><x:td fill="#333333">d</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&svg);
    let texts: Vec<&str> = out.split("<text ").skip(1).collect();
    assert_eq!(texts.len(), 4, "{out}");
    assert!(
        texts[0].contains("font-weight=\"600\"") && texts[0].contains("fill=\"#111111\""),
        "column 0 bold and tinted: {}",
        texts[0]
    );
    assert!(
        texts[1].contains("text-anchor=\"end\"") && !texts[1].contains("font-weight=\"600\""),
        "column 1 right-aligned, not bold: {}",
        texts[1]
    );
    assert!(
        texts[2].contains("font-weight=\"300\"") && texts[2].contains("fill=\"#222222\""),
        "the row overrides the column: {}",
        texts[2]
    );
    assert!(
        texts[3].contains("fill=\"#333333\"") && texts[3].contains("font-weight=\"300\""),
        "the cell overrides the row, keeps the rest: {}",
        texts[3]
    );
}

#[test]
fn table_and_list_forward_their_transform() {
    let svg = format!(
        r##"{XW}<x:table x="0" y="0" width="60" transform="rotate(10 30 10)"><x:tr><x:td>a</x:td></x:tr></x:table><x:list x="0" y="40" width="60" transform="translate(5 5)"><x:li>i</x:li></x:list></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r#"<g transform="rotate(10 30 10)">"#)
            || out.contains(r#" transform="rotate(10 30 10)">"#),
        "the table's wrapper carries its transform: {out}"
    );
    assert!(
        out.contains(r#" transform="translate(5 5)">"#),
        "the list's wrapper carries its transform: {out}"
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
        4,
        "the pie's wrapper plus a per-slice source map: {sm}"
    );
    let pie = top_level_offsets(&svg)[0];
    let range = fragment_range_impl(&svg, pie).unwrap();
    assert!(
        sm.contains(&format!(r#"<g data-xsvg-pos="{}-{}">"#, range.0, range.1)),
        "the wrapper carries the very range `fragment_range` reports: {sm}"
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
fn charts_keep_transforms_in_full_and_incremental_output() {
    let charts = [
        (
            "pie",
            r#"cx="30" cy="40" r="20""#,
            r#"<x:slice value="1"/><x:slice value="2"/>"#,
        ),
        (
            "plot",
            r#"x="10" y="20" width="100" height="80" y-ticks="2""#,
            r#"<x:bars><x:bar value="50" label="A"/></x:bars><x:line points="0,10 1,50" marker="dot" area="red"/>"#,
        ),
    ];
    for (tag, attrs, children) in charts {
        for transform in [
            "translate(35 -12)",
            "rotate(30 50 40)",
            "scale(1.5 0.75)",
            "translate(10 20) rotate(25) scale(2)",
        ] {
            let src = format!(
                r#"{XW}<g transform="translate(7 9)"><x:{tag} {attrs} transform="{transform}">{children}</x:{tag}></g></svg>"#
            );
            let offset = src.find(&format!("<x:{tag}")).unwrap();
            let source_doc = roxmltree::Document::parse(&src).unwrap();
            let range = source_doc
                .descendants()
                .find(|n| n.is_element() && n.range().start == offset)
                .unwrap()
                .range();
            let expected_pos = format!("{}-{}", range.start, range.end);
            for keyed in [false, true] {
                let full =
                    compile_impl(&src, "balanced", keyed, &Mono, &NoShaper, &NoOutliner).unwrap();
                let fragment = compile_fragment_impl(
                    &src,
                    "balanced",
                    keyed,
                    offset,
                    &Mono,
                    &NoShaper,
                    &NoOutliner,
                )
                .unwrap();
                for output in [&full, &fragment] {
                    let wrapped =
                        format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{output}</svg>"#);
                    let doc = roxmltree::Document::parse(&wrapped).unwrap();
                    let groups: Vec<_> = doc
                        .descendants()
                        .filter(|n| n.attribute("transform") == Some(transform))
                        .collect();
                    assert_eq!(
                        groups.len(),
                        1,
                        "{tag}: exactly one chart transform: {output}"
                    );
                    let group = groups[0];
                    assert_eq!(group.tag_name().name(), "g");
                    assert!(
                        group.children().any(|n| n.is_element()),
                        "chart transform wraps its artwork"
                    );
                    if keyed {
                        assert_eq!(
                            group.attribute("data-xsvg-pos"),
                            Some(expected_pos.as_str())
                        );
                    }
                    if std::ptr::eq(output, &full) {
                        assert_eq!(
                            group.parent_element().unwrap().attribute("transform"),
                            Some("translate(7 9)")
                        );
                    }
                }
            }
        }
    }
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
fn layer_identity_is_editor_metadata_only() {
    let svg = format!(
        r#"{XW}<g x:layer="foreground"><rect width="10" height="10"/></g><g><circle r="2"/></g></svg>"#
    );
    let edit = compile_impl(&svg, "fast", true, &Mono, &NoShaper, &BoxOutliner).unwrap();
    assert_eq!(edit.matches("data-xsvg-layer=\"true\"").count(), 1);
    assert!(!compile_test(&svg).contains("data-xsvg-layer"));
}

#[test]
fn shared_layer_keys_keep_artboard_instances_local() {
    let svg = format!(
        r##"{XW}<g x:artboard="A" transform="translate(10 20)"><g id="a" x:layer="content" x:layer-key="notes" x:order="2"><rect width="9" height="9" fill="#aa0000"/></g><g x:layer="content" x:order="1"><rect width="9" height="9" fill="#00aa00"/></g></g><g x:artboard="B" transform="translate(100 200)"><g id="b" x:layer="content" x:layer-key="notes" x:hidden="true"><rect width="9" height="9" fill="#0000aa"/></g></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("#aa0000") && !out.contains("#0000aa"), "{out}");
    assert!(
        out.find("#00aa00").unwrap() < out.find("#aa0000").unwrap(),
        "{out}"
    );
    assert!(
        out.contains("translate(10 20)") && out.contains("translate(100 200)"),
        "{out}"
    );
    assert!(!out.contains("layer-key"), "{out}");
    let visible = compile_test(&svg.replace(r#"x:hidden="true""#, r#"x:hidden="false""#));
    assert!(
        visible.contains("#aa0000") && visible.contains("#0000aa"),
        "{visible}"
    );
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
    assert!((5.0..40.0).contains(&size), "expected shrink, got {size}");
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
fn document_settings_compile_to_nothing() {
    // §4.4: <x:doc> and everything inside it is an editor's business — dropped
    // whole, no marker, whatever the children are (known or not); the
    // sibling content is untouched
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:doc><x:artboards width="720" height="405" fill="#fff" columns="4" gap="120 60"/><x:sparkle/><rect width="9" height="9"/></x:doc><rect width="1" height="1"/></svg>"##;
    let out = compile_test(svg);
    assert!(
        !out.contains("xsvg:"),
        "no marker for document settings: {out}"
    );
    assert!(
        !out.contains("artboards") && !out.contains("h9"),
        "nothing inside is emitted: {out}"
    );
    assert!(out.contains("h1 v1"), "siblings compile as usual: {out}");
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
fn justify_widens_the_spaces_of_full_lines() {
    // Mono: char = 0.5 × size. At width 40, size 10: "aaa bb" (30 wide, one space,
    // so 10 of slack into it), "cc dd ee" (exactly full: nothing to add), "ff" last.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><textArea x="0" y="0" width="40" font-size="10" text-align="justify">aaa bb cc dd ee ff</textArea></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains(r#"text-anchor="start""#), "{out}");
    assert!(out.contains(r#" word-spacing="10">aaa bb"#), "{out}");
    assert!(
        out.contains(r#"<tspan x="0" y="18">cc dd ee"#),
        "an exactly full line needs no widening: {out}"
    );
    assert!(
        out.contains(r#"<tspan x="0" y="29">ff"#),
        "the last line stays ragged: {out}"
    );
    // no textLength anywhere: a stretched line's decorations render per glyph,
    // dashed across the stretched gaps (found live)
    assert!(!out.contains("textLength"), "{out}");
    // an authored word-spacing is the base the slack adds to
    let spaced = svg.replace(r#"font-size="10""#, r#"font-size="10" word-spacing="4""#);
    let out = compile_test(&spaced);
    assert!(
        out.contains(r#" word-spacing="10">aaa bb"#),
        "4 authored + 6 slack: {out}"
    );
    // start alignment (no justify) emits no per-line word-spacing
    let plain = svg.replace(" text-align=\"justify\"", "");
    assert!(!compile_test(&plain).contains("word-spacing"));
}

#[test]
fn justify_on_textbox_uses_content_width() {
    // width 60, padding 10 → content width 40 is the justify target: "aaa bb" is
    // 30 wide with one space, so that space takes the 10 of slack
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="60" height="200" padding="10" align="justify" font-size="10">aaa bb cc dd ee ff</x:textbox></svg>"#;
    let out = compile_test(svg);
    assert!(out.contains(r#" word-spacing="10">aaa bb"#), "{out}");
    assert!(!out.contains("textLength"), "{out}");
    // the layout seam reports the per-space advance the line renders with, so a
    // caret on a justified line is exact prefix math
    let tl = layout_at(svg, svg.find("<x:textbox").unwrap()).unwrap();
    let lines = &tl.paras[0].lines;
    assert_eq!(lines[0].word_spacing, 10.0, "{lines:?}");
    assert_eq!(lines[1].word_spacing, 0.0, "{lines:?}");
    assert_eq!(lines[2].word_spacing, 0.0, "{lines:?}");
}

#[test]
fn justified_decorations_and_highlights_follow_the_widened_spaces() {
    // an underlined run spanning the justified line stays ONE tspan with no
    // textLength (the decoration draws continuously), and a highlight rect after
    // a space sits past the widened space, not the natural one
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="40" height="200" align="justify" font-size="10"><tspan text-decoration="underline">aaa bb</tspan> cc dd ee ff</x:textbox><x:textbox x="0" y="100" width="40" height="200" align="justify" font-size="10">aaa <tspan x:highlight="#ff0">bb</tspan> cc dd ee ff</x:textbox></svg>"##;
    let out = compile_test(svg);
    // the decoration is a rect (§6.11) as wide as the justified line — 30 natural
    // plus the 10 of slack — a tenth of an em under the baseline, no attribute
    assert!(
        out.contains(r##"<rect x="0" y="8" width="40" height="0.6" fill="#000"/>"##),
        "{out}"
    );
    assert!(
        !out.contains("text-decoration") && !out.contains("textLength"),
        "{out}"
    );
    // "aaa " is 20 wide naturally; its space widened by 10 puts "bb" at x = 30
    // (the rect's top is the baseline less the ascent)
    assert!(out.contains(r#"<rect x="30" y="99" width="10""#), "{out}");
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
fn text_decoration_bakes_rects_without_moving_layout() {
    // underline / line-through are paint-only: each becomes a rect behind the
    // text in the run's colour (§6.11) and the text markup is byte-identical to
    // the undecorated compile — decoration never touches advances or runs
    let plain = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10">Ship fast today</textArea></svg>"##;
    let deco = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10">Ship <tspan text-decoration="underline line-through">fast</tspan> today</textArea></svg>"##;
    let a = compile_test(plain);
    let b = compile_test(deco);
    // "Ship " is 25 wide, "fast" 20; the run ends at its last glyph (the following
    // inter-word space is base-styled, so the line stops at the word). Underline
    // top = baseline 7 + 0.1 em; strike center 2.5 above, 0.06 em thick.
    let underline = r##"<rect x="25" y="8" width="20" height="0.6" fill="#000"/>"##;
    let strike = r##"<rect x="25" y="4.2" width="20" height="0.6" fill="#000"/>"##;
    assert!(b.contains(&format!("{underline}{strike}<text")), "{b}");
    assert!(!b.contains("text-decoration"), "{b}");
    assert_eq!(
        b.replace(underline, "").replace(strike, ""),
        a,
        "decoration must not move a single glyph"
    );
    // between two words of ONE run the space stays in the run: one continuous rect
    let two = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10">Ship <tspan text-decoration="underline" fill="#f00">fast today</tspan></textArea></svg>"##;
    let out = compile_test(two);
    assert!(
        out.contains(r##"<rect x="25" y="8" width="50" height="0.6" fill="#f00"/>"##),
        "the run's own colour: {out}"
    );
}

#[test]
fn decoration_rects_span_letter_spacing_and_follow_wrapped_lines() {
    // letter-spacing 2 makes "ab" 12 wide and wraps the run word by word at 30:
    // one rect per line the run touches, each on its own baseline, each as wide as
    // the spaced advance — the case renderers draw dashed when left to them
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="30" height="60" font-size="10" letter-spacing="2" fill="#6366f1"><tspan text-decoration="underline">ab cd ef</tspan></x:textbox></svg>"##;
    let out = compile_test(svg);
    for (y, baseline) in [(8.0, 7.0), (20.0, 19.0), (32.0, 31.0)] {
        assert!(
            out.contains(&format!(
                r##"<rect x="0" y="{y}" width="12" height="0.6" fill="#6366f1"/>"##
            )),
            "{out}"
        );
        assert!(out.contains(&format!(r#"y="{baseline}""#)), "{out}");
    }
    assert_eq!(out.matches("<rect").count(), 3, "{out}");
    assert!(!out.contains("text-decoration"), "{out}");
}

#[test]
fn mixed_run_sizes_step_each_line_by_its_largest_run() {
    // base 10, line-height 1.5 (advance 15), cap 0.7·size. Line 1 carries a
    // 20px run: it stands 20 tall, so the FIRST baseline sits at cap(20) = 14
    // and line 2 (plain, 10) steps by ITS OWN advance 15 → 29. The run emits
    // its absolute size; "200%" resolves against the paragraph size.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="500" height="200" font-size="10" line-height="1.5">tall <tspan font-size="200%">BIG</tspan><tbreak/>small</x:textbox></svg>"##;
    let out = compile_test(svg);
    assert!(
        out.contains(r#"<tspan font-size="20">BIG</tspan>"#),
        "{out}"
    );
    assert!(
        out.contains(r#"<tspan x="0" y="14">tall "#),
        "first baseline = cap(20): {out}"
    );
    assert!(
        out.contains(r#"<tspan x="0" y="29">small</tspan>"#),
        "10-line advance: {out}"
    );
    // the other way round: the big line steps by 30 when it FOLLOWS
    let svg2 = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="500" height="200" font-size="10" line-height="1.5">small<tbreak/>tall <tspan font-size="20">BIG</tspan></x:textbox></svg>"##;
    let out2 = compile_test(svg2);
    assert!(
        out2.contains(r#"<tspan x="0" y="7">small</tspan>"#),
        "{out2}"
    );
    assert!(
        out2.contains(r#"<tspan x="0" y="37">tall "#),
        "7 + 20·1.5: {out2}"
    );
}

#[test]
fn run_sizes_scale_with_shrink_fit() {
    // shrink-fit halves everything: a 20px run in a 10px box fitted to 5
    // emits font-size 10 — the run rides the same scale as the base
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="1000" height="15" font-size="10" line-height="1" fit="shrink" fit-min="1">a<tbreak/><tspan font-size="20">b</tspan></x:textbox></svg>"##;
    let out = compile_test(svg);
    // the fit sums per-line advances: 10 (the a-line) + 20 (the b-line) = 30
    // at scale 1 > 15 → scale ½: base 5, and the run rides the same scale → 10
    assert!(out.contains(r#"font-size="5""#), "fitted base: {out}");
    assert!(
        out.contains(r#"<tspan font-size="10">b</tspan>"#),
        "run at 2× the fitted base: {out}"
    );
}

#[test]
fn baseline_shift_rides_to_the_output() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><textArea x="0" y="0" width="500" font-size="10">E=mc<tspan baseline-shift="super" font-size="58%">2</tspan></textArea></svg>"##;
    let out = compile_test(svg);
    assert!(
        out.contains(r#"<tspan font-size="5.8" baseline-shift="super">2</tspan>"#),
        "{out}"
    );
}

#[test]
fn lists_flow_inside_a_textbox_as_gutter_paragraphs() {
    // a heading paragraph, a two-item bullet list (one nested), a numbered
    // list, a closing paragraph — all in one box. Mono: size 10, line-height
    // 1.5, indent step 15 (1.5em), marker gap 5.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="300" height="300" font-size="10" line-height="1.5"><x:p>Agenda</x:p><x:list list="bullet"><x:li>one</x:li><x:li indent="1">two</x:li></x:list><x:list list="number"><x:li>first</x:li></x:list><x:p>end</x:p></x:textbox></svg>"##;
    let out = compile_test(svg);
    // the items' text columns sit at the indent step (15) and twice it (30)
    assert!(
        out.contains(r#"x="15" y="#) && out.contains(">one<"),
        "first item at the indent step: {out}"
    );
    assert!(
        out.contains(r#"x="30" y="#) && out.contains(">two<"),
        "nested item one step further: {out}"
    );
    // a bullet shape is drawn for each bullet item, a numbered marker as text
    assert!(
        out.matches("<circle").count()
            + out.matches("<rect").count()
            + out.matches("<path").count()
            >= 2,
        "drawn bullets: {out}"
    );
    assert!(
        out.contains(r#"text-anchor="end""#) && out.contains(">1.<"),
        "a numbered marker in the gutter: {out}"
    );
    // the paragraphs keep the content edge
    assert!(out.contains(r#"x="0" y="7">Agenda"#), "{out}");
    assert!(out.contains(">end<"), "{out}");
    // every paragraph and item carries its own source key; the marker <text> does not
    let keyed = compile_impl(svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert_eq!(
        keyed.matches("<text ").count(),
        6,
        "five keyed blocks and one marker text: {keyed}"
    );
}

#[test]
fn paragraphs_stack_by_the_incoming_lines_own_size() {
    // para 1 plain 10 (cap 7); para 2 opens with a 20px run: its first
    // baseline = 7 + 0.35·10 gap + 20·1.5 = 40.5, and its cap-top is 14 above
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="500" height="300" font-size="10" line-height="1.5"><x:p>one</x:p><x:p><tspan font-size="20">Two</tspan> two</x:p></x:textbox></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(r#"y="7">one"#), "{out}");
    assert!(
        out.contains(r#"y="40.5"><tspan font-size="20">Two</tspan> two"#),
        "{out}"
    );
}

#[test]
fn linked_runs_lower_to_svg_anchors() {
    // xsvg's run form: <tspan href> → <a href>text</a>; with paint overrides
    // the tspan nests inside the link. An authored <a> child reads the same.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="500" height="100" font-size="10">see <tspan href="https://xsvg.org/">here</tspan> or <a href="#s"><tspan font-weight="700">there</tspan></a> now</x:textbox></svg>"##;
    let out = compile_test(svg);
    assert!(out.contains(r##">see <a href="https://xsvg.org/">here</a> or <a href="#s"><tspan font-weight="700">there</tspan></a> now</tspan>"##), "{out}");
}

#[test]
fn highlighted_runs_bake_a_rect_behind_the_text() {
    // Mono (5 per char at size 10): "see " = 20, "mark" = 20; the em box at
    // size 10 is ascent 8 above / descent 2 below the baseline 7 → y −1,
    // h 10. The rect precedes the <text>; the run itself stays bare (no
    // tspan attribute carries the highlight).
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="500" height="100" font-size="10">see <tspan x:highlight="#ff0">mark</tspan> it</x:textbox></svg>"##;
    let out = compile_test(svg);
    let rect = r##"<rect x="20" y="-1" width="20" height="10" fill="#ff0"/>"##;
    assert!(out.contains(rect), "{out}");
    assert!(
        out.find(rect).unwrap() < out.find("<text").unwrap(),
        "behind the text: {out}"
    );
    assert!(
        out.contains(">see mark it</tspan>"),
        "the run itself is bare: {out}"
    );
    // a highlight behind a centred line starts from the line's start edge:
    // "ab" = 10 wide, centred in 100 → 45..55
    let centred = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="100" font-size="10" align="center"><tspan x:highlight="#ff0">ab</tspan></x:textbox></svg>"##;
    let out2 = compile_test(centred);
    assert!(
        out2.contains(r##"<rect x="45" y="-1" width="10" height="10" fill="#ff0"/>"##),
        "{out2}"
    );
}

#[test]
fn rtl_paragraphs_anchor_at_the_right_edge_and_emit_direction() {
    // direction="rtl": text-anchor stays "start" (the RIGHT side of rtl text
    // in SVG) but the anchor x moves to the right edge; align="end" mirrors
    // to the left edge; the <text> carries direction="rtl" so the browser
    // orders the glyphs. A highlight lays leftward from the start edge.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="100" font-size="10" direction="rtl">ab <tspan x:highlight="#ff0">cd</tspan></x:textbox></svg>"##;
    let out = compile_test(svg);
    assert!(
        out.contains(r#"<text text-anchor="start""#) && out.contains(r#" direction="rtl""#),
        "{out}"
    );
    assert!(
        out.contains(r#"<tspan x="100" y="7">ab cd</tspan>"#),
        "start = right edge: {out}"
    );
    // "ab " = 15 wide from the right (100 → 85), then "cd" = 10 → 75..85
    assert!(
        out.contains(r##"<rect x="75" y="-1" width="10" height="10" fill="#ff0"/>"##),
        "{out}"
    );
    let end = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:textbox x="0" y="0" width="100" height="100" font-size="10" direction="rtl" align="end"><x:p>ab</x:p><x:p direction="ltr">cd</x:p></x:textbox></svg>"##;
    let out2 = compile_test(end);
    assert!(out2.contains(r##"<text text-anchor="end" font-family="sans-serif" font-size="10" font-weight="normal" font-style="normal" fill="#000" direction="rtl""##), "{out2}");
    assert!(
        out2.contains(r#"<tspan x="0" y="7">ab</tspan>"#),
        "end = left edge under rtl: {out2}"
    );
    // the second paragraph opts back into ltr: end = right edge, no direction attr on it
    assert!(
        out2.contains(r#"<tspan x="100" y="22.5">cd</tspan>"#),
        "{out2}"
    );
    assert_eq!(out2.matches("direction=\"rtl\"").count(), 1, "{out2}");
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
            // a definition the compiler consumes emits nothing (§8.2)
            assert_eq!(frag.is_empty(), INC[off..].starts_with("<meshgradient"));
            let at = full[cursor..]
                .find(&frag)
                .unwrap_or_else(|| panic!("fragment not found in order: {frag}\n{full}"));
            cursor += at + frag.len();
        }
    }
}

/// Every composite emitter, so the rule below is checked against the elements that
/// build a wrapper of their own rather than passing one through.
const INC_COMPOSITE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" viewBox="0 0 400 300">
  <x:table x="0" y="0" width="100"><x:tr><x:th>h</x:th></x:tr><x:tr><x:td>a</x:td></x:tr></x:table>
  <x:pie cx="100" cy="100" r="40"><x:slice value="1"/><x:slice value="2"/></x:pie>
  <x:plot x="0" y="150" width="120" height="60" type="bar"><x:series values="1 2 3"/></x:plot>
  <x:list x="200" y="20" font-size="10"><x:li>one</x:li><x:li>two</x:li></x:list>
  <rect id="n1" x="200" y="100" width="40" height="20"/>
  <rect id="n2" x="300" y="160" width="40" height="20"/>
  <x:connector from="#n1:right" to="#n2:left" stroke="#111"/>
  <x:theme><x:color name="ink" value="#111"/></x:theme>
  <g x:layer="hidden" x:hidden="true"><rect x="0" y="0" width="10" height="10"/></g>
</svg>"##;

#[test]
fn generated_def_ids_are_all_prefixed_x() {
    // Ids the compiler invents for its own defs (`x-flt-…`, `x-mesh-…`, `x-mgc-…`)
    // seed on the node's byte start unless a host supplies `NodeKeys`, so they
    // *move* when an edit shifts earlier text. An incremental host must therefore be
    // able to tell a generated id from an authored one, and the `x-` prefix is the
    // whole signal it has. Adding a family without it would silently un-fix
    // https://…/Incremental.md §5 "stale generated ids" — hence this canary.
    let svg = format!(
        r##"{XW}<rect x="0" y="0" width="20" height="10" fill="#48a" filter="blur(2)"/><x:mesh points="0,40 40,40 40,80 0,80"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #ff0"/></x:mesh><meshgradient id="mg" x="0" y="100"><meshrow><meshpatch><stop path="l 40,0" stop-color="#e11"/><stop path="l 0,30" stop-color="#fa0"/><stop path="l -40,0" stop-color="#3b7"/><stop path="l 0,-30" stop-color="#06c"/></meshpatch></meshrow></meshgradient><rect x="0" y="100" width="40" height="30" fill="url(#mg)"/></svg>"##
    );
    let out = compile_impl(&svg, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    let authored: std::collections::HashSet<&str> = ["mg"].into_iter().collect();
    let mut generated = 0;
    for chunk in out.split(r#"id=""#).skip(1) {
        let id = &chunk[..chunk.find('"').unwrap()];
        if authored.contains(id) {
            continue;
        }
        assert!(
            id.starts_with("x-"),
            "generated id `{id}` is indistinguishable from an authored one: {out}"
        );
        generated += 1;
    }
    assert!(generated >= 3, "expected filter/mesh/clip ids: {out}");
}

#[test]
fn every_emitted_fragment_is_anchorable_by_its_own_range() {
    // The companion to the verbatim-slice invariant, and what makes surgical
    // replacement *addressable* rather than merely sound: a host holds the rendered
    // document and knows only source ranges, so the element a fragment emits has to
    // announce the range `fragment_range` reports for it. An emitter that wraps its
    // output in an untagged `<g>` is invisible to that lookup, and every edit to it
    // silently degrades to a whole-document recompile.
    //
    // Exactly one, not merely at least one: the range is the host's *key*. A
    // fragment may legitimately emit several sibling nodes — a `<filter>` def ahead
    // of the shape that uses it, marker comments ahead of both — and the host locates
    // that whole run from the one node carrying the key, so a second claimant would
    // make the lookup ambiguous.
    //
    // Elements that emit nothing at all (`<x:theme>` is a declaration; a hidden
    // layer is deliberately dropped) have no element to tag and are exempt.
    for input in [INC, INC_COMPOSITE] {
        for off in top_level_offsets(input) {
            let frag = compile_fragment_impl(
                input,
                "balanced",
                true,
                off,
                &Mono,
                &BoxShaper,
                &BoxOutliner,
            )
            .unwrap();
            let (start, end) = fragment_range_impl(input, off).unwrap();
            if !has_element(&frag) {
                continue; // nothing renderable emitted → nothing to key
            }
            let key = format!(r#" data-xsvg-pos="{start}-{end}""#);
            assert_eq!(
                frag.matches(&key).count(),
                1,
                "the fragment at {start}-{end} must carry its own range exactly once, \
                 or a host cannot address the element it has to replace: {frag}"
            );
        }
    }
}

/// Whether markup contains an element, as opposed to only comments/whitespace.
fn has_element(markup: &str) -> bool {
    let mut rest = markup.trim_start();
    while let Some(r) = rest.strip_prefix("<!--") {
        rest = r
            .split_once("-->")
            .map(|(_, t)| t)
            .unwrap_or("")
            .trim_start();
    }
    !rest.is_empty()
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
fn dependents_follow_a_connector_through_its_anchor_suffix() {
    // `from="#a:right"` names the box `#a` with a forced attachment point. The
    // reference the dependency scan sees is the whole string, so an id match that
    // did not look past the `:anchor` suffix would report no dependents at all —
    // and an editor moving the box would leave the connector drawn to where it
    // used to be.
    let svg = format!(
        r##"{XW}<rect id="a" x="10" y="10" width="40" height="20"/><rect id="b" x="120" y="80" width="40" height="20"/><x:connector from="#a:right" to="#b:left" route="x-major" stroke="#111"/></svg>"##
    );
    let offs = top_level_offsets(&svg);
    let starts = |deps: Vec<(usize, usize)>| deps.into_iter().map(|d| d.0).collect::<Vec<_>>();
    assert_eq!(starts(dependents_impl(&svg, offs[0])), vec![offs[2]]);
    assert_eq!(starts(dependents_impl(&svg, offs[1])), vec![offs[2]]);
    // Editing the connector itself pulls in nothing — the boxes do not read it.
    assert!(dependents_impl(&svg, offs[2]).is_empty());
    // And the plain, suffix-free spelling keeps working.
    let plain = format!(
        r##"{XW}<rect id="a" x="10" y="10" width="40" height="20"/><rect id="b" x="120" y="80" width="40" height="20"/><x:connector from="#a" to="#b" stroke="#111"/></svg>"##
    );
    let po = top_level_offsets(&plain);
    assert_eq!(starts(dependents_impl(&plain, po[0])), vec![po[2]]);
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
fn filters_on_text_get_a_region_from_where_the_glyphs_land() {
    // §8 on text: a live <text> used to take the ±50% fallback region, which a
    // single line's height cannot hold a shadow in. The region is now userSpace,
    // from the pen positions and measured advances (Mono: 0.5em per character,
    // em box 0.8 above / 0.2 below), padded by an em, grown by the spill.
    // "abcd" at 10px from (10, 20): [10, 30] × [12, 22]; pad 10; margin
    // 0.5 + 3·4 + max(2, 3) = 15.5 → x = -15.5, y = -13.5, w = 71, h = 61
    let svg = format!(
        r##"{XW}<text x="10" y="20" font-size="10" filter="drop-shadow(2 3 4 #123456)">abcd</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("filterUnits=\"userSpaceOnUse\""), "{out}");
    assert!(
        out.contains("x=\"-15.5\" y=\"-13.5\" width=\"71\" height=\"61\""),
        "{out}"
    );
    assert!(!out.contains("-50%"), "{out}");
    assert!(
        out.contains("<text x=\"10\" y=\"20\" font-size=\"10\" filter=\"url(#x-flt-"),
        "{out}"
    );
    // the anchor shifts the chunk back: centred on 10, it spans [0, 20]
    let svg = format!(
        r##"{XW}<text x="10" y="20" font-size="10" text-anchor="middle" filter="drop-shadow(2 3 4 #123456)">abcd</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("x=\"-25.5\" y=\"-13.5\" width=\"71\""),
        "{out}"
    );
    // a <tspan dx> moves the pen: "ab" at [0, 10], "cd" at [110, 120] → w = 120 + 51
    let svg = format!(
        r##"{XW}<text x="0" y="20" font-size="10" filter="drop-shadow(2 3 4 #123456)">ab<tspan dx="100">cd</tspan></text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("x=\"-25.5\" y=\"-13.5\" width=\"171\""),
        "{out}"
    );
    // <text inline-size> lowered its filter to nothing before (copied verbatim)
    let svg = format!(
        r##"{XW}<text x="5" y="10" font-size="10" inline-size="40" filter="drop-shadow(1 1 1 #000)">one two six</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("<filter id=\"x-flt-"), "{out}");
    assert!(!out.contains("inline-size"), "{out}");
    assert!(out.contains("filter=\"url(#x-flt-"), "{out}");
    assert!(!out.contains("drop-shadow("), "{out}");
    // what the parser declines still passes through as authored
    let svg = format!(
        r##"{XW}<text x="5" y="10" font-size="10" inline-size="40" filter="url(#soft)">one two</text></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("filter=\"url(#soft)\""), "{out}");
    assert!(!out.contains("<filter id="), "{out}");
}

#[test]
fn a_filtered_textbox_wraps_its_glyphs_and_leaves_highlights_outside() {
    // §8 on text: a textbox's filter — dropped before — lowers like a shape's,
    // the def before the element, one <g filter> around the glyphs and their
    // decorations, the highlight rect before the group (a background that
    // casts a shadow reads as a sticker). Region from the line: "ab cd ef" is
    // 40 wide from x = 0 → x = -25.5, w = 91 (pad 10, margin 15.5 each side)
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-size="10" filter="drop-shadow(2 3 4 #123456)">ab <tspan x:highlight="#ff0">cd</tspan> <tspan text-decoration="underline">ef</tspan></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<filter id=\"x-flt-").count(), 1, "{out}");
    assert!(
        out.contains("filterUnits=\"userSpaceOnUse\" x=\"-25.5\""),
        "{out}"
    );
    assert!(out.contains("width=\"91\""), "{out}");
    let def_end = out.find("</filter>").unwrap();
    let group = out.find("<g filter=\"url(#x-flt-").expect(&out);
    let highlight = out.find("fill=\"#ff0\"").expect(&out);
    let text = out.find("<text ").expect(&out);
    let underline = out.rfind("<rect ").expect(&out);
    assert!(def_end < highlight && highlight < group, "{out}");
    assert!(group < underline && underline < text, "{out}");
    assert!(out.ends_with("</text></g></svg>"), "{out}");
    // no decorations: the group holds the <text> alone
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-size="10" filter="blur(2)">ab <tspan x:highlight="#ff0">cd</tspan></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("fill=\"#ff0\"/><g filter=\"url(#x-flt-"),
        "{out}"
    );
    assert!(out.contains("\"><text text-anchor"), "{out}");
    // paragraph mode: the def precedes the keyed <g>, the filter group sits inside it
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="60" font-size="10" filter="drop-shadow(0 2 3 #0008)"><x:p>ab</x:p><x:p>cd</x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains("</filter><g><g filter=\"url(#x-flt-"), "{out}");
    assert_eq!(out.matches("<text ").count(), 2, "{out}");
    assert!(out.ends_with("</text></g></g></svg>"), "{out}");
    // outlined: the group wraps the path group, region from the lines
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true" filter="drop-shadow(2 3 4 #123456)">abcd</x:textbox></svg>"##
    );
    let out = compile_outlined(&svg);
    assert!(out.contains("<g filter=\"url(#x-flt-"), "{out}");
    assert!(out.contains("\"><g fill=\"#000\""), "{out}");
    assert!(out.contains("<path d="), "{out}");
    assert!(out.ends_with("</g></g></svg>"), "{out}");
    // a textbox without a filter is untouched: no group, no def
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-size="10">ab <tspan text-decoration="underline">ef</tspan></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert!(!out.contains("<g"), "{out}");
    assert!(!out.contains("<filter"), "{out}");
}

#[test]
fn a_filtered_textpath_shadows_every_effect() {
    // §8 on text, on a path: live forms carry the filter on their <text>, the
    // region the path's bbox grown by two ems; outlined effects wrap their
    // path group, the region the baked glyphs' own bbox
    let follow = format!(
        r##"{XW}<path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="follow" font-size="20" filter="drop-shadow(2 3 4 #123456)">wave</x:textpath></svg>"##
    );
    let out = compile_test(&follow);
    assert!(out.contains("filterUnits=\"userSpaceOnUse\""), "{out}");
    // bbox [0,120]×[20,20] grown by 40 then the 15.5 margin → x = -55.5, w = 231
    assert!(out.contains("x=\"-55.5\""), "{out}");
    assert!(out.contains("width=\"231\""), "{out}");
    assert!(out.contains("filter=\"url(#x-flt-"), "{out}");
    assert!(out.contains("><textPath href=\"#p\""), "{out}");
    let stair = format!(
        r##"{XW}<path id="p" d="M0,20 L120,20" fill="none"/><x:textpath in="#p" effect="stair" font-size="20" filter="blur(2)">wave</x:textpath></svg>"##
    );
    let out = compile_test(&stair);
    assert_eq!(out.matches("<filter id=").count(), 1, "{out}");
    assert!(out.contains("filter=\"url(#x-flt-"), "{out}");
    let skew = format!(
        r##"{XW}<path id="p" d="M0,20 C40,0 80,40 120,20" fill="none"/><x:textpath in="#p" effect="skew" font-size="20" fill="#111" filter="drop-shadow(2 3 4 #123456)">wave</x:textpath></svg>"##
    );
    let out = compile_outlined(&skew);
    assert!(out.contains("<g filter=\"url(#x-flt-"), "{out}");
    assert!(out.contains("\"><g fill=\"#111\""), "{out}");
    assert!(!out.contains("<text"), "{out}");
}

#[test]
fn an_unmeasured_advance_is_replaced_by_a_wide_guess() {
    // a measurer without the font advances nothing; a region as wide as
    // nothing would clip the shadow to a sliver
    assert_eq!(unmeasured_advance("abc", 0.0, 10.0), 18.0);
    assert_eq!(unmeasured_advance("abc", 7.0, 10.0), 7.0);
    assert_eq!(unmeasured_advance("  ", 0.0, 10.0), 0.0);
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
    // The placement is only exact under *bilinear* reconstruction, so the image asks
    // for it by name — `auto` lets resvg pick bicubic, which drifts up to ~9/255 off
    // the fitted field. Both spellings: resvg reads only the CSS, and the attribute
    // survives tools that strip `style` (scripts/mesh-raster-check.mjs measures it).
    assert!(out.contains(r#"image-rendering="smooth""#), "{out}");
    assert!(out.contains(r#"style="image-rendering:smooth""#), "{out}");
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
    for &c in &out.as_bytes()[k..] {
        if c == b'"' || c == b'=' || bytes.len() >= n {
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
    let map = MapResolver(
        files
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    );
    compile_linked_impl(
        main,
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &NoOutliner,
        &map,
        "main",
    )
    .unwrap()
}

#[test]
fn a_textbox_transform_passes_through_to_its_text() {
    // the textbox lays glyphs out in its own user space; rotating or scaling
    // the box is a transform on what it lowers to (found live: rotated and
    // scaled textboxes rendered upright)
    let main = format!(
        r##"{XW}<x:textbox x="10" y="20" width="200" height="80" font-size="18" transform="translate(5 5) rotate(30)">Hi</x:textbox></svg>"##
    );
    let out = compile_impl(&main, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(
        out.contains(r#"<text transform="translate(5 5) rotate(30)""#)
            || out.contains(r#"transform="translate(5 5) rotate(30)""#),
        "forwarded: {out}"
    );
    // paragraph mode wraps the paragraphs in one <g>: the transform rides the wrapper
    let main = format!(
        r##"{XW}<x:textbox x="10" y="20" width="200" height="80" transform="scale(2 0.5)"><x:p>a</x:p><x:p>b</x:p></x:textbox></svg>"##
    );
    let out = compile_impl(&main, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(
        out.contains(r#"<g transform="scale(2 0.5)">"#),
        "paragraph wrapper carries it: {out}"
    );
    assert_eq!(
        out.matches("transform=").count(),
        1,
        "once, not per paragraph: {out}"
    );
}

#[test]
fn keyed_textboxes_carry_their_frame() {
    // an editor selects and resizes the FRAME a textbox flows in, not the
    // ink it shows: keyed output says where that frame is; viewer output
    // (unkeyed, or sourcemapped by position) does not change
    struct Keys;
    impl NodeKeys for Keys {
        fn key(&self, _: usize) -> Option<String> {
            Some("n1".into())
        }
    }
    let main = format!(
        r##"{XW}<x:textbox x="10" y="20" width="200" height="80" font-size="18">Hi</x:textbox></svg>"##
    );
    let keyed = compile_keyed_impl(
        &main,
        "balanced",
        true,
        &Keys,
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert!(
        keyed.contains(r#"data-xsvg-frame="10 20 200 80""#),
        "keyed: {keyed}"
    );
    let mapped = compile_impl(&main, "balanced", true, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(
        !mapped.contains("data-xsvg-frame"),
        "position-mapped output is unchanged: {mapped}"
    );
    let plain = compile_impl(&main, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(
        !plain.contains("data-xsvg-frame"),
        "viewer output is unchanged: {plain}"
    );
}

#[test]
fn use_links_external_files() {
    let logo =
        format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);

    // whole-file link → the dependency inlined under a viewport <g> (§5.3): the
    // frame's position in its transform, its clip, its marker — no nested <svg>
    let main =
        format!(r##"{XW}<use href="logo.svg" x="20" y="30" width="40" height="40"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(
        out.contains(r#"<g transform="translate(20 30)" clip-path="url(#x-vp-"#),
        "viewport group: {out}"
    );
    assert!(out.contains(r#"data-xsvg-viewport="0 0 40 40""#), "{out}");
    assert_eq!(out.matches("<svg").count(), 1, "no nested <svg>: {out}");
    assert!(
        out.contains("#f00"),
        "dep content inlined (rect lowers to a path): {out}"
    );
    assert!(
        !out.contains("<use"),
        "the <use> is baked, not a live ref: {out}"
    );

    // the referrer's preserveAspectRatio decides the fit: a free resize in an
    // editor stretches the drawing (scale 4 1) instead of letterboxing it
    let boxed = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect id="mark" width="10" height="10" fill="#f00"/></svg>"##;
    let main = format!(
        r##"{XW}<use href="logo.svg" x="0" y="0" width="40" height="10" preserveAspectRatio="none"/></svg>"##
    );
    let out = compile_linked(&main, &[("logo.svg", boxed)]);
    assert!(
        out.contains(r#"transform="scale(4 1)""#),
        "stretched: {out}"
    );

    // by-id link → just that element, placed with a translate
    let main = format!(r##"{XW}<use href="logo.svg#mark" x="5" y="5"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(
        out.contains("translate(5,5)") && out.contains(r#"id="mark""#),
        "by-id placed: {out}"
    );

    // by-id with a width sizes to the *element's own* extent (10×10 rect → scale 2), not the file
    let main = format!(r##"{XW}<use href="logo.svg#mark" x="0" y="0" width="20"/></svg>"##);
    let out = compile_linked(&main, &[("logo.svg", &logo)]);
    assert!(
        out.contains("scale(2)"),
        "by-id sizes to the target's own extent: {out}"
    );

    // a group target with no declared size measures its geometry (a 10×10 rect inside → scale 3)
    let grp = format!(
        r##"{XW}<g id="mark"><rect x="0" y="0" width="10" height="10" fill="#f00"/></g></svg>"##
    );
    let main = format!(r##"{XW}<use href="g.svg#mark" x="0" y="0" width="30"/></svg>"##);
    let out = compile_linked(&main, &[("g.svg", &grp)]);
    assert!(
        out.contains("scale(3)"),
        "group by-id sizes to its geometry bbox: {out}"
    );

    // missing dependency degrades with a marker
    let out = compile_linked(&main, &[]);
    assert!(out.contains("not resolved"), "missing dep degrades: {out}");

    // a same-document <use href="#id"> is NOT linked — it stays a live reference
    let main =
        format!(r##"{XW}<rect id="r" x="0" y="0" width="4" height="4"/><use href="#r"/></svg>"##);
    let out = compile_linked(&main, &[]);
    assert!(
        out.contains(r##"<use href="#r""##),
        "same-doc use passes through: {out}"
    );

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
    let logo =
        format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);
    let main = format!(
        r##"{XW}<rect id="bg" x="0" y="0" width="100" height="100"/><use href="logo.svg" x="20" y="20" width="40"/></svg>"##
    );
    let map = MapResolver(
        [("logo.svg".to_string(), logo.clone())]
            .into_iter()
            .collect(),
    );
    let full = compile_linked_impl(
        &main,
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &NoOutliner,
        &map,
        "main",
    )
    .unwrap();
    let off = main.find("<use").unwrap();
    let frag = compile_fragment_linked_impl(
        &main,
        "balanced",
        false,
        off,
        &Mono,
        &NoShaper,
        &NoOutliner,
        &map,
        "main",
    )
    .unwrap();
    assert!(
        frag.contains(r#"data-xsvg-viewport="#) && frag.contains("#f00"),
        "fragment links: {frag}"
    );
    assert!(
        full.contains(&frag),
        "fragment is a verbatim slice of the full compile:\nfrag={frag}\nfull={full}"
    );
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
    assert!(
        out.contains(r#"stroke-width="4""#),
        "border-width doubled: {out}"
    );
    assert!(
        out.contains(r#"paint-order="stroke""#),
        "stroke behind fill: {out}"
    );

    // Raw stroke on *live* text now passes through (was silently dropped before).
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="50" stroke="#f00" stroke-width="1">Hi</x:textbox></svg>"##
    );
    let out = compile_fast(&svg);
    assert!(
        out.contains("<text") && out.contains(r##"stroke="#f00""##),
        "raw stroke on live text: {out}"
    );
}

#[test]
fn text_border_lowers_on_plain_svg_text() {
    // the same lowering on a passthrough <text> and on a <tspan> run: paint only,
    // nothing to lay out, so nothing about the text's own positioning matters
    let out = compile_fast(&format!(
        r##"{XW}<text x="10" y="40" font-size="24" fill="#fff" x:border-width="3" x:border-color="#000">bordered <tspan x:border-color="#f00">red</tspan></text><text x="10" y="80" dx="1 2 3" rotate="5">plain</text></svg>"##
    ));
    assert!(
        out.contains(r##"fill="#fff" stroke="#000" stroke-width="6" paint-order="stroke" stroke-linejoin="round">bordered"##),
        "{out}"
    );
    assert!(
        out.contains(r##"<tspan stroke="#f00" stroke-width="6" paint-order="stroke" stroke-linejoin="round">red"##),
        "a run's border, width defaulted: {out}"
    );
    assert!(
        !out.contains("border-"),
        "the x: attributes are consumed: {out}"
    );
    // without a border the text is untouched
    assert!(
        out.contains(r#"<text x="10" y="80" dx="1 2 3" rotate="5">plain"#),
        "{out}"
    );
    // a border replaces any hand-written stroke — never a duplicate attribute
    let out = compile_fast(&format!(
        r##"{XW}<text x="0" y="20" stroke="#0f0" stroke-width="1" paint-order="normal" x:border-width="2">t</text></svg>"##
    ));
    assert_eq!(out.matches("stroke=").count(), 1, "{out}");
    assert_eq!(out.matches("stroke-width=").count(), 1, "{out}");
    assert_eq!(out.matches("paint-order=").count(), 1, "{out}");
    assert!(
        out.contains(r##"stroke="#000000" stroke-width="4""##),
        "{out}"
    );
    roxmltree::Document::parse(&out[out.find("<svg").unwrap()..]).expect("well-formed");
}

#[test]
fn external_use_source_map_points_at_the_use_not_the_dependency() {
    // A linked dependency's element ranges index the *dependency* file, not the entry
    // document the viewer maps against — so baked content must carry no data-xsvg-pos,
    // and the block resolves (up the DOM) to the <use>'s range in the entry source.
    let logo =
        format!(r##"{XW}<rect id="mark" x="0" y="0" width="10" height="10" fill="#f00"/></svg>"##);
    let main =
        format!(r##"{XW}<use href="logo.svg" x="20" y="30" width="40" height="40"/></svg>"##);
    let map = MapResolver([("logo.svg".to_string(), logo)].into_iter().collect());
    let out = compile_linked_impl(
        &main,
        "balanced",
        true,
        &Mono,
        &NoShaper,
        &NoOutliner,
        &map,
        "main",
    )
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
        (
            "defs",
            r##"<g id="mark"><rect width="10" height="10"/><defs><rect width="1000" height="1000"/></defs></g>"##,
        ),
        (
            "clipPath",
            r##"<g id="mark"><rect width="10" height="10"/><clipPath id="c"><rect width="1000" height="1000"/></clipPath></g>"##,
        ),
        // explicitly hidden, by attribute and by inline style
        (
            "display-attr",
            r##"<g id="mark"><rect width="10" height="10"/><rect width="1000" height="1000" display="none"/></g>"##,
        ),
        (
            "display-style",
            r##"<g id="mark"><rect width="10" height="10"/><rect width="1000" height="1000" style="display: none"/></g>"##,
        ),
        // a nested <svg> clips to its viewport — 10 wide, not its 1000-unit viewBox content
        (
            "nested-svg",
            r##"<g id="mark"><svg x="0" y="0" width="10" height="10" viewBox="0 0 1000 1000"><rect width="1000" height="1000"/></svg></g>"##,
        ),
        // <image> is box-shaped: it has no path geometry but does have an extent
        (
            "image",
            r##"<g id="mark"><image x="0" y="0" width="10" height="10" href="p.png"/></g>"##,
        ),
    ];
    for (name, body) in cases {
        let dep = format!("{XW}{body}</svg>");
        let main = format!(r##"{XW}<use href="d.svg#mark" x="0" y="0" width="20"/></svg>"##);
        let out = compile_linked(&main, &[("d.svg", &dep)]);
        assert!(
            out.contains("scale(2)"),
            "{name}: expected scale(2) — got {out}"
        );
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
    assert!(
        out.contains("scale(2)"),
        "measured through the <use>: {out}"
    );

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
    assert!(
        out.contains("scale(2)"),
        "stroke=none adds no outset: {out}"
    );

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
    // the rect bakes to exactly its 4 corner vertices (M + 3 L + Z, the closing
    // edge being Z's straight leg) — no wasted subdivision
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
    assert_eq!(lines, 3, "{out}");
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

// ------------------------------------------------------------- keyed compiles

/// Test key provider: element start byte → stable key (the kit's serializer
/// supplies exactly this shape, for every element).
struct MapKeys(std::collections::HashMap<usize, String>);
impl NodeKeys for MapKeys {
    fn key(&self, s: usize) -> Option<String> {
        self.0.get(&s).cloned()
    }
}

/// Keys for EVERY element (root included), by document order — the shape the
/// kit's serializer supplies. Nested elements matter: emitters tag children
/// too (warp inner shapes, paragraphs), so a top-level-only provider would
/// silently reintroduce offset-dependence (pinned by the byte-stability test).
fn keys_for(input: &str) -> MapKeys {
    let doc = roxmltree::Document::parse(input).unwrap();
    MapKeys(
        doc.descendants()
            .filter(|n| n.is_element())
            .enumerate()
            .map(|(i, n)| (n.range().start, format!("n{i}")))
            .collect(),
    )
}

#[test]
fn keyed_fragments_are_verbatim_slices_of_the_full_compile() {
    // The canary invariant holds in keyed mode too.
    let keys = keys_for(INC);
    let full = compile_keyed_impl(
        INC,
        "balanced",
        true,
        &keys,
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    let mut cursor = 0;
    for off in top_level_offsets(INC) {
        let frag = compile_fragment_keyed_impl(
            INC,
            "balanced",
            true,
            off,
            &keys,
            &Mono,
            &BoxShaper,
            &BoxOutliner,
        )
        .unwrap();
        assert_eq!(frag.is_empty(), INC[off..].starts_with("<meshgradient"));
        let at = full[cursor..]
            .find(&frag)
            .unwrap_or_else(|| panic!("keyed fragment not found in order: {frag}\n{full}"));
        cursor += at + frag.len();
    }
    // identity attributes are keys, not byte spans; filter ids are keyed too
    assert!(full.contains("data-xsvg-node=\"n0\""), "{full}");
    assert!(!full.contains("data-xsvg-pos"), "{full}");
    assert!(full.contains("x-flt-n"), "{full}");
    assert!(
        !full.contains("x-flt-1"),
        "no byte-offset filter ids in keyed mode: {full}"
    );
}

#[test]
fn keyed_fragment_output_is_byte_stable_under_text_shifts() {
    // The reason the seam exists (docs/Incremental.md): an edit that shifts
    // earlier bytes must not change an untouched fragment's output.
    let shifted = INC.replace(
        "viewBox=\"0 0 400 300\">",
        "viewBox=\"0 0 400 300\">\n  <!-- padding: shifts every later byte offset -->",
    );
    assert_ne!(top_level_offsets(INC), top_level_offsets(&shifted));
    let (keys_a, keys_b) = (keys_for(INC), keys_for(&shifted));
    for (i, (off_a, off_b)) in top_level_offsets(INC)
        .into_iter()
        .zip(top_level_offsets(&shifted))
        .enumerate()
    {
        let frag_a = compile_fragment_keyed_impl(
            INC,
            "balanced",
            true,
            off_a,
            &keys_a,
            &Mono,
            &BoxShaper,
            &BoxOutliner,
        )
        .unwrap();
        let frag_b = compile_fragment_keyed_impl(
            &shifted,
            "balanced",
            true,
            off_b,
            &keys_b,
            &Mono,
            &BoxShaper,
            &BoxOutliner,
        )
        .unwrap();
        assert_eq!(
            frag_a, frag_b,
            "fragment {i} must be byte-identical across the shift"
        );
    }
}

#[test]
fn a_keyed_compile_reports_elements_the_host_failed_to_name() {
    // "Key every element" is a host contract nothing else enforces — an unkeyed
    // element silently falls back to a byte span, degrading incremental patching
    // with no error anywhere. So the compiler reports the violation in the sloppy
    // host's own output, where it will actually be seen (and where a host's tests
    // can grep for it).
    let victim = top_level_offsets(INC)[1];

    // A complete provider — and the byte-span default — get no marker.
    let full = compile_keyed_impl(
        INC,
        "balanced",
        true,
        &keys_for(INC),
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert!(!full.contains("unkeyed"), "{full}");
    let plain = compile_impl(INC, "balanced", true, &Mono, &BoxShaper, &BoxOutliner).unwrap();
    assert!(
        !plain.contains("unkeyed"),
        "byte-span identity is not a violation: {plain}"
    );

    // A provider that missed one element: the output says so, once, at the end —
    // trailing so every fragment span is untouched.
    let mut partial = keys_for(INC);
    partial.0.remove(&victim);
    let out = compile_keyed_impl(
        INC,
        "balanced",
        true,
        &partial,
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert!(
        out.ends_with("<!-- xsvg: 1 element(s) unkeyed; identity fell back to byte spans (docs/Incremental.md §3.1) -->"),
        "{out}"
    );
    assert!(
        out.contains("data-xsvg-pos"),
        "the miss fell back to a byte span: {out}"
    );

    // Fragment compiles never carry the marker — their output must stay a verbatim
    // slice of the full compile, and the marker sits outside every fragment span.
    let frag = compile_fragment_keyed_impl(
        INC,
        "balanced",
        true,
        victim,
        &partial,
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert!(!frag.contains("unkeyed"), "{frag}");
}

#[test]
fn dependents_multi_is_the_union_closure() {
    let wave = fragment_range_impl(INC, INC.find("id=\"wave\"").unwrap())
        .unwrap()
        .0;
    let blob = fragment_range_impl(INC, INC.find("id=\"blob\"").unwrap())
        .unwrap()
        .0;

    let mut union: Vec<(usize, usize)> = dependents_impl(INC, wave)
        .into_iter()
        .chain(dependents_impl(INC, blob))
        .collect();
    union.sort_unstable();
    union.dedup();
    // neither seed may appear in the union (each is excluded from its own scan,
    // and here neither depends on the other)
    let wave_range = fragment_range_impl(INC, wave).unwrap();
    let blob_range = fragment_range_impl(INC, blob).unwrap();
    union.retain(|r| *r != wave_range && *r != blob_range);

    assert_eq!(dependents_multi_impl(INC, &[wave, blob]), union);
    // duplicate seeds inside one fragment collapse
    assert_eq!(
        dependents_multi_impl(INC, &[wave, wave + 1]),
        dependents_impl(INC, wave)
    );
    // and both closures are non-trivial (textpath / textbox+boolean-use chains)
    assert!(!dependents_impl(INC, wave).is_empty());
    assert!(dependents_impl(INC, blob).len() >= 2);
}

#[test]
fn fragment_compile_reloads_theme_from_its_own_document() {
    // Regression (the stale-theme hazard): compile_fragment_impl once skipped the
    // theme load and reused whatever the last full compile left in a thread-local.
    // The theme is a per-document field now (docs/Compile Session.md §4), so this
    // is pinned as a property rather than a guarded case.
    let doc = |accent: &str| {
        format!(
            r##"{XW}<x:theme><x:color name="accent" value="{accent}"/></x:theme><rect x="0" y="0" width="10" height="10" fill="var(accent)"/></svg>"##
        )
    };
    let red = doc("#ff0000");
    let blue = doc("#0000ff");

    // Poison the thread-local with red…
    let _ = compile_test(&red);
    // …then a fragment compile of the BLUE document must resolve blue.
    let off = fragment_range_impl(&blue, blue.find("<rect").unwrap())
        .unwrap()
        .0;
    let frag = compile_fragment_impl(&blue, "balanced", false, off, &Mono, &NoShaper, &NoOutliner)
        .unwrap();
    assert!(
        frag.contains("#0000ff"),
        "fragment must use its own document's theme: {frag}"
    );
    assert!(!frag.contains("#ff0000"), "stale theme leaked: {frag}");
}

// ---------------------------------------------------------------- text layout
// The editor seam (docs/Incremental.md §6): text_layout_impl must report
// exactly the geometry the emitter renders.

/// Layout for the textbox starting at `offset` with the standard doubles.
fn layout_at(svg: &str, offset: usize) -> Option<TextLayout> {
    text_layout_impl(svg, offset, "balanced", &Mono, &NoShaper, &NoOutliner)
}

/// Byte offset of the first occurrence of `needle` — tests locate elements the
/// way a host's span table would.
fn at(svg: &str, needle: &str) -> usize {
    svg.find(needle).unwrap()
}

#[test]
fn text_layout_plain_box_wraps_with_word_ranges() {
    // Mono: char = 0.5×size = 8. "alpha beta" = 10 chars = 80 > 72 → wraps.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="10" y="20" width="72" height="90" font-size="16">alpha beta gamma</x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    assert_eq!(tl.content, (10.0, 20.0, 72.0, 90.0));
    assert_eq!(tl.paras.len(), 1);
    let p = &tl.paras[0];
    assert_eq!(p.words, ["alpha", "beta", "gamma"]);
    assert_eq!(p.style.size, 16.0);
    assert_eq!(p.anchor, "start");
    let texts: Vec<&str> = p.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["alpha", "beta", "gamma"]);
    let ranges: Vec<(usize, usize)> = p.lines.iter().map(|l| l.words).collect();
    assert_eq!(ranges, [(0, 1), (1, 2), (2, 3)]);
    // consecutive baselines step by size × line-height
    let step = p.lines[1].baseline - p.lines[0].baseline;
    assert!((step - 16.0 * p.style.line_height).abs() < 1e-9);
}

#[test]
fn text_layout_matches_emitted_tspans() {
    // The anti-drift canary: every <tspan x y>text</tspan> the compiler emits
    // for a textbox must appear in the seam's lines with identical geometry —
    // across plain, fit-shrink, valign, indent, and paragraph variants.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="80" height="60" font-size="16">alpha beta gamma delta</x:textbox>
<x:textbox x="0" y="100" width="60" height="30" font-size="20" fit="shrink" valign="middle">epsilon zeta</x:textbox>
<x:textbox x="0" y="200" width="120" height="80" font-size="10" align="center" text-indent="12">eta theta iota kappa</x:textbox>
<x:textbox x="0" y="300" width="160" height="120" font-size="12"><x:p>lambda mu</x:p><x:p align="end">nu xi omicron</x:p></x:textbox>
<x:textbox x="0" y="500" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:p>aa bb</x:p><x:p>cc dd ee ff gg hh</x:p></x:textbox>
</svg>"#;
    let out = compile_test(svg);
    assert!(out.contains("…"), "the ellipsis case must truncate: {out}");
    let mut emitted: Vec<(f64, f64, String)> = Vec::new();
    let doc = roxmltree::Document::parse(&out).unwrap();
    for n in doc.descendants().filter(|n| n.tag_name().name() == "tspan") {
        emitted.push((
            n.attribute("x").unwrap().parse().unwrap(),
            n.attribute("y").unwrap().parse().unwrap(),
            n.text().unwrap_or("").to_string(),
        ));
    }
    let mut reported: Vec<(f64, f64, String)> = Vec::new();
    let mut from = 0;
    while let Some(i) = svg[from..].find("<x:textbox") {
        let off = from + i;
        let tl = layout_at(svg, off).unwrap();
        for p in &tl.paras {
            for l in &p.lines {
                reported.push((l.x, l.baseline, l.text.clone()));
            }
        }
        from = off + 1;
    }
    // same multiset, and in fact the same order (document order both ways);
    // compare through the emitter's number formatting to match exactly
    let fmt3 = |v: &[(f64, f64, String)]| -> Vec<String> {
        v.iter()
            .map(|(x, y, t)| {
                format!(
                    "{}|{}|{}",
                    crate::compile::fmt(*x),
                    crate::compile::fmt(*y),
                    t
                )
            })
            .collect()
    };
    assert_eq!(fmt3(&reported), fmt3(&emitted));
    assert!(!emitted.is_empty());
}

#[test]
fn text_layout_fit_shrink_reports_rendered_size() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="40" height="20" font-size="32" fit="shrink">wide words here</x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    let p = &tl.paras[0];
    assert!(p.style.size < 32.0, "shrunk from 32, got {}", p.style.size);
    // metrics are at the rendered size (default proportions: ascent = 0.8×size)
    assert!((p.metrics.ascent - 0.8 * p.style.size).abs() < 1e-9);
}

#[test]
fn text_layout_paragraphs_have_own_spans_and_words() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="200" height="100" font-size="12"><x:p>alpha beta</x:p><x:p font-size="8">gamma</x:p></x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    assert_eq!(tl.paras.len(), 2);
    let (a, b) = (&tl.paras[0], &tl.paras[1]);
    assert_eq!(a.words, ["alpha", "beta"]);
    assert_eq!(b.words, ["gamma"]);
    // spans point at each <x:p>, inside the textbox's own range
    assert_eq!(&svg[a.span.0..a.span.0 + 5], "<x:p>");
    assert!(b.span.0 > a.span.1 - 1);
    assert_eq!(b.style.size, 8.0);
    assert!(b.lines[0].baseline > a.lines[0].baseline);
}

#[test]
fn text_layout_ellipsis_line_diverges_from_words() {
    // One line tall → the rest truncates into an ellipsis rewrite of line 1.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="80" height="18" font-size="16" text-overflow="ellipsis">alpha beta gamma delta</x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    let p = &tl.paras[0];
    assert_eq!(p.lines.len(), 1);
    let l = &p.lines[0];
    let joined = p.words[l.words.0..l.words.1].join(" ");
    assert_ne!(l.text, joined, "ellipsis must be detectable");
    assert!(l.text.ends_with('…'));
}

#[test]
fn text_layout_region_flow_reports_padded_bounds() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<circle id="c" cx="30" cy="30" r="30"/>
<x:textbox in="#c" font-size="10" padding="5">one two three</x:textbox></svg>"##;
    let tl = text_layout_impl(
        svg,
        at(svg, "<x:textbox"),
        "balanced",
        &Mono,
        &BoxShaper,
        &NoOutliner,
    )
    .unwrap();
    // BoxShaper pretends a 60×60 region at the origin
    assert_eq!(tl.content, (5.0, 5.0, 50.0, 50.0));
    assert!(!tl.paras[0].lines.is_empty());
    assert_eq!(tl.paras[0].words, ["one", "two", "three"]);
}

#[test]
fn text_layout_refuses_non_textboxes_and_bad_offsets() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<rect x="0" y="0" width="10" height="10"/>
<x:textbox width="50" height="20">hi</x:textbox></svg>"#;
    assert!(layout_at(svg, at(svg, "<rect")).is_none());
    assert!(layout_at(svg, 3).is_none());
    assert!(layout_at(svg, 10_000).is_none());
    assert!(layout_at("not xml <", 0).is_none());
    // broken in= reference resolves to nothing renderable
    let broken = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox in="#missing" width="50" height="20">hi</x:textbox></svg>"##;
    assert!(layout_at(broken, at(broken, "<x:textbox")).is_none());
}

#[test]
fn text_layout_valign_bottom_clip_keeps_exact_word_ranges() {
    // valign=bottom with an overflowing block clips EARLY lines — ranges must
    // stay exact for the surviving tail (the case that breaks post-hoc
    // token matching, and why ranges thread through the wrapper).
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="48" height="40" font-size="16" valign="bottom">aa bb cc dd ee</x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    let p = &tl.paras[0];
    assert!(p.lines.len() < 5, "some lines must have clipped");
    for l in &p.lines {
        assert_eq!(l.text, p.words[l.words.0..l.words.1].join(" "));
    }
    // the surviving lines are the TAIL of the word sequence
    assert_eq!(p.lines.last().unwrap().words.1, p.words.len());
}

// ---- Plugin seam (docs/Extensibility.md) ----------------------------------

use super::plugin::{
    imports_impl, ChildMode, ElementInfo, GenRequest, GenResult, GeneratorHost, InputMode,
};

const PLUG_NS: &str = "https://example.com/clipart";

/// A stand-in for a loaded WASM module. `bar` is a leaf that draws a rectangle
/// from its params, `frame` is a container that boxes its children, `edge`
/// requires `in=` geometry, and `evil` returns markup a module must not sneak in.
struct MockHost;

impl MockHost {
    fn param(req: &GenRequest, name: &str) -> Option<String> {
        req.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    }
    fn num(req: &GenRequest, name: &str, default: f64) -> f64 {
        Self::param(req, name)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
}

impl GeneratorHost for MockHost {
    fn element(&self, ns: &str, tag: &str) -> ElementInfo {
        if ns != PLUG_NS {
            return ElementInfo::Unbound;
        }
        match tag {
            "bar" | "evil" | "nested" | "warped" | "exfil" | "shadow" | "prose" => {
                ElementInfo::Ready {
                    input: InputMode::None,
                    children: ChildMode::None,
                }
            }
            "edge" => ElementInfo::Ready {
                input: InputMode::Required,
                children: ChildMode::None,
            },
            "frame" => ElementInfo::Ready {
                input: InputMode::None,
                children: ChildMode::Container,
            },
            "later" => ElementInfo::Pending,
            _ => ElementInfo::UnknownElement,
        }
    }

    fn generate(&self, _ns: &str, tag: &str, req: &GenRequest) -> GenResult {
        match tag {
            "bar" => {
                let w = Self::num(req, "width", 10.0);
                let fill = Self::param(req, "fill").unwrap_or_else(|| "#000".into());
                GenResult::Svg(format!(
                    r##"<path fill="{fill}" d="M 0 0 L {w} 0 L {w} 5 L 0 5 Z"/>"##
                ))
            }
            "edge" => GenResult::Svg(format!(
                r##"<path fill="none" d="{}"/>"##,
                req.input_d.as_deref().unwrap_or("")
            )),
            "frame" => {
                // box the children: a rect around their union bbox, then the kids
                let mut out = String::new();
                let boxes: Vec<[f64; 4]> = req.children.iter().filter_map(|c| c.bbox).collect();
                if let Some(u) = boxes.iter().copied().reduce(|a, b| {
                    let (x0, y0) = (a[0].min(b[0]), a[1].min(b[1]));
                    let (x1, y1) = ((a[0] + a[2]).max(b[0] + b[2]), (a[1] + a[3]).max(b[1] + b[3]));
                    [x0, y0, x1 - x0, y1 - y0]
                }) {
                    let p = Self::num(req, "pad", 0.0);
                    out.push_str(&format!(
                        r##"<rect class="frame" x="{}" y="{}" width="{}" height="{}"/>"##,
                        u[0] - p,
                        u[1] - p,
                        u[2] + 2.0 * p,
                        u[3] + 2.0 * p
                    ));
                }
                for c in &req.children {
                    out.push_str(&c.svg);
                }
                GenResult::Svg(out)
            }
            // a module trying to escape the static subset
            "evil" => GenResult::Svg(
                r##"<script>alert(1)</script><rect x="0" y="0" width="1" height="1" onclick="x()"/><animate dur="1s"/>"##
                    .into(),
            ),
            // a module reaching for the network, or for the document's own ids
            // (docs/Sandbox.md §4) — none of this is caught by the static subset
            "exfil" => GenResult::Svg(
                r##"<a href="javascript:alert(1)"><rect x="0" y="0" width="1" height="1"/></a><image href="https://attacker.example/p.png" x="0" y="0" width="1" height="1"/><foreignObject x="0" y="0" width="1" height="1"><iframe src="https://attacker.example/"/></foreignObject><rect x="0" y="0" width="2" height="2" fill="url(https://attacker.example/p.svg#g)" style="stroke: url('https://attacker.example/s.svg#h')"/><use href="#authored"/>"##
                    .into(),
            ),
            // a module that emits xsvg TEXT — whose runs lower through emitters that
            // never touched copy_attrs, where the sandbox's rules used to live
            "prose" => GenResult::Svg(
                r##"<x:textbox xmlns:x="https://xsvg.visioncortex.org" x="0" y="0" width="200" height="40" font-size="12">see <tspan href="javascript:alert(1)">here</tspan>, <tspan fill="url(https://attacker.example/p.svg#g)" x:highlight="url(https://attacker.example/h.svg#h)">there</tspan> and <tspan href="#own">mine</tspan></x:textbox><use href="secrets.xsvg" x="0" y="0"/><style>@import url(https://attacker.example/s.css);</style>"##.into(),
            ),
            // a module naming ids the document also uses, and referring to its own
            "shadow" => GenResult::Svg(
                r##"<linearGradient id="brand"><stop offset="0" stop-color="#000"/></linearGradient><rect id="authored" x="0" y="0" width="3" height="3" fill="url(#brand)"/>"##
                    .into(),
            ),
            // a module emitting an element in its own namespace (must not recurse)
            "nested" => GenResult::Svg(format!(r##"<c:bar xmlns:c="{PLUG_NS}" width="3"/>"##)),
            // a module emitting xsvg for the core to lower
            "warped" => GenResult::Svg(
                r##"<x:offset xmlns:x="https://xsvg.visioncortex.org" in="#s" distance="2"/><rect id="s" x="0" y="0" width="10" height="10" fill="none"/>"##
                    .into(),
            ),
            _ => GenResult::Error("unreachable".into()),
        }
    }
}

/// `[x, y, w, h]` of the mock container's frame path in compiled output.
fn frame_box(out: &str) -> Option<[f64; 4]> {
    use crate::kurbo::Shape;
    let d = out
        .split(r##"class="frame""##)
        .nth(1)?
        .split(" d=\"")
        .nth(1)?
        .split('"')
        .next()?;
    let bb = crate::kurbo::BezPath::from_svg(d).ok()?.bounding_box();
    Some([bb.x0, bb.y0, bb.width(), bb.height()])
}

fn compile_plug(svg: &str) -> String {
    compile_platform_impl(
        svg,
        "balanced",
        false,
        "",
        &Platform::new(Mono, NoShaper, NoOutliner).with_generators(MockHost),
    )
    .unwrap()
}

/// Document header binding the mock namespace.
fn plug_doc(body: &str) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" xmlns:c="{PLUG_NS}"><x:import ns="{PLUG_NS}" src="./clipart.wasm"/>{body}</svg>"##
    )
}

#[test]
fn an_imported_namespace_dispatches_to_its_generator() {
    let out = compile_plug(&plug_doc(r##"<c:bar width="20" fill="#f00"/>"##));
    assert!(
        out.contains(r##"d="M 0 0 L 20 0 L 20 5 L 0 5 Z""##),
        "{out}"
    );
    assert!(out.contains(r##"fill="#f00""##), "{out}");
    // <x:import> itself compiles to nothing
    assert!(!out.contains("x:import"), "{out}");
}

#[test]
fn without_the_import_a_plugin_element_stays_foreign() {
    // same element, no <x:import> — the namespace is unknown, so it drops as
    // editor metadata would, never reaching a generator
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:c="{PLUG_NS}"><c:bar width="20"/></svg>"##
    );
    let out = compile_plug(&svg);
    assert!(out.contains("foreign-namespace <bar> dropped"), "{out}");
}

#[test]
fn an_unbound_namespace_degrades_with_a_named_reason() {
    let body = r##"<d:thing xmlns:d="https://other.example/ns"/>"##;
    let out = compile_plug(&plug_doc(body));
    assert!(out.contains("foreign-namespace <thing> dropped"), "{out}");

    // bound namespace, element the module doesn't provide
    let out = compile_plug(&plug_doc(r##"<c:nope/>"##));
    assert!(out.contains("not provided by the imported module"), "{out}");

    // module known but not instantiated yet — the host's cue to load and retry
    let out = compile_plug(&plug_doc(r##"<c:later/>"##));
    assert!(out.contains("module not loaded yet"), "{out}");
}

#[test]
fn the_core_namespace_can_never_be_bound() {
    // an import claiming the core namespace would let a module shadow a built-in
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org"><x:import ns="https://xsvg.visioncortex.org" src="./evil.wasm"/><x:pie value="0.5" r="10"/></svg>"##;
    let out = compile_plug(svg);
    assert!(out.contains("the core namespace cannot be bound"), "{out}");
    // <x:pie>'s own diagnostic proves it still reached the built-in emitter
    // rather than being dispatched to the module that tried to claim it
    assert!(out.contains("<x:pie> has no slices"), "{out}");
}

#[test]
fn a_namespace_binds_once() {
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" xmlns:c="{PLUG_NS}"><x:import ns="{PLUG_NS}" src="./a.wasm"/><x:import ns="{PLUG_NS}" src="./b.wasm"/><c:bar width="7"/></svg>"##
    );
    let out = compile_plug(&svg);
    assert!(out.contains("already bound to another module"), "{out}");
    // the first binding still works
    assert!(out.contains("L 7 0"), "{out}");
    assert_eq!(imports_impl(&svg).len(), 1);
}

#[test]
fn generator_output_is_confined_to_the_static_subset() {
    // the sanitizer is the core's, so safety does not depend on the host
    let out = compile_plug(&plug_doc(r##"<c:evil/>"##));
    assert!(!out.contains("alert"), "{out}");
    assert!(!out.contains("onclick"), "{out}");
    assert_eq!(
        out.matches("outside the static subset — dropped").count(),
        2,
        "both <script> and <animate> dropped: {out}"
    );
    assert!(out.contains("<path"), "the harmless part survives: {out}");
}

#[test]
fn generators_do_not_nest_in_generator_output() {
    let out = compile_plug(&plug_doc(r##"<c:nested/>"##));
    assert!(
        out.contains("generators do not nest in generator output"),
        "{out}"
    );
}

#[test]
fn generator_output_may_be_xsvg_and_is_lowered_by_the_core() {
    // <x:offset> inside plugin output grows the rect by 2 on each side
    let out = compile_plug(&plug_doc(r##"<c:warped/>"##));
    assert!(
        !out.contains("x:offset"),
        "lowered, not passed through: {out}"
    );
    let d = out
        .split(" d=\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or_default();
    use crate::kurbo::Shape;
    let bb = crate::kurbo::BezPath::from_svg(d).unwrap().bounding_box();
    assert!(
        (bb.x0 + 2.0).abs() < 0.5 && (bb.x1 - 12.0).abs() < 0.5,
        "the referenced square grew by 2 on each side: {bb:?} {out}"
    );
}

#[test]
fn required_input_geometry_degrades_when_it_cannot_resolve() {
    let out = compile_plug(&plug_doc(r##"<c:edge/>"##));
    assert!(out.contains("requires in="), "{out}");

    let out = compile_plug(&plug_doc(r##"<c:edge in="#nope"/>"##));
    assert!(out.contains("target not found"), "{out}");

    // resolving normally: the referenced rect's geometry reaches the generator
    let out = compile_plug(&plug_doc(
        r##"<rect id="r" x="0" y="0" width="10" height="10"/><c:edge in="#r"/>"##,
    ));
    assert!(
        out.contains(r##"<path fill="none" d="M0,0 h10 v10 h-10 Z""##),
        "{out}"
    );
}

#[test]
fn a_leaf_generator_ignores_children_with_a_marker() {
    let out = compile_plug(&plug_doc(
        r##"<c:bar width="4"><rect width="1" height="1"/></c:bar>"##,
    ));
    assert!(out.contains("takes no children"), "{out}");
    assert!(out.contains("L 4 0"), "the element still renders: {out}");
}

#[test]
fn a_container_receives_its_children_compiled_and_measured() {
    let out = compile_plug(&plug_doc(
        r##"<c:frame pad="5"><rect x="10" y="10" width="20" height="20"/><rect x="40" y="10" width="10" height="30"/></c:frame>"##,
    ));
    // the frame is the union box (10,10)-(50,40) grown by pad=5
    assert_eq!(frame_box(&out), Some([5.0, 5.0, 50.0, 40.0]), "{out}");
    // and the children themselves are spliced through
    assert_eq!(out.matches("<path").count(), 3, "{out}");
}

#[test]
fn a_container_measures_text_children_as_line_boxes() {
    // Mono measures a fixed advance per char; the box follows the measurer, and
    // vertically it is the font's ascent above the baseline
    let out = compile_plug(&plug_doc(
        r##"<c:frame><text x="0" y="20" font-size="10">abc</text></c:frame>"##,
    ));
    let b = frame_box(&out).unwrap_or_default();
    let expect = Mono.measure("abc", &crate::TextStyle::default(), 10.0);
    assert!(
        (b[2] - expect).abs() < 0.5,
        "line box width {}: {out}",
        b[2]
    );
    // vertically the box spans ascent above the baseline to descent below it
    let fm = Mono.font_metrics(&crate::TextStyle::default(), 10.0);
    assert!(
        (b[1] - (20.0 - fm.ascent)).abs() < 0.5,
        "top of line box: {out}"
    );
    assert!(
        (b[3] - (fm.ascent + fm.descent)).abs() < 0.5,
        "line box height: {out}"
    );
}

#[test]
fn a_container_measures_multi_line_text_line_by_line() {
    // wrapped text puts each line on its own positioned <tspan>; measuring the
    // element's whole string as one run would report a box as wide as every line
    // concatenated, and place it wherever the <text> element happened to sit
    let out = compile_plug(&plug_doc(
        r##"<c:frame><x:textbox x="40" y="100" width="120" height="60" font-size="10">one two three four five</x:textbox></c:frame>"##,
    ));
    let b = frame_box(&out).unwrap_or_default();
    assert!(out.contains("<tspan"), "the textbox wrapped: {out}");
    // the box starts at the text's own left edge, not the origin
    assert!((b[0] - 40.0).abs() < 0.5, "left edge {}: {out}", b[0]);
    // and is no wider than the box the lines were laid out in
    assert!(
        b[2] <= 120.0 + 0.5,
        "width {} exceeds the textbox: {out}",
        b[2]
    );
    assert!(b[1] > 80.0, "top follows the first baseline: {out}");
}

#[test]
fn a_container_child_keeps_its_own_geometry_when_nested_in_a_group() {
    // transforms compose down the fragment when measuring
    let out = compile_plug(&plug_doc(
        r##"<c:frame><g transform="translate(100,0)"><rect x="0" y="0" width="10" height="10"/></g></c:frame>"##,
    ));
    assert_eq!(frame_box(&out), Some([100.0, 0.0, 10.0, 10.0]), "{out}");
}

#[test]
fn a_stroked_child_reports_its_painted_extent() {
    // half the stroke falls outside the geometric box; a frame that ignored it
    // would clip the child
    let out = compile_plug(&plug_doc(
        r##"<c:frame><rect x="10" y="10" width="20" height="20" stroke="#000" stroke-width="4"/></c:frame>"##,
    ));
    assert_eq!(frame_box(&out), Some([8.0, 8.0, 24.0, 24.0]), "{out}");
}

#[test]
fn theme_tokens_resolve_before_the_generator_sees_them() {
    let out = compile_plug(&plug_doc(
        r##"<x:theme><x:color name="accent" value="#0f0"/></x:theme><c:bar fill="var(accent)"/>"##,
    ));
    assert!(out.contains(r##"fill="#0f0""##), "{out}");
    assert!(!out.contains("var("), "{out}");
}

#[test]
fn a_generator_element_is_referenceable_geometry() {
    // the composition promise: plugin output harvests like a built-in x: element
    let out = compile_plug(&plug_doc(
        r##"<c:bar id="a" width="20"/><x:boolean op="union"><use href="#a"/><rect x="0" y="0" width="5" height="30"/></x:boolean>"##,
    ));
    let d = out
        .rsplit(" d=\"")
        .next()
        .and_then(|s| s.split('"').next())
        .unwrap_or_default();
    use crate::kurbo::Shape;
    let bb = crate::kurbo::BezPath::from_svg(d).unwrap().bounding_box();
    assert!(
        bb.x1 >= 19.5 && bb.y1 >= 29.5,
        "union spans both operands: {bb:?} {out}"
    );
}

#[test]
fn the_wrapper_carries_identity_and_placement() {
    let out = compile_plug(&plug_doc(
        r##"<c:bar id="a" transform="translate(3,4)" width="2"/>"##,
    ));
    assert!(
        out.contains(r##"<g id="a" transform="translate(3,4)">"##),
        "{out}"
    );
}

#[test]
fn a_plugin_element_carries_a_source_range() {
    let src = plug_doc(r##"<c:bar width="2"/>"##);
    let out = compile_platform_impl(
        &src,
        "balanced",
        true,
        "",
        &Platform::new(Mono, NoShaper, NoOutliner).with_generators(MockHost),
    )
    .unwrap();
    let start = src.find("<c:bar").unwrap();
    assert!(
        out.contains(&format!(r##"data-xsvg-pos="{start}-"##)),
        "{out}"
    );
    // only the root and the wrapper are stamped — a range inside generator
    // output would index the module's output string, not this document
    assert_eq!(out.matches("data-xsvg-pos").count(), 2, "{out}");
}

#[test]
fn imports_are_visible_before_compiling() {
    // the host pre-scan that avoids a throwaway compile round
    let src = plug_doc(r##"<c:bar/>"##);
    let found = imports_impl(&src);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].ns, PLUG_NS);
    assert_eq!(found[0].src, "./clipart.wasm");
    assert_eq!(found[0].integrity, None);
}

#[test]
fn editing_an_import_recompiles_every_element_in_its_namespace() {
    // the namespace-level recalc edge: a rebind changes what every element in
    // that namespace lowers to, even though no id is involved
    let src = plug_doc(r##"<c:bar width="1"/><rect x="0" y="0" width="1" height="1"/>"##);
    let at = src.find("<x:import").unwrap();
    let deps = dependents_multi_impl(&src, &[at]);
    let bar = src.find("<c:bar").unwrap();
    assert!(deps.iter().any(|&(s, _)| s == bar), "{deps:?}");
    // the plain rect is untouched
    let rect = src.rfind("<rect").unwrap();
    assert!(!deps.iter().any(|&(s, _)| s == rect), "{deps:?}");
}

#[test]
fn a_plugin_fragment_matches_its_span_in_the_full_compile() {
    // the incremental canary (docs/Incremental.md) extended to generators —
    // this is what makes host-side purity a hard requirement
    let src = plug_doc(r##"<rect x="0" y="0" width="4" height="4"/><c:bar width="9"/>"##);
    let platform = Platform::new(Mono, NoShaper, NoOutliner).with_generators(MockHost);
    let full = compile_platform_impl(&src, "balanced", true, "", &platform).unwrap();
    let at = src.find("<c:bar").unwrap();
    let frag = compile_fragment_platform_impl(&src, "balanced", true, at, "", &platform).unwrap();
    assert!(
        full.contains(&frag),
        "fragment is a verbatim slice:\n{frag}\n{full}"
    );
}

#[test]
fn a_document_without_generators_still_compiles() {
    // the default host degrades every plugin element, and nothing else changes
    let out = compile_test(&plug_doc(r##"<c:bar width="2"/>"##));
    assert!(out.contains("no generator bound"), "{out}");
}

// ------------------------------------------------- what the elements declare

#[test]
fn every_bordered_text_element_declares_the_border_block() {
    // `x:border-*` (§6.17) is read straight off the node by `text_border_attrs`,
    // which is exactly why it needs declaring: reading an attribute is invisible to
    // a host, so before this the convenience worked and no property panel could
    // know it existed. Pinned against the emitters that call it — textbox,
    // textpath, plain <text> and <textArea>, and a run's <tspan> — so adding
    // another caller without declaring the block fails here.
    for spec in crate::built_in_elements() {
        let declares = spec
            .all_params()
            .any(|p| p.name == "border-width" && p.ns == Some(XSVG_NS));
        let expected = matches!(
            spec.name,
            "textbox" | "textpath" | "text" | "textArea" | "tspan"
        );
        assert_eq!(
            declares, expected,
            "<x:{}> declares the border block: {declares}, expected {expected}",
            spec.name
        );
    }

    // Declared in the xsvg namespace, never as a plain name: the unprefixed
    // spellings belong to raw SVG stroke passthrough, which the same emitter
    // honours when no `x:border-*` is present.
    let textbox = crate::built_in_elements()
        .iter()
        .find(|s| s.name == "textbox")
        .unwrap();
    for p in textbox
        .all_params()
        .filter(|p| p.name.starts_with("border-"))
    {
        assert_eq!(
            p.ns,
            Some(XSVG_NS),
            "border param `{}` names no namespace",
            p.name
        );
    }
    assert!(
        !textbox.all_params().any(|p| p.name == "stroke-width"),
        "the plain stroke spellings are passthrough, not declared parameters"
    );
}

#[test]
fn placement_and_extent_are_declared_as_geometry() {
    // A host decides where geometry is edited (the viewer leaves it to the source
    // pane; an editor with handles owns it on canvas) — but only if the element
    // says which parameters place and size it. Every built-in that takes a position
    // must mark it, or that host silently offers a number field for something it
    // means to handle itself.
    for spec in crate::built_in_elements() {
        for p in spec.all_params() {
            let places = matches!(p.name, "x" | "y" | "width" | "height" | "cx" | "cy" | "r");
            if places {
                assert!(
                    p.geometry,
                    "<x:{}> `{}` places or sizes the element but is not declared geometry",
                    spec.name, p.name
                );
            }
        }
    }

    // And not everything with a length is geometry: these shape the drawing without
    // placing or sizing it, so they stay editable.
    for (el, name) in [
        ("pie", "inner-radius"),
        ("pie", "stroke-width"),
        ("connector", "bulge"),
        ("textbox", "padding"),
    ] {
        let spec = crate::built_in_elements()
            .iter()
            .find(|s| s.name == el)
            .unwrap();
        let p = spec.all_params().find(|p| p.name == name).unwrap();
        assert!(!p.geometry, "<x:{el}> `{name}` is not placement or extent");
    }
}

/// Two boxes side by side, as `dataset/connector-anchors.xsvg` arranges them.
const ANCHOR_BOXES: &str = r##"<rect id="a" x="60" y="150" width="120" height="56"/><rect id="b" x="360" y="150" width="120" height="56"/>"##;

/// The connector's route as its own compiled document, so `route_d`'s borrow lives.
fn connector_d(anchors: &str, route: &str, arrow: &str) -> String {
    let svg = format!(
        r##"{XW}{ANCHOR_BOXES}<x:connector {anchors} route="{route}" arrow="{arrow}" stroke="#111"/></svg>"##
    );
    route_d(&compile_test(&svg)).to_string()
}

/// Every `M`/`L`/`Q`/`C` segment endpoint of a route, in order.
fn waypoints(d: &str) -> Vec<crate::kurbo::Point> {
    d.split(['M', 'L', 'Q', 'C'])
        .skip(1)
        .filter_map(|seg| {
            let last = seg.split_whitespace().last()?;
            let (x, y) = last.split_once(',')?;
            Some(crate::kurbo::Point::new(x.parse().ok()?, y.parse().ok()?))
        })
        .collect()
}

#[test]
fn same_side_anchors_arc_over_instead_of_detouring() {
    // Regression. Anchoring both ends to the same side — `#a:left-top → #b:top`, the
    // "leave the top and arc over" idiom — must stay ONE smooth cubic. The elbow
    // detour is for exits that face back down the chord; a corner anchor is only 45°
    // off it and a same-side edge is 90°, and both bow round perfectly. Testing
    // "points away at all" sent them to the elbow, which routed out past the far box
    // and doubled back — two brackets where the sample asks for two Ls.
    for anchors in [
        r##"from="#a:left-top" to="#b:top""##,
        r##"from="#a:bottom" to="#b:right-bottom""##,
        r##"from="#a:top" to="#b:top""##,
    ] {
        let d = connector_d(anchors, "curve", "both");
        assert!(
            d.contains(" C") && !d.contains(" L") && !d.contains(" Q"),
            "{anchors} is a single arc, not an elbow: {d}"
        );
        // and the arc stays within the span it connects — the detour's tell was
        // running out past the far box to x=438 before doubling back to 420
        use crate::kurbo::Shape;
        let bbox = crate::kurbo::BezPath::from_svg(&d).unwrap().bounding_box();
        assert!(
            bbox.x1 <= 481.0,
            "{anchors} overshoots the far box: {bbox:?}"
        );
    }
}

#[test]
fn a_forced_anchor_decides_which_axis_its_rail_leaves_along() {
    // An anchor facing across the route is the more specific instruction: `:top` on
    // an x-major rail must leave vertically, not be forced onto x. Reading only the
    // horizontal component of a `:top` normal gives zero, whose `signum` is +1 — so
    // the rail confidently left rightward along an axis the anchor never faced.
    for route in ["x-major", "y-major"] {
        let d = connector_d(r##"from="#a:left-top" to="#b:top""##, route, "none");
        assert_eq!(d, "M60,150 L60,132 L420,132 L420,150", "{route}");
    }

    // A diagonal corner contributes a SIGN, not its 0.707 component. Scaling one
    // end's stub by that left the two stubs on different lines and put a stair-step
    // where they were meant to meet.
    let d = connector_d(
        r##"from="#a:bottom" to="#b:right-bottom""##,
        "y-major",
        "none",
    );
    assert_eq!(d, "M120,206 L120,224 L480,224 L480,206");
}

#[test]
fn perpendicular_anchors_route_through_a_corner() {
    // One end leaves horizontally, the other vertically: no shared axis, so the rail
    // turns through the corner where the two exit lines meet — stubbing clear first
    // when that corner would land inside a box or arrive out of B rather than into it.
    let d = connector_d(r##"from="#a:right" to="#b:top""##, "x-major", "none");
    assert_eq!(d, "M180,178 L198,178 L198,132 L420,132 L420,150");
}

#[test]
fn a_rail_never_repeats_or_runs_straight_through_a_waypoint() {
    // `round_rail` shares one radius across every corner, capped by the shortest
    // segment a corner eats into — so a single zero-length segment pins it to 0 and
    // leaves the whole path sharp. Rails whose two stubs land on the same coordinate
    // emitted exactly that, plus a redundant collinear vertex.
    for (anchors, route) in [
        (r##"from="#a:bottom" to="#b:right-bottom""##, "y-major"),
        (r##"from="#a:left-top" to="#b:top""##, "x-major"),
        (r##"from="#a:right" to="#b:left""##, "x-major"),
    ] {
        let d = connector_d(anchors, route, "none");
        let pts = waypoints(&d);
        assert!(pts.len() >= 2, "{anchors} {route}: {d}");
        for w in pts.windows(2) {
            assert!(
                w[0].distance(w[1]) > 1e-6,
                "{anchors} {route} repeats a waypoint: {d}"
            );
        }
        for w in pts.windows(3) {
            let (e1, e2) = (w[1] - w[0], w[2] - w[1]);
            assert!(
                (e1.x * e2.y - e1.y * e2.x).abs() > 1e-6,
                "{anchors} {route} keeps a waypoint it runs straight through: {d}"
            );
        }
    }
}

// ------------------------------------------------ the plugin sandbox (Sandbox.md)

#[test]
fn plugin_output_cannot_reach_the_network() {
    // The static subset stops output from *executing*; it never stopped it from
    // *fetching*. A generator that can make the renderer issue a request has an
    // outbound channel — and a container generator is handed its children already
    // compiled, so "read the document, encode it into a URL, emit an <image>" is
    // exfiltration built entirely from primitives that individually look benign.
    let out = compile_plug(&plug_doc(r##"<c:exfil/>"##));

    assert!(
        !out.contains("attacker.example"),
        "output reaches a remote origin: {out}"
    );
    assert!(
        !out.contains("javascript:"),
        "output can run script on click: {out}"
    );
    // the elements themselves may stay — inert, having lost only the URL
    assert!(
        out.contains("<a"),
        "the anchor is kept, just defused: {out}"
    );
    // a paint server pointing out of the document collapses rather than fetching
    assert!(
        out.contains(r##"fill="none""##),
        "remote paint not neutralized: {out}"
    );
    // <foreignObject> is an HTML escape hatch a geometry generator has no use for
    assert!(!out.contains("<iframe"), "{out}");
    assert!(out.contains("<foreignObject> from a generator"), "{out}");
    // and the document says what it lost rather than quietly rendering less
    assert!(out.contains("pointed outside the document"), "{out}");
}

#[test]
fn plugin_ids_cannot_shadow_the_document() {
    // An unscoped id from a generator captures the document's own references: emit
    // `id="brand"` and every authored `url(#brand)` resolves to plugin geometry.
    let svg = plug_doc(
        r##"<linearGradient id="brand"><stop offset="0" stop-color="#f00"/></linearGradient><rect id="authored" x="0" y="0" width="4" height="4" fill="url(#brand)"/><c:shadow/>"##,
    );
    let out = compile_plug(&svg);

    // the authored pair is untouched
    assert!(out.contains(r##"<linearGradient id="brand">"##), "{out}");
    assert!(out.contains(r##"fill="url(#brand)""##), "{out}");
    // the generator's are namespaced to the element that produced them
    assert_eq!(
        out.matches(r##"id="brand""##).count(),
        1,
        "id collision: {out}"
    );
    assert_eq!(
        out.matches(r##"id="authored""##).count(),
        1,
        "id collision: {out}"
    );
    assert!(
        out.contains("id=\"x-plg-"),
        "generator ids are not scoped: {out}"
    );
    // and a reference *within* the generator's own output still resolves, rewritten
    // to match — scoping ids must not break the module's own paint
    let scoped = out
        .split("id=\"x-plg-")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    assert!(
        out.contains(&format!("url(#x-plg-{scoped})")),
        "the module's own reference did not follow its id: {out}"
    );
}

#[test]
fn an_author_may_still_write_what_a_generator_may_not() {
    // The asymmetry is the point (docs/Sandbox.md §1): the author's own markup is
    // their decision, and tightening it here would be a different, breaking change.
    let out = compile_test(&format!(
        r##"{XW}<image href="https://example.com/p.png" x="0" y="0" width="1" height="1"/><rect id="brand" x="0" y="0" width="1" height="1"/></svg>"##
    ));
    assert!(out.contains("https://example.com/p.png"), "{out}");
    assert!(out.contains(r##"id="brand""##), "{out}");
}

#[test]
fn plugin_output_is_confined_however_it_was_lowered() {
    // The sandbox's output rules used to live in copy_attrs, and a generator may emit
    // xsvg — whose text runs lower through emitters that never call it. So a module
    // emitting an <x:textbox> got a linked run's javascript: href, a highlight's
    // remote url() and a run's remote paint straight through. The rules now run over
    // the finished markup, so how it was lowered no longer matters.
    let out = compile_plug(&plug_doc(r##"<c:prose/>"##));
    assert!(
        !out.contains("javascript:"),
        "a linked run carried script: {out}"
    );
    assert!(
        !out.contains("attacker.example"),
        "lowered text reached the network: {out}"
    );
    // a fragment link inside the module's own output still works, scoped like its ids
    assert!(
        out.contains(r##"href="#x-plg-"##),
        "own fragment link lost or unscoped: {out}"
    );
    // a cross-file link would make the HOST read another file on the module's say-so
    assert!(
        !out.contains("secrets"),
        "generator output triggered a cross-file link: {out}"
    );
    // and a stylesheet would restyle the whole document, not just this element
    assert!(out.contains("<style> from a generator"), "{out}");
    assert!(out.contains("pointed outside the document"), "{out}");
}

#[test]
fn a_forwarded_preserve_aspect_ratio_cannot_carry_markup() {
    // The <use>'s value decides the fit of the viewport a link bakes (§5.3), and
    // once it was forwarded as an attribute, formatted in raw: a quote in the
    // value closed the attribute and opened a live onload. Now it is parsed —
    // the attribute has a closed grammar — and nothing of it reaches the output
    // but the fit it names; anything outside the grammar is the default fit.
    // A square drawing in a 40×20 frame tells the fits apart.
    struct Dep;
    impl Resolver for Dep {
        fn resolve(&self, _: &str, _: &str) -> Option<(String, String)> {
            Some((
                "dep.xsvg".into(),
                r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4"/></svg>"##.to_string(),
            ))
        }
    }
    let doc = |par: &str| {
        let svg = format!(
            r##"{XW}<use href="dep.xsvg" width="40" height="20" preserveAspectRatio="{par}"/></svg>"##
        );
        compile_linked_impl(
            &svg,
            "balanced",
            false,
            &Mono,
            &NoShaper,
            &NoOutliner,
            &Dep,
            "doc.xsvg",
        )
        .unwrap()
    };
    let hostile = doc(r##"none&quot; onload=&quot;alert(1)"##);
    assert!(!hostile.contains("onload"), "{hostile}");
    assert!(!hostile.contains("preserveAspectRatio"), "{hostile}");
    // the default fit: meet, centred — scale 5, the square 10 in from the left
    assert!(
        hostile.contains(r##"transform="translate(10 0) scale(5)""##),
        "{hostile}"
    );
    for (par, tf) in [
        ("none", "scale(10 5)"),
        ("xMaxYMin", "translate(20 0) scale(5)"),
        ("xMidYMid slice", "translate(0 -10) scale(10)"),
    ] {
        let out = doc(par);
        assert!(
            out.contains(&format!(r##"transform="{tf}""##)),
            "{par}: {out}"
        );
        assert!(!out.contains("preserveAspectRatio"), "{par}: {out}");
    }
}

#[test]
fn a_highlight_colour_cannot_carry_markup() {
    // x:highlight bakes a <rect>, and its colour was formatted in raw.
    let out = compile_test(&format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="40" font-size="12">see <tspan x:highlight="#ff0&quot; onload=&quot;alert(1)">this</tspan></x:textbox></svg>"##
    ));
    // the escaped value still *contains* the text `onload=`, harmlessly — what must
    // not exist is an onload ATTRIBUTE, so ask the parser rather than the string
    let doc = roxmltree::Document::parse(&out).expect("output is well-formed");
    let smuggled = doc
        .descendants()
        .any(|n| n.attributes().any(|a| a.name() == "onload"));
    assert!(
        !smuggled,
        "the colour opened an attribute of its own: {out}"
    );
    let rect = doc.descendants().find(|n| n.has_tag_name("rect")).unwrap();
    assert_eq!(
        rect.attribute("fill"),
        Some(r##"#ff0" onload="alert(1)"##),
        "kept whole, as one value"
    );
}

#[test]
fn a_list_item_steps_by_the_size_of_its_own_first_line() {
    // Within an item, lines already stepped by their largest run; the step INTO an
    // item used the item's base size, so a 3x-size run on an item's first line put
    // its ascent up inside the item above. Every line now advances by its own size.
    let base = |second: &str| {
        let out = compile_test(&format!(
            r##"{XW}<x:list x="0" y="0" width="300" font-size="16"><x:li>one</x:li><x:li>{second}</x:li><x:li>three</x:li></x:list></svg>"##
        ));
        let ys: Vec<f64> = out
            .match_indices(r#" y=""#)
            .map(|(i, _)| {
                let v = &out[i + 4..];
                v[..v.find('"').unwrap()].parse().unwrap()
            })
            .collect();
        ys
    };
    let plain = base("two");
    let big = base(r#"<tspan font-size="300%">two</tspan>"#);
    let (lh, gap) = (1.2, 16.0 * 0.35);
    // the plain list steps by the base size throughout
    assert!(
        (plain[1] - plain[0] - (16.0 * lh + gap)).abs() < 1e-6,
        "{plain:?}"
    );
    // the item holding the 48px run is pushed down by ITS size...
    assert!(
        (big[1] - big[0] - (48.0 * lh + gap)).abs() < 1e-6,
        "{big:?}"
    );
    // ...and the next item steps by its own, back at the base size
    assert!(
        (big[2] - big[1] - (16.0 * lh + gap)).abs() < 1e-6,
        "{big:?}"
    );
}

#[test]
fn a_highlight_follows_a_raised_or_lowered_run() {
    // baseline-shift moves the glyphs at render time; the highlight is a <rect> baked
    // beside them, so it has to be moved by the same amount or a highlighted
    // superscript keeps its box on the baseline below the text it marks.
    let rect_y = |shift: &str| -> f64 {
        let out = compile_test(&format!(
            r##"{XW}<x:textbox x="0" y="0" width="300" height="60" font-size="20">a <tspan x:highlight="#ff0" baseline-shift="{shift}">b</tspan></x:textbox></svg>"##
        ));
        let r = &out[out.find("<rect").unwrap()..];
        let y = &r[r.find(r#" y=""#).unwrap() + 4..];
        y[..y.find('"').unwrap()].parse().unwrap()
    };
    let level = rect_y("baseline");
    assert!(
        (level - rect_y("10") - 10.0).abs() < 1e-6,
        "a length lifts it by that length"
    );
    assert!(
        (level - rect_y("50%") - 10.0).abs() < 1e-6,
        "a percentage is of the run's size"
    );
    assert!(
        rect_y("super") < level && rect_y("sub") > level,
        "super raises, sub lowers"
    );
    assert!(
        ((level - rect_y("super")) - (rect_y("sub") - level)).abs() < 1e-6,
        "symmetrically"
    );
}

#[test]
fn a_space_inside_a_larger_run_wraps_at_the_runs_size() {
    // The renderer draws a space inside a 2x run at 2x, but wrapping counted every
    // space at the paragraph's base size, so a line could pass the width check and
    // render wider than its box. Mono at base 10: "aa"/"bb" at 20px are 20 wide each,
    // a space is 5 at the base size and 10 at the run's — 45 fits in 47, 50 does not.
    let lines = |w: f64| {
        let out = compile_test(&format!(
            r##"{XW}<x:textbox x="0" y="0" width="{w}" height="200" font-size="10"><tspan font-size="200%">aa bb</tspan></x:textbox></svg>"##
        ));
        out.matches(r#" y=""#).count()
    };
    assert_eq!(
        lines(47.0),
        2,
        "the 2x space is 10 wide, so aa bb (50) cannot fit in 47"
    );
    assert_eq!(lines(51.0), 1, "and does fit once the box allows it");
}

// ---------------------------------------------------------------- compile session
// docs/Compile Session.md §6: the one-parse property and theme isolation. Output
// equality over the corpus is tests/session.rs.

#[test]
fn a_session_parses_its_document_once() {
    let svg = format!(
        r##"{XW}<x:theme><x:color name="ink" value="#123"/></x:theme><rect id="a" x="0" y="0" width="10" height="10" fill="var(ink)"/><x:textbox in="#a" font-size="4">one two</x:textbox><x:textbox x="0" y="20" width="40" height="20">three four</x:textbox><circle cx="5" cy="5" r="2"/></svg>"##
    );
    let tops: Vec<usize> = ["<rect", "<x:textbox in", "<x:textbox x", "<circle"]
        .iter()
        .map(|t| svg.find(t).unwrap())
        .collect();
    let platform = Platform::new(Mono, NoShaper, NoOutliner);

    // The pure entries: one parse per call — what the session replaces.
    super::session::PARSE_COUNT.with(|c| c.set(0));
    for &off in &tops {
        compile_fragment_platform_impl(&svg, "balanced", true, off, "", &platform).unwrap();
    }
    dependents_multi_impl(&svg, &tops);
    assert_eq!(
        super::session::PARSE_COUNT.with(|c| c.get()),
        tops.len() + 1
    );

    // The session: a whole flush's worth of calls on one parse.
    super::session::PARSE_COUNT.with(|c| c.set(0));
    let session = platform.session(&svg, "balanced", true, "").unwrap();
    for &off in &tops {
        session.compile_fragment(off).unwrap();
        session.dependents(&[off]);
        session.text_layout(off);
        session.fragment_range(off);
    }
    session.dependents(&tops);
    session.node_spans();
    session.compile().unwrap();
    assert_eq!(
        super::session::PARSE_COUNT.with(|c| c.get()),
        1,
        "a session parses once, however many fragments it compiles"
    );
}

#[test]
fn two_sessions_on_one_thread_keep_their_own_themes() {
    // The theme used to be a thread-local reloaded per compile; a fragment compiled
    // after another document's compile could see the wrong tokens. It is now a
    // field of the session's context, so interleaving two sessions cannot mix them.
    let doc = |accent: &str| {
        format!(
            r##"{XW}<x:theme><x:color name="accent" value="{accent}"/></x:theme><rect x="0" y="0" width="10" height="10" fill="var(accent)"/><text x="0" y="30" x:font="h">T</text><x:theme><x:font name="h" font-family="{accent}"/></x:theme></svg>"##
        )
    };
    let red = doc("#ff0000");
    let blue = doc("#0000ff");
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let rs = platform.session(&red, "balanced", false, "").unwrap();
    let bs = platform.session(&blue, "balanced", false, "").unwrap();
    for (src, s, own, other) in [
        (&red, &rs, "#ff0000", "#0000ff"),
        (&blue, &bs, "#0000ff", "#ff0000"),
        (&red, &rs, "#ff0000", "#0000ff"),
    ] {
        for tag in ["<rect", "<text"] {
            let frag = s.compile_fragment(src.find(tag).unwrap()).unwrap();
            assert!(frag.contains(own), "{tag}: own theme: {frag}");
            assert!(
                !frag.contains(other),
                "{tag}: the other session's theme leaked: {frag}"
            );
        }
    }
}

// ---------------------------------------------------------------- entry points
// Every `*_impl` entry is `compile_with` over an `Options` and a platform; this
// pins that the thin wrappers stay thin.

#[test]
fn every_entry_is_compile_with_over_options() {
    let svg = format!(
        r##"{XW}<x:theme><x:color name="ink" value="#123"/></x:theme><rect id="a" x="0" y="0" width="10" height="10" fill="var(ink)"/><x:textbox in="#a" font-size="4">one two</x:textbox><circle cx="5" cy="5" r="2"/></svg>"##
    );
    let off = svg.find("<x:textbox").unwrap();
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let whole = |sourcemap: bool| Options {
        quality: "fast",
        sourcemap,
        ..Options::default()
    };
    assert_eq!(
        compile_impl(&svg, "fast", true, &Mono, &NoShaper, &NoOutliner).unwrap(),
        compile_with(&svg, &whole(true), &platform).unwrap()
    );
    assert_eq!(
        compile_platform_impl(&svg, "fast", false, "", &platform).unwrap(),
        compile_with(&svg, &whole(false), &platform).unwrap()
    );
    let fragment = Options {
        fragment: Some(off),
        ..whole(true)
    };
    assert_eq!(
        compile_fragment_impl(&svg, "fast", true, off, &Mono, &NoShaper, &NoOutliner).unwrap(),
        compile_with(&svg, &fragment, &platform).unwrap()
    );
    assert_eq!(
        compile_fragment_platform_impl(&svg, "fast", true, off, "", &platform).unwrap(),
        compile_with(&svg, &fragment, &platform).unwrap()
    );
    // the defaults are the pure entry's defaults
    assert_eq!(
        compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &NoOutliner).unwrap(),
        compile_with(&svg, &Options::default(), &platform).unwrap()
    );
    // a fragment offset outside every top-level element fails the same way
    assert_eq!(
        compile_with(
            &svg,
            &Options {
                fragment: Some(1),
                ..Options::default()
            },
            &platform
        ),
        compile_fragment_impl(&svg, "balanced", false, 1, &Mono, &NoShaper, &NoOutliner)
    );
}

#[test]
fn an_artboard_fill_paints_its_frame_under_the_slide() {
    // the backdrop is the frame, as the group's first child — under everything
    let svg = format!(
        r##"{XW}<g x:artboard="Title" x:frame="0 0 720 405" x:fill="#0b1020"><text x="10" y="20">hi</text></g></svg>"##
    );
    let out = compile_test(&svg);
    let g = out.find("<g ").unwrap();
    let rect = out.find(r##"<rect x="0" y="0" width="720" height="405" fill="#0b1020"/>"##);
    assert!(
        rect.is_some_and(|r| r > g && r < out.find("<text").unwrap()),
        "{out}"
    );
    assert!(!out.contains("x:fill"), "the attribute is consumed: {out}");

    // a placed slide: the frame is document space, the children are the
    // group's, so the rect lands at the group's origin
    let svg = format!(
        r##"{XW}<g x:artboard="Why" x:frame="840 525 720 405" x:fill="#fff" transform="translate(840,525)"><rect x="0" y="0" width="8" height="405" fill="#6366f1"/></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<rect x="0" y="0" width="720" height="405" fill="#fff"/><path"##),
        "{out}"
    );

    // a scaled slide: not a translation, so the rect keeps the frame's
    // coordinates and carries the inverse transform
    let svg = format!(
        r##"{XW}<g x:artboard="Half" x:frame="100 0 720 405" x:fill="#eee" transform="translate(100,0) scale(0.5)"><text x="0" y="0">a</text></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<rect x="100" y="0" width="720" height="405" transform="matrix(2 0 0 2 -200 0)" fill="#eee"/>"##),
        "{out}"
    );

    // nested under a transformed parent: the whole chain is undone
    let svg = format!(
        r##"{XW}<g transform="translate(0,1000)"><g x:artboard="Deep" x:frame="0 1000 10 10" x:fill="#abc" transform="translate(5,5)"><text x="0" y="0">a</text></g></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<rect x="-5" y="-5" width="10" height="10" fill="#abc"/>"##),
        "{out}"
    );

    // the fill is a paint: theme tokens resolve, and an empty slide still paints
    let svg = format!(
        r##"{XW}<x:theme><x:color name="paper" value="#fafafa"/></x:theme><g x:artboard="Blank" x:frame="0 0 4 4" x:fill="var(paper)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"><rect x="0" y="0" width="4" height="4" fill="#fafafa"/></g>"##),
        "{out}"
    );

    // without a frame there is nothing to paint: a marker, no rect
    let svg = format!(
        r##"{XW}<g x:artboard="Loose" x:fill="#000"><text x="0" y="0">a</text></g></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("x:fill needs a valid x:frame") && !out.contains("<rect"),
        "{out}"
    );

    // a fill on a group that is not an artboard is foreign metadata: dropped
    let svg = format!(r##"{XW}<g x:fill="#000"><text x="0" y="0">a</text></g></svg>"##);
    let out = compile_test(&svg);
    assert!(!out.contains("<rect") && !out.contains("fill"), "{out}");
}

// ---------------------------------------------------------------- paragraph overflow
// §6.6 applies to paragraph mode too: `ellipsis` marks the block's last rendered
// line when any line was dropped, and any line wider than its paragraph.
// Mono measures 5px per char at font-size 10; a 60px box holds 12 chars a line;
// baselines fall at 7, then 7 + 3.5 (paragraph gap) + 12 (line advance) = 22.5,
// then 34.5 — so height 30 shows the first two and drops the third.

fn para_lines(out: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(out).unwrap();
    doc.descendants()
        .filter(|n| n.tag_name().name() == "tspan" && n.attribute("x").is_some())
        .map(|n| {
            n.descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn paragraph_mode_ellipsizes_the_blocks_last_rendered_line() {
    // the second paragraph loses its second line: its first line takes the mark
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:p>aa bb</x:p><x:p>cc dd ee ff gg hh</x:p></x:textbox></svg>"##
    );
    assert_eq!(para_lines(&compile_test(&svg)), ["aa bb", "cc dd ee ff…"]);

    // a whole paragraph falling off the bottom marks the last line that shows,
    // even though that paragraph itself fit
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:p>aa bb</x:p><x:p>cc dd</x:p><x:p>ee ff</x:p></x:textbox></svg>"##
    );
    assert_eq!(para_lines(&compile_test(&svg)), ["aa bb", "cc dd…"]);

    // inline overflow: an unbreakable word wider than the box is cut to fit
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="100" font-size="10" text-overflow="ellipsis"><x:p>abcdefghijklmnop</x:p><x:p>cc</x:p></x:textbox></svg>"##
    );
    assert_eq!(para_lines(&compile_test(&svg)), ["abcdefghijk…", "cc"]);

    // the default is clip: lines drop silently, nothing is rewritten
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="30" font-size="10"><x:p>aa bb</x:p><x:p>cc dd ee ff gg hh</x:p></x:textbox></svg>"##
    );
    assert_eq!(para_lines(&compile_test(&svg)), ["aa bb", "cc dd ee ff"]);

    // styled runs survive the cut: the mark lands inside the last run that shows
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:p>aa bb</x:p><x:p>cc <tspan font-weight="bold">dd ee ff gg hh</tspan></x:p></x:textbox></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(para_lines(&out), ["aa bb", "cc dd ee ff…"]);
    assert!(
        out.contains(r#"<tspan font-weight="bold">dd ee ff…</tspan>"#),
        "{out}"
    );

    // a list inside the box: its last visible item carries the mark
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:list list="none"><x:li>aa</x:li><x:li>bb</x:li><x:li>cc</x:li></x:list></x:textbox></svg>"##
    );
    assert_eq!(para_lines(&compile_test(&svg)), ["aa", "bb…"]);
}

#[test]
fn text_layout_reports_the_ellipsized_paragraph_line() {
    // the editor seam shows the caret what is rendered: the rewritten text, with
    // the word range it came from still intact (docs/Incremental.md §6)
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="60" height="30" font-size="10" text-overflow="ellipsis"><x:p>aa bb</x:p><x:p>cc dd ee ff gg hh</x:p></x:textbox></svg>"#;
    let tl = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    assert_eq!(tl.paras.len(), 2);
    assert_eq!(tl.paras[0].lines[0].text, "aa bb");
    let cut = &tl.paras[1].lines;
    assert_eq!(cut.len(), 1, "the dropped line is absent: {cut:?}");
    assert_eq!(cut[0].text, "cc dd ee ff…");
    assert_eq!(cut[0].words, (0, 4), "the line still covers its four words");
    assert_eq!(
        cut[0].justify_width, None,
        "an ellipsized line renders at natural width"
    );
}

// ---------------------------------------------------------------------------
// Caret stops on outlined text (stops.rs; docs/Incremental.md §7)

fn stops_at(svg: &str, needle: &str) -> Option<CaretStops> {
    let off = svg.find(needle).unwrap();
    caret_stops_impl(svg, off, "balanced", &Mono, &NoShaper, &BoxOutliner)
}

/// `(x, y)` of every `<path d="M…">` start in `out`, in document order. The
/// `BoxOutliner` traces a 1×1 box from the pen origin, so a path's `M` is the
/// line's `(start_x, baseline)` mapped through whatever fields the compile applied.
fn path_starts(out: &str) -> Vec<(f64, f64)> {
    let doc = roxmltree::Document::parse(out).unwrap();
    doc.descendants()
        .filter(|n| n.tag_name().name() == "path")
        .map(|n| {
            let d = n.attribute("d").unwrap();
            let body = d.trim_start_matches('M').trim_start();
            let end = body
                .find(|c: char| c.is_ascii_alphabetic())
                .unwrap_or(body.len());
            let nums: Vec<f64> = body[..end]
                .split(|c: char| c == ',' || c.is_whitespace())
                .filter(|s| !s.is_empty())
                .map(|s| s.parse().unwrap())
                .collect();
            (nums[0], nums[1])
        })
        .collect()
}

#[test]
fn caret_stops_none_for_live_text() {
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="80" height="20" font-size="10">hi</x:textbox><rect x="0" y="0" width="1" height="1"/><x:textbox x="0" y="40" width="80" height="20" font-size="10" outline="true">hi</x:textbox></svg>"##
    );
    assert!(
        stops_at(&svg, "<x:textbox").is_none(),
        "no outline → live text → None"
    );
    assert!(stops_at(&svg, "<rect").is_none(), "not text");
    // outline asked for but no font bytes: the compile falls back to live text, and
    // so must the query (the host measures live glyphs itself)
    let off = svg.rfind("<x:textbox").unwrap();
    assert!(caret_stops_impl(&svg, off, "balanced", &Mono, &NoShaper, &NoOutliner).is_none());
    assert!(caret_stops_impl(&svg, off, "balanced", &Mono, &NoShaper, &BoxOutliner).is_some());
}

#[test]
fn caret_stops_sit_on_the_traced_lines() {
    // width 30 wraps "alpha beta" (Mono: 5 per char) into two lines
    let svg = format!(
        r##"{XW}<x:textbox x="7" y="3" width="30" height="40" font-size="10" outline="true">alpha beta</x:textbox></svg>"##
    );
    let out = compile_outlined(&svg);
    let starts = path_starts(&out);
    assert_eq!(starts.len(), 2, "{out}");
    let cs = stops_at(&svg, "<x:textbox").unwrap();
    assert_eq!(cs, stops_at(&svg, "<x:textbox").unwrap(), "deterministic");
    let texts: Vec<&str> = cs.runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["alpha", "beta"]);
    for (run, &(mx, my)) in cs.runs.iter().zip(&starts) {
        assert_eq!(run.stops.len(), run.text.chars().count() + 1);
        // stop 0 is the pen origin the path was traced from, straddling its baseline
        let s0 = &run.stops[0];
        assert!(
            (s0.top.0 - mx).abs() < 1e-9 && (s0.bottom.0 - mx).abs() < 1e-9,
            "{s0:?} vs {mx}"
        );
        assert!(
            s0.top.1 < my && my < s0.bottom.1,
            "{s0:?} must bracket baseline {my}"
        );
        // consecutive stops differ by the outline font's advance (Mono: 0.5 em)
        for w in run.stops.windows(2) {
            assert!((w[1].top.0 - w[0].top.0 - 5.0).abs() < 1e-9, "{w:?}");
            assert_eq!(w[1].top.1, w[0].top.1);
            assert_eq!(w[1].bottom.1, w[0].bottom.1);
        }
    }
}

#[test]
fn caret_stops_ride_the_warp_field() {
    let inner = r##"<x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">hi</x:textbox>"##;
    let flat = format!("{XW}{inner}</svg>");
    let zero = format!(r##"{XW}<x:warp field="arch" bend="0">{inner}</x:warp></svg>"##);
    let bent = format!(r##"{XW}<x:warp field="arch" bend="50">{inner}</x:warp></svg>"##);
    let nested = format!(
        r##"{XW}<x:warp field="flag" bend="0"><x:warp field="arch" bend="50">{inner}</x:warp></x:warp></svg>"##
    );
    let a = stops_at(&flat, "<x:textbox").unwrap();
    assert_eq!(
        stops_at(&zero, "<x:textbox").unwrap(),
        a,
        "bend 0 is the identity"
    );
    let b = stops_at(&bent, "<x:textbox").unwrap();
    assert_ne!(b, a, "a bend moves the stops");
    assert_eq!(
        stops_at(&nested, "<x:textbox").unwrap(),
        b,
        "an identity outer warp changes nothing; the inner one still applies"
    );
    let run = &b.runs[0];
    assert_eq!(run.stops.len(), 3);
    // the arch displaces y only: x is the flat run's, still monotone — and every
    // coordinate stays finite although the caret heads lie far outside the ink bbox
    for (s, f) in run.stops.iter().zip(&a.runs[0].stops) {
        assert!((s.top.0 - f.top.0).abs() < 1e-9, "{s:?} vs {f:?}");
    }
    assert!(run.stops.windows(2).all(|w| w[1].top.0 > w[0].top.0));
    assert!(run
        .stops
        .iter()
        .all(|s| [s.top.0, s.top.1, s.bottom.0, s.bottom.1]
            .iter()
            .all(|v| v.is_finite())));
}

#[test]
fn caret_stops_follow_a_text_on_path_effect() {
    let svg = format!(
        r##"{XW}<path id="p" d="M0 50 L200 50" fill="none"/><x:textpath in="#p" effect="skew" font-size="10">hey</x:textpath><x:textpath in="#p" effect="rainbow" align="middle" font-size="10">hey</x:textpath></svg>"##
    );
    let out = compile_outlined(&svg);
    assert!(!out.contains("<text"), "{out}");
    let m = Mono.font_metrics(&TextStyle::default(), 10.0);
    let skew = stops_at(&svg, "<x:textpath").unwrap();
    assert_eq!(skew.runs.len(), 1);
    let run = &skew.runs[0];
    assert_eq!(run.text, "hey");
    assert_eq!(run.stops.len(), 4);
    for (i, s) in run.stops.iter().enumerate() {
        // a flat path: the pen advances 0.5 em per scalar from the path's start,
        // and the caret spans ascent above the path's y to descent below it
        assert!((s.top.0 - 5.0 * i as f64).abs() < 1e-9, "{s:?}");
        assert!((s.top.1 - (50.0 - m.ascent)).abs() < 1e-9, "{s:?}");
        assert!((s.bottom.1 - (50.0 + m.descent)).abs() < 1e-9, "{s:?}");
    }
    // the centred run: `run_offset` puts its pen at (200 − 15) / 2 along the path
    let off = svg.rfind("<x:textpath").unwrap();
    let rainbow = caret_stops_impl(&svg, off, "balanced", &Mono, &NoShaper, &BoxOutliner).unwrap();
    let s = &rainbow.runs[0].stops;
    assert!((s[0].top.0 - 92.5).abs() < 1e-9, "{:?}", s[0]);
    assert!((s[3].top.0 - 107.5).abs() < 1e-9, "{:?}", s[3]);
    assert!(
        (s[3].bottom.1 - (50.0 + m.descent)).abs() < 1e-9,
        "{:?}",
        s[3]
    );
}

#[test]
fn caret_stops_are_per_element_and_skip_harvests() {
    let svg = format!(
        r##"{XW}<x:warp field="arch" bend="30"><x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">one</x:textbox><x:textbox x="0" y="40" width="80" height="20" font-size="10" outline="true">two</x:textbox></x:warp><x:boolean op="union"><x:textbox x="0" y="100" width="80" height="20" font-size="10" outline="true">three</x:textbox></x:boolean></svg>"##
    );
    let offs: Vec<usize> = svg.match_indices("<x:textbox").map(|(i, _)| i).collect();
    let at = |o| caret_stops_impl(&svg, o, "balanced", &Mono, &NoShaper, &BoxOutliner);
    // two boxes share one warp (and so one envelope frame): each query answers for
    // its own element only
    let one = at(offs[0]).unwrap();
    assert_eq!(one.runs.len(), 1);
    assert_eq!(one.runs[0].text, "one");
    let two = at(offs[1]).unwrap();
    assert_eq!(two.runs.len(), 1);
    assert_eq!(two.runs[0].text, "two");
    // a boolean operand is harvested into a region, not rendered as itself
    assert!(at(offs[2]).is_none());
}

/// The scratch compile starts at the element (or its outermost non-plain-SVG
/// ancestor), not its top-level fragment: on every dataset document, every
/// element's stops equal what compiling its whole fragment records — the plain
/// SVG above the scope neither moves nor claims them.
#[test]
fn caret_stops_scope_matches_the_whole_fragment() {
    let platform = Platform::new(Mono, NoShaper, BoxOutliner);
    let backend = platform.backend();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dataset");
    let mut outlined = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("xsvg") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let parsed = session::Parsed::new(&src).unwrap();
        // stops are only ever recorded for these two
        let text = |n: &roxmltree::Node| {
            n.tag_name().namespace() == Some(XSVG_NS)
                && matches!(n.tag_name().name(), "textbox" | "textpath")
        };
        for node in parsed.doc.root_element().descendants().filter(text) {
            let off = node.range().start;
            let Some(top) = top_level_at(&parsed.doc, off) else {
                continue;
            };
            let whole = parsed.caret_stops_from(node, top, &backend, QualityProfile::Balanced);
            let scoped = parsed.caret_stops(off, &backend, QualityProfile::Balanced);
            let doc_island = in_doc_island(node);
            assert_eq!(
                scoped,
                if doc_island { None } else { whole },
                "{}: <{}> at {off}",
                path.display(),
                node.tag_name().name()
            );
            outlined += scoped.is_some() as usize;
        }
    }
    assert!(
        outlined >= 40,
        "the dataset exercises outlined text ({outlined})"
    );
}

/// An outliner whose `advances` disagree with the text it is asked about — a
/// stand-in for a host answering for the wrong string or with NaNs.
struct BadAdvances;
impl GlyphOutliner for BadAdvances {
    fn outline(&self, _t: &str, _s: &TextStyle, _sz: f64, x: f64, b: f64) -> Option<String> {
        Some(format!("M{x},{b} h1 v-1 h-1 Z"))
    }
    fn advances(&self, text: &str, _s: &TextStyle, size: f64) -> Option<Vec<f64>> {
        // one entry short, and a NaN for good measure
        let mut v: Vec<f64> = (1..text.chars().count())
            .map(|i| i as f64 * 0.5 * size)
            .collect();
        if let Some(first) = v.first_mut() {
            *first = f64::NAN;
        }
        Some(v)
    }
}

#[test]
fn caret_stops_decline_when_the_advances_do_not_cover_the_text() {
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="80" height="20" font-size="10" outline="true">hey</x:textbox></svg>"##
    );
    // the compile still outlines (the glyph paths need no advances) …
    let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &BadAdvances).unwrap();
    assert!(
        out.contains("<path d=\"M") && !out.contains("<text"),
        "{out}"
    );
    // … but a stop run that cannot sit between the glyphs is not reported at all,
    // rather than reported short or with a NaN in it
    let off = svg.find("<x:textbox").unwrap();
    assert!(caret_stops_impl(&svg, off, "balanced", &Mono, &NoShaper, &BadAdvances).is_none());
}

#[test]
fn caret_stops_record_a_referenced_box_once() {
    // inside one top-level <g>, the outlined box renders in place AND is harvested
    // as the region of a second box (`in="#t"` → x_output_geometry, outlining
    // forced) during the same scratch compile; only its own rendering may record,
    // so the query sees one run, not one per harvest
    let svg = format!(
        r##"{XW}<g><x:textbox id="t" x="0" y="0" width="80" height="20" font-size="10" outline="true">hi</x:textbox><x:textbox in="#t" font-size="4">tiny words here</x:textbox></g></svg>"##
    );
    let out = compile_impl(&svg, "balanced", false, &Mono, &BoxShaper, &BoxOutliner).unwrap();
    assert!(out.contains("<path d=\"M"), "{out}");
    let off = svg.find("<x:textbox").unwrap();
    let cs = caret_stops_impl(&svg, off, "balanced", &Mono, &BoxShaper, &BoxOutliner).unwrap();
    assert_eq!(cs.runs.len(), 1, "{cs:?}");
    assert_eq!(cs.runs[0].text, "hi");
}

// ---------------------------------------------------------------------------
// font-family inheritance into compiled text (§4.1)

/// The `font-family="…"` each `<text>` in `out` carries, in document order.
fn text_families(out: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(out).unwrap();
    doc.descendants()
        .filter(|n| n.tag_name().name() == "text")
        .map(|n| n.attribute("font-family").unwrap_or("").to_string())
        .collect()
}

#[test]
fn compiled_text_inherits_font_family_from_ancestors() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" font-family="Root">
<x:theme><x:font name="tok" font-family="Token" font-size="40"/></x:theme>
<x:textbox x="0" y="0" width="80" height="20" font-size="10">root</x:textbox>
<g font-family="Group"><x:textbox x="0" y="0" width="80" height="20" font-size="10">group</x:textbox></g>
<g font-family="Group"><x:textbox x="0" y="0" width="80" height="20" font-size="10" font-family="Own">own</x:textbox></g>
<g font-family="Group"><x:textbox x="0" y="0" width="80" height="20" font-size="10" x:font="tok">own token</x:textbox></g>
<g x:font="tok"><x:textbox x="0" y="0" width="80" height="20" font-size="10">group token</x:textbox></g>
<g font-family="Group" x:font="tok"><x:textbox x="0" y="0" width="80" height="20" font-size="10">attr beats token</x:textbox></g>
<g font-family="Outer"><g><x:textbox x="0" y="0" width="80" height="20" font-size="10">nearest</x:textbox></g></g>
<x:list x="0" y="0" width="80" font-size="10"><x:li>list</x:li></x:list>
<textArea x="0" y="0" width="80" height="20" font-size="10">area</textArea>
</svg>"##;
    let out = compile_test(svg);
    assert_eq!(
        text_families(&out),
        ["Root", "Group", "Own", "Token", "Token", "Group", "Outer", "Root", "Root"],
        "{out}"
    );
    // the layout seam measures in the same family it will render in
    let off = svg.find("<g font-family=\"Group\">").unwrap();
    let tl = layout_at(svg, svg[off..].find("<x:textbox").unwrap() + off).unwrap();
    assert_eq!(tl.paras[0].style.family, "Group");
}

#[test]
fn only_font_family_inherits() {
    // a group's size and a root token's size never reach a box; the box keeps its
    // own size (here the default 16), so nothing above it can reflow it
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" x:font="body">
<x:theme><x:font name="body" font-family="Body" font-size="40" font-weight="bold"/></x:theme>
<g font-size="30" font-weight="bold" font-style="italic"><x:textbox x="0" y="0" width="200" height="40">words</x:textbox></g>
</svg>"##;
    let out = compile_test(svg);
    assert_eq!(text_families(&out), ["Body"], "{out}");
    let doc = roxmltree::Document::parse(&out).unwrap();
    let text = doc
        .descendants()
        .find(|n| n.tag_name().name() == "text")
        .unwrap();
    assert_eq!(text.attribute("font-size"), Some("16"), "{out}");
    assert_eq!(text.attribute("font-weight"), Some("normal"), "{out}");
    assert_eq!(text.attribute("font-style"), Some("normal"), "{out}");
    // the token still lands whole on the root's output, where plain text inherits it
    assert!(out.contains(r#"<svg xmlns="http://www.w3.org/2000/svg" font-family="Body" font-size="40" font-weight="bold">"#), "{out}");
}

#[test]
fn inherited_font_family_keeps_fragment_parity() {
    // a fragment compiles against the whole parsed document, so the box inside the
    // group sees the root's family exactly as in the full compile
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" font-family="Root">
<rect width="10" height="10"/>
<g><x:textbox x="0" y="0" width="80" height="20" font-size="10">inside</x:textbox></g>
</svg>"##;
    let full = compile_test(svg);
    let off = svg.find("<g>").unwrap();
    let frag =
        compile_fragment_impl(svg, "balanced", false, off, &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(frag.contains(r#"font-family="Root""#), "{frag}");
    assert!(
        full.contains(&frag),
        "fragment must be a byte-exact span of the full compile:\n{frag}\n---\n{full}"
    );
}

// ---------------------------------------------------------------------------
// <x:doc> is an island the compiler never reads (§4.4)

#[test]
fn doc_island_is_inert() {
    // the island carries everything the compiler otherwise scans for — an id that
    // clashes with a real element's (and comes first, so it would win the id table),
    // a theme token, an import binding a namespace the body then uses, a textbox, a
    // layer — and must change nothing: output, references, dependents, text layout
    let island = r##"<x:doc><x:settings id="box" x:layer="hidden"/><x:theme><x:color name="leak" value="#f00"/></x:theme><x:import ns="urn:ed" src="ed.wasm"/><x:textbox id="hidden" x="0" y="0" width="50" height="20">never</x:textbox><ed:note xmlns:ed="urn:ed">ok</ed:note></x:doc>"##;
    let body = r##"<rect id="box" x="0" y="0" width="40" height="40" fill="var(leak, #0f0)"/><x:textbox in="#box" font-size="10">words</x:textbox><ed:thing xmlns:ed="urn:ed"/>"##;
    let with = format!("{XW}{island}{body}</svg>");
    let without = format!("{XW}{body}</svg>");
    let out = compile_test(&with);
    assert_eq!(out, compile_test(&without), "{out}");
    assert!(
        out.contains(r##"fill="#0f0""##),
        "the island's token must not resolve: {out}"
    );
    assert!(
        out.contains("words"),
        "the textbox must bind to the real rect: {out}"
    );
    assert!(!out.contains("never"), "{out}");
    // a nested island is an island too (the serializer self-closes a <g> that was
    // empty in the source but not one whose only child was dropped — cosmetic)
    let nested = format!("{XW}<g>{island}</g>{body}</svg>");
    assert_eq!(
        compile_test(&nested).replace("<g></g>", "<g/>"),
        compile_test(&format!("{XW}<g/>{body}</svg>"))
    );
    // editing inside the island invalidates nothing; editing the real rect still does
    assert_eq!(
        dependents_impl(&with, with.find("<x:settings").unwrap()),
        Vec::<(usize, usize)>::new()
    );
    assert!(!dependents_impl(&with, with.find("<rect").unwrap()).is_empty());
    // text that never renders has no layout
    assert!(layout_at(&with, with.find(r#"<x:textbox id="hidden""#).unwrap()).is_none());
    assert!(layout_at(&with, with.find(r##"<x:textbox in="#box""##).unwrap()).is_some());
}

// ---------------------------------------------------------------------------
// measure_textbox: how big a box must be for its content (Incremental.md §8)

fn measure_at(svg: &str, needle: &str) -> Option<TextboxSize> {
    let off = svg.find(needle).unwrap();
    measure_textbox_impl(svg, off, "balanced", &Mono, &BoxShaper, &NoOutliner)
}

fn tspans(out: &str) -> usize {
    out.matches("<tspan").count()
}

#[test]
fn measure_textbox_is_the_cap_band_plus_padding() {
    // Mono: 5 per char at size 10; content width 50 → "alpha beta" | "gamma" | "delta"
    let svg = format!(
        r##"{XW}<x:textbox x="4" y="9" width="56" height="5" padding="3" font-size="10" text-overflow="ellipsis">alpha beta gamma delta</x:textbox></svg>"##
    );
    let fm = Mono.font_metrics(&TextStyle::default(), 10.0);
    let size = measure_at(&svg, "<x:textbox").unwrap();
    // cap of the first line, two more advances of size × line-height, the last descent
    let band = fm.cap_height + 2.0 * 12.0 + fm.descent;
    assert!((size.height - (band + 6.0)).abs() < 1e-9, "{size:?}");
    assert!(
        (size.width - 56.0).abs() < 1e-9,
        "the widest line fills the box: {size:?}"
    );
    // compiled at that height the box holds every line unclipped; a hair less loses one
    let at = |h: f64| compile_test(&svg.replace(r#"height="5""#, &format!(r#"height="{h}""#)));
    let fits = at(size.height);
    assert_eq!(tspans(&fits), 3, "{fits}");
    assert!(!fits.contains('…'), "{fits}");
    // §6.5 clips by BASELINE, so the box may lose up to the last descent before a
    // line goes; one hair under that and it does
    let tight = at(size.height - fm.descent - 0.5);
    assert!(tspans(&tight) < 3 || tight.contains('…'), "{tight}");
}

#[test]
fn measure_textbox_width_is_the_content_and_empty_is_one_line() {
    let svg = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="5" padding="3" font-size="10">hi</x:textbox><x:textbox x="0" y="0" width="200" height="5" font-size="10"></x:textbox></svg>"##
    );
    let short = measure_at(&svg, "<x:textbox").unwrap();
    assert!(
        (short.width - 16.0).abs() < 1e-9 && (short.height - 15.0).abs() < 1e-9,
        "{short:?}"
    );
    let off = svg.rfind("<x:textbox").unwrap();
    let empty = measure_textbox_impl(&svg, off, "balanced", &Mono, &NoShaper, &NoOutliner).unwrap();
    assert!(
        (empty.height - 9.0).abs() < 1e-9 && empty.width == 0.0,
        "cap + descent: {empty:?}"
    );
}

#[test]
fn measure_textbox_stacks_paragraphs_and_lists() {
    let svg = format!(
        r##"{XW}<x:textbox x="10" y="20" width="100" height="5" padding="2" font-size="10"><x:p>intro words</x:p><x:list><x:li>first</x:li><x:li>second</x:li></x:list><x:p>outro</x:p></x:textbox></svg>"##
    );
    let size = measure_at(&svg, "<x:textbox").unwrap();
    let with = |h: f64| svg.replace(r#"height="5""#, &format!(r#"height="{h}""#));
    // every block's lines render at the measured height, and the layout seam puts
    // the last line's descent exactly on the padded bottom edge
    let fits = compile_test(&with(size.height));
    assert_eq!(tspans(&fits), 4, "{fits}");
    let tl = layout_at(&with(size.height), svg.find("<x:textbox").unwrap()).unwrap();
    let last = tl.paras.last().unwrap().lines.last().unwrap();
    let fm = Mono.font_metrics(&TextStyle::default(), 10.0);
    assert!(
        (last.baseline + fm.descent - (20.0 + size.height - 2.0)).abs() < 1e-9,
        "{tl:?}"
    );
    assert!(tspans(&compile_test(&with(size.height - fm.descent - 0.5))) < 4);
    // the same answer when the geometry is a referenced rect
    let via_rect = format!(
        r##"{XW}<rect id="r" x="10" y="20" width="100" height="5"/><x:textbox in="#r" padding="2" font-size="10"><x:p>intro words</x:p><x:list><x:li>first</x:li><x:li>second</x:li></x:list><x:p>outro</x:p></x:textbox></svg>"##
    );
    assert_eq!(measure_at(&via_rect, "<x:textbox"), Some(size));
}

#[test]
fn measure_textbox_none_off_a_rectangular_box() {
    let svg = format!(
        r##"{XW}<circle id="c" cx="30" cy="30" r="30"/><x:textbox in="#c" font-size="10">curved</x:textbox><rect x="0" y="0" width="1" height="1"/></svg>"##
    );
    assert!(
        measure_at(&svg, "<x:textbox").is_none(),
        "curved region flow has no height to grow"
    );
    assert!(measure_at(&svg, "<rect").is_none());
}

// ---------------------------------------------------------------------------
// Units: incremental compilation one level below the top (Incremental.md §9)

fn unit_platform() -> Platform<&'static Mono, &'static BoxShaper, &'static BoxOutliner> {
    Platform::new(&Mono, &BoxShaper, &BoxOutliner)
}

/// Top-level elements in the order the root writes them: source order unless a
/// child carries `x:layer`/`x:order`, then stable-sorted by (band, order) as
/// `emit_passthrough` does (§5.1).
fn output_order<'a>(tops: &[roxmltree::Node<'a, 'a>]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..tops.len()).collect();
    if tops.iter().any(|c| {
        c.attribute((XSVG_NS, "layer")).is_some() || c.attribute((XSVG_NS, "order")).is_some()
    }) {
        idx.sort_by(|&a, &b| {
            let k = |n: roxmltree::Node| (layer_band(n).unwrap_or(0), attr_num_ns(n, "order", 0.0));
            let (ka, kb) = (k(tops[a]), k(tops[b]));
            ka.0.cmp(&kb.0)
                .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
                .then(a.cmp(&b))
        });
    }
    idx
}

/// Index keys: every element named by its document-order index, so the same
/// element keeps its key across an edit that only shifts bytes (what the kit's
/// serializer supplies). Generated ids then seed on keys and stay put.
fn index_keys(src: &str) -> MapKeys {
    MapKeys(
        node_spans_impl(src)
            .into_iter()
            .enumerate()
            .map(|(i, (start, _))| (start, format!("k{i}")))
            .collect(),
    )
}

fn dataset_files() -> Vec<std::path::PathBuf> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../dataset");
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "xsvg"))
        .collect();
    files.sort();
    assert!(files.len() > 40, "{}", files.len());
    files
}

#[test]
fn units_are_verbatim_slices_of_their_fragment_over_the_dataset() {
    // THE §9 invariant, over every sample: each unit's span in its fragment's
    // output is byte-identical to the unit compiled alone, the spans are in
    // output order and never overlap, `unit_range` names the same unit — and
    // each fragment is a slice of the full compile, so the chain unit ⊂ fragment
    // ⊂ full is complete in-process.
    let p = unit_platform();
    let (mut units, mut containers) = (0, 0);
    for path in &dataset_files() {
        let src = std::fs::read_to_string(path).unwrap();
        let doc = roxmltree::Document::parse(&src).unwrap();
        let tops: Vec<roxmltree::Node> = doc
            .root_element()
            .children()
            .filter(|c| c.is_element())
            .collect();
        for sourcemap in [false, true] {
            let full = compile_platform_impl(&src, "balanced", sourcemap, "", &p).unwrap();
            let mut in_full = 0;
            for &ti in &output_order(&tops) {
                let off = tops[ti].range().start;
                let (frag, spans) =
                    compile_fragment_spans_platform_impl(&src, "balanced", sourcemap, off, "", &p)
                        .unwrap();
                assert_eq!(
                    frag,
                    compile_fragment_platform_impl(&src, "balanced", sourcemap, off, "", &p)
                        .unwrap(),
                    "{path:?}: the spans compile is the fragment compile"
                );
                if !frag.is_empty() {
                    let at = full[in_full..]
                        .find(&frag)
                        .unwrap_or_else(|| panic!("{path:?}: fragment not found in order"));
                    in_full += at + frag.len();
                }
                if !spans.is_empty() {
                    containers += 1;
                }
                let mut cursor = 0;
                for span in &spans {
                    assert!(
                        span.output.0 >= cursor && span.output.1 >= span.output.0,
                        "{path:?} {span:?}"
                    );
                    cursor = span.output.1;
                    let alone = compile_unit_platform_impl(
                        &src,
                        "balanced",
                        sourcemap,
                        span.source.0,
                        "",
                        &p,
                    )
                    .unwrap();
                    assert_eq!(
                        &frag[span.output.0..span.output.1],
                        alone,
                        "{path:?}: unit at {} differs when compiled alone",
                        span.source.0
                    );
                    assert_eq!(
                        unit_range_impl(&src, span.source.0),
                        Some(span.source),
                        "{path:?}"
                    );
                    units += 1;
                }
            }
        }
    }
    assert!(
        units > 100 && containers > 10,
        "units {units}, containers {containers}"
    );
}

#[test]
fn unit_at_follows_the_serializer() {
    let svg = format!(
        r##"{XW}<g id="board" font-family="Root"><rect id="r" x="0" y="0" width="10" height="10"/><x:textbox x="0" y="20" width="80" height="20" font-size="10">words</x:textbox></g><g x:hidden="true"><rect x="0" y="0" width="10" height="10"/></g><g><rect x:hidden="true" x="0" y="0" width="10" height="10"/><circle cx="5" cy="5" r="2"/></g><text x="0" y="50"><tspan>plain</tspan></text><g><rect x:order="2" x="0" y="0" width="1" height="1"/><rect x:order="1" x="2" y="0" width="1" height="1"/></g><g><rect filter="blur(2)" x="0" y="0" width="10" height="10"/></g></svg>"##
    );
    let p = unit_platform();
    let at = |needle: &str| svg.find(needle).unwrap();
    // inside a container: the child; in the container's own tag: the container
    let tb = at("<x:textbox");
    assert_eq!(
        unit_range_impl(&svg, tb),
        Some((
            tb,
            tb + svg[tb..].find("</x:textbox>").unwrap() + "</x:textbox>".len()
        ))
    );
    let board = at(r#"<g id="board""#);
    assert_eq!(unit_range_impl(&svg, board + 3).map(|r| r.0), Some(board));
    // the unit sees its ancestors: the inherited family is in its own output
    let alone = compile_unit_platform_impl(&svg, "balanced", false, tb, "", &p).unwrap();
    assert!(alone.contains(r#"font-family="Root""#), "{alone}");
    // a hidden container compiles to nothing, so it is its own (empty) unit
    let hidden = at(r#"<g x:hidden="true">"#);
    assert_eq!(
        unit_range_impl(&svg, hidden + 30).map(|r| r.0),
        Some(hidden)
    );
    let (frag, spans) =
        compile_fragment_spans_platform_impl(&svg, "balanced", false, hidden, "", &p).unwrap();
    assert!(frag.is_empty() && spans.is_empty(), "{frag} {spans:?}");
    // a hidden child is a unit that compiles to nothing: an empty span, in place
    let hidden_rect = at(r#"<rect x:hidden="true""#);
    let g3 = svg[..hidden_rect].rfind("<g>").unwrap();
    let (_, spans) =
        compile_fragment_spans_platform_impl(&svg, "balanced", false, g3, "", &p).unwrap();
    assert_eq!(spans.len(), 2, "{spans:?}");
    assert_eq!(spans[0].output.0, spans[0].output.1, "{spans:?}");
    assert_eq!(
        compile_unit_platform_impl(&svg, "balanced", false, hidden_rect, "", &p).unwrap(),
        ""
    );
    // text content is not a container: the <text> is its own unit, the tspan nothing's
    let text = at("<text ");
    let tspan = at("<tspan>");
    assert_eq!(unit_range_impl(&svg, tspan).map(|r| r.0), Some(text));
    let (_, spans) =
        compile_fragment_spans_platform_impl(&svg, "balanced", false, text, "", &p).unwrap();
    assert!(spans.is_empty(), "{spans:?}");
    // a restacked container lists its units in OUTPUT order, not source order
    let ordered = at(r#"<rect x:order="2""#);
    let g4 = svg[..ordered].rfind("<g>").unwrap();
    let (frag, spans) =
        compile_fragment_spans_platform_impl(&svg, "balanced", false, g4, "", &p).unwrap();
    assert_eq!(spans.len(), 2);
    assert!(
        spans[0].source.0 > spans[1].source.0,
        "output order: {spans:?}"
    );
    assert!(
        frag[spans[0].output.0..spans[0].output.1].contains("M2,0"),
        "{frag}"
    );
    // a filter def is written before its element, inside the element's own span
    let filtered = at(r#"<rect filter="#);
    let g5 = svg[..filtered].rfind("<g>").unwrap();
    let (frag, spans) =
        compile_fragment_spans_platform_impl(&svg, "balanced", false, g5, "", &p).unwrap();
    let span = &frag[spans[0].output.0..spans[0].output.1];
    assert!(span.starts_with("<filter"), "{span}");
    assert_eq!(
        span,
        compile_unit_platform_impl(&svg, "balanced", false, filtered, "", &p).unwrap()
    );
}

#[test]
fn units_are_the_deepest_leaves_under_nested_containers() {
    let svg = format!(
        r##"{XW}<defs><filter id="f"><feOffset dx="1"/></filter></defs><g id="board" x:artboard="1"><g x:layer="foreground"><g id="inner"><x:textbox x="0" y="20" width="80" height="20" font-size="10">words</x:textbox><rect id="deep" x="0" y="0" width="4" height="4"/></g><circle cx="5" cy="5" r="2"/></g><g id="blurred" filter="blur(2)"><rect id="inblur" x="0" y="0" width="6" height="6"/><circle cx="9" cy="9" r="1"/></g><g filter="url(#f)"><rect id="inref" x="0" y="0" width="7" height="7"/></g><g style="filter:blur(1px)"><rect id="instyle" x="0" y="0" width="8" height="8"/></g><g id="empty"/></g></svg>"##
    );
    let p = unit_platform();
    let at = |needle: &str| svg.find(needle).unwrap();
    let unit = |needle: &str| unit_range_impl(&svg, at(needle)).map(|r| r.0);
    // two plain containers down, the unit is still the leaf itself
    assert_eq!(unit("<x:textbox"), Some(at("<x:textbox")));
    assert_eq!(unit(r#"<rect id="deep""#), Some(at(r#"<rect id="deep""#)));
    // a container's own tag is that container, at any depth
    assert_eq!(
        unit_range_impl(&svg, at(r#"<g id="inner""#) + 3).map(|r| r.0),
        Some(at(r#"<g id="inner""#))
    );
    // a lowered filter's region is measured from the children: one unit
    assert_eq!(unit(r#"<rect id="inblur""#), Some(at(r#"<g id="blurred""#)));
    // a filter the compiler passes through leaves the children independent
    assert_eq!(unit(r#"<rect id="inref""#), Some(at(r#"<rect id="inref""#)));
    assert_eq!(
        unit(r#"<rect id="instyle""#),
        Some(at(r#"<rect id="instyle""#))
    );

    // the fragment lists exactly those units, in output order, each the
    // byte-identical compile of that unit alone
    let board = at(r#"<g id="board""#);
    for sourcemap in [false, true] {
        let (frag, spans) =
            compile_fragment_spans_platform_impl(&svg, "balanced", sourcemap, board, "", &p)
                .unwrap();
        // OUTPUT order: the foreground layer restacks above its siblings
        let want = [
            at(r#"<g id="blurred""#),
            at(r#"<rect id="inref""#),
            at(r#"<rect id="instyle""#),
            at(r#"<g id="empty""#),
            at("<x:textbox"),
            at(r#"<rect id="deep""#),
            at("<circle cx=\"5\""),
        ];
        let got: Vec<usize> = spans.iter().map(|s| s.source.0).collect();
        assert_eq!(got, want, "{spans:?}");
        let mut cursor = 0;
        for span in &spans {
            assert!(span.output.0 >= cursor, "{spans:?}");
            cursor = span.output.1;
            assert_eq!(
                &frag[span.output.0..span.output.1],
                compile_unit_platform_impl(&svg, "balanced", sourcemap, span.source.0, "", &p)
                    .unwrap()
            );
            assert_eq!(unit_range_impl(&svg, span.source.0), Some(span.source));
        }
        // the filtered group's span holds its generated <filter> and both children
        let blurred = &frag[spans[0].output.0..spans[0].output.1];
        assert!(
            blurred.starts_with("<filter") && blurred.contains("<circle"),
            "{blurred}"
        );
    }
}

#[test]
fn unit_dependents_follow_ancestors_and_clip_geometry() {
    let svg = format!(
        r##"{XW}<defs><g id="drawing"><g><rect id="leaf" x="0" y="0" width="10" height="20"/></g></g><clipPath id="src"><circle id="hole" r="9"/></clipPath><clipPath id="cut"><use href="#drawing"/></clipPath><clipPath id="chained"><g clip-path="url(#src)"><circle r="5" transform="translate(3 0)"/></g></clipPath><clipPath id="plain"><circle r="3"/></clipPath></defs><g id="board"><g id="layer"><rect id="deep" x="0" y="0" width="5" height="5"/></g><x:offset in="#layer" d="2"/><x:offset in="#deep" d="1"/><use href="#layer"/></g></svg>"##
    );
    let at = |needle: &str| svg.find(needle).unwrap();
    let deps = |needle: &str| -> Vec<usize> {
        unit_dependents_impl(&svg, &[at(needle)])
            .iter()
            .map(|d| d.0)
            .collect()
    };
    // a nested leaf changes what every ancestor draws: both offsets re-emit,
    // the one tracing the leaf and the one tracing its group (the live <use>
    // re-resolves in the DOM and is not reported)
    let offsets: Vec<usize> = svg.match_indices("<x:offset").map(|(i, _)| i).collect();
    assert_eq!(deps(r#"<rect id="deep""#), offsets);
    // a clip expands a referenced group in place, so it bakes that geometry
    assert_eq!(deps(r#"<rect id="leaf""#), [at(r#"<clipPath id="cut""#)]);
    // and re-emits another clip as a derived one when a wrapper asks for it
    assert_eq!(
        deps(r#"<circle id="hole""#),
        [at(r#"<clipPath id="chained""#)]
    );
}

#[test]
fn unit_dependents_see_inside_a_container() {
    // the deck's case: an outlined word inside a warp, traced by an <x:offset>
    // beside it in the same artboard. Editing the word is editing the warp's
    // unit; the offset bakes the warp's geometry, so it must re-emit — and the
    // top-level scan, which only looks between artboards, cannot say so.
    let svg = format!(
        r##"{XW}<g x:artboard="1"><x:warp id="w" field="arch" bend="30"><x:textbox id="archword" x="0" y="0" width="80" height="20" font-size="10" outline="true">ARCH</x:textbox></x:warp><x:offset in="#w" d="3"/><rect x="0" y="0" width="5" height="5"/></g><g x:artboard="2"><x:offset in="#w" d="5"/></g>"##
    ) + "</svg>";
    let word = svg.find("<x:textbox").unwrap();
    let offsets: Vec<usize> = svg.match_indices("<x:offset").map(|(i, _)| i).collect();
    let deps = unit_dependents_impl(&svg, &[word]);
    let starts: Vec<usize> = deps.iter().map(|d| d.0).collect();
    assert_eq!(
        starts, offsets,
        "both offsets, the sibling one included: {deps:?}"
    );
    // the top-level scan reports only the other artboard
    let tops = dependents_impl(&svg, word);
    assert_eq!(tops.len(), 1, "{tops:?}");
    assert!(tops[0].0 < offsets[1] && offsets[1] < tops[0].1);
    // and nothing when the seed is the plain rect
    assert!(unit_dependents_impl(&svg, &[svg.find("<rect").unwrap()]).is_empty());
}

/// A small edit inside the unit at `span`, as an editor would make one: a
/// character appended to its first real text node, else one of its own numeric
/// geometry attributes bumped. `None` for a unit nothing of the kind applies to.
fn synthetic_edit(
    src: &str,
    doc: &roxmltree::Document,
    span: (usize, usize),
) -> Option<(String, usize)> {
    let unit = doc
        .descendants()
        .find(|n| n.is_element() && n.range().start == span.0)?;
    if unit.tag_name().namespace() == Some(XSVG_NS)
        && matches!(unit.tag_name().name(), "theme" | "doc" | "import")
    {
        return None; // declarations: the protocol falls back to a full compile
    }
    if let Some(t) = unit
        .descendants()
        .find(|n| n.is_text() && n.text().is_some_and(|t| !t.trim().is_empty()))
    {
        let end = t.range().end;
        return Some((format!("{}x{}", &src[..end], &src[end..]), end));
    }
    let tag_end = span.0 + src[span.0..span.1].find('>')?;
    let tag = &src[span.0..tag_end];
    for name in ["width", "height", "r", "cx", "cy", "x", "y"] {
        let needle = format!(" {name}=\"");
        if let Some(i) = tag.find(&needle) {
            let vs = span.0 + i + needle.len();
            let ve = vs + src[vs..].find('"')?;
            let v: f64 = src[vs..ve].parse().ok()?;
            let bumped = fmt(v + 1.0);
            return Some((format!("{}{}{}", &src[..vs], bumped, &src[ve..]), vs));
        }
    }
    None
}

#[test]
fn unit_patches_equal_a_full_recompile_over_the_dataset() {
    // THE §9 edit oracle: for every unit of every sample, make a small edit inside
    // it, then rebuild the document the way a host would — the edited unit and
    // every `unit_dependents` unit recompiled alone and spliced over their old
    // spans, everything else kept verbatim — and demand the result be
    // byte-identical to a full compile of the edited source. A missing dependency
    // edge, or a unit that is not really independent of its neighbours, fails here.
    // Keyed and with the sourcemap on, as a host compiles: generated ids then seed
    // on keys, so an untouched unit's bytes survive the edit shifting its offset.
    let (mut edits, mut with_deps) = (0, 0);
    for path in &dataset_files() {
        let src = std::fs::read_to_string(path).unwrap();
        let doc = roxmltree::Document::parse(&src).unwrap();
        let p = unit_platform().with_keys(index_keys(&src));
        let full = compile_platform_impl(&src, "balanced", true, "", &p).unwrap();
        // every top-level element's fragment, where it sits in the full output (the
        // root's own order), and its unit spans
        let tops: Vec<roxmltree::Node> = doc
            .root_element()
            .children()
            .filter(|c| c.is_element())
            .collect();
        let order = output_order(&tops);
        let mut frags: Vec<(usize, String, Vec<UnitSpan>)> = Vec::new();
        let mut cursor = 0;
        for &ti in &order {
            let (frag, spans) = compile_fragment_spans_platform_impl(
                &src,
                "balanced",
                true,
                tops[ti].range().start,
                "",
                &p,
            )
            .unwrap();
            let start = if frag.is_empty() {
                cursor
            } else {
                cursor + full[cursor..].find(&frag).unwrap()
            };
            cursor = start + frag.len();
            frags.push((start, frag, spans));
        }
        for oi in 0..order.len() {
            let spans = frags[oi].2.clone();
            for (si, span) in spans.iter().enumerate() {
                let Some((edited, at)) = synthetic_edit(&src, &doc, span.source) else {
                    continue;
                };
                let delta = edited.len() as isize - src.len() as isize;
                let shift = |off: usize| {
                    if off > at {
                        (off as isize + delta) as usize
                    } else {
                        off
                    }
                };
                let pe = unit_platform().with_keys(index_keys(&edited));
                let fresh_full = compile_platform_impl(&edited, "balanced", true, "", &pe).unwrap();
                let deps = unit_dependents_impl(&edited, &[shift(span.source.0)]);
                with_deps += usize::from(!deps.is_empty());
                let mut patched = String::new();
                let mut cur = 0;
                for (oj, (start, frag, spans_j)) in frags.iter().enumerate() {
                    patched.push_str(&full[cur..*start]);
                    cur = start + frag.len();
                    let top_now = shift(tops[order[oj]].range().start);
                    let mut frag_now = frag.clone();
                    if spans_j.is_empty() {
                        // its own unit: refreshed only when it is a dependent
                        if deps.iter().any(|d| d.0 == top_now) {
                            frag_now = compile_fragment_platform_impl(
                                &edited, "balanced", true, top_now, "", &pe,
                            )
                            .unwrap();
                        }
                    } else {
                        // the edited unit and the dependent units, spliced last to first
                        for (sj, s) in spans_j.iter().enumerate().rev() {
                            let now = shift(s.source.0);
                            if (oj == oi && sj == si) || deps.iter().any(|d| d.0 == now) {
                                let fresh = compile_unit_platform_impl(
                                    &edited, "balanced", true, now, "", &pe,
                                )
                                .unwrap();
                                frag_now.replace_range(s.output.0..s.output.1, &fresh);
                            }
                        }
                    }
                    patched.push_str(&frag_now);
                }
                patched.push_str(&full[cur..]);
                assert_eq!(
                    patched, fresh_full,
                    "{path:?}: editing the unit at {} and patching it with its dependents {deps:?} does not match a full recompile",
                    span.source.0
                );
                edits += 1;
            }
        }
    }
    eprintln!("unit edit oracle: {edits} edits, {with_deps} with dependents");
    assert!(
        edits > 100 && with_deps > 5,
        "edits {edits}, with dependents {with_deps}"
    );
}

// ---------------------------------------------------------------------------
// text-transform (§6.18): the case text renders in, baked before layout

/// Capitals twice as wide as anything else, so a case change moves a line break.
struct Caps;
impl Measurer for Caps {
    fn measure(&self, text: &str, _style: &TextStyle, size: f64) -> f64 {
        text.chars()
            .map(|c| if c.is_uppercase() { size } else { 0.5 * size })
            .sum()
    }
}

fn caps(svg: &str) -> String {
    compile_impl(svg, "balanced", false, &Caps, &NoShaper, &BoxOutliner).unwrap()
}

#[test]
fn text_transform_uppercases_what_renders_and_what_wraps() {
    // "ab cd ef" is 40 wide and fits 60; "AB CD EF" is 70 and must wrap
    let box_ = |tf: &str| {
        format!(
            r#"{XW}<x:textbox x="0" y="0" width="60" height="80" font-size="10"{tf}>ab cd ef</x:textbox></svg>"#
        )
    };
    let plain = caps(&box_(""));
    assert!(plain.contains("ab cd ef"), "{plain}");
    let up = caps(&box_(r#" text-transform="uppercase""#));
    assert!(!up.contains("ab") && up.contains("AB CD"), "{up}");
    assert!(
        up.contains(">EF<"),
        "the capitals' width wraps the last word: {up}"
    );
    assert!(
        !up.contains("text-transform"),
        "baked, never copied out: {up}"
    );
    // and back down
    let low = caps(&format!(
        r#"{XW}<x:textbox x="0" y="0" width="60" height="80" font-size="10" text-transform="lowercase">AB CD EF</x:textbox></svg>"#
    ));
    assert!(low.contains("ab cd ef"), "{low}");
}

#[test]
fn text_transform_cascades_box_paragraph_run() {
    let out = caps(&format!(
        r#"{XW}<x:textbox x="0" y="0" width="400" height="80" font-size="10" text-transform="uppercase"><x:p>one <tspan text-transform="none" font-weight="700">two</tspan> three</x:p><x:p text-transform="lowercase">FOUR <tspan text-transform="uppercase">five</tspan></x:p></x:textbox></svg>"#
    ));
    for want in ["ONE", "two", "THREE", "four", "FIVE"] {
        assert!(out.contains(want), "{want}: {out}");
    }
    // a run that differs only in case is the base run: no tspan for it
    assert!(!out.contains("<tspan>FIVE"), "{out}");
}

#[test]
fn text_transform_reaches_lists_tables_tokens_and_plain_text() {
    let out = caps(&format!(
        r##"{XW}<x:theme><x:font name="eyebrow" font-size="9" text-transform="uppercase"/></x:theme>
<x:list x="0" y="0" font-size="10" text-transform="uppercase"><x:li>alpha</x:li><x:li text-transform="none">beta</x:li></x:list>
<x:table x="0" y="100" width="200"><x:col text-transform="uppercase"/><x:col/><x:tr><x:td>gamma</x:td><x:td>delta</x:td></x:tr></x:table>
<x:textbox x="0" y="200" width="200" height="20" x:font="eyebrow">epsilon</x:textbox>
<text x="0" y="300" text-transform="uppercase">zeta <tspan text-transform="lowercase">ETA</tspan> theta</text>
<text x="0" y="320" x:font="eyebrow">iota</text>
<g text-transform="uppercase"><text x="0" y="340">kappa</text></g>
<textArea x="0" y="360" width="200" height="40" text-transform="uppercase">lambda <tspan font-weight="700">mu</tspan></textArea>
<text x="0" y="420" inline-size="200" text-transform="uppercase">nu xi</text></svg>"##
    ));
    for want in [
        "ALPHA", "beta", "GAMMA", "delta", "EPSILON", "ZETA", "eta", "THETA", "IOTA", "LAMBDA",
        "MU", "NU XI",
    ] {
        assert!(out.contains(want), "{want}: {out}");
    }
    // like every font property but the family, it is the text element's own (§4.1)
    assert!(out.contains("kappa"), "{out}");
    assert!(!out.contains("text-transform"), "{out}");
}

#[test]
fn text_transform_shapes_outlines_layout_and_caret_stops() {
    // text on a path: the run outlined, measured and stopped is the capitals
    let svg = format!(
        r##"{XW}<path id="p" d="M0,50 C40,0 80,100 120,50"/><x:textpath in="#p" effect="rainbow" text-transform="uppercase" font-size="10">skew</x:textpath></svg>"##
    );
    let off = svg.find("<x:textpath").unwrap();
    let stops = caret_stops_impl(&svg, off, "balanced", &Caps, &NoShaper, &BoxOutliner).unwrap();
    assert_eq!(stops.runs[0].text, "SKEW");
    assert_eq!(
        stops.runs[0].stops.len(),
        5,
        "one stop per source character, and the end"
    );
    // the editor's layout seam reports the rendered lines and words
    let svg = format!(
        r#"{XW}<x:textbox x="0" y="0" width="60" height="80" font-size="10" text-transform="uppercase">ab cd ef</x:textbox></svg>"#
    );
    let tl = text_layout_impl(
        &svg,
        svg.find("<x:textbox").unwrap(),
        "balanced",
        &Caps,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    let p = &tl.paras[0];
    assert_eq!(p.style.text_transform, TextTransform::Uppercase);
    assert_eq!(p.words, ["AB", "CD", "EF"]);
    assert_eq!(p.lines.len(), 2, "{:?}", p.lines);
    assert_eq!(p.lines[1].text, "EF");
    // and sizing to content counts the capitals: two lines, not one
    let size = measure_textbox_impl(
        &svg,
        svg.find("<x:textbox").unwrap(),
        "balanced",
        &Caps,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    let one = measure_textbox_impl(
        &svg.replace(r#" text-transform="uppercase""#, ""),
        svg.find("<x:textbox").unwrap(),
        "balanced",
        &Caps,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert!(size.height > one.height, "{size:?} vs {one:?}");
}

// ---------------------------------------------------------------------------
// overflow="visible" (§6.6): nothing cropped, the cropped lines stamped

const STAMP: &str = r#"data-xsvg-overflow="1""#;

fn editor(svg: &str) -> String {
    compile_impl(svg, "balanced", true, &Mono, &BoxShaper, &BoxOutliner).unwrap()
}

/// A 12-tall box at font-size 10 shows one line of the three this text wraps to.
fn short_box(extra: &str) -> String {
    format!(
        r#"{XW}<x:textbox x="0" y="0" width="60" height="12" font-size="10"{extra}>alpha beta gamma delta epsilon zeta</x:textbox></svg>"#
    )
}

#[test]
fn overflow_visible_renders_every_line_and_stamps_the_cropped_ones() {
    let lines = |out: &str| out.matches("<tspan x=").count();
    assert_eq!(lines(&editor(&short_box(""))), 1, "a box crops by default");
    let out = editor(&short_box(r#" overflow="visible""#));
    assert_eq!(lines(&out), 3, "{out}");
    assert_eq!(
        out.matches(STAMP).count(),
        2,
        "the two lines a box would drop: {out}"
    );
    assert!(out.contains("zeta"), "{out}");
    // text-overflow has nothing to act on: no ellipsis, block or inline
    let out = editor(&short_box(
        r#" overflow="visible" text-overflow="ellipsis""#,
    ));
    assert!(!out.contains('…') && out.contains("zeta"), "{out}");
    // the stamp is editor output only; the attribute is never copied out
    let export = compile_impl(
        &short_box(r#" overflow="visible""#),
        "balanced",
        false,
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert_eq!(lines(&export), 3);
    assert!(
        !export.contains("data-xsvg-overflow") && !export.contains("overflow="),
        "{export}"
    );
}

#[test]
fn overflow_visible_keeps_the_valign_placement() {
    // middle-aligned overflow spills both ways: lines above the box and below
    let svg = short_box(r#" overflow="visible" valign="middle""#);
    let tl = text_layout_impl(
        &svg,
        svg.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    let ls = &tl.paras[0].lines;
    assert_eq!(ls.len(), 3);
    assert!(ls[0].baseline < 0.0 && ls[0].overflow, "{ls:?}");
    assert!(
        !ls[1].overflow,
        "the middle line is the one the box shows: {ls:?}"
    );
    assert!(ls[2].baseline > 12.0 && ls[2].overflow, "{ls:?}");
    // and the same box cropping shows exactly the unflagged line, where it was
    let hidden = short_box(r#" valign="middle""#);
    let tl = text_layout_impl(
        &hidden,
        hidden.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert_eq!(tl.paras[0].lines.len(), 1);
    assert_eq!(tl.paras[0].lines[0].baseline, ls[1].baseline);
}

#[test]
fn overflow_visible_stamps_outlines_backdrops_and_markers() {
    // outlined: one path per line, the cropped ones stamped
    let out = editor(&short_box(r#" overflow="visible" outline="true""#));
    assert_eq!(out.matches("<path").count(), 3, "{out}");
    assert_eq!(out.matches(STAMP).count(), 2, "{out}");
    // a highlighted run's rect on a cropped line carries the stamp too
    let out = editor(&format!(
        r##"{XW}<x:textbox x="0" y="0" width="60" height="12" font-size="10" overflow="visible">alpha beta gamma <tspan x:highlight="#ff0">delta</tspan></x:textbox></svg>"##
    ));
    assert!(
        out.contains(&format!(r##"fill="#ff0" {STAMP}/>"##)),
        "{out}"
    );
    // paragraph mode: an item below the box keeps its marker, stamped
    let out = editor(&format!(
        r#"{XW}<x:textbox x="0" y="0" width="200" height="12" font-size="10" overflow="visible"><x:p>one</x:p><x:list><x:li>two</x:li></x:list></x:textbox></svg>"#
    ));
    assert!(out.contains("two"), "{out}");
    assert!(
        out.matches(STAMP).count() >= 2,
        "the item's line and its marker: {out}"
    );
    let cropped = editor(&format!(
        r#"{XW}<x:textbox x="0" y="0" width="200" height="12" font-size="10"><x:p>one</x:p><x:list><x:li>two</x:li></x:list></x:textbox></svg>"#
    ));
    assert!(
        !cropped.contains("two") && !cropped.contains("data-xsvg-overflow"),
        "{cropped}"
    );
}

#[test]
fn overflow_visible_flows_on_past_a_region_and_a_text_area() {
    // region flow (the 60×60 BoxShaper region): lines past the shape continue at
    // its bounding-box width
    let words = "one two three four five six seven eight nine ten eleven twelve thirteen";
    let region = |extra: &str| {
        format!(
            r##"{XW}<rect id="r" x="0" y="0" width="60" height="60"/><x:textbox in="#r" font-size="10"{extra}>{words}</x:textbox></svg>"##
        )
    };
    let cropped = editor(&region(""));
    let shown = editor(&region(r#" overflow="visible""#));
    assert!(
        !cropped.contains("thirteen") && shown.contains("thirteen"),
        "{shown}"
    );
    assert!(shown.contains(STAMP) && !cropped.contains(STAMP), "{shown}");
    // <textArea>: its own ink-band rule, the same switch
    let area = |extra: &str| {
        format!(
            r#"{XW}<textArea x="0" y="0" width="60" height="12" font-size="10"{extra}>alpha beta gamma delta epsilon zeta</textArea></svg>"#
        )
    };
    assert!(!editor(&area("")).contains("zeta"));
    let out = editor(&area(r#" overflow="visible""#));
    assert!(out.contains("zeta") && out.contains(STAMP), "{out}");
}

// ---------------------------------------------------------------------------
// Paint runs on text on a path (§6.13): fill and border per <tspan>

fn tp(effect: &str, body: &str) -> String {
    format!(
        r##"{XW}<path id="p" d="M0,50 L300,50"/><x:textpath in="#p" effect="{effect}" font-size="10">{body}</x:textpath></svg>"##
    )
}

const RUNS: &str =
    r##"skew<tspan fill="#f00">red</tspan> <tspan x:border-color="#00f">blue</tspan>"##;

#[test]
fn textpath_outlines_each_paint_run_at_its_pen_position() {
    let out = compile_impl(
        &tp("skew", RUNS),
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    // one piece per painted stretch (the lone space draws nothing), each traced
    // where the whole string puts it — BoxOutliner advances 5 per character —
    // in two layers: every border, then every fill
    let glyphs = &out[out.find("<g").unwrap()..];
    assert!(
        glyphs.starts_with(r##"<g fill="#000"><g fill="none"><path d="M40,50"##),
        "the border layer first, holding only the bordered run: {out}"
    );
    let borders = &glyphs[..glyphs.find("</g>").unwrap()];
    assert_eq!(borders.matches("<path").count(), 1, "{out}");
    assert!(
        borders.contains(r##"stroke="#00f" stroke-width="6""##),
        "{out}"
    );
    let fills: Vec<&str> = glyphs[glyphs.find("</g>").unwrap()..]
        .split("<path d=\"")
        .skip(1)
        .collect();
    assert_eq!(fills.len(), 3, "{out}");
    assert!(
        fills[0].starts_with("M0,50") && !fills[0].contains("fill="),
        "{out}"
    );
    assert!(
        fills[1].starts_with("M20,50") && fills[1].contains(r##"fill="#f00""##),
        "{out}"
    );
    assert!(
        fills[2].starts_with("M40,50") && !fills[2].contains("stroke"),
        "{out}"
    );
    // the stops still cover the whole string, one per character and the end
    let svg = tp("rainbow", RUNS);
    let off = svg.find("<x:textpath").unwrap();
    let stops = caret_stops_impl(&svg, off, "balanced", &Mono, &NoShaper, &BoxOutliner).unwrap();
    assert_eq!(stops.runs[0].text, "skewred blue");
    assert_eq!(stops.runs[0].stops.len(), 13);
}

#[test]
fn textpath_run_borders_inherit_attribute_by_attribute() {
    // the element's border is every run's; a run naming only a colour keeps
    // the element's width, and a fill-only run keeps the element's border whole
    let svg = tp("skew", RUNS).replace(
        r#"font-size="10">"#,
        r##"font-size="10" x:border-width="2" x:border-color="#000">"##,
    );
    let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
    // the border layer states the element's border once; every piece sits in
    // it, and only the run that differs restates its own
    let layer = r##"<g fill="none" stroke="#000" stroke-width="4" paint-order="stroke" stroke-linejoin="round">"##;
    let borders = &out[out.find(layer).unwrap_or_else(|| panic!("{out}"))..];
    let borders = &borders[..borders.find("</g>").unwrap()];
    assert_eq!(
        borders.matches("<path").count(),
        3,
        "every piece is bordered: {out}"
    );
    assert_eq!(
        borders.matches("stroke=").count(),
        2,
        "the layer and the blue run: {out}"
    );
    assert!(
        borders.contains(r##"stroke="#00f" stroke-width="4""##),
        "the run's colour, the element's width: {out}"
    );
    // nothing on the outer group strokes over the fills
    assert!(
        !out[out.find("<g").unwrap()..].starts_with(r##"<g fill="#000" stroke"##),
        "{out}"
    );
}

#[test]
fn textpath_live_effects_keep_paint_runs_as_tspans() {
    // follow: the runs ride inside SVG's own <textPath>; the element's border
    // reaches the live <text> too
    let out = compile_fast(&tp("follow", RUNS).replace(
        r#"font-size="10">"#,
        r##"font-size="10" x:border-width="2">"##,
    ));
    assert!(
        out.contains(r##"stroke="#000000" stroke-width="4""##),
        "the element's border on the <text>: {out}"
    );
    assert!(
        out.contains(
            r##">skew<tspan fill="#f00">red</tspan> <tspan stroke="#00f" stroke-width="4""##
        ),
        "a run restates only a border that differs, its width inherited: {out}"
    );
    // stair: the <text>'s per-glyph lists position characters inside the runs;
    // edge whitespace trims across them
    let out = compile_fast(&tp("stair", &format!("  {RUNS}  ")));
    assert!(
        out.contains(r##">skew<tspan fill="#f00">red</tspan> <tspan"##),
        "{out}"
    );
    assert!(out.contains(">blue</tspan></text>"), "{out}");
    let xs = out
        .split(" x=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(xs.split(' ').count(), "skewred blue".len(), "{out}");
}

#[test]
fn textpath_typography_runs_stay_one_run() {
    // weight, size, family: the element's for every run — no tspan, and the
    // output is the unstyled element's byte for byte
    let plain = compile_impl(
        &tp("skew", "skew red"),
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    let typo = compile_impl(
        &tp(
            "skew",
            r#"skew <tspan font-weight="700" font-size="30">red</tspan>"#,
        ),
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert_eq!(plain, typo);
    let follow = compile_fast(&tp(
        "follow",
        r#"skew <tspan font-weight="700">red</tspan>"#,
    ));
    assert!(follow.contains(">skew red</textPath>"), "{follow}");
}

// ---------------------------------------------------------------------------
// Paint runs in boxes: a run's border (live), fill and border (outlined)

const BOX_RUNS: &str =
    r##"plain <tspan fill="#f00">red</tspan> <tspan x:border-color="#00f">blue</tspan>"##;

#[test]
fn box_runs_carry_their_border_on_live_text() {
    for (open, close) in [
        (
            r#"<x:textbox x="0" y="0" width="300" height="40" font-size="10""#,
            "</x:textbox>",
        ),
        (
            r#"<textArea x="0" y="0" width="300" height="40" font-size="10""#,
            "</textArea>",
        ),
    ] {
        let out = compile_fast(&format!("{XW}{open}>{BOX_RUNS}{close}</svg>"));
        assert!(
            out.contains(r##"<tspan stroke="#00f" stroke-width="6" paint-order="stroke" stroke-linejoin="round">blue"##),
            "{out}"
        );
        assert!(!out.contains("border-"), "{out}");
        // inside a bordered box a run naming only its colour keeps the box's width
        let out = compile_fast(&format!(
            r##"{XW}{open} x:border-width="2" x:border-color="#000">{BOX_RUNS}{close}</svg>"##
        ));
        assert!(
            out.contains(r##"<tspan stroke="#00f" stroke-width="4""##),
            "{out}"
        );
        assert!(
            out.contains(r##"<tspan fill="#f00">red"##),
            "a fill-only run restates no border: {out}"
        );
    }
    // a paragraph's runs complete their border from the box around it
    let out = compile_fast(&format!(
        r##"{XW}<x:textbox x="0" y="0" width="300" height="40" font-size="10" x:border-width="2"><x:p>{BOX_RUNS}</x:p></x:textbox></svg>"##
    ));
    assert!(
        out.contains(r##"<tspan stroke="#00f" stroke-width="4""##),
        "{out}"
    );
}

#[test]
fn outlined_box_runs_trace_their_paint_in_two_layers() {
    let svg = format!(
        r#"{XW}<x:textbox x="0" y="0" width="300" height="40" font-size="10" outline="true">{BOX_RUNS}</x:textbox></svg>"#
    );
    let out = compile_impl(&svg, "balanced", false, &Mono, &NoShaper, &BoxOutliner).unwrap();
    // the pieces sit where the whole line puts them (5 per character): "plain "
    // at 0, "red" at 30, "blue" at 50; the lone space draws nothing
    let g = &out[out
        .find(r##"<g fill="#000">"##)
        .unwrap_or_else(|| panic!("{out}"))..];
    let (layer, fills) = g.split_once("</g>").unwrap();
    assert_eq!(
        layer.matches("<path").count(),
        1,
        "only blue is bordered: {out}"
    );
    assert!(
        layer.contains(r#"<path d="M50,"#) && layer.contains(r##"stroke="#00f""##),
        "{out}"
    );
    let fills: Vec<&str> = fills.split("<path d=\"").skip(1).collect();
    assert_eq!(fills.len(), 3, "{out}");
    assert!(fills[0].starts_with("M0,"), "{out}");
    assert!(
        fills[1].starts_with("M30,") && fills[1].contains(r##"fill="#f00""##),
        "{out}"
    );
    assert!(
        fills[2].starts_with("M50,") && !fills[2].contains("stroke"),
        "{out}"
    );
    // an unpainted outlined box keeps its one path per line
    let plain = compile_impl(
        &svg.replace(BOX_RUNS, "plain red blue"),
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert_eq!(plain.matches("<path").count(), 1, "{plain}");
    // and the pieces bend with a warp like any other path
    let warped = compile_impl(
        &svg.replace("<x:textbox", r#"<x:warp field="arch" bend="40"><x:textbox"#)
            .replace("</x:textbox>", "</x:textbox></x:warp>"),
        "balanced",
        false,
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert!(
        warped.contains(r##"fill="#f00""##) && warped.contains(r##"stroke="#00f""##),
        "{warped}"
    );
}

#[test]
fn inline_size_text_flows_its_runs() {
    let out = compile_fast(&format!(
        r##"{XW}<text x="0" y="20" inline-size="300" font-size="10">plain <tspan fill="#f00">red</tspan> <tspan x:border-color="#00f">blue</tspan> <tspan x:highlight="#ff0" font-weight="700">marked</tspan></text></svg>"##
    ));
    assert!(out.contains(r##"<tspan fill="#f00">red</tspan>"##), "{out}");
    assert!(
        out.contains(r##"<tspan stroke="#00f" stroke-width="6""##),
        "{out}"
    );
    assert!(
        out.contains(r##"<tspan font-weight="700">marked</tspan>"##),
        "typography too: {out}"
    );
    // the highlight bakes behind the text, as in a box
    let rect = out.find(r##"<rect"##).unwrap_or_else(|| panic!("{out}"));
    assert!(
        rect < out.find("<text").unwrap() && out.contains(r##"fill="#ff0""##),
        "{out}"
    );
    assert!(!out.contains("x:") && !out.contains("highlight="), "{out}");
}

#[test]
fn region_flow_paints_its_runs_live_and_outlined() {
    let svg = format!(
        r##"{XW}<rect id="r" x="0" y="0" width="60" height="60"/><x:textbox in="#r" font-size="10">aa <tspan fill="#f00">bb</tspan> <tspan x:border-color="#00f">cc</tspan></x:textbox></svg>"##
    );
    let live = compile_impl(&svg, "balanced", false, &Mono, &BoxShaper, &NoOutliner).unwrap();
    assert!(
        live.contains(r##"<tspan fill="#f00">bb</tspan>"##),
        "{live}"
    );
    assert!(live.contains(r##"<tspan stroke="#00f""##), "{live}");
    // the editor's layout seam reports the same line, its runs intact
    let tl = text_layout_impl(
        &svg,
        svg.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &BoxShaper,
        &NoOutliner,
    )
    .unwrap();
    assert_eq!(tl.paras[0].lines[0].text, "aa bb cc");
    // outlined: one piece per painted stretch, two layers
    let out = compile_impl(
        &svg.replace(r#"font-size="10">"#, r#"font-size="10" outline="true">"#),
        "balanced",
        false,
        &Mono,
        &BoxShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert!(out.contains(r#"<g fill="none">"#), "{out}");
    assert!(
        out.contains(r##"fill="#f00""##) && out.contains(r##"stroke="#00f""##),
        "{out}"
    );
}

// ---- document queries (query.rs; xsvg-kit docs/Queries.md) -----------------

/// Byte offset of the `nth` occurrence of `needle` in `src`.
fn nth_at(src: &str, needle: &str, nth: usize) -> usize {
    src.match_indices(needle).nth(nth).expect(needle).0
}

#[test]
fn resolved_walks_the_cascade_the_emitters_lay_out_from() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:theme><x:font name="body" font-family="Inter" font-size="14" font-weight="500"/><x:color name="accent" value="#6366f1"/></x:theme>
<g font-family="Lora">
<x:textbox x="0" y="0" width="200" height="200" x:font="body" font-size="18" fill="var(accent)">
<x:p font-size="30">Title</x:p>
<x:p>Body</x:p>
<x:list font-weight="700" fill="#f00"><x:li font-size="11">one</x:li></x:list>
</x:textbox>
<x:textbox x="0" y="300" width="200" height="40" font-size="0">bad size</x:textbox>
</g>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let props = [
        "font-family",
        "font-size",
        "font-weight",
        "fill",
        "line-height",
    ];
    let get = |off: usize| s.resolved(off, &props).unwrap();
    let boxo = nth_at(src, "<x:textbox", 0);
    let g = nth_at(src, "<g ", 0);

    // the box: own size, its token's weight and family over the ancestor's
    let b = get(boxo);
    assert_eq!(b[0].value.as_deref(), Some("Inter"));
    assert_eq!(b[0].source, Source::Token("body".into()));
    assert_eq!(b[1].value.as_deref(), Some("18"));
    assert_eq!(b[1].source, Source::Own);
    assert_eq!(b[2].value.as_deref(), Some("500"));
    assert_eq!(b[3].value.as_deref(), Some("var(accent)"));
    assert_eq!(b[3].computed.as_deref(), Some("#6366f1"));
    assert_eq!(b[4].value.as_deref(), Some("1.2"));
    assert_eq!(b[4].source, Source::Initial);

    // a paragraph: its own size, the rest from its box (the token named there)
    let p = get(nth_at(src, "<x:p", 0));
    assert_eq!(p[1].value.as_deref(), Some("30"));
    assert_eq!(p[1].source, Source::Own);
    assert_eq!(
        p[0].source,
        Source::Inherited {
            from: boxo,
            token: Some("body".into())
        }
    );
    let p2 = get(nth_at(src, "<x:p", 1));
    assert_eq!(p2[1].value.as_deref(), Some("18"));
    assert_eq!(
        p2[1].source,
        Source::Inherited {
            from: boxo,
            token: None
        }
    );

    // an item in a box: its size, its list's weight and colour, the box's family
    let list = nth_at(src, "<x:list", 0);
    let li = get(nth_at(src, "<x:li ", 0));
    assert_eq!(li[1].value.as_deref(), Some("11"));
    assert_eq!(li[2].value.as_deref(), Some("700"));
    assert_eq!(
        li[2].source,
        Source::Inherited {
            from: list,
            token: None
        }
    );
    assert_eq!(li[3].value.as_deref(), Some("#f00"));
    assert_eq!(li[0].value.as_deref(), Some("Inter"));

    // an invalid value does not count: the cascade falls through (CSS's rule)
    let bad = get(nth_at(src, "<x:textbox", 1));
    assert_eq!(bad[1].value.as_deref(), Some("16"));
    assert_eq!(bad[1].source, Source::Initial);
    // no token on the second box: the family comes from the <g>
    assert_eq!(bad[0].value.as_deref(), Some("Lora"));
    assert_eq!(
        bad[0].source,
        Source::Inherited {
            from: g,
            token: None
        }
    );

    // what the query says is what was laid out: the paragraph renders at 30
    let out = compile_test(src);
    assert!(out.contains("font-size=\"30\""), "{out}");
    assert!(
        s.resolved(boxo + 1, &props).is_none(),
        "no element starts there"
    );
}

#[test]
fn resolved_cells_and_standalone_lists_and_declared_defaults() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:q="https://xsvg.visioncortex.org">
<q:table x="0" y="0" width="200" font-size="12">
<q:col font-size="20"/><q:col/>
<q:tr><q:th>A</q:th><q:th font-weight="300">B</q:th></q:tr>
<q:tr font-style="italic"><q:td>a</q:td><q:td fill="#0a0">b</q:td></q:tr>
</q:table>
<q:list x="0" y="200" width="200" font-size="10"><q:li font-size="8" fill="#f00">x</q:li></q:list>
<q:textbox x="0" y="300" width="100" height="30" q:border-width="2">t</q:textbox>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let props = ["font-size", "font-weight", "font-style", "fill"];
    let get = |off: usize| s.resolved(off, &props).unwrap();
    let col0 = nth_at(src, "<q:col", 0);
    let row1 = nth_at(src, "<q:tr", 1);

    // a header cell: the column's size, bold implied
    let th = get(nth_at(src, "<q:th", 0));
    assert_eq!(th[0].value.as_deref(), Some("20"));
    assert_eq!(
        th[0].source,
        Source::Inherited {
            from: col0,
            token: None
        }
    );
    assert_eq!(th[1].value.as_deref(), Some("700"));
    assert_eq!(th[1].source, Source::Implied);
    // a header cell's own weight beats the implied bold
    assert_eq!(
        get(nth_at(src, "<q:th", 1))[1].value.as_deref(),
        Some("300")
    );
    // a body cell: the row's style, the table's declared text colour
    let td = get(nth_at(src, "<q:td", 0));
    assert_eq!(td[2].value.as_deref(), Some("italic"));
    assert_eq!(
        td[2].source,
        Source::Inherited {
            from: row1,
            token: None
        }
    );
    assert_eq!(td[3].value.as_deref(), Some("#0f172a"));
    assert_eq!(td[3].source, Source::Initial);
    assert_eq!(
        get(nth_at(src, "<q:td", 1))[3].value.as_deref(),
        Some("#0a0")
    );

    // a standalone list's item: its size, and its own colour (the type block
    // of §6.1 is the item's over the list's, standalone or boxed)
    let li = get(nth_at(src, "<q:li ", 0));
    assert_eq!(li[0].value.as_deref(), Some("8"));
    assert_eq!(li[3].value.as_deref(), Some("#f00"));
    assert_eq!(li[3].source, Source::Own);

    // any other attribute: own (x: names the namespace, whatever its prefix),
    // else the element's declared default
    let tb = nth_at(src, "<q:textbox", 0);
    let r = s
        .resolved(tb, &["x:border-width", "align", "x:nonsense"])
        .unwrap();
    assert_eq!(r[0].value.as_deref(), Some("2"));
    assert_eq!(r[0].source, Source::Own);
    assert_eq!(r[1].value.as_deref(), Some("start"));
    assert_eq!(r[1].source, Source::Initial);
    assert_eq!(r[2].value, None);
}

#[test]
fn the_initial_style_is_the_declared_font_defaults() {
    // the font block every text element declares (describe()) and the style
    // the compiler starts from must be one set of values
    let init = super::query::initial_style();
    let declared = |name: &str| {
        FONT_PARAMS
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.default)
    };
    assert_eq!(declared("font-family"), Some(init.family.as_str()));
    assert_eq!(declared("font-size").and_then(parse_num), Some(init.size));
    assert_eq!(declared("font-weight"), Some(init.weight.as_str()));
    assert_eq!(declared("font-style"), Some(init.style.as_str()));
    assert_eq!(
        declared("line-height").and_then(parse_num),
        Some(init.line_height)
    );
    assert_eq!(
        declared("text-transform").and_then(TextTransform::parse),
        Some(init.text_transform)
    );
}

#[test]
fn resolved_agrees_with_the_emitters_over_the_dataset() {
    // for every element of every sample, the style `resolved` reports parses to
    // the style the element's emitter lays its text out in
    let props = super::query::STYLE_PROPS;
    let mut checked = 0;
    for path in dataset_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let Ok(s) = ParsedDocument::new(&src) else {
            continue;
        };
        let doc = &s.parsed.doc;
        let theme = &s.parsed.theme;
        for node in super::visible(doc.root()).filter(|n| n.is_element()) {
            let Some(r) = s.resolved(node.range().start, &props) else {
                continue;
            };
            let v = |i: usize| r[i].value.clone().unwrap();
            let spacing = |v: String| {
                if v == "normal" {
                    0.0
                } else {
                    parse_num(&v).unwrap()
                }
            };
            let reported = TextStyle {
                family: v(0),
                size: parse_num(&v(1)).unwrap(),
                weight: v(2),
                style: v(3),
                line_height: parse_num(&v(4)).unwrap(),
                letter_spacing: spacing(v(5)),
                word_spacing: spacing(v(6)),
                text_transform: TextTransform::parse(&v(7)).unwrap(),
            };
            // the emitters: a block over its text element's style, an element
            // over the initial style
            let laid = match super::query::block_levels(node, Some(theme)) {
                Some((levels, base)) => super::query::overlay(
                    &levels,
                    &super::query::overlay(
                        &super::query::element_levels(base, theme),
                        &super::query::initial_style(),
                    ),
                ),
                None => super::query::overlay(
                    &super::query::element_levels(node, theme),
                    &super::query::initial_style(),
                ),
            };
            assert_eq!(reported, laid, "{}: {:?}", path.display(), node.range());
            checked += 1;
        }
    }
    assert!(checked > 1000, "{checked}");
}

#[test]
fn text_style_reports_pieces_in_utf16_over_the_text_content() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="200" height="100" font-size="10" fill="#111" text-transform="uppercase">ß😀 <tspan font-weight="700" text-decoration="underline">bold</tspan><tbreak/><tspan x:highlight="#ff0" href="https://e.x">hi</tspan> end</x:textbox>
<x:textpath in="#p" font-size="20">a<tspan font-weight="700" fill="#f00" x:border-width="1">b</tspan></x:textpath>
<path id="p" d="M0,0 L100,0"/>
<text x="0" y="0">plain<tspan fill="#f00">x</tspan></text>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let pieces = s.text_style(nth_at(src, "<x:textbox", 0)).unwrap();
    // "ß😀 " is 4 UTF-16 units (the emoji is two), uppercasing keeps the count
    let spans: Vec<_> = pieces.iter().map(|p| (p.start, p.end)).collect();
    assert_eq!(
        spans,
        vec![(0, 4), (4, 8), (8, 9), (9, 11), (11, 15)],
        "{pieces:?}"
    );
    assert_eq!(pieces[0].weight, "normal");
    assert_eq!(pieces[0].fill, "#111");
    assert_eq!(pieces[1].weight, "700");
    assert_eq!(pieces[1].decoration.as_deref(), Some("underline"));
    assert_eq!(pieces[3].highlight.as_deref(), Some("#ff0"));
    assert_eq!(pieces[3].href.as_deref(), Some("https://e.x"));
    assert_eq!(pieces[4].size, 10.0);
    // text on a path draws paint runs only: the run's weight is not reported
    let tp = s.text_style(nth_at(src, "<x:textpath", 0)).unwrap();
    assert_eq!(tp.len(), 2, "{tp:?}");
    assert_eq!(tp[1].weight, "normal");
    assert_eq!(tp[1].fill, "#f00");
    assert_eq!(
        tp[1].border.as_ref().and_then(|b| b.width.as_deref()),
        Some("1")
    );
    // a plain <text> passes through, and the browser composes its runs: each
    // piece is reported in its own style, as it draws
    let plain = s.text_style(nth_at(src, "<text", 0)).unwrap();
    assert_eq!(plain.len(), 2, "{plain:?}");
    assert_eq!(plain[0].fill, "#000");
    assert_eq!(plain[1].fill, "#f00");
    assert!(s.text_style(nth_at(src, "<path", 0)).is_none());
}

#[test]
fn blocks_and_run_support_follow_the_emitters() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="200" height="200"><x:p>a</x:p><x:list list="number"><x:li>b</x:li><x:li indent="1" list="bullet">c</x:li></x:list><x:p>d</x:p></x:textbox>
<x:list x="0" y="300" width="100"><x:li>e</x:li></x:list>
<x:textbox x="0" y="400" width="100" height="20">flow</x:textbox>
<text x="0" y="0" inline-size="50">w</text><text x="0" y="0">t</text>
<x:textpath in="#p">p</x:textpath><path id="p" d="M0,0 L9,0"/>
<x:table x="0" y="0" width="90"><x:tr><x:td>c</x:td></x:tr></x:table>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let list = nth_at(src, "<x:list", 0);
    let b = s.blocks(nth_at(src, "<x:textbox", 0)).unwrap();
    let kinds: Vec<_> = b.iter().map(|b| b.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            BlockKind::Paragraph,
            BlockKind::Item {
                level: 0,
                list,
                marker: "number".into()
            },
            BlockKind::Item {
                level: 1,
                list,
                marker: "bullet".into()
            },
            BlockKind::Paragraph,
        ]
    );
    assert_eq!(b[0].start, nth_at(src, "<x:p", 0));
    assert_eq!(&src[b[3].start..b[3].end], "<x:p>d</x:p>");
    assert_eq!(s.blocks(nth_at(src, "<x:list", 1)).unwrap().len(), 1);
    assert_eq!(s.blocks(nth_at(src, "<x:textbox", 1)).unwrap(), vec![]);
    assert!(s.blocks(nth_at(src, "<text", 0)).is_none());

    let all = RunSupport {
        typography: true,
        fill: true,
        border: true,
        highlight: true,
        decoration: true,
        shift: true,
        href: true,
    };
    let sup = |needle: &str, nth: usize| s.run_support(nth_at(src, needle, nth));
    for (needle, nth) in [
        ("<x:textbox", 0),
        ("<x:p", 0),
        ("<x:li>", 0),
        ("<x:list", 1),
        ("<text", 0),
        ("<x:td", 0),
    ] {
        assert_eq!(sup(needle, nth), Some(all), "{needle}");
    }
    // plain <text>: everything SVG draws natively plus the lowered border; no highlight
    let plain = sup("<text", 1).unwrap();
    assert_eq!(
        plain,
        RunSupport {
            highlight: false,
            ..all
        }
    );
    let tp = sup("<x:textpath", 0).unwrap();
    assert!(tp.fill && tp.border && !tp.typography && !tp.highlight && !tp.decoration);
    assert_eq!(sup("<path", 0), None);
}

#[test]
fn theme_tokens_come_in_document_order_without_the_doc_island() {
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:theme><x:color name="a" value="#111"/><x:color name="b" value="#222"/><x:font name="h" font-size="30" font-family="Inter" data-x="ignored"/></x:theme>
<x:doc><x:theme><x:color name="island" value="#999"/></x:theme></x:doc>
<x:theme><x:color name="a" value="#333"/></x:theme>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let t = s.theme();
    assert_eq!(
        t.colors,
        vec![("b".into(), "#222".into()), ("a".into(), "#333".into())]
    );
    // a font token carries the token properties, in their order, nothing else
    assert_eq!(
        t.fonts,
        vec![(
            "h".into(),
            vec![
                ("font-family".into(), "Inter".into()),
                ("font-size".into(), "30".into())
            ]
        )]
    );
}

#[test]
fn a_session_from_a_held_parse_compiles_the_same_bytes_without_parsing() {
    // the host keeps one ParsedDocument per change and compiles from it: the
    // bytes are a fresh session's, and no parse happens
    for path in dataset_files().into_iter().step_by(5) {
        let src = std::fs::read_to_string(&path).unwrap();
        let platform = Platform::new(&Mono, &NoShaper, &NoOutliner);
        let Ok(fresh) = platform.session(&src, "balanced", true, "") else {
            continue;
        };
        let doc = ParsedDocument::new(&src).unwrap();
        super::session::PARSE_COUNT.with(|c| c.set(0));
        let held = CompileSession::from_parsed(&platform, &doc, "balanced", true, "");
        assert_eq!(held.compile(), fresh.compile(), "{}", path.display());
        for (start, _) in doc
            .parsed
            .doc
            .root_element()
            .children()
            .filter(|c| c.is_element())
            .map(|c| (c.range().start, ()))
        {
            assert_eq!(held.compile_fragment(start), fresh.compile_fragment(start));
        }
        assert_eq!(
            super::session::PARSE_COUNT.with(|c| c.get()),
            0,
            "{}",
            path.display()
        );
        // the session's own document answers the queries the held one does
        assert_eq!(held.document().theme(), doc.theme());
    }
}

#[test]
fn resolved_cascades_a_blocks_layout_from_its_box() {
    // align and direction: a paragraph's own, else the box's; text-indent the
    // paragraph's own, else none; an item always start-aligned, unindented
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:textbox x="0" y="0" width="200" height="200" align="center" direction="rtl" text-indent="9"><x:p>a</x:p><x:p align="end" text-indent="4" direction="ltr">b</x:p><x:list><x:li align="end">c</x:li></x:list></x:textbox>
<x:textbox x="0" y="300" width="200" height="40"><x:p>d</x:p></x:textbox>
</svg>"##;
    let s = ParsedDocument::new(src).unwrap();
    let props = ["align", "text-indent", "direction"];
    let boxo = nth_at(src, "<x:textbox", 0);
    let get = |needle: &str, nth: usize| s.resolved(nth_at(src, needle, nth), &props).unwrap();
    let p = get("<x:p", 0);
    assert_eq!(p[0].value.as_deref(), Some("center"));
    assert_eq!(
        p[0].source,
        Source::Inherited {
            from: boxo,
            token: None
        }
    );
    assert_eq!(
        p[1].value.as_deref(),
        Some("0"),
        "a box's indent is for its own flow"
    );
    assert_eq!(p[1].source, Source::Initial);
    assert_eq!(p[2].value.as_deref(), Some("rtl"));
    assert_eq!(
        p[2].source,
        Source::Inherited {
            from: boxo,
            token: None
        }
    );
    let own = get("<x:p", 1);
    assert_eq!(own[0].value.as_deref(), Some("end"));
    assert_eq!(own[1].value.as_deref(), Some("4"));
    assert_eq!(own[2].value.as_deref(), Some("ltr"));
    assert!(own.iter().all(|r| r.source == Source::Own));
    let li = get("<x:li ", 0);
    assert_eq!(li[0].value.as_deref(), Some("start"));
    assert_eq!(li[0].source, Source::Implied);
    assert_eq!(li[1].value.as_deref(), Some("0"));
    assert_eq!(li[1].source, Source::Implied);
    assert_eq!(li[2].value.as_deref(), Some("rtl"));
    let bare = get("<x:p", 2);
    assert_eq!(bare[0].value.as_deref(), Some("start"));
    assert_eq!(bare[0].source, Source::Initial);
    assert_eq!(bare[2].value.as_deref(), Some("ltr"));
    // the box itself: its own, else its declared default
    let b = s.resolved(nth_at(src, "<x:textbox", 1), &props).unwrap();
    assert_eq!(b[0].value.as_deref(), Some("start"));
    assert_eq!(b[0].source, Source::Initial);
    // what the query says is what was laid out: paragraph "a" centres, "b" ends
    let out = compile_test(src);
    assert!(out.contains("text-anchor=\"middle\""), "{out}");
    assert!(out.contains("text-anchor=\"end\""), "{out}");
}

// ---- nested viewports (viewport.rs; §5.3) ------------------------------------

#[test]
fn a_nested_svg_lowers_to_one_clipped_group_with_the_fit_folded_in() {
    // a crop as the kit writes it: a unit picture under a fractional viewBox,
    // stretched (`none`) into a rotated frame — one <g>, the element's own
    // transform first, then the frame's position, then the fit; the clip and
    // the marker are the viewBox itself
    let svg = format!(
        r##"{XW}<svg x="20" y="20" width="120" height="80" viewBox="0.25 0.1 0.5 0.5" preserveAspectRatio="none" transform="rotate(10 80 60)" opacity="0.5"><image x="0" y="0" width="1" height="1" preserveAspectRatio="none" href="p.png"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<svg").count(), 1, "{out}");
    assert!(
        out.contains(r##"<clipPath id="x-vp-"##)
            && out.contains(r##"><rect x="0.25" y="0.1" width="0.5" height="0.5"/></clipPath>"##),
        "{out}"
    );
    assert!(
        out.contains(r##"<g transform="rotate(10 80 60) translate(20 20) scale(240 160) translate(-0.25 -0.1)" clip-path="url(#x-vp-"##),
        "{out}"
    );
    assert!(
        out.contains(r##"data-xsvg-viewport="0.25 0.1 0.5 0.5" opacity="0.5">"##),
        "{out}"
    );
    assert!(
        !out.contains("viewBox=\"0.25") && !out.contains("preserveAspectRatio=\"none\" transform"),
        "{out}"
    );
    assert!(out.ends_with("</g></svg>"), "{out}");

    // meet (the default) letterboxes: one scale, the content centred, the clip
    // the frame mapped back — 200 units square around 200×100 content
    let svg = format!(
        r##"{XW}<svg x="200" y="20" width="100" height="100" viewBox="0 0 200 100"><rect width="200" height="100"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<g transform="translate(200 45) scale(0.5)" clip-path"##),
        "{out}"
    );
    assert!(
        out.contains(r##"data-xsvg-viewport="0 -50 200 200""##),
        "{out}"
    );
    assert!(
        out.contains(r##"<rect x="0" y="-50" width="200" height="200"/></clipPath>"##),
        "{out}"
    );

    // slice covers, aligned to the far corner; overflow="visible" drops the clip
    // but keeps the marker — the frame is still the editor's box
    let svg = format!(
        r##"{XW}<svg x="200" y="150" width="100" height="100" viewBox="0 0 200 100" preserveAspectRatio="xMaxYMax slice" overflow="visible"><rect width="200" height="100"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<g transform="translate(100 150)" data-xsvg-viewport="100 0 100 100">"##),
        "{out}"
    );
    assert!(!out.contains("clipPath"), "{out}");

    // no viewBox: no scaling, the frame is the clip at the origin
    let svg = format!(
        r##"{XW}<svg x="5" y="6" width="30" height="40"><rect width="9" height="9"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<g transform="translate(5 6)" clip-path="url(#x-vp-"##),
        "{out}"
    );
    assert!(out.contains(r##"data-xsvg-viewport="0 0 30 40""##), "{out}");

    // the def precedes the element: a fragment is self-contained and keyed ids stay stable
    let keyed = unit_platform()
        .with_keys(index_keys(&svg))
        .session(&svg, "balanced", true, "")
        .unwrap()
        .compile()
        .unwrap();
    assert!(keyed.contains("id=\"x-vp-k"), "{keyed}");
}

/// The clip a compile writes for its `n`th viewport: the `<path>`'s `d` and
/// `transform` (empty when it has none), and the marker rect of its group.
fn viewport_clip(out: &str, n: usize) -> (String, String, [f64; 4]) {
    let doc = roxmltree::Document::parse(out).unwrap();
    let clip = doc
        .descendants()
        .filter(|e| e.tag_name().name() == "clipPath")
        .nth(n)
        .unwrap();
    let path = clip.first_element_child().unwrap();
    assert_eq!(path.tag_name().name(), "path", "{out}");
    let id = clip.attribute("id").unwrap();
    let group = doc
        .descendants()
        .find(|e| e.attribute("clip-path") == Some(&format!("url(#{id})")))
        .unwrap();
    let marker: Vec<f64> = group
        .attribute("data-xsvg-viewport")
        .unwrap()
        .split(' ')
        .map(|v| v.parse().unwrap())
        .collect();
    (
        path.attribute("d").unwrap().to_string(),
        path.attribute("transform").unwrap_or("").to_string(),
        [marker[0], marker[1], marker[2], marker[3]],
    )
}

#[test]
fn a_viewport_with_corners_clips_to_its_rounded_frame() {
    // the crop of the test above, its corners rounded: the same one <g>, the
    // same marker, and the clip the frame's rounded outline — written in the
    // frame's units, with the fit undone on the path
    let svg = format!(
        r##"{XW}<svg x="20" y="20" width="120" height="80" viewBox="0.25 0.1 0.5 0.5" preserveAspectRatio="none" transform="rotate(10 80 60)" x:corners="16"><image x="0" y="0" width="1" height="1" preserveAspectRatio="none" href="p.png"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"><path transform="translate(0.25 0.1) scale(0.00416666666667 0.00625) translate(-20 -20)" d="M36 20 L124 20 A16 16 0 0 1 140 36 L140 84 A16 16 0 0 1 124 100 L36 100 A16 16 0 0 1 20 84 L20 36 A16 16 0 0 1 36 20 Z"/></clipPath>"##),
        "{out}"
    );
    assert!(
        out.contains(r##"<g transform="rotate(10 80 60) translate(20 20) scale(240 160) translate(-0.25 -0.1)" clip-path="url(#x-vp-"##)
            && out.contains(r##"data-xsvg-viewport="0.25 0.1 0.5 0.5">"##),
        "{out}"
    );
    assert!(!out.contains("corners") && !out.contains("<rect"), "{out}");

    // Under the group's transform the clip is the frame again, exactly: the
    // path's transform undoes the fit whatever the fit is — stretched, fitted
    // inside with slack, covering, or none
    for (attrs, frame) in [
        (
            r#"x="20" y="20" width="120" height="80" viewBox="0.25 0.1 0.5 0.5" preserveAspectRatio="none""#,
            [20.0, 20.0, 120.0, 80.0],
        ),
        (
            r#"x="7" y="9" width="120" height="80" viewBox="10 20 100 100""#,
            [7.0, 9.0, 120.0, 80.0],
        ),
        (
            r#"x="200" y="150" width="100" height="60" viewBox="0 0 200 100" preserveAspectRatio="xMaxYMin slice""#,
            [200.0, 150.0, 100.0, 60.0],
        ),
        (
            r#"x="5" y="6" width="30" height="40""#,
            [5.0, 6.0, 30.0, 40.0],
        ),
        (r#"width="30" height="40""#, [0.0, 0.0, 30.0, 40.0]),
    ] {
        let svg =
            format!(r##"{XW}<svg {attrs} x:corners="8"><rect width="9" height="9"/></svg></svg>"##);
        let out = compile_test(&svg);
        let (d, undo, marker) = viewport_clip(&out, 0);
        let doc = roxmltree::Document::parse(&out).unwrap();
        let fit = doc
            .descendants()
            .find(|e| e.attribute("data-xsvg-viewport").is_some())
            .and_then(|g| g.attribute("transform"))
            .map(parse_transform)
            .unwrap_or_default();
        let undo = parse_transform(&undo);
        let path = crate::kurbo::BezPath::from_svg(&d).unwrap();
        use crate::kurbo::Shape;
        // in content space the clip's box is the marker's
        let content = (undo * path.clone()).bounding_box();
        for (got, want) in [
            (content.x0, marker[0]),
            (content.y0, marker[1]),
            (content.width(), marker[2]),
            (content.height(), marker[3]),
        ] {
            // (the marker is written to three decimals)
            assert!((got - want).abs() < 1e-3, "{attrs}: {content:?} {marker:?}");
        }
        // and drawn, it is the frame with a radius of 8 at each corner: the
        // frame's area less four corner squares, plus a circle
        let drawn = fit * undo * path;
        let b = drawn.bounding_box();
        for (got, want) in [
            (b.x0, frame[0]),
            (b.y0, frame[1]),
            (b.width(), frame[2]),
            (b.height(), frame[3]),
        ] {
            assert!((got - want).abs() < 1e-6, "{attrs}: {b:?}");
        }
        // (to the arcs' flattening into cubics, which is all the slack)
        let want = frame[2] * frame[3] - (4.0 - std::f64::consts::PI) * 64.0;
        assert!(
            (drawn.area().abs() - want).abs() < 0.1,
            "{attrs}: area {} want {want}",
            drawn.area()
        );
        if attrs == r#"width="30" height="40""# {
            assert!(!out.contains("<path transform"), "nothing to undo: {out}");
        }
    }
}

#[test]
fn a_viewport_takes_corners_as_a_rect_does() {
    let clip = |attrs: &str| {
        let svg = format!(
            r##"{XW}<x:theme><x:color name="r" value="8"/></x:theme><svg x="10" y="10" width="40" height="20" {attrs}><rect width="9" height="9"/></svg></svg>"##
        );
        compile_test(&svg)
    };
    // corners by anchor, 0–3 clockwise from the top-left, and a style at one
    let out = clip(r#"x:corners="0:10 2:5" x:corner-style="round 2:chamfer""#);
    assert_eq!(
        viewport_clip(&out, 0).0,
        "M20 10 L50 10 L50 25 L45 30 L10 30 L10 20 A10 10 0 0 1 20 10 Z",
        "{out}"
    );
    // a token
    assert_eq!(
        viewport_clip(&clip(r#"x:corners="var(r)""#), 0).0,
        viewport_clip(&clip(r#"x:corners="8""#), 0).0
    );
    // more than the frame has: a stadium, the short sides consumed
    assert_eq!(
        viewport_clip(&clip(r#"x:corners="999""#), 0).0,
        "M20 10 L40 10 A10 10 0 0 1 50 20 A10 10 0 0 1 40 30 L20 30 A10 10 0 0 1 10 20 A10 10 0 0 1 20 10 Z"
    );
    // nothing to draw: the plain rect, as without the attribute
    let plain = clip("");
    assert!(plain.contains(r#"<rect x="0" y="0" width="40" height="20"/></clipPath>"#));
    for none in [
        r#"x:corners="0""#,
        r#"x:corners="nope""#,
        r#"x:corners="""#,
        r#"x:corner-style="chamfer""#,
    ] {
        assert_eq!(clip(none), plain, "{none}");
    }
    // a viewport that does not clip has no corners to round, and the
    // attribute goes nowhere
    let open = clip(r#"overflow="visible" x:corners="8""#);
    assert!(
        !open.contains("clipPath") && !open.contains("corners"),
        "{open}"
    );

    // a kept viewport — here one a <use> sizes — stays the square it is
    let svg = format!(
        r##"{XW}<svg id="v" width="40" height="20" x:corners="8"><rect width="9" height="9"/></svg><use href="#v" width="80"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("nested <svg> kept") && !out.contains("corners"),
        "{out}"
    );

    // the corners query answers for a viewport as for a rect, in the space
    // its x y width height are written in
    let src = format!(
        r##"{XW}<svg x="10" y="10" width="40" height="20" viewBox="0 0 4 2" x:corners="0:6 2:99"><rect width="9" height="9"/></svg></svg>"##
    );
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<svg", 1)).unwrap();
    assert_eq!(c.len(), 2, "{c:?}");
    assert_eq!(
        (c[0].anchor, c[0].at, c[0].effective),
        (0, (10.0, 10.0), 6.0)
    );
    assert!(
        c[1].anchor == 2
            && c[1].at == (50.0, 30.0)
            && c[1].clamped
            && (c[1].effective - 20.0).abs() < 1e-9,
        "{c:?}"
    );
    // the root is no viewport to round
    assert!(s.corners(nth_at(&src, "<svg", 0)).is_none());
}

#[test]
fn a_viewports_sizes_resolve_against_the_enclosing_viewport() {
    // percentages and a missing size take the nearest viewport's extent: the
    // root's viewBox, or an enclosing lowered viewport's own viewBox
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300"><svg x="10%" width="50%" height="20"><rect width="1" height="1"/></svg><svg x="0" y="0" width="100" height="100" viewBox="0 0 10 10"><svg x="1" y="1" height="50%"><rect width="1" height="1"/></svg></svg></svg>"##;
    let out = compile_test(svg);
    assert!(
        out.contains(r##"<g transform="translate(40)" clip-path"##)
            || out.contains(r##"<g transform="translate(40 0)" clip-path"##),
        "{out}"
    );
    assert!(
        out.contains(r##"data-xsvg-viewport="0 0 200 20""##),
        "{out}"
    );
    // inside the 10-unit viewBox: width 100% = 10, height 50% = 5
    assert!(out.contains(r##"data-xsvg-viewport="0 0 10 5""##), "{out}");
    assert_eq!(out.matches("<svg").count(), 1, "{out}");

    // a root sized by its window gives a percentage nothing to take of: kept,
    // its transform hoisted so no nested <svg> in the output carries one
    let svg = format!(
        r##"{XW}<svg width="50%" height="20" transform="rotate(3)"><rect width="1" height="1"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<!-- xsvg: nested <svg> kept: its frame is not resolvable here -->"),
        "{out}"
    );
    assert!(
        out.contains(r##"<g transform="rotate(3)"><svg width="50%" height="20">"##),
        "{out}"
    );
}

#[test]
fn a_frame_effect_wraps_the_viewport_in_frame_space() {
    // a drop shadow on a crop is so many frame units, not fractions of the unit
    // picture: the filter goes on a wrapper at the frame's origin with the exact
    // frame region, the fit stays on the inner clipped <g>, paint goes with it
    let svg = format!(
        r##"{XW}<svg x="20" y="150" width="200" height="20" viewBox="0 0 1 1" preserveAspectRatio="none" filter="drop-shadow(2 3 4 #0008)" fill="#f00"><rect width="1" height="1"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(out.contains(r##"<filter id="x-flt-"##), "{out}");
    // region: the frame (0 0 200 20) grown by 0.5 + 12 + 3
    assert!(
        out.contains(r##"x="-15.5" y="-15.5" width="231" height="51""##),
        "{out}"
    );
    assert!(
        out.contains(r##"<g transform="translate(20 150)" filter="url(#x-flt-"##),
        "{out}"
    );
    assert!(
        out.contains(r##"><g transform="scale(200 20)" clip-path="url(#x-vp-"##),
        "{out}"
    );
    assert!(
        out.contains(r##"data-xsvg-viewport="0 0 1 1" fill="#f00">"##),
        "{out}"
    );
    assert!(out.ends_with("</g></g></svg>"), "{out}");
    // an authored mask rides the wrapper too, resolved like any paint
    let svg = format!(
        r##"{XW}<svg x="0" y="0" width="10" height="10" mask="url(#m)" transform="translate(1 2)"><rect width="1" height="1"/></svg></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"<g transform="translate(1 2)" mask="url(#m)"><g clip-path"##),
        "{out}"
    );
}

#[test]
fn a_viewport_a_use_sizes_is_kept_without_its_transform() {
    // width/height on a <use> resize an <svg> target and are ignored on a <g>:
    // that target stays a nested <svg>, marked, its transform hoisted onto a
    // wrapper the instance does not see
    let svg = format!(
        r##"{XW}<svg id="icon" x="0" y="0" width="24" height="24" viewBox="0 0 100 100" transform="rotate(5)"><circle cx="50" cy="50" r="50"/></svg><use href="#icon" x="300" y="250" width="48"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains("<!-- xsvg: nested <svg> kept: a <use> sizes it -->"),
        "{out}"
    );
    assert!(
        out.contains(r##"<g transform="rotate(5)"><svg id="icon" x="0" y="0" width="24" height="24" viewBox="0 0 100 100"><circle"##),
        "{out}"
    );
    // the <use> deep-clones its target, transform included, so the instance
    // would lose the hoisted rotation: the <use> takes it on, conjugated by its
    // own placement — rotate(5) about the instance's origin (300, 250)
    assert!(
        out.contains(r##"<use href="#icon" x="300" y="250" width="48" transform="matrix(0.996 0.087 -0.087 0.996 22.931 -25.195)"/>"##),
        "{out}"
    );
    // a <use> with its own transform leads the compensation
    let svg = format!(
        r##"{XW}<svg id="icon" width="24" height="24" transform="translate(3 4)"><circle r="5"/></svg><use href="#icon" x="10" y="0" width="48" transform="scale(2)"/></svg>"##
    );
    let out = compile_test(&svg);
    assert!(
        out.contains(r##"transform="matrix(2 0 0 2 6 8)""##),
        "{out}"
    );
    // a <use> without a size does not keep it: the id stays on the <g>
    let svg = format!(
        r##"{XW}<svg id="icon" x="0" y="0" width="24" height="24" viewBox="0 0 100 100"><circle cx="50" cy="50" r="50"/></svg><use href="#icon" x="1"/></svg>"##
    );
    let out = compile_test(&svg);
    assert_eq!(out.matches("<svg").count(), 1, "{out}");
    assert!(
        out.contains(r##"data-xsvg-viewport="0 0 100 100" id="icon">"##),
        "{out}"
    );
}

#[test]
fn a_linked_drawings_use_keeps_its_transform_opacity_and_filter() {
    // the baked whole-file form goes through the same lowering: the <use>'s own
    // transform leads the folded one, its opacity rides the <g>, its filter
    // wraps in frame space, and its paint reaches the content
    let logo = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10"/></svg>"##;
    let main = format!(
        r##"{XW}<use href="logo.svg" x="20" y="30" width="40" height="40" transform="rotate(10)" opacity="0.5" filter="blur(1)" fill="#0f0"/></svg>"##
    );
    let out = compile_linked(&main, &[("logo.svg", logo)]);
    assert!(
        out.contains(r##"<g transform="rotate(10) translate(20 30)" filter="url(#x-flt-"##),
        "{out}"
    );
    assert!(
        out.contains(r##"><g transform="scale(4)" clip-path="url(#x-vp-"##),
        "{out}"
    );
    assert!(
        out.contains(r##"data-xsvg-viewport="0 0 10 10" opacity="0.5" fill="#0f0">"##),
        "{out}"
    );
    assert!(!out.contains("href="), "{out}");
    assert_eq!(out.matches("<svg").count(), 1, "{out}");
}

#[test]
fn no_compiled_sample_holds_a_nested_svg() {
    // the output contract: a viewport is a <g>; the root is the one <svg>
    for path in dataset_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let platform = Platform::new(&Mono, &NoShaper, &NoOutliner);
        let Ok(s) = platform.session(&src, "balanced", false, "") else {
            continue;
        };
        let Ok(out) = s.compile() else { continue };
        assert_eq!(out.matches("<svg").count(), 1, "{}", path.display());
    }
}

// ---- convert to path (trace.rs; Incremental.md §11) ---------------------------

fn to_path_at(src: &str, needle: &str) -> Result<String, ToPathError> {
    to_path_impl(
        src,
        src.find(needle).expect(needle),
        "balanced",
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
}

fn to_spline_at(src: &str, needle: &str) -> Result<String, ToPathError> {
    to_spline_impl(
        src,
        src.find(needle).expect(needle),
        "balanced",
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
}

#[test]
fn to_path_returns_the_outlined_box_with_its_attributes_kept_as_authored() {
    // the compiler's own outlined output for the node, in the source's terms:
    // the theme token unresolved, the filter list as written, the editor's
    // label and the transform on the group, no key, no font, no frame
    let src = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:theme><x:color name="accent" value="#6366f1"/><x:font name="h" font-size="30"/></x:theme>
<x:textbox id="t1" x:label="Hello" x="10" y="10" width="200" height="60" x:font="h" fill="var(accent)" opacity="0.8" filter="drop-shadow(2 3 4 #0008)" transform="rotate(5)" x:layer="foreground" class="title">Hello</x:textbox>
</svg>"##;
    let out = to_path_at(src, "<x:textbox").unwrap();
    assert!(
        out.starts_with(r##"<g fill="var(accent)" transform="rotate(5)""##),
        "{out}"
    );
    for kept in [
        r#"id="t1""#,
        r#"x:label="Hello""#,
        r#"opacity="0.8""#,
        r#"filter="drop-shadow(2 3 4 #0008)""#,
        r#"x:layer="foreground""#,
        r#"class="title""#,
    ] {
        assert!(out.contains(kept), "{kept} kept: {out}");
    }
    assert!(out.contains("<path d=\"M"), "{out}");
    assert!(out.ends_with("</g>"), "{out}");
    for gone in [
        "<text",
        "data-xsvg",
        "x:font",
        "font-size",
        "width=",
        "<filter",
        "#6366f1",
    ] {
        assert!(!out.contains(gone), "{gone} spent: {out}");
    }
    // the same node rendered keeps resolving and lowering as ever
    let rendered = compile_outlined(src);
    assert!(
        rendered.contains("#6366f1") && rendered.contains("<filter"),
        "{rendered}"
    );
}

#[test]
fn to_path_keeps_backdrops_inside_one_group_and_outlines_paragraphs() {
    // a highlighted run and an underlined run: the rects the box bakes ride
    // inside the one group that replaces the node (and, rendered with
    // outline="true", are no longer dropped)
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="60" font-size="10" transform="translate(1 2)">ab <tspan x:highlight="#ff0">cd</tspan> <tspan text-decoration="underline">ef</tspan></x:textbox></svg>"##
    );
    let out = to_path_at(&src, "<x:textbox").unwrap();
    assert!(
        out.starts_with(r##"<g transform="translate(1 2)">"##),
        "{out}"
    );
    assert_eq!(out.matches("<rect ").count(), 2, "{out}");
    assert!(
        out.contains(r##"fill="#ff0""##) && out.contains("<g fill=\"#000\"><path d="),
        "{out}"
    );
    assert!(out.ends_with("</g></g>"), "{out}");
    let rendered = compile_outlined(&src.replace("transform=", "outline=\"true\" transform="));
    assert_eq!(rendered.matches("<rect ").count(), 2, "{rendered}");

    // paragraph mode: a group per paragraph in its own style, the list's
    // marker traced, everything under the box's group
    let src = format!(
        r##"{XW}<x:textbox id="b" x="0" y="0" width="200" height="200" font-size="10"><x:p>one</x:p><x:p font-size="20" fill="#f00">two</x:p><x:list list="number"><x:li>three</x:li></x:list></x:textbox></svg>"##
    );
    let out = to_path_at(&src, "<x:textbox").unwrap();
    assert!(out.starts_with(r##"<g id="b">"##), "{out}");
    assert_eq!(
        out.matches("<g fill=").count(),
        4,
        "three blocks and a marker: {out}"
    );
    assert!(out.contains(r##"<g fill="#f00"><path"##), "{out}");
    assert!(!out.contains("<text"), "{out}");
    // rendered with outline="true" the box no longer stays live
    let rendered = compile_outlined(&src.replace(r#"id="b""#, r#"id="b" outline="true""#));
    assert!(
        !rendered.contains("<text") && rendered.contains("<path d=\"M"),
        "{rendered}"
    );
}

#[test]
fn to_path_traces_point_text_chunk_by_chunk() {
    // a plain <text>: each pen position a path, the anchor honoured, a tspan's
    // own fill on its piece, the <text>'s fill on the group; and the same by
    // x:outline="true" when rendered
    let src = format!(
        r##"{XW}<text id="p" x="100" y="20" font-size="10" text-anchor="middle" fill="#123" transform="scale(2)">ab<tspan fill="#f00" dy="5">cd</tspan><tspan x="10" y="40" text-anchor="start">ef</tspan></text></svg>"##
    );
    let out = to_path_at(&src, "<text").unwrap();
    assert!(
        out.starts_with(r##"<g fill="#123" transform="scale(2)" id="p">"##),
        "{out}"
    );
    assert_eq!(out.matches("<path d=").count(), 3, "{out}");
    assert!(out.contains(r##" fill="#f00""##), "{out}");
    // BoxOutliner boxes each glyph: "ab" (10 wide at size 10, 5 per char)
    // centred on 100 starts at 90; "cd" follows at 100, 5 lower; "ef" at 10,40
    assert!(out.contains("M90,"), "first chunk anchored middle: {out}");
    assert!(out.contains("M100,"), "second chunk at the pen: {out}");
    assert!(out.contains("M10,"), "third chunk at its own x: {out}");
    assert!(!out.contains("<text"), "{out}");
    let rendered = compile_outlined(&src.replace(r#"id="p""#, r#"id="p" x:outline="true""#));
    assert!(
        !rendered.contains("<text") && rendered.contains("<path"),
        "{rendered}"
    );
    // without the ask, point text passes through untouched
    assert!(compile_outlined(&src).contains("<text"), "{src}");
}

#[test]
fn to_path_refuses_what_it_cannot_trace_and_names_the_font() {
    // no font: the family, weight and style the outliner declined
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-family="Inter" font-weight="700" font-style="italic">no font</x:textbox></svg>"##
    );
    let err = to_path_impl(
        &src,
        src.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap_err();
    assert_eq!(err.family.as_deref(), Some("Inter"));
    assert_eq!(err.weight.as_deref(), Some("700"));
    assert_eq!(err.style.as_deref(), Some("italic"));
    assert!(err.reason.contains("no font file"), "{err:?}");
    // the named limits: per-glyph lists, rotate, a textPath, an effect with no
    // outline form, an element that is not text or a shape, an empty text
    for (body, needle, what) in [
        (
            r##"<text x="0 5 10" y="0">abc</text>"##,
            "<text",
            "per-glyph",
        ),
        (r##"<text rotate="10">abc</text>"##, "<text", "`rotate`"),
        (
            r##"<text><textPath href="#q">abc</textPath></text>"##,
            "<text",
            "<textPath>",
        ),
        (
            r##"<path id="q" d="M0,0 L50,0"/><x:textpath in="#q" effect="follow">abc</x:textpath>"##,
            "<x:textpath",
            "`follow`",
        ),
        (
            r##"<image href="a.png" width="1" height="1"/>"##,
            "<image",
            "not convertible",
        ),
        (r##"<text x="0" y="0">  </text>"##, "<text", "no characters"),
    ] {
        let src = format!("{XW}{body}</svg>");
        let err = to_path_at(&src, needle).unwrap_err();
        assert!(err.reason.contains(what), "{what}: {err:?}");
    }
    assert!(to_path_at(
        &format!("{XW}<rect width=\"1\" height=\"1\"/></svg>"),
        "</svg>"
    )
    .is_err());
}

#[test]
fn to_path_converts_shapes_keeping_their_attributes() {
    let src = format!(
        r##"{XW}<rect id="r" x:label="Box" x="1" y="2" width="10" height="5" rx="1" fill="var(accent)" filter="blur(2)" transform="rotate(3)"/><circle cx="5" cy="5" r="2" stroke="#000"/><polygon points="0,0 4,0 2,3" class="tri"/></svg>"##
    );
    let out = to_path_at(&src, "<rect").unwrap();
    assert!(out.starts_with("<path"), "{out}");
    for kept in [
        r#"id="r""#,
        r#"x:label="Box""#,
        r#"fill="var(accent)""#,
        r#"filter="blur(2)""#,
        r#"transform="rotate(3)""#,
    ] {
        assert!(out.contains(kept), "{kept}: {out}");
    }
    for gone in ["width=", "rx=", " x=\"1\"", "<filter"] {
        assert!(!out.contains(gone), "{gone}: {out}");
    }
    // a rounded rect: the sharp rect with its rounding kept live as x:corners
    // (§5.4), so the corner widget still works on the result
    assert!(
        out.contains(" x:corners=\"1\" d=\"M1,2 h10 v5 h-10 Z\""),
        "{out}"
    );
    let out = to_path_at(&src, "<circle").unwrap();
    // a circle: four quarter arcs from the rightmost point, clockwise in
    // y-down space, the last landing on the start — anchors at the extremes
    assert!(
        out.contains(r##"stroke="#000""##) && !out.contains("cx="),
        "{out}"
    );
    assert!(
        out.contains(" d=\"M7 5 A2 2 0 0 1 5 7 A2 2 0 0 1 3 5 A2 2 0 0 1 5 3 A2 2 0 0 1 7 5 Z\""),
        "{out}"
    );
    let out = to_path_at(&src, "<polygon").unwrap();
    assert!(
        out.contains(r#"class="tri""#) && !out.contains("points="),
        "{out}"
    );
}

#[test]
fn to_path_on_text_on_a_path_and_textarea_carries_the_transform() {
    // emitters that write no transform of their own: the carry brings it
    let src = format!(
        r##"{XW}<path id="q" d="M0,20 C40,0 80,40 120,20" fill="none"/><x:textpath id="w" in="#q" effect="skew" font-size="20" fill="#111" transform="translate(5 6)">wave</x:textpath><textArea id="a" x="0" y="0" width="100" font-size="10" transform="rotate(1)">area</textArea></svg>"##
    );
    let out = to_path_at(&src, "<x:textpath").unwrap();
    assert!(
        out.starts_with(r##"<g fill="#111" transform="translate(5 6)" id="w">"##),
        "{out}"
    );
    let out = to_path_at(&src, "<textArea").unwrap();
    assert!(
        out.starts_with(r##"<g fill="#000" transform="rotate(1)" id="a">"##),
        "{out}"
    );
}

#[test]
fn to_path_markup_compiles_to_what_the_outlined_text_rendered() {
    // the round trip the editor makes: replace the node by its to_path markup,
    // compile — the glyph paths are byte-identical to the node rendered with
    // outline="true" (keys and tokens aside), over the dataset's text boxes
    let mut checked = 0;
    for path in dataset_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let platform = Platform::new(&Mono, &NoShaper, &BoxOutliner);
        let Ok(s) = platform.session(&src, "balanced", false, "") else {
            continue;
        };
        let doc = s.document();
        let boxes: Vec<(usize, usize)> = doc
            .parsed
            .doc
            .root_element()
            .descendants()
            .filter(|n| {
                n.is_element()
                    && n.tag_name().namespace() == Some(XSVG_NS)
                    && n.tag_name().name() == "textbox"
                    && n.attribute("in").is_none()
                    && n.parent_element()
                        .is_some_and(|p| p.parent_element().is_none())
            })
            .map(|n| (n.range().start, n.range().end))
            .collect();
        for (start, end) in boxes {
            let Ok(markup) = s.to_path(start) else {
                continue;
            };
            let replaced = format!("{}{}{}", &src[..start], markup, &src[end..]);
            let again = platform
                .session(&replaced, "balanced", false, "")
                .unwrap()
                .compile()
                .unwrap();
            let ds = |o: &str| o.matches(" d=\"M").count();
            let rendered = platform
                .session(&src, "balanced", false, "")
                .unwrap()
                .compile_fragment(start)
                .unwrap();
            // every traced path of the conversion is in the recompiled document
            for d in markup
                .split(" d=\"")
                .skip(1)
                .map(|t| t.split('"').next().unwrap())
            {
                assert!(
                    again.contains(d),
                    "{}: a path was lost in the round trip",
                    path.display()
                );
            }
            assert!(ds(&markup) >= 1, "{}: {rendered}", path.display());
            checked += 1;
        }
    }
    assert!(checked > 5, "{checked}");
}

#[test]
fn every_outliner_s_contours_come_out_closed() {
    // opentype.js 2 closes no contour, drawing a line back to the start instead;
    // a font parser's builder closes each. The compiler closes for both, so the
    // editor's path tools see closed shapes whichever outliner traced them.
    struct Open;
    impl GlyphOutliner for Open {
        fn outline(&self, _: &str, _: &TextStyle, _: f64, x: f64, b: f64) -> Option<String> {
            Some(format!(
                "M{x} {b}L{} {b}L{} {}L{x} {}L{x} {b}",
                x + 4.0,
                x + 4.0,
                b - 4.0,
                b - 4.0
            ))
        }
        fn advances(&self, text: &str, _: &TextStyle, size: f64) -> Option<Vec<f64>> {
            Some(
                (1..=text.chars().count())
                    .map(|i| i as f64 * size * 0.5)
                    .collect(),
            )
        }
    }
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="40" font-size="10" outline="true">ab</x:textbox><text x="5" y="30" x:outline="true">c</text></svg>"##
    );
    let out = compile_impl(&src, "balanced", false, &Mono, &NoShaper, &Open).unwrap();
    for d in out
        .split(" d=\"")
        .skip(1)
        .map(|t| t.split('"').next().unwrap())
    {
        assert!(d.ends_with('Z'), "closed: {d}");
        assert_eq!(
            d.matches('L').count(),
            3,
            "the line back to the start is Z now: {d}"
        );
    }
    assert_eq!(out.matches(" d=\"").count(), 2, "{out}");
    let markup = to_path_impl(
        &src,
        src.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &Open,
    )
    .unwrap();
    assert!(
        markup.contains("L0 -4Z\"") || markup.contains("Z\"/>"),
        "{markup}"
    );
    // a closing outliner's bytes are untouched
    let closed = compile_outlined(&src);
    assert!(closed.contains("h1 v-1 h-1 Z"), "{closed}");
}

#[test]
fn outlined_runs_are_traced_in_their_own_weight_slant_and_size() {
    // a bold, an italic and a larger run inside a box: each traced in its own
    // style, the pen advanced by that style's width — the outliner here makes
    // bold twice as wide and encodes weight, slant and size in each piece's height
    struct Styled;
    impl GlyphOutliner for Styled {
        fn outline(&self, text: &str, st: &TextStyle, size: f64, x: f64, b: f64) -> Option<String> {
            let w = self.advances(text, st, size)?.last().copied()?;
            let h = size / 10.0
                + if st.weight == "700" { 1.0 } else { 0.0 }
                + if st.style == "italic" { 2.0 } else { 0.0 };
            Some(format!(
                "M{} {} h{} v-{} h-{} Z",
                fmt(x),
                fmt(b),
                fmt(w),
                fmt(h),
                fmt(w)
            ))
        }
        fn advances(&self, text: &str, st: &TextStyle, size: f64) -> Option<Vec<f64>> {
            let per = size * if st.weight == "700" { 1.0 } else { 0.5 };
            Some((1..=text.chars().count()).map(|i| i as f64 * per).collect())
        }
    }
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="400" height="40" font-size="10" outline="true">ab<tspan font-weight="700">cd</tspan><tspan font-style="italic">ef</tspan><tspan font-size="20">gh</tspan>ij</x:textbox></svg>"##
    );
    let out = compile_impl(&src, "balanced", false, &Mono, &NoShaper, &Styled).unwrap();
    let ds: Vec<&str> = out
        .split(" d=\"")
        .skip(1)
        .map(|t| t.split('"').next().unwrap())
        .collect();
    // on the line's baseline (14: its tallest run is the 20px one) —
    // ab: regular, 10 wide, height 1 | cd: bold at 10, 20 wide, height 2
    // ef: italic at 30, 10 wide, height 3 | gh: size 20 at 40, 20 wide, height 2 | ij at 60
    assert_eq!(
        ds,
        vec![
            "M0 14 h10 v-1 h-10 Z",
            "M10 14 h20 v-2 h-20 Z",
            "M30 14 h10 v-3 h-10 Z",
            "M40 14 h20 v-2 h-20 Z",
            "M60 14 h10 v-1 h-10 Z",
        ],
        "{out}"
    );
    // and the miss names the run's own face when it is the run the outliner lacks
    struct NoBold;
    impl GlyphOutliner for NoBold {
        fn outline(&self, _: &str, st: &TextStyle, _: f64, x: f64, b: f64) -> Option<String> {
            (st.weight != "700").then(|| format!("M{} {} h1 v-1 h-1 Z", fmt(x), fmt(b)))
        }
        fn advances(&self, text: &str, st: &TextStyle, size: f64) -> Option<Vec<f64>> {
            (st.weight != "700").then(|| {
                (1..=text.chars().count())
                    .map(|i| i as f64 * size * 0.5)
                    .collect()
            })
        }
    }
    let err = to_path_impl(
        &src,
        src.find("<x:textbox").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &NoBold,
    )
    .unwrap_err();
    assert_eq!(err.weight.as_deref(), Some("700"), "{err:?}");
}

#[test]
fn outlines_apply_tracking_word_gaps_justification_and_glyph_scale() {
    // an outliner with 5-unit glyphs: tracking goes after every glyph, the
    // word gap after every space, the justify slack with it — the pens the
    // live text is laid by (line_advance); a glyph-x-scale stretches the line
    // from its left edge as textLength does
    struct Five;
    impl GlyphOutliner for Five {
        fn outline(&self, text: &str, _: &TextStyle, _: f64, x: f64, b: f64) -> Option<String> {
            let w = 5.0 * text.chars().filter(|c| !c.is_whitespace()).count() as f64;
            (w > 0.0).then(|| format!("M{} {} h{} v-1 h-{} Z", fmt(x), fmt(b), fmt(w), fmt(w)))
        }
        fn advances(&self, text: &str, _: &TextStyle, _: f64) -> Option<Vec<f64>> {
            Some((1..=text.chars().count()).map(|i| i as f64 * 5.0).collect())
        }
    }
    let ds = |src: &str| -> Vec<String> {
        let out = compile_impl(src, "balanced", false, &Mono, &NoShaper, &Five).unwrap();
        out.split(" d=\"")
            .skip(1)
            .map(|t| t.split('"').next().unwrap().to_string())
            .collect()
    };
    // a b at 0, 7; the space at 14 draws nothing; c at 15 + 2+2+2 + 3 = 24, d at 31
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="400" height="40" font-size="10" letter-spacing="2" word-spacing="3" outline="true">ab cd</x:textbox></svg>"##
    );
    assert_eq!(
        ds(&src),
        vec!["M0 7 h5 v-1 h-5 ZM7 7 h5 v-1 h-5 ZM24 7 h5 v-1 h-5 ZM31 7 h5 v-1 h-5 Z"]
    );
    // a bold run inside keeps the same pens: the run's width counts its tracking
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="400" height="40" font-size="10" letter-spacing="2" outline="true">ab<tspan font-weight="700">cd</tspan>e</x:textbox></svg>"##
    );
    assert_eq!(
        ds(&src),
        vec![
            "M0 7 h5 v-1 h-5 ZM7 7 h5 v-1 h-5 Z",
            "M14 7 h5 v-1 h-5 ZM21 7 h5 v-1 h-5 Z",
            "M28 7 h5 v-1 h-5 Z"
        ]
    );
    // justified: a full line of "aa bb cc" in a 50-wide box (natural 40, two
    // spaces) gets 5 extra per space; the last line stays natural
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="50" height="100" font-size="10" align="justify" outline="true">aa bb cc dd</x:textbox></svg>"##
    );
    let d = ds(&src);
    assert_eq!(
        d[0], "M0 7 h5 v-1 h-5 ZM5 7 h5 v-1 h-5 ZM20 7 h5 v-1 h-5 ZM25 7 h5 v-1 h-5 ZM40 7 h5 v-1 h-5 ZM45 7 h5 v-1 h-5 Z",
        "{d:?}"
    );
    assert!(d[1].starts_with("M0 "), "{d:?}");
    // glyph-x-scale 2: the line stretched from its left edge, 10 → 20 wide, at x = 10
    let src = format!(
        r##"{XW}<x:textbox x="10" y="0" width="400" height="40" font-size="10" glyph-x-scale="2" outline="true">ab</x:textbox></svg>"##
    );
    let d = ds(&src);
    assert!(d[0].starts_with("M10") && d[0].contains("30"), "{d:?}");
}

#[test]
fn every_outlined_contour_in_the_dataset_has_the_closed_form_editors_fold() {
    // The contract an editor's path model folds against (CHANGELOG, §6.12):
    // every contour of every traced glyph ends in Z; a line never returns to
    // the start before it (Z is that edge); a curve before Z ends at the start
    // (a curved seam, its endpoint the first node) or short of it (Z the
    // straight seam); no contour is a move and its close alone. Over the to_path of every text element of every
    // dataset sample — only glyph paths, nothing else the document draws —
    // through the deterministic outliner and the real CLI alike.
    use crate::kurbo::{BezPath, PathEl};
    fn check(d: &str, what: &str) -> usize {
        let path = BezPath::from_svg(d).unwrap_or_else(|e| panic!("{what}: {e}"));
        let mut contours = 0;
        let mut start = None;
        let mut prev: Option<PathEl> = None;
        for el in path.elements() {
            match el {
                PathEl::MoveTo(p) => {
                    assert!(
                        matches!(prev, None | Some(PathEl::ClosePath)),
                        "{what}: a contour without Z"
                    );
                    start = Some(*p);
                    contours += 1;
                }
                PathEl::ClosePath => {
                    let s = start.expect("a move first");
                    match prev {
                        Some(PathEl::LineTo(p)) => assert!(
                            (p - s).hypot() > 1e-6,
                            "{what}: a line back to the start before Z"
                        ),
                        // a curve may end at the start (a curved seam) or short
                        // of it (Z is then the straight seam): both are edges
                        Some(PathEl::QuadTo(..)) | Some(PathEl::CurveTo(..)) => {}
                        Some(PathEl::MoveTo(_)) | None => {
                            panic!("{what}: a move and its close alone")
                        }
                        Some(PathEl::ClosePath) => panic!("{what}: Z twice"),
                    }
                }
                _ => {}
            }
            prev = Some(*el);
        }
        assert!(
            matches!(prev, Some(PathEl::ClosePath)),
            "{what}: the last contour is open"
        );
        contours
    }
    let paths_in = |markup: &str| -> Vec<String> {
        markup
            .split(" d=\"")
            .skip(1)
            .map(|t| t.split('"').next().unwrap().to_string())
            .collect()
    };
    let is_text = |n: &roxmltree::Node| {
        n.is_element()
            && ((n.tag_name().namespace() == Some(XSVG_NS)
                && matches!(n.tag_name().name(), "textbox" | "textpath"))
                || (n.tag_name().namespace() == Some(SVG_NS)
                    && matches!(n.tag_name().name(), "text" | "textArea")))
    };
    let cli = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug/xsvg");
    let fonts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts");
    let native = std::path::Path::new(cli).exists() && std::path::Path::new(fonts).is_dir();
    let (mut contours, mut native_contours) = (0, 0);
    for path in dataset_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let platform = Platform::new(&Mono, &NoShaper, &BoxOutliner);
        let Ok(s) = platform.session(&src, "balanced", false, "") else {
            continue;
        };
        let spans: Vec<(usize, usize)> = s
            .document()
            .parsed
            .doc
            .root_element()
            .descendants()
            .filter(is_text)
            .map(|n| (n.range().start, n.range().end))
            .collect();
        for &(start, end) in &spans {
            let what = format!("{} @{start}", path.display());
            if let Ok(markup) = s.to_path(start) {
                for d in paths_in(&markup) {
                    contours += check(&d, &what);
                }
            }
            if native {
                // the CLI writes the source with the node replaced: the markup is
                // what sits between the untouched prefix and suffix
                let tmp =
                    std::env::temp_dir().join(format!("xsvg-seam-{}.xsvg", std::process::id()));
                std::fs::write(&tmp, &src).unwrap();
                let run = std::process::Command::new(cli)
                    .args(["--font-directory", fonts, "--to-path", &start.to_string()])
                    .arg(&tmp)
                    .output()
                    .expect("the CLI runs");
                let _ = std::fs::remove_file(&tmp);
                if !run.status.success() {
                    continue; // a refusal (an effect with no outline form): not a glyph
                }
                let out = String::from_utf8_lossy(&run.stdout);
                let suffix = src.len() - end;
                let markup = &out[start..out.len() - suffix];
                for d in paths_in(markup) {
                    native_contours += check(&d, &format!("{what} (CLI)"));
                }
            }
        }
    }
    assert!(contours > 300, "{contours}");
    if native {
        assert!(native_contours > 300, "{native_contours}");
    }
}

#[test]
fn curved_shapes_convert_to_exact_arcs_in_the_closed_form() {
    // an ellipse and rounded rects convert to circular arcs — the compiler's
    // own geometry, exact — in the closed form: the last arc lands on the
    // start, then Z; a stadium (rx = w/2) has no straight edges at all
    let src = format!(
        r##"{XW}<ellipse cx="50" cy="40" rx="30" ry="20"/><rect x="0" y="0" width="20" height="10" rx="5"/><rect id="s" x="0" y="0" width="20" height="10" rx="10" ry="5"/></svg>"##
    );
    let ellipse = to_path_at(&src, "<ellipse").unwrap();
    assert!(
        ellipse.contains(" d=\"M80 40 A30 20 0 0 1 50 60 A30 20 0 0 1 20 40 A30 20 0 0 1 50 20 A30 20 0 0 1 80 40 Z\""),
        "{ellipse}"
    );
    // circular corners stay live under to_path; to_spline bakes them — 20×10
    // with r 5: the short sides are all arc, no zero-length edge written
    let needle = "<rect x=\"0\" y=\"0\" width=\"20\" height=\"10\" rx=\"5\"";
    let rounded = to_path_at(&src, needle).unwrap();
    assert!(
        rounded.contains(" x:corners=\"5\" d=\"M0,0 h20 v10 h-20 Z\""),
        "{rounded}"
    );
    let baked = to_spline_at(&src, needle).unwrap();
    assert_eq!(baked.matches(" C").count(), 4, "{baked}");
    assert!(
        baked.contains(" h10 C17.761 0 20 2.239 20 5 ")
            && !baked.contains("corners")
            && !baked.contains(" v0"),
        "{baked}"
    );
    // elliptical corners have no live form: arcs, as the compiler draws them
    let stadium = to_path_at(&src, "<rect id=\"s\"").unwrap();
    assert_eq!(stadium.matches(" a10,5").count(), 4, "{stadium}");
    assert!(
        !stadium.contains(" h") && !stadium.contains(" v"),
        "no straight edge at all: {stadium}"
    );
}

#[test]
fn to_path_keeps_live_corners_as_illustrator_does_and_to_spline_expands() {
    // Convert to path leaves a live corner live: a token radius is carried
    // verbatim, a polygon's and a polyline's corners ride onto the path (the
    // polyline opened), a rect's `rx` becomes `x:corners` — also when only `ry`
    // is given, since SVG defaults each to the other; `rx ≠ ry` bakes. Expand
    // (to_spline) spends the corners and writes Béziers only.
    let src = format!(
        r##"{XW}<x:theme><x:color name="r" value="8"/></x:theme><rect x="0" y="0" width="40" height="20" x:corners="var(r)"/><polygon points="0,0 40,0 20,30" x:corners="0:4"/><polyline points="0,0 40,0 40,40" x:corners="6"/><rect id="ry" x="0" y="0" width="40" height="20" ry="3"/><rect id="el" x="0" y="0" width="40" height="20" rx="3" ry="2"/></svg>"##
    );
    let path = |needle: &str| to_path_at(&src, needle).unwrap();
    let spline = |needle: &str| to_spline_at(&src, needle).unwrap();
    assert!(
        path("<rect x").contains(r#" x:corners="var(r)" d="M0,0 h40 v20 h-40 Z""#),
        "{}",
        path("<rect x")
    );
    assert!(
        path("<polygon").contains(r#" x:corners="0:4" d="M0,0 L40,0 L20,30 Z""#),
        "{}",
        path("<polygon")
    );
    assert!(
        path("<polyline").contains(r#" x:corners="6" d="M0,0 L40,0 L40,40""#),
        "{}",
        path("<polyline")
    );
    assert!(
        path(r#"<rect id="ry""#).contains(r#" x:corners="3" d="M0,0 h40 v20 h-40 Z""#),
        "{}",
        path(r#"<rect id="ry""#)
    );
    let el = path(r#"<rect id="el""#);
    assert!(
        el.matches(" a3,2").count() == 4 && !el.contains("corners"),
        "{el}"
    );
    for needle in [
        "<rect x",
        "<polygon",
        "<polyline",
        r#"<rect id="ry""#,
        r#"<rect id="el""#,
    ] {
        let s = spline(needle);
        assert!(
            !s.contains("corners") && !s.contains(" A") && !s.contains(" a"),
            "{s}"
        );
    }
    // the token resolves when baked: r 8 → the handles κ·8 = 4.418 from the corner
    assert!(
        spline("<rect x").contains(" C36.418 0 40 3.582 40 8 "),
        "{}",
        spline("<rect x")
    );
    // the replaced document draws the same: the live form compiles to the
    // rounding the rect had
    let doc = src.replace(
        r##"<rect x="0" y="0" width="40" height="20" x:corners="var(r)"/>"##,
        &path("<rect x"),
    );
    let first_path = |out: String| {
        out.split("<path")
            .nth(1)
            .unwrap()
            .split("/>")
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(
        first_path(compile_test(&doc)),
        first_path(compile_test(&src))
    );
}

#[test]
fn a_path_converts_to_itself_under_to_path_and_bakes_under_to_spline() {
    // a <path> is already a path: to_path hands it back with its corners live
    // and its `d` as authored (a mixed selection's Convert to path need not skip
    // it); to_spline bakes its corners and expands its arcs like any shape's
    let src = format!(
        r##"{XW}<path id="p" d="M0 0 L100 0 L100 50 L0 50 Z" x:corners="10" fill="#123"/><path d="M0 0 A10 10 0 0 1 10 10 l5 0"/></svg>"##
    );
    assert_eq!(
        to_path_at(&src, r#"<path id="p""#).unwrap(),
        r##"<path id="p" fill="#123" x:corners="10" d="M0 0 L100 0 L100 50 L0 50 Z"/>"##
    );
    assert_eq!(
        to_spline_at(&src, r#"<path id="p""#).unwrap(),
        r##"<path id="p" fill="#123" d="M10 0 L90 0 C95.523 0 100 4.477 100 10 L100 40 C100 45.523 95.523 50 90 50 L10 50 C4.477 50 0 45.523 0 40 L0 10 C0 4.477 4.477 0 10 0 Z"/>"##
    );
    assert_eq!(
        to_path_at(&src, r#"<path d="M0 0 A"#).unwrap(),
        r##"<path d="M0 0 A10 10 0 0 1 10 10 l5 0"/>"##
    );
    assert_eq!(
        to_spline_at(&src, r#"<path d="M0 0 A"#).unwrap(),
        r##"<path d="M0 0 C5.523 0 10 4.477 10 10 l5 0"/>"##
    );
}

#[test]
fn an_x_boolean_converts_to_its_flattened_path_and_hands_its_operands_out() {
    // the compiler's own answer for an <x:boolean> is the flattened path with
    // `fill` and the editor's `x:label` kept, `op` and the children spent;
    // the curve-keeping answer is xsvg-path-ops' (its own tests pin it), fed
    // by the operands query: the circle as its four κ cubics, the rect as
    // lines, the op, and the same flattened `d` to gate against
    let src = format!(
        r##"{XW}<x:boolean id="b" op="union" fill="#48a" x:label="Badge"><rect x="10" y="10" width="100" height="50"/><circle cx="110" cy="35" r="30"/></x:boolean></svg>"##
    );
    let out = to_path_at(&src, "<x:boolean").unwrap();
    assert!(
        out.starts_with(r##"<path id="b" fill="#48a" x:label="Badge" d="M10,60l0-50 83.46,0 "##),
        "{out}"
    );
    assert_eq!(to_spline_at(&src, "<x:boolean").unwrap(), out);
    let q = boolean_operands_impl(
        &src,
        src.find("<x:boolean").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert_eq!(q.op, BoolOp::Union);
    assert_eq!(q.operands.len(), 2);
    assert_eq!(
        q.operands[0].paths,
        vec!["M10,10 h100 v50 h-100 Z".to_string()]
    );
    assert!(
        q.operands[1].paths[0].starts_with("M140 35 C140 51.569 126.569 65 110 65 "),
        "{:?}",
        q.operands[1]
    );
    assert!(!q.operands.iter().any(|o| o.even_odd));
    let d = out
        .split(" d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(q.flat.as_deref(), Some(d));
    // a legitimately empty result has nothing to write, and no operands to hand out
    let empty = format!(
        r##"{XW}<x:boolean op="intersect"><rect x="0" y="0" width="10" height="10"/><rect x="50" y="50" width="10" height="10"/></x:boolean></svg>"##
    );
    assert!(to_path_at(&empty, "<x:boolean").is_err());
    let q = boolean_operands_impl(
        &empty,
        empty.find("<x:boolean").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &BoxOutliner,
    )
    .unwrap();
    assert_eq!(q.flat.as_deref(), Some(""));
    assert!(boolean_operands_impl(
        &src,
        src.find("<rect").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &BoxOutliner
    )
    .is_none());
}

#[test]
fn an_x_boolean_converted_draws_what_the_compiler_draws() {
    // subtract with a hole, a text operand (outlined by the box outliner), a
    // evenodd operand, a stroked operand whose ink joins: the converted path
    // (lowered under convert-to-path, text outlined) and the compiled path
    // agree at every sampled point
    let src = format!(
        r##"{XW}<x:boolean op="subtract" fill="#000"><rect x="0" y="0" width="120" height="80" rx="12"/><circle cx="60" cy="40" r="20"/><x:textbox x="80" y="50" width="40" height="30" font-size="12">Hi</x:textbox><polygon points="0,0 40,40 40,0 0,40" fill-rule="evenodd"/><rect x="100" y="0" width="10" height="10" stroke="#000" stroke-width="4"/></x:boolean></svg>"##
    );
    let converted = to_path_at(&src, "<x:boolean").unwrap();
    assert!(
        converted.starts_with("<path fill=\"#000\" d=\"") && !converted.contains("<text"),
        "{converted}"
    );
    let d_of = |markup: &str| {
        markup
            .split(" d=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let curved = crate::kurbo::BezPath::from_svg(&d_of(&converted)).unwrap();
    let flat = crate::kurbo::BezPath::from_svg(&d_of(&compile_test(&src))).unwrap();
    use crate::kurbo::{Point, Shape};
    let bb = flat.bounding_box().union(curved.bounding_box());
    let (n, mut differ) = (96, 0);
    for j in 0..n {
        for i in 0..n {
            let p = Point::new(
                bb.x0 + (i as f64 + 0.37) * bb.width() / n as f64,
                bb.y0 + (j as f64 + 0.61) * bb.height() / n as f64,
            );
            if (curved.winding(p) != 0) != (flat.winding(p) != 0) {
                differ += 1;
            }
        }
    }
    assert!(
        differ <= n * n / 100,
        "{differ} of {} samples differ\n{converted}",
        n * n
    );
    // the hole is there, wound opposite to the outer
    assert!(
        curved
            .elements()
            .iter()
            .filter(|e| matches!(e, crate::kurbo::PathEl::MoveTo(_)))
            .count()
            >= 2,
        "{converted}"
    );
}

// ---- to_spline (spline.rs; Incremental.md §11) ------------------------------------

#[test]
fn arcs_to_cubics_expands_only_the_arcs_a_quarter_turn_per_cubic() {
    use super::arcs_to_cubics;
    // a circle's four quarter arcs become the four classic cubics (κ·r = 5.523)
    let circle =
        "M10 0 A10 10 0 0 1 0 10 A10 10 0 0 1 -10 0 A10 10 0 0 1 0 -10 A10 10 0 0 1 10 0 Z";
    assert_eq!(
        arcs_to_cubics(circle),
        "M10 0 C10 5.523 5.523 10 0 10 C-5.523 10 -10 5.523 -10 0 C-10 -5.523 -5.523 -10 0 -10 C5.523 -10 10 -5.523 10 0 Z"
    );
    // a half turn splits in two; a 60° corner arc is one cubic; the rest of the
    // data — relative commands, glued numbers, implicit repeats — passes through
    // byte for byte, and a relative arc's endpoint is resolved from the pen
    let d = "m5,5 l10-2 A10 10 0 0 1 35 3 h-1.5e1 a4 4 0 0,1 4 4 L1 2 3 4z";
    let out = arcs_to_cubics(d);
    assert!(out.starts_with("m5,5 l10-2 C"), "{out}");
    assert!(out.contains(" h-1.5e1 "), "{out}");
    assert!(out.ends_with("L1 2 3 4z"), "{out}");
    assert_eq!(out.matches(" C").count(), 3, "{out}");
    // the half turn's two cubics end where the arc did
    assert!(out.contains(" 35 3 h"), "{out}");
    // glued arc flags, implicit repeats of A, a degenerate arc (dropped) and a
    // zero radius (the straight line)
    assert_eq!(
        arcs_to_cubics("M0 0 A5 5 0 01 10 0 5 5 0 0110 0"),
        "M0 0 C0 -2.761 2.239 -5 5 -5 C7.761 -5 10 -2.761 10 0"
    );
    assert_eq!(arcs_to_cubics("M3 3 A5 5 0 0 1 3 3 L4 4"), "M3 3 L4 4");
    assert_eq!(arcs_to_cubics("M0 0 A0 5 0 0 1 10 0"), "M0 0 L10 0");
    // no arcs: identical
    assert_eq!(
        arcs_to_cubics("M0 0 L1 1 Q2 2 3 3 Z"),
        "M0 0 L1 1 Q2 2 3 3 Z"
    );
    // garbage is handed back rather than guessed at
    assert_eq!(arcs_to_cubics("M0 0 A5 5 0 0 1 x"), "M0 0 A5 5 0 0 1 x");
}

#[test]
fn arcs_to_cubics_stays_on_the_ellipse_to_a_quarter_circle_cubics_precision() {
    use crate::kurbo::{Arc, BezPath, ParamCurve, PathSeg, Point, SvgArc, Vec2};
    // every cubic of a converted arc sits on the arc's ellipse (as SVG's
    // endpoint-to-centre conversion defines it) within the one-cubic-per-quadrant
    // error (0.03% of the radius), rotation and radius scaling included;
    // endpoints exact
    type Case = (
        &'static str,
        (f64, f64),
        (f64, f64),
        (f64, f64),
        f64,
        bool,
        bool,
    );
    let cases: [Case; 5] = [
        (
            "M100 0 A100 100 0 0 1 0 100",
            (100.0, 0.0),
            (0.0, 100.0),
            (100.0, 100.0),
            0.0,
            false,
            true,
        ),
        (
            "M100 0 A100 100 0 1 0 0 100",
            (100.0, 0.0),
            (0.0, 100.0),
            (100.0, 100.0),
            0.0,
            true,
            false,
        ),
        (
            "M60 20 A40 20 0 1 1 -20 20",
            (60.0, 20.0),
            (-20.0, 20.0),
            (40.0, 20.0),
            0.0,
            true,
            true,
        ),
        (
            "M20 0 A20 10 30 0 1 -20 0",
            (20.0, 0.0),
            (-20.0, 0.0),
            (20.0, 10.0),
            30.0,
            false,
            true,
        ),
        (
            "M0 0 A1 1 0 0 1 10 0",
            (0.0, 0.0),
            (10.0, 0.0),
            (1.0, 1.0),
            0.0,
            false,
            true,
        ),
    ];
    for (d, from, to, radii, rot_deg, large, sweep) in cases {
        let out = super::arcs_to_cubics(d);
        assert!(!out.contains('A'), "{out}");
        let path = BezPath::from_svg(&out).unwrap();
        let arc = Arc::from_svg_arc(&SvgArc {
            from: Point::new(from.0, from.1),
            to: Point::new(to.0, to.1),
            radii: Vec2::new(radii.0, radii.1),
            x_rotation: rot_deg.to_radians(),
            large_arc: large,
            sweep,
        })
        .unwrap();
        let (cx, cy, rx, ry) = (arc.center.x, arc.center.y, arc.radii.x, arc.radii.y);
        let (s, c) = arc.x_rotation.sin_cos();
        let on_ellipse = |p: Point| {
            let (dx, dy) = (p.x - cx, p.y - cy);
            let (u, v) = (dx * c + dy * s, -dx * s + dy * c);
            ((u / rx).powi(2) + (v / ry).powi(2)).sqrt() - 1.0
        };
        let mut n = 0;
        for seg in path.segments() {
            let PathSeg::Cubic(cu) = seg else {
                panic!("{out}")
            };
            n += 1;
            for k in 0..=20 {
                let e = on_ellipse(cu.eval(k as f64 / 20.0)).abs();
                assert!(e < 3e-4, "{d}: off the ellipse by {e} at {k}/20 in {out}");
            }
        }
        assert!(n >= 1, "{out}");
        let last = path.segments().last().unwrap().end();
        let end: Vec<f64> = d.rsplit(' ').take(2).map(|t| t.parse().unwrap()).collect();
        assert!(
            (last.x - end[1]).abs() < 1e-9 && (last.y - end[0]).abs() < 1e-9,
            "{out}"
        );
    }
}

#[test]
fn to_spline_is_to_path_with_its_arcs_as_cubics() {
    let src = format!(
        r##"{XW}<circle cx="5" cy="5" r="2" fill="var(ink)" id="c"/><rect x="2" y="2" width="10" height="5" rx="1" x:label="card"/><rect x="0" y="0" width="100" height="50" x:corners="10" x:corner-style="chamfer"/></svg>"##
    );
    let spline = |needle: &str| {
        to_spline_impl(
            &src,
            nth_at(&src, needle, 0),
            "balanced",
            &Mono,
            &NoShaper,
            &NoOutliner,
        )
    };
    let path = |needle: &str| to_path_at(&src, needle);
    let circle = spline("<circle").unwrap();
    assert_eq!(
        circle,
        r##"<path fill="var(ink)" id="c" d="M7 5 C7 6.105 6.105 7 5 7 C3.895 7 3 6.105 3 5 C3 3.895 3.895 3 5 3 C6.105 3 7 3.895 7 5 Z"/>"##
    );
    // the arcs are the only difference from to_path
    let rounded = spline("<rect").unwrap();
    assert!(
        !rounded.contains('A') && rounded.contains(r#"x:label="card""#),
        "{rounded}"
    );
    assert_eq!(rounded.matches(" C").count(), 4, "{rounded}");
    // the circle has no live form, so there to_spline is exactly to_path's
    // arcs as cubics; the rounded rect's to_path keeps its corners live instead
    assert_eq!(
        super::spline::markup_to_spline(&path("<circle").unwrap()),
        circle
    );
    assert!(path("<rect").unwrap().contains(r#"x:corners="1""#));
    // a chamfer bakes to lines alone while to_path keeps it live
    let chamfer = spline(r#"<rect x="0""#).unwrap();
    assert_eq!(
        chamfer,
        r#"<path d="M10 0 L90 0 L100 10 L100 40 L90 50 L10 50 L0 40 L0 10 Z"/>"#
    );
    assert!(path(r#"<rect x="0""#)
        .unwrap()
        .contains(r#"x:corner-style="chamfer""#));
    // an `id` is not a `d`
    assert!(circle.contains(r#"id="c""#));
}

// ---- corners (corners.rs; §5.4) ------------------------------------------------

fn corner_paths(src: &str) -> Vec<String> {
    compile_test(src)
        .split(" d=\"")
        .skip(1)
        .map(|t| t.split('"').next().unwrap().to_string())
        .collect()
}

#[test]
fn indexed_items_is_the_one_parser_for_index_value_lists() {
    let items: Vec<_> = super::indexed_items("8, 1:12 3:var(r)\n 2:4px").collect();
    assert_eq!(
        items,
        vec![
            (None, "8"),
            (Some(1), "12"),
            (Some(3), "var(r)"),
            (Some(2), "4px")
        ]
    );
    // A keyed item needs a parsable index; its value is handed back untouched for
    // the caller to interpret (a number, a token, an unspent `:`).
    let items: Vec<_> = super::indexed_items("x:8 :8 1: 1:2:3").collect();
    assert_eq!(items, vec![(Some(1), ""), (Some(1), "2:3")]);
    assert_eq!(super::indexed_items("  ,, ").count(), 0);
}

#[test]
fn corners_round_a_rect_exactly_as_rx_does() {
    // x:corners="10" on a sharp rect is the rounded rect's own arcs, clockwise,
    // the last landing on the start; a token radius resolves; per-anchor radii
    // leave the other corners sharp; a chamfer is the straight cut, the closing
    // one being Z itself; inverted flips the sweep
    let src = format!(
        r##"{XW}<x:theme><x:color name="r" value="8"/></x:theme><rect x="0" y="0" width="100" height="50" x:corners="10"/><rect x="0" y="0" width="100" height="50" x:corners="0:10 1:var(r)"/><rect x="0" y="0" width="100" height="50" x:corners="10" x:corner-style="chamfer"/><rect x="0" y="0" width="100" height="50" x:corners="10" x:corner-style="inverted"/></svg>"##
    );
    let d = corner_paths(&src);
    assert_eq!(d[0], "M10 0 L90 0 A10 10 0 0 1 100 10 L100 40 A10 10 0 0 1 90 50 L10 50 A10 10 0 0 1 0 40 L0 10 A10 10 0 0 1 10 0 Z");
    assert_eq!(
        d[1],
        "M10 0 L92 0 A8 8 0 0 1 100 8 L100 50 L0 50 L0 10 A10 10 0 0 1 10 0 Z"
    );
    assert_eq!(
        d[2],
        "M10 0 L90 0 L100 10 L100 40 L90 50 L10 50 L0 40 L0 10 Z"
    );
    assert_eq!(d[3], "M10 0 L90 0 A10 10 0 0 0 100 10 L100 40 A10 10 0 0 0 90 50 L10 50 A10 10 0 0 0 0 40 L0 10 A10 10 0 0 0 10 0 Z");
    // the output is a <path>; the corner attributes and the geometry are spent
    let out = compile_test(&src);
    assert!(!out.contains("<rect") && !out.contains("corners"), "{out}");
    // the same rect as a <rect rx> compiles to the same arcs (modulo spelling)
    let rx = compile_test(&format!(
        r##"{XW}<rect x="0" y="0" width="100" height="50" rx="10"/></svg>"##
    ));
    assert!(
        rx.contains("rx=\"10\""),
        "a rounded rect passes through: {rx}"
    );
}

#[test]
fn corners_clamp_trim_and_skip_where_illustrator_does() {
    // two corners asking for more than the edge between them share it (a
    // 20-wide rect asking 40 gets 10, its short edges gone); a lone corner may
    // take the whole edge, so a square's one corner at its side length is a
    // quarter circle, and unequal corners share an edge in proportion;
    // a 60° corner trims r/tan(30°) along each side; a polyline's ends and
    // collinear anchors (a line into a tangent curve) have no corner; an
    // unclosed path is open, so its first and last anchors are ends
    let src = format!(
        r##"{XW}<rect x="0" y="0" width="20" height="50" x:corners="40"/><polygon points="0,0 100,0 50,86.6" x:corners="10"/><polyline points="0,0 100,0 100,100" x:corners="10"/><path d="M0 0 L100 0 C150 0 150 50 100 50 L0 50 Z" x:corners="10"/><path d="M0 0 L100 0 L100 50 L0 50 L0 0" x:corners="10"/><rect x="0" y="0" width="100" height="100" x:corners="0:100"/><rect x="0" y="0" width="100" height="100" x:corners="0:90 1:60"/></svg>"##
    );
    let d = corner_paths(&src);
    assert_eq!(d[0], "M10 0 A10 10 0 0 1 20 10 L20 40 A10 10 0 0 1 10 50 A10 10 0 0 1 0 40 L0 10 A10 10 0 0 1 10 0 Z");
    assert_eq!(d[5], "M100 0 L100 100 L0 100 A100 100 0 0 1 100 0 Z");
    // 90 + 60 on a 100 edge: scaled by 100/150 to 60 and 40
    assert_eq!(
        d[6],
        "M60 0 A40 40 0 0 1 100 40 L100 100 L0 100 L0 60 A60 60 0 0 1 60 0 Z"
    );
    assert_eq!(d[1], "M17.321 0 L82.679 0 A10 10 0 0 1 91.339 15 L58.66 71.601 A10 10 0 0 1 41.34 71.601 L8.661 15 A10 10 0 0 1 17.321 0 Z");
    assert_eq!(d[2], "M0 0 L90 0 A10 10 0 0 1 100 10 L100 100");
    assert_eq!(
        d[3],
        "M10 0 L100 0 C150 0 150 50 100 50 L10 50 A10 10 0 0 1 0 40 L0 10 A10 10 0 0 1 10 0 Z"
    );
    assert_eq!(
        d[4],
        "M0 0 L90 0 A10 10 0 0 1 100 10 L100 40 A10 10 0 0 1 90 50 L10 50 A10 10 0 0 1 0 40 L0 0"
    );
}

#[test]
fn the_corners_query_reports_each_corner_as_drawn() {
    let src = format!(
        r##"{XW}<rect x="0" y="0" width="20" height="50" x:corners="40"/><polygon points="0,0 100,0 50,86.6" x:corners="0:10"/><rect x="0" y="0" width="9" height="9"/></svg>"##
    );
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<rect", 0)).unwrap();
    assert_eq!(c.len(), 4);
    assert_eq!(c[0].anchor, 0);
    assert_eq!(c[0].at, (0.0, 0.0));
    let near = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9;
    assert_eq!(c[0].requested, 40.0);
    assert!(
        (c[0].effective - 10.0).abs() < 1e-9 && c[0].clamped,
        "{c:?}"
    );
    assert!(
        near(c[0].from, (0.0, 10.0)) && near(c[0].to, (10.0, 0.0)),
        "{c:?}"
    );
    assert!(near(c[0].center.unwrap(), (10.0, 10.0)), "{c:?}");
    let c = s.corners(nth_at(&src, "<polygon", 0)).unwrap();
    assert_eq!(c.len(), 1, "one corner asked: {c:?}");
    assert!(!c[0].clamped && (c[0].effective - 10.0).abs() < 1e-9);
    assert!(
        s.corners(nth_at(&src, "<rect", 1)).is_none(),
        "no corners asked"
    );
}

/// The area of a compiled corner path, its `A` arcs flattened finely (kurbo's
/// `from_svg` approximates an arc with cubics to 0.1, too coarse to check
/// geometry against). Reads the absolute, space-separated form the compiler emits.
fn path_area(d: &str) -> f64 {
    use crate::kurbo::{Arc, BezPath, Point, Shape, SvgArc, Vec2};
    let toks: Vec<&str> = d.split_whitespace().collect();
    let num = |t: &str| {
        t.trim_start_matches(|c: char| c.is_ascii_alphabetic())
            .parse::<f64>()
            .unwrap()
    };
    let mut path = BezPath::new();
    let mut cur = Point::ZERO;
    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        // the command letter is glued to its first number; the rest follow
        let at = |k: usize| Point::new(num(toks[i + k]), num(toks[i + k + 1]));
        match t.chars().next().unwrap() {
            'M' => {
                cur = at(0);
                path.move_to(cur);
                i += 2;
            }
            'L' => {
                cur = at(0);
                path.line_to(cur);
                i += 2;
            }
            'Q' => {
                let c1 = at(0);
                cur = at(2);
                path.quad_to(c1, cur);
                i += 4;
            }
            'C' => {
                let (c1, c2) = (at(0), at(2));
                cur = at(4);
                path.curve_to(c1, c2, cur);
                i += 6;
            }
            'A' => {
                let to = at(5);
                let arc = Arc::from_svg_arc(&SvgArc {
                    from: cur,
                    to,
                    radii: Vec2::new(num(toks[i]), num(toks[i + 1])),
                    x_rotation: num(toks[i + 2]).to_radians(),
                    large_arc: num(toks[i + 3]) != 0.0,
                    sweep: num(toks[i + 4]) != 0.0,
                })
                .expect("a real arc");
                path.extend(arc.append_iter(1e-6));
                cur = to;
                i += 7;
            }
            'Z' => {
                path.close_path();
                i += 1;
            }
            other => panic!("unexpected {other} in {d}"),
        }
    }
    path.area().abs()
}

#[test]
fn corners_change_the_area_by_exactly_the_fillet() {
    // the normative geometry, checked as area rather than text: on a 90°
    // corner with r = 10, a round corner removes (1 − π/4)·r², a chamfer r²/2,
    // an inverted corner the quarter disc π·r²/4; a concave corner gains what a
    // convex one loses
    let sq = |extra: &str| {
        format!(r##"{XW}<rect x="0" y="0" width="100" height="100" x:corners="10"{extra}/></svg>"##)
    };
    let fillet = (1.0 - std::f64::consts::FRAC_PI_4) * 100.0;
    let near = |a: f64, b: f64| {
        assert!((a - b).abs() < 1e-3, "area {a} expected {b}");
        true
    };
    assert!(near(
        path_area(&corner_paths(&sq(""))[0]),
        10000.0 - 4.0 * fillet
    ));
    assert!(near(
        path_area(&corner_paths(&sq(r#" x:corner-style="chamfer""#))[0]),
        10000.0 - 4.0 * 50.0
    ));
    assert!(near(
        path_area(&corner_paths(&sq(r#" x:corner-style="inverted""#))[0]),
        10000.0 - 100.0 * std::f64::consts::PI
    ));
    // an L: anchor 3 is the inner (concave) corner, anchor 0 a convex one
    let l = |c: &str| {
        format!(
            r##"{XW}<path d="M0 0 L100 0 L100 50 L50 50 L50 100 L0 100 Z" x:corners="{c}"/></svg>"##
        )
    };
    assert!(near(
        path_area(&corner_paths(&l("3:10"))[0]),
        7500.0 + fillet
    ));
    assert!(near(
        path_area(&corner_paths(&l("0:10"))[0]),
        7500.0 - fillet
    ));
    assert!(near(
        path_area(&corner_paths(&l("10"))[0]),
        7500.0 - 5.0 * fillet + fillet
    ));
}

#[test]
fn corners_do_not_depend_on_the_winding() {
    // the same rect authored clockwise and counter-clockwise rounds the same
    // way: the arcs sweep the other way, the shape is identical
    let src = format!(
        r##"{XW}<polygon points="0,0 100,0 100,50 0,50" x:corners="10"/><polygon points="0,0 0,50 100,50 100,0" x:corners="10"/></svg>"##
    );
    let d = corner_paths(&src);
    assert_eq!(
        d[0],
        "M10 0 L90 0 A10 10 0 0 1 100 10 L100 40 A10 10 0 0 1 90 50 L10 50 A10 10 0 0 1 0 40 L0 10 A10 10 0 0 1 10 0 Z"
    );
    assert_eq!(
        d[1],
        "M0 10 L0 40 A10 10 0 0 0 10 50 L90 50 A10 10 0 0 0 100 40 L100 10 A10 10 0 0 0 90 0 L10 0 A10 10 0 0 0 0 10 Z"
    );
    assert!((path_area(&d[0]) - path_area(&d[1])).abs() < 1e-6);
}

#[test]
fn corners_trim_a_curved_segment_at_its_arc_length() {
    // a line meets a quadratic at a right angle: the trim point on the curve
    // sits d = r along the curve's arc length, not along its chord or parameter,
    // and the rest of the curve is kept
    let src = format!(r##"{XW}<path d="M0 0 L100 0 Q100 100 0 100 Z" x:corners="1:10"/></svg>"##);
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<path", 0)).unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].anchor, 1);
    assert!((c[0].effective - 10.0).abs() < 1e-9 && !c[0].clamped);
    assert!((c[0].from.0 - 90.0).abs() < 1e-9 && c[0].from.1.abs() < 1e-9);
    use crate::kurbo::{ParamCurve, ParamCurveArclen, Point, QuadBez};
    let q = QuadBez::new(
        Point::new(100.0, 0.0),
        Point::new(100.0, 100.0),
        Point::new(0.0, 100.0),
    );
    let on_curve = q.eval(q.inv_arclen(10.0, 1e-9));
    assert!(
        (c[0].to.0 - on_curve.x).abs() < 1e-6 && (c[0].to.1 - on_curve.y).abs() < 1e-6,
        "to {:?} on-curve {on_curve:?}",
        c[0].to
    );
    let d = corner_paths(&src).remove(0);
    assert!(d.starts_with("M0 0 L90 0 A10 10 0 0 1 "), "{d}");
    // the kept curve still ends at (0, 100); the Z draws the straight side home
    assert!(d.contains(" Q") && d.ends_with(" 0 100 Z"), "{d}");
    // the arc ends where the kept curve begins: the point at arc length 10 (the
    // fillet drawn to the tangent's projection (100, 10) would leave a gap)
    let arc_end = d
        .split(" A10 10 0 0 1 ")
        .nth(1)
        .unwrap()
        .split(" Q")
        .next()
        .unwrap();
    assert_eq!(arc_end, format!("{} {}", fmt(on_curve.x), fmt(on_curve.y)));
}

#[test]
fn corners_number_anchors_across_subpaths_and_at_every_angle() {
    // anchors continue across subpaths (the second square starts at 4); the
    // trim follows the angle — 120° trims r/tan(60°), 45° trims r/tan(22.5°)
    let src = format!(
        r##"{XW}<path d="M0 0 L50 0 L50 50 L0 50 Z M100 0 L150 0 L150 50 L100 50 Z" x:corners="4:10"/><polygon points="0,0 100,0 100,100" x:corners="0:10"/><polygon points="50,0 93.301,25 93.301,75 50,100 6.699,75 6.699,25" x:corners="0:10"/></svg>"##
    );
    let d = corner_paths(&src);
    assert_eq!(
        d[0],
        "M0 0 L50 0 L50 50 L0 50 Z M110 0 L150 0 L150 50 L100 50 L100 10 A10 10 0 0 1 110 0 Z"
    );
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<path", 0)).unwrap();
    assert_eq!((c.len(), c[0].anchor, c[0].at), (1, 4, (100.0, 0.0)));
    let trim = |c: &CornerInfo| ((c.from.0 - c.at.0).powi(2) + (c.from.1 - c.at.1).powi(2)).sqrt();
    let c = s.corners(nth_at(&src, "<polygon", 0)).unwrap();
    assert!(
        (trim(&c[0]) - 10.0 / (22.5f64.to_radians()).tan()).abs() < 1e-6,
        "{c:?}"
    );
    assert!((c[0].effective - 10.0).abs() < 1e-9);
    let c = s.corners(nth_at(&src, "<polygon", 1)).unwrap();
    assert!(
        (trim(&c[0]) - 10.0 / (60f64.to_radians()).tan()).abs() < 1e-3,
        "{c:?}"
    );
}

#[test]
fn corners_leave_what_is_not_a_corner_alone() {
    // a zero or negative radius draws nothing (the element still lowers to a
    // path of its own geometry); a repeated point and a spike have no tangent
    // pair to round and are skipped without upsetting their neighbours; a
    // single-anchor list on a missing anchor is ignored
    let src = format!(
        r##"{XW}<rect x="0" y="0" width="100" height="50" x:corners="0"/><rect x="0" y="0" width="100" height="50" x:corners="-5"/><polygon points="0,0 0,0 100,0 100,50 0,50" x:corners="10"/><path d="M0 0 L100 0 L50 0 L50 50 Z" x:corners="10"/><rect x="0" y="0" width="100" height="50" x:corners="9:10"/></svg>"##
    );
    let d = corner_paths(&src);
    assert_eq!(d[0], "M0 0 L100 0 L100 50 L0 50 Z");
    assert_eq!(d[1], "M0 0 L100 0 L100 50 L0 50 Z");
    assert_eq!(
        d[2],
        "M0 0 L90 0 A10 10 0 0 1 100 10 L100 40 A10 10 0 0 1 90 50 L10 50 A10 10 0 0 1 0 40 Z"
    );
    // the spike at (100, 0) stays sharp; the other three anchors round
    assert!(
        d[3].contains(" L100 0 L60 0 ") && d[3].matches(" A").count() == 3,
        "{}",
        d[3]
    );
    assert_eq!(d[4], "M0 0 L100 0 L100 50 L0 50 Z");
}

#[test]
fn the_corners_query_reports_a_shared_edge_as_clamped() {
    let src = format!(
        r##"{XW}<rect x="0" y="0" width="100" height="100" x:corners="0:90 1:60"/></svg>"##
    );
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<rect", 0)).unwrap();
    assert_eq!(c.len(), 2);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(
        c[0].requested == 90.0 && near(c[0].effective, 60.0) && c[0].clamped,
        "{c:?}"
    );
    assert!(
        c[1].requested == 60.0 && near(c[1].effective, 40.0) && c[1].clamped,
        "{c:?}"
    );
    assert!(near(c[0].to.0, 60.0) && near(c[0].to.1, 0.0), "{c:?}");
    assert!(near(c[1].from.0, 60.0) && near(c[1].from.1, 0.0), "{c:?}");
}

#[test]
fn corners_reach_geometry_consumers_and_stay_live_on_conversion() {
    // a boolean over a cornered rect sees the rounding; to_path returns the
    // sharp path with its corners still live (§5.4) and the rest kept, and
    // to_spline bakes them
    let src = format!(
        r##"{XW}<rect id="r" x:label="Card" x="0" y="0" width="100" height="50" x:corners="10" x:corner-style="chamfer" fill="var(accent)"/><x:boolean op="union"><use href="#r"/></x:boolean></svg>"##
    );
    let markup = to_path_at(&src, "<rect").unwrap();
    assert!(
        markup.starts_with(r##"<path id="r""##) && markup.contains(r##"x:label="Card""##),
        "{markup}"
    );
    assert!(
        markup.contains(r##" x:corners="10" x:corner-style="chamfer" d="M0,0 h100 v50 h-100 Z""##),
        "{markup}"
    );
    assert!(markup.contains(r##"fill="var(accent)""##), "{markup}");
    let baked = to_spline_at(&src, "<rect").unwrap();
    assert!(
        baked.contains(r##" d="M10 0 L90 0 L100 10 L100 40 L90 50 L10 50 L0 40 L0 10 Z""##)
            && !baked.contains("corner"),
        "{baked}"
    );
    let out = compile_test(&src);
    // the union of the rounded rect: its flattened outline has no sharp corner at the origin
    assert!(!out.contains("M0,0") || out.contains("A10"), "{out}");
}

#[test]
fn a_paragraph_sets_its_own_letter_and_word_spacing() {
    // a spaced paragraph wraps on its own spacing, writes it on its <text>,
    // and the query and the layout seam report it; a plain paragraph beside
    // it keeps the box's (none); a cell reads it through its column
    let src = format!(
        r##"{XW}<x:textbox x="0" y="0" width="100" height="200" font-size="10"><x:p>aaaa aaaa aaaa aaaa aaaa aaaa</x:p><x:p letter-spacing="2" word-spacing="4">aaaa aaaa aaaa aaaa aaaa aaaa</x:p></x:textbox><x:table x="0" y="0" width="100" font-size="10"><x:col letter-spacing="3"/><x:tr><x:td>ab</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&src);
    let texts: Vec<&str> = out.split("<text ").skip(1).collect();
    assert!(!texts[0].contains("letter-spacing"), "{}", texts[0]);
    assert!(
        texts[1].starts_with("text-anchor=\"start\"")
            && texts[1].contains(" letter-spacing=\"2\" word-spacing=\"4\""),
        "{}",
        texts[1]
    );
    // Mono: 5 per char at size 10; the plain line holds four words (95) and
    // not five (120); spaced (2 per char, 4 more per space) a word is 28 and a
    // space 11, so two words (67) fit and three (106) do not
    let first_line = |t: &str| {
        t.split("<tspan")
            .nth(1)
            .unwrap()
            .split('>')
            .nth(1)
            .unwrap()
            .split('<')
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(first_line(texts[0]), "aaaa aaaa aaaa aaaa");
    assert_eq!(first_line(texts[1]), "aaaa aaaa");
    // the cell: its column's spacing
    assert!(texts[2].contains(" letter-spacing=\"3\""), "{}", texts[2]);
    let s = ParsedDocument::new(&src).unwrap();
    let r = s
        .resolved(
            nth_at(&src, "<x:p letter", 0),
            &["letter-spacing", "word-spacing"],
        )
        .unwrap();
    assert_eq!(
        (r[0].value.as_deref(), r[0].source.clone()),
        (Some("2"), Source::Own)
    );
    assert_eq!(
        (r[1].value.as_deref(), r[1].source.clone()),
        (Some("4"), Source::Own)
    );
    let r = s
        .resolved(nth_at(&src, "<x:td", 0), &["letter-spacing"])
        .unwrap();
    assert_eq!(r[0].value.as_deref(), Some("3"));
    let layout = layout_at(&src, nth_at(&src, "<x:textbox", 0)).unwrap();
    assert_eq!(layout.paras[0].style.letter_spacing, 0.0);
    assert_eq!(layout.paras[1].style.letter_spacing, 2.0);
    assert_eq!(layout.paras[1].style.word_spacing, 4.0);
}

fn ns_of_el(elements: &[xsvg_schema::ElementSpec], name: &str) -> Option<&'static str> {
    elements.iter().find(|e| e.name == name).unwrap().ns
}

#[test]
fn the_manifest_declares_the_typography_matrix() {
    // §6.1's matrix, as the describe manifest states it: which typography
    // property each text element and block takes. A panel reads this; the
    // cascade (query.rs) and the emitters read the same names
    let elements = super::elements::built_in_elements();
    let params_of = |name: &str| -> Vec<&str> {
        elements
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("{name} is not declared"))
            .all_params()
            .map(|p| p.name)
            .collect()
    };
    const FONT: [&str; 8] = [
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "line-height",
        "letter-spacing",
        "word-spacing",
        "text-transform",
    ];
    for (name, own) in [
        (
            "textbox",
            vec![
                "align",
                "valign",
                "direction",
                "text-indent",
                "paragraph-spacing",
                "fit",
                "fit-min",
                "text-overflow",
                "overflow",
                "padding",
                "outline",
                "fill",
            ],
        ),
        (
            "textpath",
            vec!["align", "start", "baseline-shift", "effect", "fill"],
        ),
        (
            "list",
            vec![
                "list",
                "marker",
                "marker-fill",
                "marker-size",
                "marker-gap",
                "indent",
                "item-spacing",
                "valign",
                "fill",
                "outline",
            ],
        ),
        (
            "table",
            vec![
                "cols",
                "cell-padding",
                "row-min-height",
                "align",
                "valign",
                "fill",
                "header-fill",
                "cell-fill",
                "stripe",
                "stroke",
                "stroke-width",
                "outline",
            ],
        ),
        (
            "p",
            vec![
                "align",
                "direction",
                "text-indent",
                "space-before",
                "space-after",
                "fill",
            ],
        ),
        ("li", vec!["indent", "list", "marker", "direction", "fill"]),
        ("td", vec!["align", "valign", "direction", "bg", "fill"]),
        ("th", vec!["align", "valign", "direction", "bg", "fill"]),
        ("tr", vec!["align", "valign", "direction", "bg", "fill"]),
        ("col", vec!["align", "valign", "direction", "bg", "fill"]),
        (
            "text",
            vec![
                "inline-size",
                "text-anchor",
                "fill",
                "outline",
                "glyph-x-scale",
            ],
        ),
        (
            "textArea",
            vec![
                "width",
                "height",
                "text-align",
                "display-align",
                "line-increment",
                "overflow",
                "fill",
                "outline",
                "glyph-x-scale",
            ],
        ),
    ] {
        let declared = params_of(name);
        for p in FONT.iter().chain(own.iter()) {
            assert!(
                declared.contains(p),
                "<{name}> should declare {p}: {declared:?}"
            );
        }
    }
    // what a block must NOT claim: a box-level control
    for (name, not) in [
        ("p", "valign"),
        ("li", "align"),
        ("td", "padding"),
        ("col", "text-indent"),
        ("list", "text-indent"),
        ("list", "paragraph-spacing"),
    ] {
        assert!(
            !params_of(name).contains(&not),
            "<{name}> should not declare {not}"
        );
    }
    // a run takes the face, size, case and paint — not the line's metrics
    let tspan = params_of("tspan");
    for p in [
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "text-transform",
        "fill",
        "text-decoration",
        "baseline-shift",
        "highlight",
        "border-width",
    ] {
        assert!(tspan.contains(&p), "<tspan> should declare {p}: {tspan:?}");
    }
    for not in ["line-height", "letter-spacing", "align"] {
        assert!(!tspan.contains(&not), "<tspan> should not declare {not}");
    }
    // the x: attributes on SVG elements are declared by namespace, never by
    // a written prefix
    let param = |el: &str, name: &str| {
        elements
            .iter()
            .find(|e| e.name == el)
            .unwrap()
            .all_params()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("<{el}> should declare {name}"))
    };
    for (el, name) in [
        ("text", "outline"),
        ("textArea", "outline"),
        ("text", "glyph-x-scale"),
        ("tspan", "highlight"),
        ("rect", "corners"),
        ("polygon", "corners"),
        ("polyline", "corner-style"),
        ("path", "corners"),
    ] {
        assert_eq!(param(el, name).ns, Some(XSVG_NS), "<{el}> {name}");
        assert_eq!(ns_of_el(elements, el), Some("http://www.w3.org/2000/svg"));
    }
    assert_eq!(param("textbox", "glyph-x-scale").ns, None);
    for spec in elements {
        assert!(
            !spec.all_params().any(|p| p.name.starts_with("x:")),
            "<{}> spells a prefix into a parameter name",
            spec.name
        );
    }
    // a property a block inherits declares no default of its own: the value
    // comes from its container (`resolved` reports which)
    for (el, name) in [
        ("p", "align"),
        ("p", "direction"),
        ("li", "direction"),
        ("td", "align"),
        ("td", "valign"),
        ("th", "valign"),
        ("tr", "align"),
        ("tr", "valign"),
        ("col", "align"),
        ("col", "valign"),
        ("td", "direction"),
        ("col", "direction"),
    ] {
        let p = param(el, name);
        assert_eq!(p.default, None, "<x:{el}> {name} inherits");
        assert!(p.doc.is_some(), "<x:{el}> {name} says where it comes from");
        assert!(!p.values.is_empty(), "<x:{el}> {name} keeps its spellings");
    }
    assert_eq!(param("table", "valign").default, Some("middle"));
    assert_eq!(param("table", "direction").default, Some("ltr"));
    assert_eq!(param("list", "direction").default, Some("ltr"));
    assert_eq!(param("textbox", "align").default, Some("start"));
    // the SVG text elements carry their namespace; the x: ones none
    let ns_of = |name: &str| elements.iter().find(|e| e.name == name).unwrap().ns;
    assert_eq!(ns_of("text"), Some("http://www.w3.org/2000/svg"));
    assert_eq!(ns_of("textArea"), Some("http://www.w3.org/2000/svg"));
    assert_eq!(ns_of("textbox"), None);
    let json = super::elements::describe_impl();
    assert!(
        json.contains(r#""text":{"ns":"http://www.w3.org/2000/svg""#),
        "{}",
        &json[..200]
    );
    assert!(json.contains(r#""p":{"doc""#));
}

#[test]
fn a_list_item_takes_the_whole_type_block_like_a_paragraph() {
    // §6.14: an <x:li> sets font-*, line-height, the spacing and fill over its
    // list — standalone and inside a textbox alike — where it used to take a
    // size, a case and (in a box) a colour alone
    let src = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10"><x:li>plain</x:li><x:li font-weight="bold" font-style="italic" letter-spacing="2" fill="#f00">styled</x:li></x:list><x:textbox x="0" y="0" width="200" height="200" font-size="10"><x:list><x:li font-family="Mono" word-spacing="3">boxed</x:li></x:list></x:textbox></svg>"##
    );
    let out = compile_test(&src);
    let texts: Vec<&str> = out.split("<text ").skip(1).collect();
    let styled = texts.iter().find(|t| t.contains(">styled<")).unwrap();
    assert!(
        styled.contains(r#"font-weight="bold""#)
            && styled.contains(r#"font-style="italic""#)
            && styled.contains(r#" letter-spacing="2""#)
            && styled.contains(r##"fill="#f00""##),
        "{styled}"
    );
    let plain = texts.iter().find(|t| t.contains(">plain<")).unwrap();
    assert!(
        plain.contains(r#"font-weight="normal""#) && !plain.contains("letter-spacing"),
        "{plain}"
    );
    let boxed = texts.iter().find(|t| t.contains(">boxed<")).unwrap();
    assert!(
        boxed.contains(r#"font-family="Mono""#) && boxed.contains(r#" word-spacing="3""#),
        "{boxed}"
    );
    let s = ParsedDocument::new(&src).unwrap();
    let r = s
        .resolved(
            nth_at(&src, "<x:li font-weight", 0),
            &["font-weight", "letter-spacing", "fill"],
        )
        .unwrap();
    assert!(r.iter().all(|v| v.source == Source::Own), "{r:?}");
}

#[test]
fn a_cell_resolves_its_alignment_through_its_row_and_column() {
    // what the compiler draws (cell › row › column › table, §6.15) is what the
    // query answers: a column's `align` reaches its cells over the table's,
    // a row's beats the column's, a cell's beats the row's; `valign` likewise
    let src = format!(
        r##"{XW}<x:table x="10" y="10" width="280" font-size="12" align="end" valign="bottom"><x:col/><x:col align="center" valign="middle" font-weight="bold"/><x:col align="center"/><x:tr><x:td>a</x:td><x:td>b</x:td><x:td>c</x:td></x:tr><x:tr align="start"><x:td>d</x:td><x:td>e</x:td><x:td align="justify" valign="top">f</x:td></x:tr></x:table></svg>"##
    );
    let s = ParsedDocument::new(&src).unwrap();
    let props = ["align", "valign", "font-weight"];
    let get = |n: usize| s.resolved(nth_at(&src, "<x:td", n), &props).unwrap();
    let table = nth_at(&src, "<x:table", 0);
    let col1 = nth_at(&src, "<x:col align=\"center\" valign", 0);
    let col2 = nth_at(&src, "<x:col align=\"center\"/>", 0);
    let row1 = nth_at(&src, "<x:tr align", 0);
    let a = get(0);
    assert_eq!(
        (a[0].value.as_deref(), a[0].source.clone()),
        (
            Some("end"),
            Source::Inherited {
                from: table,
                token: None
            }
        )
    );
    assert_eq!(
        (a[1].value.as_deref(), a[1].source.clone()),
        (
            Some("bottom"),
            Source::Inherited {
                from: table,
                token: None
            }
        )
    );
    let b = get(1);
    assert_eq!(
        (b[0].value.as_deref(), b[0].source.clone()),
        (
            Some("center"),
            Source::Inherited {
                from: col1,
                token: None
            }
        )
    );
    assert_eq!(
        (b[1].value.as_deref(), b[1].source.clone()),
        (
            Some("middle"),
            Source::Inherited {
                from: col1,
                token: None
            }
        )
    );
    assert_eq!(
        (b[2].value.as_deref(), b[2].source.clone()),
        (
            Some("bold"),
            Source::Inherited {
                from: col1,
                token: None
            }
        )
    );
    let e = get(4);
    assert_eq!(
        (e[0].value.as_deref(), e[0].source.clone()),
        (
            Some("start"),
            Source::Inherited {
                from: row1,
                token: None
            }
        )
    );
    assert_eq!(
        (e[1].value.as_deref(), e[1].source.clone()),
        (
            Some("middle"),
            Source::Inherited {
                from: col1,
                token: None
            }
        )
    );
    let f = get(5);
    assert_eq!(
        (f[0].value.as_deref(), f[0].source.clone()),
        (Some("justify"), Source::Own)
    );
    assert_eq!(
        (f[1].value.as_deref(), f[1].source.clone()),
        (Some("top"), Source::Own)
    );
    let _ = col2;
    // and the render agrees: column two centred, row two's first cells at start
    let out = compile_test(&src);
    let anchors: Vec<&str> = out.matches("text-anchor=\"").map(|_| "").collect();
    assert_eq!(anchors.len(), 6);
    let anchor_of = |text: &str| {
        let i = out.find(&format!(">{text}</tspan>")).unwrap();
        let head = &out[..i];
        let j = head.rfind("text-anchor=\"").unwrap();
        head[j + 13..].split('"').next().unwrap().to_string()
    };
    assert_eq!(anchor_of("a"), "end");
    assert_eq!(anchor_of("b"), "middle");
    assert_eq!(anchor_of("c"), "middle");
    assert_eq!(anchor_of("d"), "start");
    assert_eq!(anchor_of("e"), "start");
    assert_eq!(anchor_of("f"), "start"); // justify renders start-anchored
}

#[test]
fn boolean_nested_transforms_reach_render_and_operand_query() {
    use crate::kurbo::{BezPath, Shape};
    // Inner transforms compose in operand space. The outer transform must
    // stay on the result, rather than also being baked into its operands.
    let src = format!(
        r#"{XW}<x:boolean transform="translate(200 0)" op="intersect"><x:boolean transform="translate(30 10) scale(2)" op="union"><path transform="translate(5 0)" d="M0 0H10V10H0Z"/></x:boolean><path d="M45 15H55V25H45Z"/></x:boolean></svg>"#
    );
    let out = compile_test(&src);
    assert!(out.contains("<path"), "{out}");
    let bb = first_path(&out).bounding_box();
    assert_eq!(bb, crate::kurbo::Rect::new(45., 15., 55., 25.));
    assert!(out.contains(r#"transform="translate(200 0)""#), "{out}");
    let q = boolean_operands_impl(
        &src,
        src.find("<x:boolean").unwrap(),
        "balanced",
        &Mono,
        &NoShaper,
        &NoOutliner,
    )
    .unwrap();
    assert_eq!(
        BezPath::from_svg(&q.operands[0].paths[0])
            .unwrap()
            .bounding_box(),
        crate::kurbo::Rect::new(40., 10., 60., 30.)
    );
    assert_eq!(
        BezPath::from_svg(q.flat.as_ref().unwrap())
            .unwrap()
            .bounding_box(),
        bb
    );
    assert_eq!(
        first_path(&to_path_at(&src, "<x:boolean").unwrap()).bounding_box(),
        bb
    );
}

#[test]
fn boolean_group_and_stroke_transforms_are_baked_once() {
    use crate::kurbo::Shape;
    for rule in ["nonzero", "evenodd"] {
        let src = format!(
            r#"{XW}<x:boolean><path transform="translate(30 10) scale(2)" fill-rule="{rule}" stroke="black" stroke-width="2" d="M0 0H10V10H0Z"/></x:boolean></svg>"#
        );
        let bb = first_path(&compile_test(&src)).bounding_box();
        assert_eq!(bb, crate::kurbo::Rect::new(28., 8., 52., 32.));
    }
    let src = format!(
        r#"{XW}<x:boolean><g transform="translate(30 10)"><g transform="scale(2)"><path transform="rotate(90)" d="M0 0H10V5H0Z"/></g></g></x:boolean></svg>"#
    );
    assert_eq!(
        first_path(&compile_test(&src)).bounding_box(),
        crate::kurbo::Rect::new(20., 10., 30., 30.)
    );
}

// Two 20×20 squares overlap by 10×20 only AFTER the nested transforms.
// Probe all three regions: bounds alone would miss a broken subtraction/XOR.
fn check_nested_boolean_operation(op: &str, area: f64, regions: [bool; 3]) {
    use crate::kurbo::{BezPath, Point, Shape};
    for transform in [
        "translate(30 10) scale(2)",
        "translate(60 0) rotate(90) scale(2)",
    ] {
        let src = format!(
            r#"{XW}<x:boolean op="{op}"><x:boolean transform="{transform}" op="union"><path transform="translate(5 0)" d="M0 0H10V10H0Z"/></x:boolean><path transform="translate(50 10)" d="M0 0H20V20H0Z"/></x:boolean></svg>"#
        );
        let query = boolean_operands_impl(
            &src,
            src.find("<x:boolean").unwrap(),
            "balanced",
            &Mono,
            &NoShaper,
            &NoOutliner,
        )
        .unwrap();
        let paths = [
            first_path(&compile_test(&src)),
            first_path(&to_path_at(&src, "<x:boolean").unwrap()),
            BezPath::from_svg(query.flat.as_ref().unwrap()).unwrap(),
        ];
        for path in paths {
            assert!(
                (path.area().abs() - area).abs() < 0.01,
                "{op}: {}",
                path.area()
            );
            for (x, inside) in [45., 55., 65.].into_iter().zip(regions) {
                assert_eq!(
                    path.winding(Point::new(x, 20.)) != 0,
                    inside,
                    "{op} at x={x}"
                );
            }
            assert_eq!(
                path.winding(Point::new(5., 5.)),
                0,
                "{op}: untransformed geometry leaked"
            );
        }
    }
}

#[test]
fn boolean_nested_intersect_respects_operand_rotation() {
    check_nested_boolean_operation("intersect", 200., [false, true, false]);
}

#[test]
fn boolean_nested_union_respects_operand_transforms() {
    check_nested_boolean_operation("union", 600., [true, true, true]);
}

#[test]
fn boolean_nested_subtract_respects_operand_transforms_and_order() {
    check_nested_boolean_operation("subtract", 200., [true, false, false]);
}

#[test]
fn boolean_nested_exclude_respects_operand_transforms() {
    check_nested_boolean_operation("exclude", 400., [true, false, true]);
}

#[test]
fn boolean_nested_subtract_hole_survives_reflection_and_union() {
    use crate::kurbo::{Point, Shape};
    // The inner subtraction leaves a hole; a reflection flips BOTH contour
    // windings. The outer union must preserve that hole in its new location.
    let src = format!(
        r#"{XW}<x:boolean op="union"><x:boolean op="subtract" transform="translate(60 10) scale(-2 2)"><path d="M0 0H10V10H0Z"/><path transform="translate(2.5 2.5)" d="M0 0H5V5H0Z"/></x:boolean><path transform="translate(80 10)" d="M0 0H10V10H0Z"/></x:boolean></svg>"#
    );
    let path = first_path(&compile_test(&src));
    assert!((path.area().abs() - 400.).abs() < 0.01);
    for (x, y, inside) in [
        (42., 20., true),
        (50., 20., false),
        (58., 20., true),
        (85., 15., true),
        (70., 20., false),
    ] {
        assert_eq!(path.winding(Point::new(x, y)) != 0, inside, "at ({x}, {y})");
    }
}

#[test]
fn theme_font_tokens_style_paragraphs_list_items_and_cells() {
    use super::query::Source;
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org">
<x:theme><x:font name="heading" font-family="serif" font-size="30" font-weight="700" line-height="1.4"/><x:font name="body" font-size="16"/></x:theme>
<x:textbox width="400" height="300" font-size="10"><x:p id="p" x:font="heading">Alpha</x:p><x:p id="own" x:font="heading" font-size="12">Beta</x:p><x:list x:font="heading"><x:li id="nested">Nested</x:li></x:list></x:textbox>
<x:list width="300" y="310" font-size="10"><x:li id="li" x:font="heading">Gamma</x:li></x:list>
<x:table y="400" width="300"><x:col x:font="heading"/><x:col/><x:tr><x:td id="col">Delta</x:td><x:td id="cell" x:font="heading">Epsilon</x:td></x:tr><x:tr x:font="heading"><x:td id="row">Zeta</x:td><x:td id="override" x:font="body">Eta</x:td></x:tr></x:table></svg>"#;
    let parsed = ParsedDocument::new(svg).unwrap();
    let source = roxmltree::Document::parse(svg).unwrap();
    for (id, size) in [
        ("p", "30"),
        ("own", "12"),
        ("nested", "30"),
        ("li", "30"),
        ("col", "30"),
        ("cell", "30"),
        ("row", "30"),
        ("override", "16"),
    ] {
        let node = source
            .descendants()
            .find(|n| n.attribute("id") == Some(id))
            .unwrap();
        let resolved = parsed.resolved(node.range().start, &["font-size"]).unwrap();
        assert_eq!(resolved[0].value.as_deref(), Some(size), "{id}");
        if id == "p" {
            assert_eq!(resolved[0].source, Source::Token("heading".into()));
        }
        if id == "own" {
            assert_eq!(resolved[0].source, Source::Own);
        }
    }
    let layout = layout_at(svg, at(svg, "<x:textbox")).unwrap();
    assert_eq!(layout.paras[0].style.size, 30.0);
    assert_eq!(layout.paras[0].style.family, "serif");
    assert_eq!(layout.paras[0].style.line_height, 1.4);
    assert_eq!(layout.paras[1].style.size, 12.0);
    assert_eq!(layout.paras[2].style.size, 30.0);
    let output = compile_test(svg);
    let rendered = roxmltree::Document::parse(&output).unwrap();
    for (word, size) in [
        ("Alpha", "30"),
        ("Beta", "12"),
        ("Gamma", "30"),
        ("Delta", "30"),
        ("Epsilon", "30"),
        ("Zeta", "30"),
        ("Eta", "16"),
    ] {
        let text = rendered
            .descendants()
            .find(|n| n.is_text() && n.text() == Some(word))
            .unwrap();
        let actual = text.ancestors().find_map(|n| n.attribute("font-size"));
        assert_eq!(actual, Some(size), "{word}");
    }
}

#[test]
fn drop_shadows_on_generated_objects_cover_the_complete_result() {
    let cases = [
        r##"<x:pie SHADOW cx="100" cy="100" r="50"><x:slice value="30"/><x:slice value="70"/></x:pie>"##,
        r##"<x:plot SHADOW x="20" y="20" width="200" height="100"><x:bars><x:bar value="5"/><x:bar value="10"/></x:bars></x:plot>"##,
        r##"<x:boolean SHADOW op="union"><rect width="100" height="80"/><circle cx="100" cy="40" r="40"/></x:boolean>"##,
        r##"<rect id="source" width="100" height="80"/><x:offset SHADOW in="#source" distance="5"/>"##,
        r##"<x:warp SHADOW field="arch" bend="20"><rect width="100" height="80"/></x:warp>"##,
        r##"<x:list SHADOW width="200"><x:li>First</x:li><x:li>Second</x:li></x:list>"##,
        r##"<x:table SHADOW width="200"><x:tr><x:td>One</x:td><x:td>Two</x:td></x:tr></x:table>"##,
        r##"<x:mesh SHADOW width="100" height="80" cols="1" rows="1" fill="#f00 #0f0 #00f #fff"/>"##,
        r##"<rect id="a" width="20" height="20"/><rect id="b" x="150" width="20" height="20"/><x:connector SHADOW from="#a" to="#b" arrow="both"/>"##,
        r##"<defs><meshgradient id="mesh"><meshrow><meshpatch><stop path="l 100 0" stop-color="#f00"/><stop path="l 0 80" stop-color="#0f0"/><stop path="l -100 0" stop-color="#00f"/><stop path="l 0 -80" stop-color="#fff"/></meshpatch></meshrow></meshgradient></defs><rect SHADOW width="100" height="80" fill="url(#mesh)" stroke="#123" stroke-width="3"/>"##,
    ];
    for body in cases {
        let source = format!(
            "{XW}{}</svg>",
            body.replace(
                "SHADOW",
                r##"filter="drop-shadow(3 4 5 #1238)" transform="rotate(15)""##
            )
        );
        let out = compile_test(&source);
        assert_eq!(out.matches("<feDropShadow").count(), 1, "{source}: {out}");
        assert_eq!(
            out.matches("filter=\"url(#x-flt-").count(),
            1,
            "{source}: {out}"
        );
        assert_eq!(
            out.matches("transform=\"rotate(15)\"").count(),
            1,
            "{source}: {out}"
        );
        assert!(!out.contains("filter=\"drop-shadow"), "{out}");
        let doc = roxmltree::Document::parse(&out).unwrap();
        let target = doc
            .descendants()
            .find(|n| {
                n.attribute("filter")
                    .is_some_and(|f| f.starts_with("url(#x-flt-"))
            })
            .unwrap();
        assert_eq!(target.attribute("transform"), Some("rotate(15)"), "{out}");
        if body.contains("meshgradient") {
            assert!(
                target
                    .descendants()
                    .any(|n| n.attribute("clip-path").is_some()),
                "mesh clipping stays inside the shadow: {out}"
            );
            assert!(
                target
                    .descendants()
                    .any(|n| n.attribute("stroke") == Some("#123")),
                "stroke shares the shadow: {out}"
            );
            assert!(
                target.attribute("clip-path").is_none(),
                "shadow must not be clipped to the fill: {out}"
            );
        }
        if body.contains("connector") {
            assert_eq!(
                target
                    .descendants()
                    .filter(|n| n.has_tag_name((SVG_NS, "path")))
                    .count(),
                3,
                "path and both arrowheads share the effect: {out}"
            );
        }
        let custom = compile_test(&format!(
            "{XW}{}</svg>",
            body.replace("SHADOW", r##"filter="url(#custom)""##)
        ));
        assert_eq!(
            custom.matches("filter=\"url(#custom)\"").count(),
            1,
            "custom filters survive: {custom}"
        );
    }
}

#[test]
fn linked_drawings_resolve_their_own_theme_and_restore_the_parent_scope() {
    let leaf = format!(
        r##"{XW}<x:theme><x:color name="accent" value="#abcdef"/><x:color name="local" value="#123456"/><x:font name="label" font-family="LeafFont" font-size="23"/></x:theme><rect width="20" height="10" fill="var(local)" stroke="var(accent)"/><x:textbox x:font="label" width="100" height="40">Leaf</x:textbox></svg>"##
    );
    let middle = format!(
        r##"{XW}<x:theme><x:color name="accent" value="#112233"/></x:theme><use href="leaf"/><rect width="10" height="10" fill="var(accent)"/></svg>"##
    );
    let main = format!(
        r##"{XW}<x:theme><x:color name="accent" value="#ff0000"/><x:font name="label" font-family="ParentFont" font-size="12"/></x:theme><use href="middle"/><use href="leaf"/><rect width="10" height="10" fill="var(accent)"/></svg>"##
    );
    let out = compile_linked(&main, &[("middle", &middle), ("leaf", &leaf)]);
    for value in ["#abcdef", "#123456", "#112233", "#ff0000", "LeafFont"] {
        assert!(out.contains(value), "{value}: {out}");
    }
    assert!(!out.contains("var("), "tokens unresolved: {out}");
    assert_eq!(out.matches("font-size=\"23\"").count(), 2, "{out}");
    assert!(!out.contains("ParentFont"), "parent font leaked: {out}");
}

#[test]
fn linked_drawing_retains_root_paint_and_plain_text_font() {
    let file = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50" fill="#123456" font-family="RootFont"><rect width="10" height="10"/><text y="30">Root style</text></svg>"##;
    let main = format!(r##"{XW}<use href="drawing" width="200" height="100"/></svg>"##);
    let out = compile_linked(&main, &[("drawing", file)]);
    assert!(
        out.contains("fill=\"#123456\" font-family=\"RootFont\""),
        "{out}"
    );
}

#[test]
fn wrapped_text_outlines_like_point_text() {
    // §6.12: `x:outline` on <text inline-size> traces the wrapped lines — it
    // used to stay live, where point text and <textArea> outlined
    let src = format!(
        r##"{XW}<text x="10" y="20" inline-size="60" font-size="10" fill="#123" x:outline="true">one two three four five six</text></svg>"##
    );
    let out = compile_outlined(&src);
    assert!(!out.contains("<text"), "{out}");
    assert!(out.contains(r##"<g fill="#123""##), "{out}");
    assert!(out.matches("<path").count() >= 2, "one per line: {out}");
    // a run's paint and highlight survive
    let runs = format!(
        r##"{XW}<text x="10" y="20" inline-size="200" font-size="10" x:outline="true">plain <tspan fill="#c00" x:highlight="#ff0">red</tspan></text></svg>"##
    );
    let out = compile_outlined(&runs);
    assert!(
        !out.contains("<text") && out.contains(r##"fill="#c00""##),
        "{out}"
    );
    assert!(out.contains(r##"<rect"##) && out.contains("#ff0"), "{out}");
    // no font to trace: live, as every outline request degrades
    let live = compile_test(&src);
    assert!(live.contains("<text") && !live.contains("<path"), "{live}");
    // and without the request nothing changes
    let plain = compile_outlined(&src.replace(r#" x:outline="true""#, ""));
    assert!(
        plain.contains("<text") && !plain.contains("<path"),
        "{plain}"
    );
}

#[test]
fn a_list_outlines_its_items_and_generated_markers() {
    // §6.12 on <x:list>: every item's lines and every number traced; drawn
    // bullets stay the shapes they are
    let src = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10" list="number" outline="true"><x:li>one</x:li><x:li fill="#c00">two</x:li></x:list><x:list x="0" y="100" width="200" font-size="10" outline="true"><x:li>dot</x:li></x:list></svg>"##
    );
    let out = compile_outlined(&src);
    assert!(!out.contains("<text"), "{out}");
    // two numbers and two items in the first list, one item in the second
    assert_eq!(out.matches("<path").count(), 5, "{out}");
    assert!(out.contains(r##"fill="#c00""##), "{out}");
    assert!(out.contains("<circle"), "the bullet is still drawn: {out}");
    // all-or-nothing: with no font the whole list is live
    let live = compile_test(&src);
    assert!(live.contains("<text") && !live.contains("<path"), "{live}");
    assert_eq!(
        live,
        compile_test(&src.replace(r#" outline="true""#, "")),
        "an untraceable outline request changes nothing"
    );
}

#[test]
fn a_table_outlines_its_cells_and_keeps_its_grid() {
    let src = format!(
        r##"{XW}<x:table x="0" y="0" width="200" font-size="10" outline="true"><x:tr><x:th>H</x:th><x:th>I</x:th></x:tr><x:tr><x:td>a</x:td><x:td></x:td></x:tr></x:table></svg>"##
    );
    let out = compile_outlined(&src);
    assert!(!out.contains("<text"), "{out}");
    assert_eq!(
        out.matches("<path").count(),
        3,
        "the three cells with text: {out}"
    );
    assert_eq!(
        out.matches("<rect").count(),
        4,
        "every cell keeps its box: {out}"
    );
    let live = compile_test(&src);
    assert_eq!(live, compile_test(&src.replace(r#" outline="true""#, "")));
}

#[test]
fn to_path_converts_a_list_and_a_table() {
    // the text-bearing blocks convert like a text box: glyphs as paths, the
    // node's own attributes kept, the layout parameters spent
    let src = format!(
        r##"{XW}<x:list id="l" x="0" y="0" width="200" font-size="10" list="number" item-spacing="4" transform="rotate(3)" opacity="0.5"><x:li>one</x:li></x:list><x:table id="t" x="0" y="50" width="200" cols="1 1" cell-padding="4" stroke="#999" stroke-width="2" x:label="T"><x:tr><x:td>a</x:td><x:td>b</x:td></x:tr></x:table><x:textbox x="0" y="0" width="100" height="100"><x:list><x:li>in</x:li></x:list></x:textbox></svg>"##
    );
    let list = to_path_at(&src, "<x:list id").unwrap();
    assert!(list.starts_with("<g") && !list.contains("<text"), "{list}");
    for kept in [r#"id="l""#, r#"transform="rotate(3)""#, r#"opacity="0.5""#] {
        assert!(list.contains(kept), "{kept}: {list}");
    }
    assert!(
        !list.contains("item-spacing") && !list.contains("list="),
        "{list}"
    );
    assert_eq!(list.matches("<path").count(), 2, "marker and item: {list}");

    let table = to_path_at(&src, "<x:table").unwrap();
    assert!(
        table.starts_with("<g") && !table.contains("<text"),
        "{table}"
    );
    assert!(
        table.contains(r#"id="t""#) && table.contains(r#"x:label="T""#),
        "{table}"
    );
    // the grid's stroke is on the cell boxes, not carried onto the group
    let open = &table[..table.find('>').unwrap()];
    assert!(!open.contains("stroke") && !open.contains("cols"), "{open}");
    assert!(table.contains(r##"stroke="#999""##), "{table}");

    // a boxed list is its box's to convert
    let nested = to_path_at(&src, "<x:list><x:li>in").unwrap_err();
    assert!(
        format!("{nested:?}").contains("converts with its box"),
        "{nested:?}"
    );
    // wrapped text converts too
    let wrapped = format!(
        r##"{XW}<text id="w" x="10" y="20" inline-size="60" font-size="10" class="k">one two three four</text></svg>"##
    );
    let out = to_path_at(&wrapped, "<text").unwrap();
    assert!(
        out.contains(r#"id="w""#) && out.contains(r#"class="k""#),
        "{out}"
    );
    assert!(
        !out.contains("<text") && !out.contains("inline-size"),
        "{out}"
    );
}

#[test]
fn a_spacing_with_a_unit_or_a_var_is_invalid_and_falls_through() {
    // §6.8: the spacings are a bare number or `normal`. A unit or a var() is
    // not read as its leading number — the cascade falls through (§4.1), on
    // every front-end alike, and the query says so
    use super::query::Source;
    let src = format!(
        r##"{XW}<x:theme><x:color name="g" value="3"/><x:font name="t" letter-spacing="2"/></x:theme><x:textbox x="0" y="0" width="200" height="40" font-size="10" letter-spacing="0.2em">em</x:textbox><x:textbox x="0" y="50" width="200" height="40" font-size="10" letter-spacing="var(g)">var</x:textbox><x:textbox x="0" y="100" width="200" height="40" font-size="10" x:font="t" letter-spacing="1pt">token</x:textbox><x:textbox x="0" y="150" width="200" height="80" font-size="10" word-spacing="4"><x:p word-spacing="50%">para</x:p></x:textbox><text x="0" y="300" inline-size="100" font-size="10" letter-spacing="0.5em">wrapped</text></svg>"##
    );
    let out = compile_test(&src);
    let text_with = |needle: &str| {
        out.split("<text ")
            .find(|t| t.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {out}"))
            .to_string()
    };
    assert!(!text_with(">em<").contains("letter-spacing"), "{out}");
    assert!(!text_with(">var<").contains("letter-spacing"), "{out}");
    // the invalid attribute falls through to the token's value
    assert!(
        text_with(">token<").contains(r#"letter-spacing="2""#),
        "{out}"
    );
    // and a paragraph's to its box's
    assert!(text_with(">para<").contains(r#"word-spacing="4""#), "{out}");
    // wrapped text was measured untracked, so the refused value is not written
    assert!(!text_with(">wrapped<").contains("letter-spacing"), "{out}");
    // px is the user unit; a valid value passes through as authored
    let px = compile_test(&format!(
        r##"{XW}<x:textbox x="0" y="0" width="200" height="40" font-size="10" letter-spacing="1.5px">px</x:textbox><text x="0" y="60" inline-size="100" font-size="10" letter-spacing="2">kept</text></svg>"##
    ));
    assert!(px.contains(r#"letter-spacing="1.5""#), "{px}");
    assert!(px.contains(r#"font-size="10" letter-spacing="2">"#), "{px}");
    let parsed = ParsedDocument::new(&src).unwrap();
    let r = parsed
        .resolved(at(&src, r#"<x:textbox x="0" y="100""#), &["letter-spacing"])
        .unwrap();
    assert_eq!(r[0].value.as_deref(), Some("2"));
    assert_eq!(r[0].source, Source::Token("t".into()));
    let r = parsed
        .resolved(at(&src, r#"<x:textbox x="0" y="0""#), &["letter-spacing"])
        .unwrap();
    assert_eq!(
        (r[0].value.as_deref(), &r[0].source),
        (Some("normal"), &Source::Initial)
    );
}

#[test]
fn an_empty_item_and_an_empty_cell_lay_one_blank_line() {
    // §6.4: every empty flow lays one blank line — the row an editor's caret
    // sits on. A standalone list's empty item and a table's empty cell used
    // to emit nothing, where an empty box, paragraph and boxed item did
    let src = format!(
        r##"{XW}<x:list x="10" y="0" width="200" font-size="10"><x:li></x:li><x:li>b<tbreak/><tbreak/>c</x:li></x:list><x:table x="0" y="100" width="200" font-size="10"><x:tr><x:td></x:td><x:td>b</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_mapped(&src);
    let li = src.find("<x:li></x:li>").unwrap();
    let item = out
        .split("<text ")
        .find(|t| t.contains(&format!("data-xsvg-pos=\"{li}-")))
        .unwrap_or_else(|| panic!("the empty item has a keyed <text>: {out}"));
    assert!(item.contains("></tspan></text>"), "{item}");
    assert!(
        item.contains(r#"<tspan x="25""#),
        "at the text column: {item}"
    );
    // a forced blank line inside an item with text still emits nothing
    assert_eq!(out.matches("></tspan>").count(), 2, "item and cell: {out}");
    let td = src.find("<x:td></x:td>").unwrap();
    let cell = out
        .split(&format!("data-xsvg-pos=\"{td}-"))
        .nth(1)
        .and_then(|c| c.split("</g>").next())
        .unwrap();
    assert!(
        cell.contains("<rect") && cell.contains("></tspan></text>"),
        "{cell}"
    );
    // the blank line changes no geometry: the row is as tall as it was
    assert!(cell.contains(r#"height="25""#), "{cell}");
    // outlined, a blank line is nothing
    let outlined = compile_outlined(&src.replace("<x:list ", r#"<x:list outline="true" "#));
    assert!(
        !outlined.split("<g").nth(1).unwrap().contains("<tspan"),
        "{outlined}"
    );
}

#[test]
fn direction_reaches_standalone_lists_and_table_cells() {
    // §6.4 direction on every block that lays text: an item's own, else its
    // list's (standalone or boxed), else its box's; a cell's own, else its
    // row's, column's, table's — as the queries report
    use super::query::Source;
    let src = format!(
        r##"{XW}<x:list x="0" y="0" width="200" font-size="10" direction="rtl"><x:li id="a">one</x:li><x:li id="b" direction="ltr">two</x:li></x:list><x:textbox x="0" y="100" width="200" height="100" font-size="10"><x:list direction="rtl"><x:li id="c">three</x:li></x:list></x:textbox><x:table x="0" y="200" width="200" font-size="10"><x:col direction="rtl"/><x:col/><x:tr><x:td id="d">four</x:td><x:td id="e">five</x:td></x:tr></x:table><x:table x="0" y="300" width="100" font-size="10" direction="rtl"><x:tr><x:td id="f">six</x:td></x:tr></x:table></svg>"##
    );
    let out = compile_test(&src);
    let text_with = |needle: &str| {
        out.split("<text ")
            .find(|t| t.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {out}"))
            .to_string()
    };
    for (word, rtl) in [
        ("one", true),
        ("two", false),
        ("three", true),
        ("four", true),
        ("five", false),
        ("six", true),
    ] {
        let t = text_with(&format!(">{word}<"));
        assert_eq!(t.contains(r#"direction="rtl""#), rtl, "{word}: {t}");
    }
    // an rtl item starts at the right edge of its column (x=0, width=200)
    assert!(text_with(">one<").contains(r#"<tspan x="200""#), "{out}");
    assert!(text_with(">two<").contains(r#"<tspan x="15""#), "{out}");
    // an rtl cell starts at its right edge less the padding
    assert!(text_with(">four<").contains(r#"<tspan x="92""#), "{out}");
    let parsed = ParsedDocument::new(&src).unwrap();
    let dir = |id: &str| {
        let tag = if matches!(id, "a" | "b" | "c") {
            "li"
        } else {
            "td"
        };
        let r = parsed
            .resolved(at(&src, &format!(r#"<x:{tag} id="{id}""#)), &["direction"])
            .unwrap();
        (r[0].value.clone().unwrap(), r[0].source.clone())
    };
    assert!(matches!(dir("a"), (v, Source::Inherited { .. }) if v == "rtl"));
    assert_eq!(dir("b"), ("ltr".to_string(), Source::Own));
    assert!(matches!(dir("c"), (v, Source::Inherited { .. }) if v == "rtl"));
    assert!(matches!(dir("d"), (v, Source::Inherited { .. }) if v == "rtl"));
    assert_eq!(dir("e"), ("ltr".to_string(), Source::Initial));
    assert!(matches!(dir("f"), (v, Source::Inherited { .. }) if v == "rtl"));
}

#[test]
fn a_corner_style_is_set_per_anchor_like_a_radius() {
    // §5.4: `x:corner-style` takes the shape `x:corners` does — one style for
    // every corner, `anchor:style` pairs for some, a pair winning at its anchor
    use super::CornerStyle;
    let src = format!(
        r##"{XW}<rect x="20" y="20" width="200" height="120" x:corners="24" x:corner-style="chamfer 0:round 2:inverted 3:bogus"/><rect x="0" y="0" width="50" height="50" x:corners="5" x:corner-style="1:chamfer"/></svg>"##
    );
    let s = ParsedDocument::new(&src).unwrap();
    let c = s.corners(nth_at(&src, "<rect", 0)).unwrap();
    let styles: Vec<CornerStyle> = c.iter().map(|c| c.style).collect();
    assert_eq!(
        styles,
        [
            CornerStyle::Round,
            CornerStyle::Chamfer,
            CornerStyle::Inverted,
            // an unknown style in a pair is dropped: the element's applies
            CornerStyle::Chamfer
        ]
    );
    // a chamfer has no centre; the others do
    assert!(c[0].center.is_some() && c[1].center.is_none() && c[2].center.is_some());
    // with pairs alone the rest stay round
    let c = s.corners(nth_at(&src, "<rect", 1)).unwrap();
    assert_eq!(c[0].style, CornerStyle::Round);
    assert_eq!(c[1].style, CornerStyle::Chamfer);
    // drawn: two arcs (anchors 0 and 2, sweeping opposite ways) and two chords
    let out = compile_test(&src);
    let d = out
        .split(" d=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    assert_eq!(d.matches('A').count(), 2, "{d}");
    assert!(
        d.contains("A24 24 0 0 1") && d.contains("A24 24 0 0 0"),
        "{d}"
    );
    // the chamfer at anchor 1 is the straight chord between its trim points
    assert!(d.contains("L196 20 L220 44"), "{d}");
}

#[test]
fn to_path_converts_the_elements_that_compile_to_paths() {
    // an offset, a warp and a connector already lower to paths: `to_path`
    // returns that lowering in the source's terms — parameters spent, the
    // editor's attributes and paint tokens kept
    let src = format!(
        r##"{XW}<x:theme><x:color name="ink" value="#123456"/></x:theme><rect id="a" x="0" y="0" width="40" height="40"/><rect id="b" x="120" y="0" width="40" height="40"/><x:offset id="o" in="#a" distance="5" join="round" fill="var(ink)" x:label="Grown" opacity="0.5"/><x:warp id="w" field="arc" bend="30" fill="var(ink)" x:label="Bent"><rect x="0" y="60" width="100" height="20"/><x:textbox x="0" y="60" width="100" height="20" font-size="10" outline="true">hi</x:textbox></x:warp><x:connector id="c" from="#a" to="#b" stroke="var(ink)" stroke-width="2" x:label="Link" class="k"/><x:connector id="n" from="#a" to="#b" arrow="none" stroke="#000" x:label="Bare"/><x:offset in="#gone" distance="5"/></svg>"##
    );
    let offset = to_path_at(&src, "<x:offset id").unwrap();
    assert!(
        offset.starts_with("<path") && offset.ends_with("/>"),
        "{offset}"
    );
    for kept in [
        r#"id="o""#,
        r#"fill="var(ink)""#,
        r#"x:label="Grown""#,
        r#"opacity="0.5""#,
    ] {
        assert!(offset.contains(kept), "{kept}: {offset}");
    }
    for spent in ["in=", "distance", "join"] {
        assert!(!offset.contains(spent), "{spent}: {offset}");
    }

    let warp = to_path_at(&src, "<x:warp").unwrap();
    assert!(warp.starts_with("<g") && warp.ends_with("</g>"), "{warp}");
    assert!(warp.contains(r#"x:label="Bent""#) && warp.contains(r#"fill="var(ink)""#));
    assert!(!warp.contains("field") && !warp.contains("bend"), "{warp}");
    assert!(!warp.contains("<text") && !warp.contains("<rect"), "{warp}");
    assert!(
        warp.matches("<path").count() >= 2,
        "shape and glyphs: {warp}"
    );

    // arrowheads make it a group: the stroke on the route, not on the heads
    let conn = to_path_at(&src, "<x:connector id=\"c\"").unwrap();
    assert!(conn.starts_with("<g") && conn.ends_with("</g>"), "{conn}");
    let open = &conn[..conn.find('>').unwrap()];
    assert!(
        open.contains(r#"id="c""#) && open.contains(r#"x:label="Link""#),
        "{open}"
    );
    assert!(
        open.contains(r#"class="k""#) && !open.contains("stroke"),
        "{open}"
    );
    assert!(
        conn.contains(r#"<path stroke="var(ink)" stroke-width="2" fill="none""#),
        "{conn}"
    );
    assert!(
        conn.contains(r#"<path fill="var(ink)""#),
        "the head: {conn}"
    );
    assert!(!conn.contains("from=") && !conn.contains("to="), "{conn}");
    // without heads it is the one path
    let bare = to_path_at(&src, "<x:connector id=\"n\"").unwrap();
    assert!(
        bare.starts_with("<path") && bare.matches('<').count() == 1,
        "{bare}"
    );
    assert!(
        bare.contains(r#"x:label="Bare""#) && !bare.contains("arrow"),
        "{bare}"
    );

    // what the compiler could only mark is refused with the marker's reason
    let err = to_path_at(&src, "<x:offset in=\"#gone\"").unwrap_err();
    assert!(format!("{err:?}").contains("target not found"), "{err:?}");
    // each result compiles to what the element compiled to
    for (needle, markup) in [("<x:offset id", &offset), ("<x:warp", &warp)] {
        let at = src.find(needle).unwrap();
        let end = at
            + src[at..]
                .find(if needle == "<x:warp" {
                    "</x:warp>"
                } else {
                    "/>"
                })
                .unwrap()
            + if needle == "<x:warp" { 9 } else { 2 };
        let replaced = format!("{}{markup}{}", &src[..at], &src[end..]);
        let strip = |o: &str| {
            o.replace(r#" x:label="Grown""#, "")
                .replace(r#" x:label="Bent""#, "")
        };
        assert_eq!(
            strip(&compile_outlined(&replaced)),
            compile_outlined(&src),
            "{needle} round-trips"
        );
    }
}

#[test]
fn to_path_converts_a_group_child_by_child() {
    // a <g> is its source with every convertible child converted in place:
    // its own tag, its other children and the text between them as authored
    let src = format!(
        r##"{XW}<g id="grp" transform="translate(5 5)" x:label="Group">
  <rect x="0" y="0" width="10" height="10" rx="2"/>
  <image href="a.png" width="4" height="4"/>
  <g class="inner"><x:textbox x="0" y="0" width="100" height="20" font-size="10">hi</x:textbox></g>
</g><g id="empty"><image href="a.png" width="4" height="4"/></g><g id="bad"><x:offset in="#gone" distance="2"/></g></svg>"##
    );
    let out = to_path_at(&src, "<g id=\"grp\"").unwrap();
    assert!(
        out.starts_with(r#"<g id="grp" transform="translate(5 5)" x:label="Group">"#),
        "{out}"
    );
    assert!(
        out.contains("\n  <image href=\"a.png\" width=\"4\" height=\"4\"/>\n"),
        "{out}"
    );
    assert!(
        out.contains(r##"<g class="inner"><g fill="#000""##),
        "{out}"
    );
    assert!(
        !out.contains("<rect") && !out.contains("<x:textbox"),
        "{out}"
    );
    assert!(
        out.contains(r#"x:corners="2""#),
        "the rect's corner stays live: {out}"
    );
    assert!(out.ends_with("</g>\n</g>"), "{out}");
    // to_spline expands every converted child
    let spline = to_spline_at(&src, "<g id=\"grp\"").unwrap();
    assert!(
        !spline.contains("x:corners") && spline.contains(" C"),
        "{spline}"
    );
    // nothing convertible, and a child that cannot convert
    let err = to_path_at(&src, "<g id=\"empty\"").unwrap_err();
    assert!(format!("{err:?}").contains("nothing to convert"), "{err:?}");
    let err = to_path_at(&src, "<g id=\"bad\"").unwrap_err();
    assert!(format!("{err:?}").contains("target not found"), "{err:?}");
    // and the result is source: it parses and compiles in the group's place
    // (`npm run check:outline` pins that it draws the same pixels)
    let at = src.find("<g id=\"grp\"").unwrap();
    let end = src.find("<g id=\"empty\"").unwrap();
    let replaced = compile_outlined(&format!("{}{out}{}", &src[..at], &src[end..]));
    assert!(
        replaced.contains("<image") && !replaced.contains("<text"),
        "{replaced}"
    );
}

#[test]
fn path_data_is_found_in_tags_only() {
    // a converted <g> passes authored children through, so `d="…"` can sit in
    // a text's words, a comment or another attribute's value: only a tag's
    // own `d` attribute is path data
    let markup = r#"<g title=' d="M0 0 A1 1 0 0 1 2 0"'><!-- d="M0 0 A1 1 0 0 1 2 0" --><text>say d="M0 0 A1 1 0 0 1 2 0"</text><path id="p" d="M0 0 A1 1 0 0 1 2 0"/><path
 d='M5 5 A1 1 0 0 1 7 5' data-d="A"/></g>"#;
    let ranges = super::spline::d_attr_ranges(markup);
    let found: Vec<&str> = ranges.iter().map(|&(a, b)| &markup[a..b]).collect();
    assert_eq!(found, ["M0 0 A1 1 0 0 1 2 0", "M5 5 A1 1 0 0 1 7 5"]);
    let spline = super::spline::markup_to_spline(markup);
    assert_eq!(
        spline.matches(" A1 1").count(),
        3,
        "the three non-paths: {spline}"
    );
    assert_eq!(
        spline.matches('C').count(),
        4,
        "two half turns, two cubics each: {spline}"
    );
    assert!(spline.contains(r#"data-d="A""#), "{spline}");

    // a boolean's curved result goes where its flattened path is, or nowhere
    let b = BooleanOperands {
        op: crate::BoolOp::Union,
        operands: Vec::new(),
        flat: Some("M0 0 L1 0 L1 1 Z".to_string()),
    };
    assert_eq!(
        b.with_curves(
            r#"<path x:label="d=" d="M0 0 L1 0 L1 1 Z" fill="red"/>"#,
            "M9 9 Z"
        )
        .as_deref(),
        Some(r#"<path x:label="d=" d="M9 9 Z" fill="red"/>"#)
    );
    assert_eq!(b.with_curves(r#"<path d="M2 2 Z"/>"#, "M9 9 Z"), None);
    assert_eq!(
        b.with_curves(r#"<text>M0 0 L1 0 L1 1 Z</text>"#, "M9 9 Z"),
        None
    );
}

#[test]
fn display_resolution_meshes_are_emitted_at_transformed_density() {
    let mesh = r##"<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let sizes = |svg: &str| {
        let bytes = b64_prefix(svg, 24);
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    };
    for (open, close, sx, sy) in [
        ("", "", 1.0, 1.0),
        ("<g transform=\"rotate(30)\"><g transform=\"matrix(0 3 -2 0 0 0)\">", "</g></g>", 3.0, 2.0),
        ("<svg width=\"400\" height=\"100\" viewBox=\"0 0 100 50\" preserveAspectRatio=\"none\">", "</svg>", 4.0, 2.0),
    ] {
        let source = format!("{XW}{open}{mesh}{close}</svg>");
        let compact = compile_test(&source);
        let options = Options { rasterize_at_display_resolution: Some(1.0), ..Default::default() };
        let display = compile_with(&source, &options, &platform).unwrap();
        let dense = sizes(&display);
        let doc = roxmltree::Document::parse(&display).unwrap();
        let image = doc.descendants().find(|n| n.has_tag_name("image")).unwrap();
        let w: f64 = image.attribute("width").unwrap().parse().unwrap();
        let h: f64 = image.attribute("height").unwrap().parse().unwrap();
        assert!((dense.0 as f64 - (w * sx).ceil()).abs() <= 1.0, "{dense:?}");
        assert!((dense.1 as f64 - (h * sy).ceil()).abs() <= 1.0, "{dense:?}");
        assert!(dense.0 > sizes(&compact).0);
        let old = roxmltree::Document::parse(&compact).unwrap();
        let original = old.descendants().find(|n| n.has_tag_name("image")).unwrap();
        for attr in original.attributes().filter(|a| a.name() != "href") { assert_eq!(image.attribute(attr.name()), Some(attr.value())); }
        let twice = compile_with(&source, &Options { rasterize_at_display_resolution: Some(2.0), ..options }, &platform).unwrap();
        assert!((sizes(&twice).0 as i64 - 2 * dense.0 as i64).abs() <= 1);
        let fragment = compile_with(&source, &Options { fragment: Some(source.find("<x:mesh").unwrap()), ..options }, &platform).unwrap();
        assert_eq!(sizes(&fragment), dense);
        assert_eq!(compile_with(&source, &Options::default(), &platform).unwrap(), compact);
    }
    for invalid in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        assert!(compile_with(
            &format!("{XW}{mesh}</svg>"),
            &Options {
                rasterize_at_display_resolution: Some(invalid),
                ..Default::default()
            },
            &platform
        )
        .is_err());
    }
}

#[test]
fn display_resolution_covers_curved_and_linked_meshes() {
    let sizes = |svg: &str| {
        let bytes = b64_prefix(svg, 24);
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    };
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let options = Options {
        rasterize_at_display_resolution: Some(1.0),
        ..Default::default()
    };
    let curved = format!(
        r##"{XW}{INKMESH}<rect width="200" height="80" transform="scale(2)" fill="url(#m)"/></svg>"##
    );
    let out = compile_with(&curved, &options, &platform).unwrap();
    let (w, h) = sizes(&out);
    assert!(w >= 400 && h >= 160, "{w}x{h}");
    let dep = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="https://xsvg.visioncortex.org" width="100" height="50" viewBox="0 0 100 50"><x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></svg>"##;
    let resolver = MapResolver(std::collections::HashMap::from([(
        "mesh.svg".into(),
        dep.into(),
    )]));
    let platform = platform.with_resolver(resolver);
    let source = format!(
        r##"{XW}<g transform="scale(2)"><use href="mesh.svg" width="300" height="200" preserveAspectRatio="none"/></g></svg>"##
    );
    let out = compile_with(&source, &options, &platform).unwrap();
    let (w, h) = sizes(&out);
    assert!(w >= 600 && h >= 400, "{w}x{h}");
}

/// Decode a PNG the compiler emitted: inflate IDAT and undo the scanline
/// filters, independently of the encoder. Returns (width, height, channels, pixels).
fn decode_png(png: &[u8]) -> (usize, usize, usize, Vec<u8>) {
    let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
    let (w, h) = (be(16), be(20));
    let bpp = if png[25] == 6 { 4 } else { 3 };
    let (mut at, mut idat) = (8, Vec::new());
    while at < png.len() {
        let len = be(at);
        if &png[at + 4..at + 8] == b"IDAT" {
            idat.extend_from_slice(&png[at + 8..at + 8 + len]);
        }
        at += 12 + len;
    }
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat).unwrap();
    let stride = w * bpp;
    assert_eq!(raw.len(), (stride + 1) * h);
    let mut px = vec![0u8; stride * h];
    for row in 0..h {
        let line = &raw[row * (stride + 1)..(row + 1) * (stride + 1)];
        for i in 0..stride {
            let a = if i >= bpp {
                px[row * stride + i - bpp] as i16
            } else {
                0
            };
            let b = if row > 0 {
                px[(row - 1) * stride + i] as i16
            } else {
                0
            };
            let c = if row > 0 && i >= bpp {
                px[(row - 1) * stride + i - bpp] as i16
            } else {
                0
            };
            let predicted = match line[0] {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let (pa, pb, pc) = ((b - c).abs(), (a - c).abs(), (a + b - 2 * c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                other => panic!("filter {other}"),
            };
            px[row * stride + i] = line[1 + i].wrapping_add(predicted as u8);
        }
    }
    (w, h, bpp, px)
}

fn first_png(svg: &str) -> Vec<u8> {
    b64_prefix(svg, usize::MAX)
}

fn display_options(density: f64) -> Options<'static> {
    Options {
        rasterize_at_display_resolution: Some(density),
        ..Default::default()
    }
}

const QUAD_MESH: &str = r##"<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;

#[test]
fn display_resolution_textures_are_compressed_and_match_the_compact_field() {
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let source = format!(r#"{XW}<g transform="scale(8 9)">{QUAD_MESH}</g></svg>"#);
    let compact = compile_test(&source);
    let dense = compile_with(&source, &display_options(1.0), &platform).unwrap();

    let (cw, ch, cc, cpx) = decode_png(&first_png(&compact));
    let (w, h, c, px) = decode_png(&first_png(&dense));
    assert_eq!(c, cc);
    assert!(w >= 800 && h >= 450, "{w}x{h}");
    // megapixel-scale raw data, and the whole document is a small fraction of it
    assert!(w * h * c > 1_000_000);
    assert!(
        dense.len() < w * h * c / 20,
        "{} bytes for {w}x{h}",
        dense.len()
    );

    // The dense texture is the compact one, bilinearly reconstructed: the
    // corners are the grid's corner texels and every pixel sits within
    // rounding of the reconstruction a renderer would have made.
    let texel = |x: usize, y: usize, k: usize| cpx[(y * cw + x) * cc + k] as f64;
    let mut worst = 0.0_f64;
    for y in (0..h).step_by(7) {
        for x in (0..w).step_by(11) {
            let u = ((x as f64 + 0.5) * cw as f64 / w as f64 - 0.5).clamp(0.0, (cw - 1) as f64);
            let v = ((y as f64 + 0.5) * ch as f64 / h as f64 - 0.5).clamp(0.0, (ch - 1) as f64);
            let (x0, y0) = (u.floor() as usize, v.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(cw - 1), (y0 + 1).min(ch - 1));
            let (fx, fy) = (u - x0 as f64, v - y0 as f64);
            for k in 0..3 {
                let expected = (texel(x0, y0, k) * (1.0 - fx) + texel(x1, y0, k) * fx) * (1.0 - fy)
                    + (texel(x0, y1, k) * (1.0 - fx) + texel(x1, y1, k) * fx) * fy;
                worst = worst.max((px[(y * w + x) * c + k] as f64 - expected).abs());
            }
        }
    }
    assert!(worst <= 0.51, "off the compact field by {worst}");
    for (dx, dy, sx, sy) in [
        (0, 0, 0, 0),
        (w - 1, 0, cw - 1, 0),
        (0, h - 1, 0, ch - 1),
        (w - 1, h - 1, cw - 1, ch - 1),
    ] {
        assert_eq!(
            px[(dy * w + dx) * c..][..3],
            cpx[(sy * cw + sx) * cc..][..3]
        );
    }
    // the compact path is untouched: still the stored encoding
    assert_eq!(first_png(&compact)[41..43], [0x78, 0x01]);
}

#[test]
fn display_resolution_follows_use_instances() {
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let width = |body: &str| {
        let source = format!("{XW}{body}</svg>");
        let out = compile_with(&source, &display_options(1.0), &platform).unwrap();
        let png = b64_prefix(&out, 24);
        let size = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as f64;
        (size(16), size(20))
    };
    let (w1, h1) = width(QUAD_MESH);
    let near = |got: (f64, f64), sx: f64, sy: f64| {
        assert!(
            (got.0 - w1 * sx).abs() <= sx + 1.0 && (got.1 - h1 * sy).abs() <= sy + 1.0,
            "{got:?} vs {w1}x{h1} * {sx},{sy}"
        );
    };
    // a definition drawn only through a scaled <use>
    near(
        width(&format!(
            r##"<defs><g id="m">{QUAD_MESH}</g></defs><use href="#m" transform="scale(8)"/>"##
        )),
        8.0,
        8.0,
    );
    // a <symbol> fitted to the size its <use> gives it
    near(
        width(&format!(
            r##"<symbol id="s" viewBox="0 0 100 50" preserveAspectRatio="none">{QUAD_MESH}</symbol><use href="#s" width="800" height="200"/>"##
        )),
        8.0,
        4.0,
    );
    // the densest of several placements decides, the definition's own included
    near(
        width(&format!(
            r##"<g id="m">{QUAD_MESH}</g><use href="#m" transform="scale(2)"/><g transform="scale(1 5)"><use href="#m" x="10"/></g>"##
        )),
        2.0,
        5.0,
    );
    // a <use> of a <use>
    near(
        width(&format!(
            r##"<defs><g id="m" transform="scale(2)">{QUAD_MESH}</g><use id="u" href="#m" transform="scale(3)"/></defs><use href="#u"/>"##
        )),
        6.0,
        6.0,
    );
    // an unused definition and a self-referencing chain both fall back to their own space
    near(
        width(&format!(r##"<defs><g id="m">{QUAD_MESH}</g></defs>"##)),
        1.0,
        1.0,
    );
    near(
        width(&format!(
            r##"<defs><g id="m">{QUAD_MESH}<use href="#m" transform="scale(2)"/></g></defs>"##
        )),
        1.0,
        1.0,
    );
}

/// An `n×n`-patch `<meshgradient id="m">` over `n·cell` units, in the
/// Inkscape dialect: curved edges, smoothly varying corner colours.
fn patch_grid_meshgradient(n: usize, cell: f64) -> String {
    let col = |i: usize, j: usize| {
        format!(
            "#{:02x}{:02x}{:02x}",
            40 + (i * 200 / n) % 216,
            60 + (j * 180 / n) % 196,
            255 - ((i + j) * 110 / n) % 200
        )
    };
    let (c, b, t) = (cell, cell * 0.12, cell / 3.0);
    let mut s = String::from(r#"<meshgradient id="m" x="0" y="0">"#);
    for j in 0..n {
        s.push_str("<meshrow>");
        for i in 0..n {
            s.push_str("<meshpatch>");
            if j == 0 {
                s.push_str(&format!(
                    r#"<stop path="c {t},{b} {},{} {c},0" stop-color="{}"/>"#,
                    2.0 * t,
                    -b,
                    col(i + 1, j)
                ));
            }
            s.push_str(&format!(
                r#"<stop path="c {b},{t} {},{} 0,{c}" stop-color="{}"/>"#,
                -b,
                2.0 * t,
                col(i + 1, j + 1)
            ));
            s.push_str(&format!(
                r#"<stop path="c {},{b} {},{} {},0" stop-color="{}"/>"#,
                -t,
                -2.0 * t,
                -b,
                -c,
                col(i, j + 1)
            ));
            if i == 0 {
                s.push_str(&format!(
                    r#"<stop path="c {b},{} {},{} 0,{}" stop-color="{}"/>"#,
                    -t,
                    -b,
                    -2.0 * t,
                    -c,
                    col(i, j)
                ));
            }
            s.push_str("</meshpatch>");
        }
        s.push_str("</meshrow>");
    }
    s.push_str("</meshgradient>");
    s
}

/// Every mesh region clip in compiled output: (subpaths, vertices, twice the
/// signed area) of each `<clipPath id="x-mesh-…">`'s path.
fn region_clips(out: &str) -> Vec<(usize, usize, f64)> {
    let doc = roxmltree::Document::parse(out).unwrap();
    doc.descendants()
        .filter(|n| {
            n.has_tag_name("clipPath")
                && n.attribute("id").is_some_and(|i| i.starts_with("x-mesh-"))
        })
        .map(|clip| {
            let d = clip.first_element_child().unwrap().attribute("d").unwrap();
            let (mut vertices, mut area) = (0, 0.0);
            let subpaths: Vec<&str> = d.split('Z').filter(|s| !s.is_empty()).collect();
            for sub in &subpaths {
                let pts: Vec<(f64, f64)> = sub
                    .trim_start_matches('M')
                    .split('L')
                    .map(|p| {
                        let (x, y) = p.split_once(',').unwrap();
                        (x.parse().unwrap(), y.parse().unwrap())
                    })
                    .collect();
                vertices += pts.len();
                for i in 0..pts.len() {
                    let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                    area += a.0 * b.1 - b.0 * a.1;
                }
            }
            (subpaths.len(), vertices, area)
        })
        .collect()
}

#[test]
fn a_mesh_region_is_clipped_by_its_outline_not_by_every_face() {
    // 8×8 patches, 12×12 quads each at this quality: 9,216 faces, one region
    let svg = format!(
        r##"{XW}{}<rect width="400" height="400" fill="url(#m)"/></svg>"##,
        patch_grid_meshgradient(8, 50.0)
    );
    let out = compile_test(&svg);
    let clips = region_clips(&out);
    assert_eq!(clips.len(), 1, "one smooth region");
    let (subpaths, vertices, area) = clips[0];
    assert_eq!(subpaths, 1);
    // the rim of a 96×96 grid of quads
    assert_eq!(vertices, 4 * 96);
    // the patch edges bow in and out by equal amounts: the area is the square's
    assert!((area.abs() / 2.0 - 160_000.0).abs() < 160.0, "{area}");
    // it was 554 KB when every face was named
    assert!(out.len() < 24 * 1024, "{} bytes", out.len());
    assert_eq!(out.matches("<image").count(), 1);
}

#[test]
fn mesh_region_clips_cover_exactly_their_faces() {
    // authored meshes, where the answer can be written down
    for (mesh, expected) in [
        // one quad
        (
            r##"<x:mesh points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##,
            vec![(1, 4, 10_000.0)],
        ),
        // two quads agreeing along their shared edge: one region, the seam gone
        (
            r##"<x:mesh points="0,0 50,0 100,0 0,50 50,50 100,50"><x:face v="0 1 4 3" fill="#f00 #0f0 #00f #fff"/><x:face v="1 2 5 4" fill="#0f0 #f0f #ff0 #00f"/></x:mesh>"##,
            vec![(1, 6, 10_000.0)],
        ),
        // the same two with a colour crack between them: a region each
        (
            r##"<x:mesh points="0,0 50,0 100,0 0,50 50,50 100,50"><x:face v="0 1 4 3" fill="#f00 #0f0 #00f #fff"/><x:face v="1 2 5 4" fill="#000 #f0f #ff0 #000"/></x:mesh>"##,
            vec![(1, 4, 5_000.0), (1, 4, 5_000.0)],
        ),
        // wound the other way round, and a triangle
        (
            r##"<x:mesh points="0,0 50,0 100,0 0,50 50,50"><x:face v="3 4 1 0" fill="#fff #00f #0f0 #f00"/><x:face v="1 2 4" fill="#0f0 #f0f #00f"/></x:mesh>"##,
            vec![(1, 5, 7_500.0)],
        ),
    ] {
        let out = compile_test(&format!("{XW}{mesh}</svg>"));
        let clips: Vec<(usize, usize, f64)> = region_clips(&out)
            .into_iter()
            .map(|(s, v, a)| (s, v, a.abs()))
            .collect();
        assert_eq!(clips, expected, "{out}");
    }
}

#[test]
fn a_flat_mesh_region_is_one_path() {
    // every corner the same colour: no image, the outline filled directly
    let out = compile_test(&format!(
        r##"{XW}<x:mesh points="0,0 50,0 100,0 0,50 50,50 100,50"><x:face v="0 1 4 3" fill="#369 #369 #369 #369"/><x:face v="1 2 5 4" fill="#369 #369 #369 #369"/></x:mesh></svg>"##
    ));
    assert!(!out.contains("<image"), "{out}");
    assert!(
        out.contains(r##"d="M0,0L50,0L100,0L100,50L50,50L0,50Z""##),
        "{out}"
    );
}

#[test]
fn mesh_outlines_are_stable_across_compiles_and_fragments() {
    let svg = format!(
        r##"{XW}{}<rect width="200" height="200" fill="url(#m)"/></svg>"##,
        patch_grid_meshgradient(3, 200.0 / 3.0)
    );
    let whole = compile_test(&svg);
    for _ in 0..4 {
        assert_eq!(compile_test(&svg), whole);
    }
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let fragment = compile_with(
        &svg,
        &Options {
            fragment: Some(svg.find("<rect").unwrap()),
            ..Default::default()
        },
        &platform,
    )
    .unwrap();
    assert!(
        whole.contains(&fragment),
        "a fragment is its span in the whole compile"
    );
    assert_eq!(region_clips(&whole).len(), 1);
}

#[test]
fn a_meshgradient_definition_is_consumed_not_emitted() {
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    for (source, images) in [
        // in <defs>, where Inkscape writes it, and at the top level
        (
            format!(
                r##"{XW}<defs>{INKMESH}</defs><rect width="200" height="80" fill="url(#m)"/></svg>"##
            ),
            1,
        ),
        (
            format!(r##"{XW}{INKMESH}<rect width="200" height="80" fill="url(#m)"/></svg>"##),
            1,
        ),
        // filling two shapes, and after its first use
        (
            format!(
                r##"{XW}<rect width="200" height="80" fill="url(#m)"/><circle cx="100" cy="40" r="30" fill="url(#m)"/><defs>{INKMESH}</defs></svg>"##
            ),
            2,
        ),
        // referenced by nothing
        (
            format!(r##"{XW}<defs>{INKMESH}</defs><rect width="200" height="80"/></svg>"##),
            0,
        ),
    ] {
        let out = compile_test(&source);
        assert!(!out.contains("meshgradient"), "{out}");
        assert!(
            !out.contains("meshpatch") && !out.contains("<stop"),
            "{out}"
        );
        assert_eq!(out.matches("<image").count(), images, "{out}");
        roxmltree::Document::parse(&out).unwrap();
        // the definition's own fragment is empty; a shape's is unchanged
        let at = source.find("<meshgradient").unwrap();
        let own = compile_with(
            &source,
            &Options {
                fragment: Some(at),
                ..Default::default()
            },
            &platform,
        )
        .unwrap();
        assert!(!own.contains("meshgradient"), "{own}");
    }
}

#[test]
fn an_unlowered_meshgradient_reference_dangles_like_an_unsupported_paint() {
    // a dialect the compiler cannot parse: the shape stays as authored, with
    // its marker, and the definition is still not part of the output (§9)
    let out = compile_test(&format!(
        r##"{XW}<defs><meshgradient id="m"><meshrow><meshpatch><stop path="q 1,2 3,4"/></meshpatch></meshrow></meshgradient></defs><rect width="20" height="20" rx="2" fill="url(#m)" stroke="url(#m)"/></svg>"##
    ));
    assert!(!out.contains("<meshgradient"), "{out}");
    assert!(out.contains("meshgradient fill left live"), "{out}");
    assert!(out.contains(r##"fill="url(#m)""##), "{out}");
    // other gradients are untouched
    let out = compile_test(&format!(
        r##"{XW}<defs><linearGradient id="l"><stop offset="0" stop-color="#f00"/></linearGradient></defs><rect width="20" height="20" rx="2" fill="url(#l)"/></svg>"##
    ));
    assert!(
        out.contains("<linearGradient") && out.contains("<stop"),
        "{out}"
    );
}

#[test]
fn editing_a_meshgradient_still_re_emits_the_shapes_it_fills() {
    let source = format!(
        r##"{XW}<defs>{INKMESH}</defs><rect width="200" height="80" fill="url(#m)"/><rect y="90" width="20" height="20"/></svg>"##
    );
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let session = CompileSession::new(&platform, &source, "balanced", false, "").unwrap();
    let mesh = source.find("<meshgradient").unwrap();
    let filled = source.find("<rect width=\"200\"").unwrap();
    let deps = session.unit_dependents(&[mesh]);
    assert_eq!(
        deps.iter().map(|d| d.0).collect::<Vec<_>>(),
        vec![filled],
        "{deps:?}"
    );
}

/// The `<image …/>` elements of compiled output with their clip ids removed:
/// what a mesh paints, independent of who asked for it.
fn mesh_images(out: &str) -> Vec<String> {
    let mut images = Vec::new();
    let mut rest = out;
    while let Some(at) = rest.find("<image") {
        let end = at + rest[at..].find("/>").unwrap() + 2;
        let image = &rest[at..end];
        let clip = image.find(" clip-path=\"").unwrap();
        let close = clip + 12 + image[clip + 12..].find('"').unwrap() + 1;
        images.push(format!("{}{}", &image[..clip], &image[close..]));
        rest = &rest[end..];
    }
    images
}

const PAINT_MESH: &str = r##"<x:mesh id="m" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;

#[test]
fn a_path_filled_by_an_x_mesh_is_clipped_mesh_paint() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{PAINT_MESH}</defs><path d="M10,10 L90,10 L50,45 Z" fill="url(#m)" stroke="#000" stroke-width="2"/></svg>"##
    ));
    // the shape is the clip, the mesh the paint, the stroke on top
    assert!(out.contains(r##"<clipPath id="x-mgc-"##), "{out}");
    assert!(
        out.contains(r##"<path d="M10,10 L90,10 L50,45 Z"/></clipPath><g clip-path="url(#x-mgc-"##),
        "{out}"
    );
    assert!(
        out.contains(
            r##"<path stroke="#000" stroke-width="2" fill="none" d="M10,10 L90,10 L50,45 Z"/>"##
        ),
        "{out}"
    );
    assert!(
        !out.contains(r##"fill="url(#m)""##),
        "the reference is compiled: {out}"
    );
    // the paint is the mesh's own: once where it is defined, once in the shape
    let images = mesh_images(&out);
    assert_eq!(images.len(), 2, "{out}");
    assert_eq!(images[0], images[1]);
    assert_eq!(region_clips(&out), vec![(1, 4, 10_000.0), (1, 4, 10_000.0)]);
    roxmltree::Document::parse(&out).unwrap();
}

#[test]
fn every_shape_takes_an_x_mesh_fill_wherever_the_mesh_stands() {
    for shape in [
        r##"<path d="M0,0H80V40H0Z" fill="url(#m)"/>"##,
        r##"<rect x="5" y="5" width="60" height="30" fill="url(#m)"/>"##,
        r##"<rect x="5" y="5" width="60" height="30" rx="6" fill="url(#m)"/>"##,
        r##"<circle cx="50" cy="25" r="20" fill="url(#m)"/>"##,
        r##"<ellipse cx="50" cy="25" rx="30" ry="15" fill="url(#m)"/>"##,
        r##"<polygon points="10,10 90,10 50,45" fill="url(#m)"/>"##,
        r##"<path d="M0,0H80V40H0Z" fill="url(#m)" transform="rotate(10)" opacity=".5"/>"##,
    ] {
        for source in [
            format!("{XW}<defs>{PAINT_MESH}</defs>{shape}</svg>"),
            // a forward reference, and a mesh that is also drawn in place
            format!("{XW}{shape}{PAINT_MESH}</svg>"),
            // nothing above the mesh places the paint
            format!(
                r##"{XW}<g transform="translate(500 500) scale(9)"><defs>{PAINT_MESH}</defs></g>{shape}</svg>"##
            ),
        ] {
            let out = compile_test(&source);
            assert_eq!(out.matches("x-mgc-").count(), 2, "{shape}: {out}");
            let images = mesh_images(&out);
            assert_eq!(images.len(), 2, "{shape}: {out}");
            assert_eq!(images[0], images[1], "{shape}");
            assert!(!out.contains("left live"), "{out}");
        }
    }
}

#[test]
fn an_x_mesh_fill_reads_the_grid_form_cracks_and_feathering() {
    // grid sugar
    let out = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="m" x="10" y="20" width="80" height="40" cols="2" rows="1" fill="#f00 #0f0 #00f #ff0 #f0f #0ff"/></defs><circle cx="50" cy="40" r="15" fill="url(#m)"/></svg>"##
    ));
    assert_eq!(mesh_images(&out).len(), 2, "{out}");
    assert_eq!(region_clips(&out)[1], (1, 6, 6_400.0));
    // a colour crack: the fill has both regions
    let out = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="m" points="0,0 50,0 100,0 0,50 50,50 100,50"><x:face v="0 1 4 3" fill="#f00 #0f0 #00f #fff"/><x:face v="1 2 5 4" fill="#000 #f0f #ff0 #000"/></x:mesh></defs><rect width="100" height="50" fill="url(#m)"/></svg>"##
    ));
    assert_eq!(mesh_images(&out).len(), 4, "{out}");
    assert_eq!(region_clips(&out).len(), 4);
    // per-corner alpha: the fill's texture carries it
    let out = compile_test(&format!(
        r##"{XW}<rect width="100" height="50" fill="url(#m)"/><defs><x:mesh id="m" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00f #0f0f #00f0 #fff0"/></x:mesh></defs></svg>"##
    ));
    assert_eq!(b64_prefix(&out, 26)[25], 6, "RGBA: {out}");
    // a skipped face does not stop the rest filling
    let out = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="m" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/><x:face v="0 1 9" fill="#000"/></x:mesh></defs><rect width="100" height="50" fill="url(#m)"/></svg>"##
    ));
    assert_eq!(mesh_images(&out).len(), 2, "{out}");
    assert_eq!(
        out.matches("<x:face> skipped").count(),
        1,
        "said once, where the mesh stands: {out}"
    );
}

#[test]
fn an_x_mesh_fill_is_placed_by_the_meshs_own_transform() {
    let clip = |mesh_attrs: &str| {
        let out = compile_test(&format!(
            r##"{XW}<rect width="400" height="300" fill="url(#m)"/><defs><x:mesh id="m" points="0,0 100,0 100,50 0,50"{mesh_attrs}><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs></svg>"##
        ));
        let doc = roxmltree::Document::parse(&out).unwrap();
        let d = doc
            .descendants()
            .find(|n| n.attribute("id").is_some_and(|i| i.starts_with("x-mesh-")))
            .and_then(|n| n.first_element_child())
            .and_then(|p| p.attribute("d"))
            .unwrap()
            .to_string();
        d
    };
    assert_eq!(clip(""), "M0,0L100,0L100,50L0,50Z");
    // as gradientTransform places a gradient
    assert_eq!(
        clip(r#" transform="translate(20 10) scale(2)""#),
        "M20,10L220,10L220,110L20,110Z"
    );
    assert_eq!(
        clip(r#" transform="matrix(0 1 -1 0 60 0)""#),
        "M60,0L60,100L10,100L10,0Z"
    );
    assert_eq!(
        clip(r#" style="transform:translate(5px,5px)""#),
        "M5,5L105,5L105,55L5,55Z"
    );
}

#[test]
fn an_unusable_x_mesh_fill_stays_live_with_a_marker() {
    for mesh in [
        r##"<x:mesh id="m" points="0,0 1,0"/>"##,
        r##"<x:mesh id="m" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 7" fill="#f00"/></x:mesh>"##,
        r##"<x:mesh id="m" x="0" y="0" width="10" height="10" cols="2" rows="2" fill="#f00"/>"##,
    ] {
        let out = compile_test(&format!(
            r##"{XW}<defs>{mesh}</defs><rect width="20" height="20" rx="2" fill="url(#m)"/></svg>"##
        ));
        assert!(out.contains("x:mesh fill left live"), "{out}");
        assert!(out.contains(r##"fill="url(#m)""##), "{out}");
        assert!(!out.contains("<image"), "{out}");
        roxmltree::Document::parse(&out).unwrap();
    }
    // a mesh with no extent parses but has nothing to paint
    let out = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="m" points="0,0 10,0 20,0"><x:face v="0 1 2" fill="#f00"/></x:mesh></defs><rect width="20" height="20" fill="url(#m)"/></svg>"##
    ));
    assert!(out.contains("mesh fill degenerate extent"), "{out}");
}

#[test]
fn only_real_meshes_are_mesh_paint() {
    for (paint, compiled) in [
        (PAINT_MESH.to_string(), true),
        (INKMESH.to_string(), true),
        // right names, wrong namespaces
        (
            r##"<mesh id="m" points="0,0 100,0 100,50 0,50"/>"##.to_string(),
            false,
        ),
        (r##"<x:meshgradient id="m"/>"##.to_string(), false),
        // inside the editor's island an id does not exist (§4.4)
        (format!("<x:doc>{PAINT_MESH}</x:doc>"), false),
        (
            r##"<linearGradient id="m"><stop offset="0" stop-color="#f00"/></linearGradient>"##
                .to_string(),
            false,
        ),
    ] {
        let out = compile_test(&format!(
            r##"{XW}<defs>{paint}</defs><rect width="100" height="50" rx="3" fill="url(#m)"/></svg>"##
        ));
        assert_eq!(out.contains("x-mgc-"), compiled, "{paint}: {out}");
        assert_eq!(
            out.contains(r##"fill="url(#m)""##),
            !compiled,
            "{paint}: {out}"
        );
    }
}

#[test]
fn an_x_mesh_fill_is_a_baked_reference() {
    let source = format!(
        r##"{XW}<defs>{PAINT_MESH}</defs><path d="M0,0H80V40H0Z" fill="url(#m)"/><circle cx="9" cy="9" r="5" fill="url(#m)"/><rect y="90" width="20" height="20"/></svg>"##
    );
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let session = CompileSession::new(&platform, &source, "balanced", false, "").unwrap();
    let mesh = source.find("<x:mesh").unwrap();
    let (path, circle) = (
        source.find("<path").unwrap(),
        source.find("<circle").unwrap(),
    );
    let starts = |deps: Vec<(usize, usize)>| deps.iter().map(|d| d.0).collect::<Vec<_>>();
    // editing the mesh re-emits both shapes it fills, and nothing else
    assert_eq!(starts(session.unit_dependents(&[mesh])), vec![path, circle]);
    assert_eq!(
        starts(session.dependents(&[source.find("<defs").unwrap()])),
        vec![path, circle]
    );
    // each shape's fragment is its span in the whole compile
    let whole = compile_test(&source);
    for at in [path, circle] {
        let fragment = compile_with(
            &source,
            &Options {
                fragment: Some(at),
                ..Default::default()
            },
            &platform,
        )
        .unwrap();
        assert!(
            fragment.contains("x-mgc-") && whole.contains(&fragment),
            "{fragment}"
        );
    }
}

#[test]
fn an_x_mesh_fill_inside_a_clip_path_is_geometry_only() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{PAINT_MESH}<clipPath id="c"><circle cx="50" cy="25" r="20" fill="url(#m)"/></clipPath></defs><rect width="100" height="50" clip-path="url(#c)"/></svg>"##
    ));
    let doc = roxmltree::Document::parse(&out).unwrap();
    let clip = doc
        .descendants()
        .find(|n| n.attribute("id") == Some("c"))
        .unwrap();
    assert!(
        clip.descendants().all(|n| !n.has_tag_name("image")),
        "{out}"
    );
    assert_eq!(
        mesh_images(&out).len(),
        1,
        "only where the mesh is defined: {out}"
    );
}

#[test]
fn an_x_mesh_fill_is_sized_for_the_shape_at_display_resolution() {
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let out = compile_with(
        &format!(
            r##"{XW}<defs>{PAINT_MESH}</defs><g transform="scale(4)"><path d="M0,0H100V50H0Z" fill="url(#m)"/></g></svg>"##
        ),
        &display_options(1.0),
        &platform,
    )
    .unwrap();
    // the second texture is the fill's: drawn 4× the size of the definition's
    let pngs: Vec<Vec<u8>> = out
        .match_indices("base64,")
        .map(|(at, _)| b64_prefix(&out[at - 1..], 24))
        .collect();
    let width = |png: &[u8]| u32::from_be_bytes(png[16..20].try_into().unwrap()) as f64;
    assert_eq!(pngs.len(), 2);
    assert!(
        (width(&pngs[1]) / width(&pngs[0]) - 4.0).abs() < 0.1,
        "{} vs {}",
        width(&pngs[1]),
        width(&pngs[0])
    );
}

// ---- a mesh as any paint: stroke, bounding-box units, every bearer (§8.2) ----

/// A solid mesh covering everything: the paint is then exactly the clip.
const WASH: &str = r##"<x:mesh id="w" points="-900,-900 900,-900 900,900 -900,900"><x:face v="0 1 2 3" fill="#c33"/></x:mesh>"##;

/// The clip whose id starts with `prefix` (`x-mgc-` a fill's, `x-mgs-` a
/// stroke's), as geometry.
fn paint_clip(out: &str, prefix: &str) -> crate::kurbo::BezPath {
    let doc = roxmltree::Document::parse(out).unwrap_or_else(|e| panic!("{e}: {out}"));
    let clip = doc
        .descendants()
        .find(|n| {
            n.has_tag_name("clipPath") && n.attribute("id").is_some_and(|i| i.starts_with(prefix))
        })
        .unwrap_or_else(|| panic!("no {prefix} clip: {out}"));
    let d = clip.first_element_child().unwrap().attribute("d").unwrap();
    crate::kurbo::BezPath::from_svg(d).unwrap()
}

fn covers(path: &crate::kurbo::BezPath, x: f64, y: f64) -> bool {
    use crate::kurbo::Shape;
    path.winding(crate::kurbo::Point::new(x, y)) != 0
}

fn stroke_clip_of(shape: &str) -> crate::kurbo::BezPath {
    let out = compile_test(&format!("{XW}<defs>{WASH}</defs>{shape}</svg>"));
    assert!(!out.contains("left live"), "{out}");
    paint_clip(&out, "x-mgs-")
}

#[test]
fn a_mesh_stroke_is_mesh_paint_clipped_to_the_stroke_outline() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{PAINT_MESH}</defs><rect x="10" y="10" width="80" height="30" fill="#eea" stroke="url(#m)" stroke-width="8"/></svg>"##
    ));
    assert!(
        !out.contains(r#"stroke="url(#m)""#),
        "the reference is compiled: {out}"
    );
    // the plain fill stays a path, under the stroke's mesh
    let fill = out
        .find(r##"<path fill="#eea" stroke-width="8" stroke="none" d=""##)
        .expect(&out);
    let stroke = out.find(r#"<clipPath id="x-mgs-"#).expect(&out);
    assert!(fill < stroke, "{out}");
    assert!(!out.contains("x-mgc-"), "no fill piece: {out}");
    // the paint is the mesh's own
    let images = mesh_images(&out);
    assert_eq!(images.len(), 2, "{out}");
    assert_eq!(images[0], images[1]);
    // and the clip is the band 4 either side of the outline
    let band = paint_clip(&out, "x-mgs-");
    for (x, y, on) in [
        (10.0, 25.0, true),
        (7.0, 25.0, true),
        (13.0, 25.0, true),
        (5.0, 25.0, false),
        (15.0, 25.0, false),
        (50.0, 25.0, false),
        (50.0, 10.0, true),
        (50.0, 43.0, true),
        (50.0, 45.0, false),
        // a miter corner is square
        (6.5, 6.5, true),
    ] {
        assert_eq!(covers(&band, x, y), on, "({x},{y}) in {out}");
    }
    // outer 88×38 less inner 72×22, by counting: joins overlap, so the signed
    // area of the outline is not the area it covers
    let covered = (0..200 * 100)
        .filter(|i| {
            covers(
                &band,
                (i % 200) as f64 * 0.5 + 0.25,
                (i / 200) as f64 * 0.5 + 0.25,
            )
        })
        .count() as f64
        * 0.25;
    assert_eq!(covered, 88.0 * 38.0 - 72.0 * 22.0);
}

#[test]
fn a_mesh_stroke_honours_caps() {
    let line = |cap: &str| {
        stroke_clip_of(&format!(
            r##"<line x1="20" y1="50" x2="80" y2="50" stroke="url(#w)" stroke-width="10" {cap}/>"##
        ))
    };
    let (butt, round, square) = (
        line(""),
        line(r#"stroke-linecap="round""#),
        line(r#"stroke-linecap="square""#),
    );
    for clip in [&butt, &round, &square] {
        assert!(covers(clip, 50.0, 46.0) && !covers(clip, 50.0, 56.0));
    }
    // past the end, on the axis: only a cap reaches
    assert!(!covers(&butt, 83.0, 50.0));
    assert!(covers(&round, 83.0, 50.0) && covers(&square, 83.0, 50.0));
    // the corner of the cap's box: only square reaches
    assert!(!covers(&round, 84.0, 54.0) && covers(&square, 84.0, 54.0));
    assert!(!covers(&square, 86.0, 50.0));
}

#[test]
fn a_mesh_stroke_honours_joins_and_the_miter_limit() {
    // a 53° corner at (50,10): miter length is 2.24 widths
    let corner = |style: &str| {
        stroke_clip_of(&format!(
            r##"<path d="M30,50 L50,10 L70,50" fill="none" stroke="url(#w)" stroke-width="10" {style}/>"##
        ))
    };
    let tip = |clip: &crate::kurbo::BezPath| covers(clip, 50.0, 2.0);
    assert!(tip(&corner("")), "the default limit of 4 keeps this miter");
    assert!(!tip(&corner(r#"stroke-miterlimit="2""#)));
    assert!(!tip(&corner(r#"stroke-linejoin="bevel""#)));
    let round = corner(r#"stroke-linejoin="round""#);
    assert!(!tip(&round) && covers(&round, 50.0, 6.0));
    // a limit below 1 is not a limit SVG knows; it must not panic or widen
    assert!(!tip(&corner(r#"stroke-miterlimit="0""#)));
}

#[test]
fn a_mesh_stroke_honours_dashes() {
    let dashed = |style: &str| {
        stroke_clip_of(&format!(
            r##"<line x1="0" y1="50" x2="100" y2="50" stroke="url(#w)" stroke-width="6" {style}/>"##
        ))
    };
    let on = |clip: &crate::kurbo::BezPath, x: f64| covers(clip, x, 50.0);
    let d = dashed(r#"stroke-dasharray="10 5""#);
    assert!(on(&d, 5.0) && !on(&d, 12.5) && on(&d, 20.0) && !on(&d, 27.5));
    // the offset slides the pattern back along the path
    let d = dashed(r#"stroke-dasharray="10 5" stroke-dashoffset="10""#);
    assert!(!on(&d, 2.5) && on(&d, 10.0));
    // an odd list repeats: 10 on, 10 off
    let d = dashed(r#"stroke-dasharray="10""#);
    assert!(on(&d, 5.0) && !on(&d, 15.0) && on(&d, 25.0));
    // comma separated
    let d = dashed(r#"stroke-dasharray="10,5""#);
    assert!(on(&d, 5.0) && !on(&d, 12.5));
    // not a pattern: solid
    for solid in [
        r#"stroke-dasharray="none""#,
        r#"stroke-dasharray="0 0""#,
        r#"stroke-dasharray="10 -5""#,
        r#"stroke-dasharray="10 x""#,
        r#"stroke-dasharray="""#,
    ] {
        let d = dashed(solid);
        assert!((0..20).all(|i| on(&d, 2.5 + 5.0 * i as f64)), "{solid}");
    }
}

#[test]
fn a_mesh_stroke_inherits_its_style() {
    let band = stroke_clip_of(
        r##"<g stroke-width="20" stroke-linecap="round"><g><line x1="20" y1="50" x2="80" y2="50" stroke="url(#w)"/></g></g>"##,
    );
    assert!(covers(&band, 50.0, 41.0) && !covers(&band, 50.0, 61.0));
    assert!(covers(&band, 88.0, 50.0), "the inherited cap");
    // the element's own value wins
    let band = stroke_clip_of(
        r##"<g stroke-width="20"><line x1="20" y1="50" x2="80" y2="50" stroke="url(#w)" stroke-width="4"/></g>"##,
    );
    assert!(covers(&band, 50.0, 51.0) && !covers(&band, 50.0, 53.0));
    // the default width is 1
    let band = stroke_clip_of(r##"<line x1="20" y1="50" x2="80" y2="50" stroke="url(#w)"/>"##);
    assert!(covers(&band, 50.0, 50.4) && !covers(&band, 50.0, 50.6));
}

#[test]
fn a_stroke_with_no_width_paints_nothing_and_leaves_no_reference() {
    for width in ["0", "-3"] {
        let out = compile_test(&format!(
            r##"{XW}<defs>{WASH}</defs><rect width="40" height="20" fill="#eea" stroke="url(#w)" stroke-width="{width}"/></svg>"##
        ));
        assert!(!out.contains("x-mgs-") && !out.contains("url(#w)"), "{out}");
        assert!(
            out.contains(r##"fill="#eea""##) && out.contains(r#"stroke="none""#),
            "{out}"
        );
        roxmltree::Document::parse(&out).unwrap();
    }
}

#[test]
fn a_non_scaling_mesh_stroke_stays_live_and_the_fill_is_still_lowered() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><rect width="40" height="20" fill="url(#w)" stroke="url(#w)" vector-effect="non-scaling-stroke"/></svg>"##
    ));
    assert!(
        out.contains("mesh stroke left live (non-scaling-stroke)"),
        "{out}"
    );
    assert!(out.contains("x-mgc-") && !out.contains("x-mgs-"), "{out}");
    // the stroke is as authored, on the path that keeps the element's paint
    assert!(
        out.contains(
            r##"<path stroke="url(#w)" vector-effect="non-scaling-stroke" fill="none" d="##
        ),
        "{out}"
    );
    // alone, the element is left exactly as it would be without the feature
    let alone = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><rect width="40" height="20" rx="2" stroke="url(#w)" vector-effect="non-scaling-stroke"/></svg>"##
    ));
    assert!(
        alone.contains(r##"<rect width="40" height="20" rx="2" stroke="url(#w)" vector-effect="non-scaling-stroke"/>"##),
        "{alone}"
    );
}

#[test]
fn both_paints_as_meshes_make_two_pieces_with_ids_of_their_own() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{PAINT_MESH}<x:mesh id="n" points="0,0 100,0 100,50 0,50"><x:face v="0 1 2 3" fill="#000 #fff #000 #fff"/></x:mesh></defs><rect x="10" y="10" width="80" height="30" fill="url(#m)" stroke="url(#n)" stroke-width="6"/></svg>"##
    ));
    let (fill, stroke) = (out.find("x-mgc-").unwrap(), out.find("x-mgs-").unwrap());
    assert!(fill < stroke, "fill under stroke: {out}");
    assert!(
        !out.contains("url(#m)") && !out.contains("url(#n)"),
        "{out}"
    );
    // nothing is left for a path of the element's own
    assert!(!out.contains(r#"fill="none""#), "{out}");
    // each piece carries its own mesh
    let images = mesh_images(&out);
    assert_eq!(images.len(), 4, "{out}");
    assert_eq!((&images[0], &images[1]), (&images[2], &images[3]));
    assert_ne!(images[2], images[3]);
    // every id in the output is defined once
    let doc = roxmltree::Document::parse(&out).unwrap();
    let mut ids: Vec<&str> = doc
        .descendants()
        .filter_map(|n| n.attribute("id"))
        .collect();
    let all = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), all, "{out}");
    // and every clip reference resolves
    for n in doc.descendants() {
        if let Some(id) = n.attribute("clip-path").and_then(paint_ref) {
            assert!(ids.contains(&id), "{id} in {out}");
        }
    }
}

#[test]
fn the_group_carries_the_element_and_the_path_only_its_paint() {
    let out = compile_mapped(&format!(
        r##"{XW}<defs>{WASH}</defs><rect id="card" class="k" data-role="hero" x="10" y="10" width="80" height="30" transform="rotate(5)" opacity=".5" mask="url(#q)" fill="url(#w)" stroke="#222" stroke-width="3" stroke-dasharray="4 2"/></svg>"##
    ));
    let doc = roxmltree::Document::parse(&out).unwrap();
    let group = doc
        .descendants()
        .find(|n| n.attribute("id") == Some("card"))
        .expect(&out);
    assert_eq!(group.tag_name().name(), "g");
    for (name, value) in [
        ("class", "k"),
        ("data-role", "hero"),
        ("transform", "rotate(5)"),
        ("opacity", ".5"),
        ("mask", "url(#q)"),
    ] {
        assert_eq!(group.attribute(name), Some(value), "{name}: {out}");
        assert_eq!(
            out.matches(&format!(" {name}=\"")).count(),
            1,
            "{name} once: {out}"
        );
    }
    assert!(group.attribute("data-xsvg-pos").is_some(), "{out}");
    for gone in [
        "x",
        "y",
        "width",
        "height",
        "fill",
        "stroke",
        "stroke-width",
    ] {
        assert!(group.attribute(gone).is_none(), "{gone}: {out}");
    }
    let kids: Vec<_> = group.children().filter(|n| n.is_element()).collect();
    let names: Vec<&str> = kids.iter().map(|n| n.tag_name().name()).collect();
    assert_eq!(names, ["clipPath", "g", "path"], "{out}");
    let path = kids[2];
    let attrs: Vec<(&str, &str)> = path.attributes().map(|a| (a.name(), a.value())).collect();
    assert_eq!(
        attrs,
        [
            ("stroke", "#222"),
            ("stroke-width", "3"),
            ("stroke-dasharray", "4 2"),
            ("fill", "none"),
            ("d", "M10,10 h80 v30 h-80 Z"),
        ],
        "{out}"
    );
}

#[test]
fn an_unstroked_mesh_filled_shape_keeps_its_identity() {
    // there is no path to hang them on: the group is the element
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><circle id="dot" cx="20" cy="20" r="9" opacity=".3" display="none" fill="url(#w)"/><use href="#dot" x="40"/></svg>"##
    ));
    assert!(
        out.contains(r#"<g id="dot" opacity=".3" display="none"><clipPath id="x-mgc-"#),
        "{out}"
    );
    assert_eq!(
        out.matches("<path").count(),
        2 + 1,
        "wash ×2 and the clip: {out}"
    );
}

#[test]
fn paint_order_stacks_the_pieces() {
    let order = |attrs: &str| {
        let out = compile_test(&format!(
            r##"{XW}<defs>{PAINT_MESH}</defs><circle cx="50" cy="50" r="30" stroke-width="8" {attrs}/></svg>"##
        ));
        assert!(!out.contains("url(#m)"), "{out}");
        let mut at: Vec<(usize, &str)> = [
            ("x-mgc-", "fill-mesh"),
            ("x-mgs-", "stroke-mesh"),
            // the element's own path, in the order its attributes were written
            ("<path stroke-width=", "path"),
        ]
        .iter()
        .filter_map(|(needle, name)| out.find(needle).map(|i| (i, *name)))
        .collect();
        at.sort_unstable();
        at.into_iter().map(|(_, n)| n).collect::<Vec<_>>().join(" ")
    };
    let (f, s) = (r#"fill="url(#m)""#, r#"stroke="url(#m)""#);
    assert_eq!(order(&format!("{f} {s}")), "fill-mesh stroke-mesh");
    assert_eq!(order(&format!(r#"{f} stroke="red""#)), "fill-mesh path");
    assert_eq!(order(&format!(r#"fill="red" {s}"#)), "path stroke-mesh");
    assert_eq!(order(s), "path stroke-mesh", "an unset fill is black");
    assert_eq!(order(&format!(r#"fill="none" {s}"#)), "stroke-mesh");
    assert_eq!(order(f), "fill-mesh");
    assert_eq!(order(&format!(r#"{f} stroke="none""#)), "fill-mesh");
    for first in [
        "stroke",
        "stroke markers",
        "markers stroke",
        "stroke fill markers",
    ] {
        let po = format!(r#"paint-order="{first}""#);
        assert_eq!(
            order(&format!("{po} {f} {s}")),
            "stroke-mesh fill-mesh",
            "{first}"
        );
        assert_eq!(
            order(&format!(r#"{f} stroke="red" {po}"#)),
            "path fill-mesh",
            "{first}"
        );
        assert_eq!(
            order(&format!(r#"{po} fill="red" {s}"#)),
            "stroke-mesh path",
            "{first}"
        );
    }
    for normal in ["normal", "fill", "fill stroke", "markers", "markers fill"] {
        let po = format!(r#"paint-order="{normal}""#);
        assert_eq!(
            order(&format!("{f} {s} {po}")),
            "fill-mesh stroke-mesh",
            "{normal}"
        );
    }
    // markers are the path's: it stays even with nothing else to paint
    assert_eq!(
        order(&format!(r#"marker-end="url(#a)" {f} {s}"#)),
        "fill-mesh stroke-mesh path"
    );
    assert_eq!(
        order(&format!(r#"marker-end="url(#a)" {f}"#)),
        "fill-mesh path"
    );
    assert_eq!(order(&format!(r#"marker-end="none" {f}"#)), "fill-mesh");
}

#[test]
fn a_mesh_fill_is_clipped_by_the_shape_s_own_winding() {
    let holed = r#"d="M0,0 H100 V60 H0 Z M30,20 H70 V40 H30 Z""#;
    let clip = |attrs: &str, wrap: (&str, &str)| {
        let out = compile_test(&format!(
            r##"{XW}<defs>{WASH}</defs>{}<path {holed} fill="url(#w)" {attrs}/>{}</svg>"##,
            wrap.0, wrap.1
        ));
        let doc = roxmltree::Document::parse(&out).unwrap();
        let path = doc
            .descendants()
            .find(|n| n.attribute("id").is_some_and(|i| i.starts_with("x-mgc-")))
            .unwrap()
            .first_element_child()
            .unwrap();
        path.attribute("clip-rule").map(str::to_string)
    };
    assert_eq!(clip("", ("", "")), None);
    assert_eq!(clip(r#"fill-rule="nonzero""#, ("", "")), None);
    assert_eq!(
        clip(r#"fill-rule="evenodd""#, ("", "")).as_deref(),
        Some("evenodd")
    );
    // inherited, and overridden
    let g = (r#"<g fill-rule="evenodd">"#, "</g>");
    assert_eq!(clip("", g).as_deref(), Some("evenodd"));
    assert_eq!(clip(r#"fill-rule="nonzero""#, g), None);
}

#[test]
fn paint_opacities_go_on_their_pieces() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g stroke-opacity=".25"><rect width="40" height="20" opacity=".9" fill-opacity="0.4" fill="url(#w)" stroke="url(#w)"/></g></svg>"##
    ));
    assert!(
        out.contains(r#"<g opacity=".9"><clipPath id="x-mgc-"#),
        "{out}"
    );
    let piece = |prefix: &str| {
        let at = out.find(&format!("<g clip-path=\"url(#{prefix}")).unwrap();
        out[at..out[at..].find('>').unwrap() + at].to_string()
    };
    assert!(piece("x-mgc-").ends_with(r#" opacity="0.4""#), "{out}");
    assert!(piece("x-mgs-").ends_with(r#" opacity=".25""#), "{out}");
    // none set: none written
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><rect width="40" height="20" fill="url(#w)"/></svg>"##
    ));
    assert_eq!(out.matches("opacity").count(), 0, "{out}");
}

#[test]
fn a_polyline_is_stroked_open() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><polyline points="10,10 50,10 50,50" fill="url(#w)" stroke="url(#w)" stroke-width="4"/><polyline points="10,10 50,10 50,50" fill="url(#w)" stroke="#000"/></svg>"##
    ));
    let band = paint_clip(&out, "x-mgs-");
    assert!(covers(&band, 30.0, 10.0) && covers(&band, 50.0, 30.0));
    assert!(
        !covers(&band, 30.0, 30.0),
        "no closing edge back to the start: {out}"
    );
    // the fill still covers the triangle the closing edge would bound
    let mut area = paint_clip(&out, "x-mgc-");
    area.close_path(); // as a fill closes it
    assert!(covers(&area, 40.0, 20.0));
    // and the plain stroke of the second is open too
    assert!(
        out.contains(r##"<path stroke="#000" fill="none" d="M10,10 L50,10 L50,50"/>"##),
        "{out}"
    );
    // a polygon closes
    let closed = stroke_clip_of(
        r##"<polygon points="10,10 50,10 50,50" fill="none" stroke="url(#w)" stroke-width="4"/>"##,
    );
    assert!(covers(&closed, 30.0, 30.0));
}

#[test]
fn every_shape_takes_a_mesh_stroke_of_either_kind() {
    let mg = r##"<meshgradient id="m" x="0" y="0" gradientUnits="userSpaceOnUse"><meshrow><meshpatch><stop path="l 100,0" stop-color="#f00"/><stop path="l 0,50" stop-color="#0f0"/><stop path="l -100,0" stop-color="#00f"/><stop path="l 0,-50" stop-color="#fff"/></meshpatch></meshrow></meshgradient>"##;
    for (paint, defined) in [(PAINT_MESH, 1), (mg, 0)] {
        for shape in [
            r##"<path d="M0,0H80V40H0Z" fill="none" stroke="url(#m)"/>"##,
            r##"<rect x="5" y="5" width="60" height="30" fill="none" stroke="url(#m)"/>"##,
            r##"<rect x="5" y="5" width="60" height="30" rx="6" fill="none" stroke="url(#m)"/>"##,
            r##"<circle cx="50" cy="25" r="20" fill="none" stroke="url(#m)"/>"##,
            r##"<ellipse cx="50" cy="25" rx="30" ry="15" fill="none" stroke="url(#m)"/>"##,
            r##"<polygon points="10,10 90,10 50,45" fill="none" stroke="url(#m)"/>"##,
            r##"<polyline points="10,10 90,10 50,45" fill="none" stroke="url(#m)"/>"##,
            r##"<line x1="10" y1="10" x2="90" y2="40" stroke="url(#m)"/>"##,
        ] {
            let out = compile_test(&format!("{XW}<defs>{paint}</defs>{shape}</svg>"));
            assert_eq!(
                out.matches("<clipPath id=\"x-mgs-").count(),
                1,
                "{shape}: {out}"
            );
            assert_eq!(mesh_images(&out).len(), defined + 1, "{shape}: {out}");
            assert!(
                !out.contains("url(#m)") && !out.contains("left live"),
                "{shape}: {out}"
            );
            roxmltree::Document::parse(&out).unwrap();
        }
    }
}

#[test]
fn a_mesh_paint_is_inherited_like_any_paint() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g id="grp" fill="url(#w)" stroke="url(#w)" stroke-width="2"><rect width="20" height="20"/><g><circle cx="50" cy="10" r="8"/></g><path d="M0,40H20V60Z" fill="#00f" stroke="none"/><path d="M30,40H50V60Z" fill="none"/></g></svg>"##
    ));
    // rect and circle: both paints; the last path: only the stroke
    assert_eq!(out.matches("<clipPath id=\"x-mgc-").count(), 2, "{out}");
    assert_eq!(out.matches("<clipPath id=\"x-mgs-").count(), 3, "{out}");
    // the path with paints of its own is untouched
    assert!(
        out.contains(r##"<path d="M0,40H20V60Z" fill="#00f" stroke="none"/>"##),
        "{out}"
    );
    // with no fill and a mesh stroke, the last has nothing left for a path
    assert_eq!(out.matches("<path d=\"M30,40H50V60Z\"").count(), 0, "{out}");
    // an inherited paint has no attribute to drop: where a path remains, it
    // is overridden
    let out2 = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g stroke="url(#w)"><rect width="20" height="20" fill="#eea"/></g></svg>"##
    ));
    assert!(
        out2.contains(r##"<path fill="#eea" stroke="none" d=""##),
        "{out2}"
    );
    roxmltree::Document::parse(&out).unwrap();
    // editing the mesh re-emits the group whose shapes it paints
    let source = format!(
        r##"{XW}<defs>{WASH}</defs><g stroke="url(#w)"><rect width="20" height="20"/></g><rect y="90" width="20" height="20"/></svg>"##
    );
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let session = CompileSession::new(&platform, &source, "balanced", false, "").unwrap();
    let deps = session.unit_dependents(&[source.find("<x:mesh").unwrap()]);
    assert_eq!(
        deps.iter().map(|d| d.0).collect::<Vec<_>>(),
        vec![source.find("<rect").unwrap()],
        "the shape that inherits it, not the group that carries it"
    );
}

#[test]
fn a_mesh_stroke_is_a_baked_reference() {
    let source = format!(
        r##"{XW}<defs>{PAINT_MESH}</defs><path d="M0,0H80V40H0Z" stroke="url(#m)"/><circle cx="9" cy="9" r="5" stroke="url(#other)"/><x:boolean stroke="url(#m)"><rect width="9" height="9"/></x:boolean></svg>"##
    );
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let session = CompileSession::new(&platform, &source, "balanced", false, "").unwrap();
    let deps = session.unit_dependents(&[source.find("<x:mesh").unwrap()]);
    assert_eq!(
        deps.iter().map(|d| d.0).collect::<Vec<_>>(),
        vec![
            source.find("<path").unwrap(),
            source.find("<x:boolean").unwrap()
        ]
    );
}

#[test]
fn paint_references_are_read_in_every_spelling() {
    assert_eq!(paint_ref("url(#m)"), Some("m"));
    assert_eq!(paint_ref("  url( #m )  "), Some("m"));
    assert_eq!(paint_ref(r##"url("#m")"##), Some("m"));
    assert_eq!(paint_ref("url('#m') #888"), Some("m"));
    assert_eq!(paint_ref("url(#m) none"), Some("m"));
    for not in [
        "",
        "none",
        "#m",
        "url(m)",
        "url(#)",
        "url(#m",
        "red url(#m)",
        "url(other.svg#m)",
    ] {
        assert_eq!(paint_ref(not), None, "{not}");
    }
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><rect width="9" height="9" fill='url("#w") #888'/></svg>"##
    ));
    assert!(out.contains("x-mgc-") && !out.contains("#888"), "{out}");
}

#[test]
fn what_the_element_inherits_does_not_paint_the_mesh_again() {
    // a solid region is a plain path: an inherited stroke would outline it,
    // an inherited fill-opacity would fade it twice
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g stroke="#00f" stroke-width="5" fill-opacity=".5"><rect width="40" height="20" fill="url(#w)"/></g></svg>"##
    ));
    assert!(
        out.contains(r#"<g clip-path="url(#x-mgc-"#)
            && out.contains(
                r##"" opacity=".5" stroke="none" fill-opacity="1"><path fill="#cc3333""##
            ),
        "{out}"
    );
    // the inherited stroke still strokes the element
    assert!(
        out.contains(r#"<path fill="none" d="M0,0 h40 v20 h-40 Z"/>"#),
        "{out}"
    );
    // nothing inherited: nothing reset
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g><rect width="40" height="20" stroke="#00f" fill-opacity=".5" fill="url(#w)"/></g></svg>"##
    ));
    assert!(
        out.contains(r##"" opacity=".5"><path fill="#cc3333""##),
        "{out}"
    );
}

#[test]
fn a_dash_pattern_too_fine_to_outline_leaves_the_stroke_live() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><path d="M0,0 H100000" stroke="url(#w)" stroke-dasharray="0.001"/><path d="M0,9 H100" stroke="url(#w)" stroke-dasharray="1 1"/></svg>"##
    ));
    assert_eq!(
        out.matches("mesh stroke left live (dash pattern too fine)")
            .count(),
        1,
        "{out}"
    );
    assert_eq!(out.matches("<clipPath id=\"x-mgs-").count(), 1, "{out}");
    assert!(
        out.contains(r#"<path d="M0,0 H100000" stroke="url(#w)" stroke-dasharray="0.001"/>"#),
        "{out}"
    );
    // an offset of any size is the same pattern
    let near = stroke_clip_of(
        r##"<line x2="40" stroke="url(#w)" stroke-width="2" stroke-dasharray="4" stroke-dashoffset="2"/>"##,
    );
    let far = stroke_clip_of(
        r##"<line x2="40" stroke="url(#w)" stroke-width="2" stroke-dasharray="4" stroke-dashoffset="8000000002"/>"##,
    );
    let back = stroke_clip_of(
        r##"<line x2="40" stroke="url(#w)" stroke-width="2" stroke-dasharray="4" stroke-dashoffset="-6"/>"##,
    );
    for x in 0..40 {
        let on = covers(&near, x as f64 + 0.5, 0.0);
        assert_eq!(covers(&far, x as f64 + 0.5, 0.0), on, "{x}");
        assert_eq!(covers(&back, x as f64 + 0.5, 0.0), on, "{x}");
    }
}

#[test]
fn a_sliver_of_a_mesh_is_fitted_on_a_bounded_raster() {
    // 100000:1 — the long axis no longer buys pixels without limit
    let out = compile_test(&format!(
        r##"{XW}<x:mesh points="0,0 100000,0 100000,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh><defs><x:mesh id="u" units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs><rect width="50000" height="0.01" fill="url(#u)"/></svg>"##
    ));
    let images = mesh_images(&out);
    assert_eq!(images.len(), 3, "{out}");
    assert!(out.len() < 60_000, "{}", out.len());
    // each texture still spans its mesh: placed across the whole long axis
    let span = |image: &str, name: &str| -> f64 {
        let at = image.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        image[at..at + image[at..].find('"').unwrap()]
            .parse()
            .unwrap()
    };
    assert!(
        span(&images[0], "width") >= 100_000.0 && span(&images[0], "height") >= 1.0,
        "{}",
        images[0]
    );
    assert!(
        span(&images[2], "width") >= 50_000.0 && span(&images[2], "height") >= 0.01,
        "{}",
        images[2]
    );
    // and a mesh of ordinary proportions is fitted exactly as before
    let wide = compile_test(&format!(
        r##"{XW}<x:mesh points="0,0 2000,0 2000,10 0,10"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></svg>"##
    ));
    assert_eq!(region_clips(&wide), vec![(1, 4, 40_000.0)]);
}

// ---- bounding-box units ----

#[test]
fn an_x_mesh_in_bounding_box_units_spans_the_box_of_what_it_paints() {
    let unit = r##"<x:mesh id="u" units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;
    let user = r##"<x:mesh id="a" points="20,10 120,10 120,60 20,60"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh>"##;
    let paint = |defs: &str, id: &str, shape: &str| {
        let out = compile_test(&format!(
            "{XW}<defs>{defs}</defs>{}</svg>",
            shape.replace("ID", id)
        ));
        assert!(!out.contains("left live"), "{out}");
        (
            mesh_images(&out).pop().unwrap(),
            region_clips(&out).pop().unwrap(),
        )
    };
    // every shape whose box is (20,10)–(120,60): the unit mesh is the user one
    for shape in [
        r##"<rect x="20" y="10" width="100" height="50" fill="url(#ID)"/>"##,
        r##"<ellipse cx="70" cy="35" rx="50" ry="25" fill="url(#ID)"/>"##,
        r##"<path d="M20,10 L120,35 L20,60 Z" fill="url(#ID)"/>"##,
        r##"<polygon points="20,60 70,10 120,60" fill="url(#ID)"/>"##,
        // a stroke takes the fill box, not the box of its ink
        r##"<rect x="20" y="10" width="100" height="50" fill="none" stroke="url(#ID)" stroke-width="12"/>"##,
        // the element's transform is outside its own user space
        r##"<rect x="20" y="10" width="100" height="50" transform="translate(300) scale(3)" fill="url(#ID)"/>"##,
    ] {
        assert_eq!(paint(unit, "u", shape), paint(user, "a", shape), "{shape}");
    }
    // each shape has a box of its own: one mesh, two extents
    let out = compile_test(&format!(
        r##"{XW}<defs>{unit}</defs><rect width="10" height="10" fill="url(#u)"/><rect x="50" width="200" height="40" fill="url(#u)"/></svg>"##
    ));
    let clips = region_clips(&out);
    // (twice the area, as `region_clips` counts it)
    assert_eq!((clips[1].2, clips[2].2), (200.0, 16_000.0), "{out}");
}

#[test]
fn bounding_box_units_compose_with_the_mesh_s_transform_and_the_grid_form() {
    // SVG's order for gradientTransform: the box map applies last
    let moved = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="u" units="objectBoundingBox" transform="translate(.5 0) scale(.5 1)" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs><rect x="20" y="10" width="100" height="50" fill="url(#u)"/></svg>"##
    ));
    let direct = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="a" points="70,10 120,10 120,60 70,60"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs><rect x="20" y="10" width="100" height="50" fill="url(#a)"/></svg>"##
    ));
    assert_eq!(mesh_images(&moved).pop(), mesh_images(&direct).pop());
    assert_eq!(region_clips(&moved).pop(), region_clips(&direct).pop());
    // grid sugar: x/y/width/height are fractions too
    let grid = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="u" units="objectBoundingBox" width="1" height="1" cols="2" rows="1" fill="#f00 #0f0 #00f #ff0 #f0f #0ff"/></defs><rect x="20" y="10" width="100" height="50" fill="url(#u)"/></svg>"##
    ));
    assert_eq!(region_clips(&grid).pop(), Some((1, 6, 10_000.0)), "{grid}");
}

#[test]
fn bounding_box_units_need_a_box() {
    // a horizontal line has no height: SVG paints nothing, and so do we
    let out = compile_test(&format!(
        r##"{XW}<defs><x:mesh id="u" units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00"/></x:mesh></defs><line x1="0" y1="5" x2="90" y2="5" stroke="url(#u)" stroke-width="4"/></svg>"##
    ));
    assert!(
        out.contains("no bounding box for objectBoundingBox units"),
        "{out}"
    );
    assert!(!out.contains("x-mgs-"), "{out}");
    // the same mesh in user space is fine on the same line
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><line x1="0" y1="5" x2="90" y2="5" stroke="url(#w)" stroke-width="4"/></svg>"##
    ));
    assert!(
        out.contains("x-mgs-") && !out.contains("left live"),
        "{out}"
    );
    // a meshgradient plays by the same rule
    let out = compile_test(&format!(
        r##"{XW}<defs><meshgradient id="g" gradientUnits="objectBoundingBox"><meshrow><meshpatch><stop path="l 1,0" stop-color="#f00"/><stop path="l 0,1" stop-color="#0f0"/><stop path="l -1,0" stop-color="#00f"/><stop path="l 0,-1" stop-color="#fff"/></meshpatch></meshrow></meshgradient></defs><line x1="0" y1="5" x2="90" y2="5" stroke="url(#g)" stroke-width="4"/><rect y="20" width="30" height="20" stroke="url(#g)" stroke-width="4"/></svg>"##
    ));
    assert_eq!(out.matches("no bounding box").count(), 1, "{out}");
    assert_eq!(out.matches("<clipPath id=\"x-mgs-").count(), 1, "{out}");
}

#[test]
fn units_say_nothing_where_the_mesh_is_drawn() {
    let out = compile_test(&format!(
        r##"{XW}<x:mesh units="objectBoundingBox" points="0,0 40,0 40,30 0,30"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></svg>"##
    ));
    assert!(!out.contains("units"), "consumed: {out}");
    assert_eq!(region_clips(&out), vec![(1, 4, 2_400.0)]);
    // any other value is user space
    for units in ["userSpaceOnUse", "bogus", ""] {
        let out = compile_test(&format!(
            r##"{XW}<defs><x:mesh id="u" units="{units}" points="0,0 40,0 40,30 0,30"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs><rect x="500" width="9" height="9" fill="url(#u)"/></svg>"##
        ));
        assert_eq!(region_clips(&out).pop(), Some((1, 4, 2_400.0)), "{units}");
    }
}

// ---- the x: elements and text ----

/// The `d` of the one path a compiled element is.
fn only_path_d(out: &str) -> String {
    let doc = roxmltree::Document::parse(out).unwrap();
    let paths: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("path"))
        .collect();
    assert_eq!(paths.len(), 1, "{out}");
    paths[0].attribute("d").unwrap().to_string()
}

#[test]
fn a_boolean_takes_mesh_paint_over_its_result() {
    let operands = r#"<circle cx="40" cy="40" r="30"/><circle cx="70" cy="40" r="30"/>"#;
    for op in ["union", "intersect", "subtract", "exclude"] {
        let plain = only_path_d(&compile_test(&format!(
            r##"{XW}<x:boolean op="{op}" fill="#abc">{operands}</x:boolean></svg>"##
        )));
        let out = compile_test(&format!(
            r##"{XW}<defs>{WASH}</defs><x:boolean id="b" op="{op}" transform="translate(5 6)" opacity=".5" fill="url(#w)" stroke="url(#w)" stroke-width="4">{operands}</x:boolean></svg>"##
        ));
        assert!(
            !out.contains("left live") && !out.contains("url(#w)"),
            "{op}: {out}"
        );
        // the fill's clip is the boolean's own result
        let doc = roxmltree::Document::parse(&out).unwrap();
        let clip = doc
            .descendants()
            .find(|n| n.attribute("id").is_some_and(|i| i.starts_with("x-mgc-")))
            .unwrap();
        assert_eq!(
            clip.first_element_child().unwrap().attribute("d"),
            Some(plain.as_str()),
            "{op}"
        );
        // the element's own vocabulary stays out of the output
        assert!(
            out.contains(r#"<g id="b" transform="translate(5 6)" opacity=".5"><clipPath"#),
            "{op}: {out}"
        );
        assert!(!out.contains(" op="), "{op}: {out}");
        // the stroke follows the result's outline, not the operands'
        let band = paint_clip(&out, "x-mgs-");
        let seam = covers(&band, 55.0, 40.0); // where the circles overlap, mid-figure
        assert!(!seam, "{op}: {out}");
    }
}

#[test]
fn a_boolean_with_a_plain_paint_keeps_it_beside_the_mesh() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><x:boolean fill-rule="evenodd" fill="url(#w)" stroke="#222" stroke-width="2"><rect width="40" height="40"/><rect x="20" y="20" width="40" height="40"/></x:boolean></svg>"##
    ));
    // one path of the result, stroked; the operands' winding is already spent
    assert!(
        out.contains(r##"<path stroke="#222" stroke-width="2" fill="none" d=""##),
        "{out}"
    );
    assert!(
        !out.contains("fill-rule") && !out.contains("clip-rule"),
        "{out}"
    );
    let fill = paint_clip(&out, "x-mgc-");
    assert!(covers(&fill, 10.0, 10.0) && covers(&fill, 30.0, 30.0) && covers(&fill, 50.0, 50.0));
}

#[test]
fn a_boolean_inherits_a_mesh_paint_and_nests() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}</defs><g fill="url(#w)"><x:boolean><rect width="40" height="40"/><rect x="20" y="20" width="40" height="40"/></x:boolean></g></svg>"##
    ));
    assert_eq!(out.matches("<clipPath id=\"x-mgc-").count(), 1, "{out}");
    // a mesh-painted boolean as an operand gives geometry, not its paint
    let nested = |inner_fill: &str| {
        compile_test(&format!(
            r##"{XW}<defs>{WASH}</defs><x:boolean op="subtract" fill="#abc"><rect width="60" height="60"/><x:boolean {inner_fill}><circle cx="30" cy="30" r="10"/><circle cx="40" cy="30" r="10"/></x:boolean></x:boolean></svg>"##
        ))
    };
    assert_eq!(nested(r#"fill="url(#w)" stroke="url(#w)""#), nested(""));
    // and by reference
    let by_ref = |paint: &str| {
        let out = compile_test(&format!(
            r##"{XW}<defs>{WASH}</defs><x:boolean op="subtract" fill="#abc"><rect width="60" height="60"/><use href="#hole"/></x:boolean><x:boolean id="hole" {paint}><circle cx="30" cy="30" r="10"/></x:boolean></svg>"##
        ));
        let at = out.find(r##"<path fill="#abc""##).expect(&out);
        out[at..at + out[at..].find("/>").unwrap()].to_string()
    };
    assert_eq!(by_ref(r#"fill="url(#w)" stroke="url(#w)""#), by_ref(""));
}

#[test]
fn an_offset_takes_mesh_paint() {
    let plain = compile_test(&format!(
        r##"{XW}<x:offset in="#o" distance="6" fill="#abc"/><defs><path id="o" d="M20,20 H80 V60 H20 Z"/></defs></svg>"##
    ));
    let plain = roxmltree::Document::parse(&plain)
        .unwrap()
        .descendants()
        .find(|n| n.attribute("fill") == Some("#abc"))
        .and_then(|n| n.attribute("d").map(str::to_string))
        .unwrap();
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}<path id="o" d="M20,20 H80 V60 H20 Z"/></defs><x:offset in="#o" distance="6" fill="url(#w)" stroke="url(#w)" stroke-width="2"/></svg>"##
    ));
    assert!(
        !out.contains("left live") && !out.contains(" distance=") && !out.contains(" in="),
        "{out}"
    );
    let doc = roxmltree::Document::parse(&out).unwrap();
    let clip = doc
        .descendants()
        .find(|n| n.attribute("id").is_some_and(|i| i.starts_with("x-mgc-")))
        .unwrap();
    assert_eq!(
        clip.first_element_child().unwrap().attribute("d"),
        Some(plain.as_str())
    );
    let band = paint_clip(&out, "x-mgs-");
    assert!(
        covers(&band, 14.0, 40.0) && !covers(&band, 20.0, 40.0),
        "{out}"
    );
}

#[test]
fn text_takes_mesh_paint_over_its_glyph_outlines() {
    for text in [
        r##"<x:textbox x="10" y="10" width="200" height="80" font-size="20" PAINT>Mesh</x:textbox>"##,
        r##"<text x="10" y="40" font-size="20" PAINT>Mesh</text>"##,
        r##"<x:textpath in="#spine" font-size="20" PAINT>Mesh</x:textpath>"##,
    ] {
        let spine = r#"<path id="spine" d="M10,60 H300" fill="none"/>"#;
        let outlined = compile_outlined(&format!(
            r##"{XW}{spine}{}</svg>"##,
            text.replace(
                "PAINT",
                if text.starts_with("<text") {
                    r#"x:outline="true""#
                } else {
                    r#"outline="true""#
                }
            )
        ));
        let glyphs = outlined.matches("<path").count() - 1;
        assert!(glyphs > 0, "{outlined}");
        for paint in [
            r#"fill="url(#w)""#,
            r#"fill="url(#w)" stroke="url(#w)" stroke-width="2""#,
            r##"fill="#eea" stroke="url(#w)""##,
        ] {
            let out = compile_outlined(&format!(
                "{XW}<defs>{WASH}</defs>{spine}{}</svg>",
                text.replace("PAINT", paint)
            ));
            assert!(
                !out.contains("left live") && !out.contains("url(#w)"),
                "{text} {paint}: {out}"
            );
            assert!(
                !out.contains("<text") && !out.contains("font-size"),
                "{text}: {out}"
            );
            let prefix = if paint.starts_with("fill=\"url") {
                "x-mgc-"
            } else {
                "x-mgs-"
            };
            use crate::kurbo::Shape;
            assert!(paint_clip(&out, prefix).area().abs() > 0.0, "{text}: {out}");
            roxmltree::Document::parse(&out).unwrap();
        }
    }
}

#[test]
fn text_that_cannot_be_outlined_keeps_its_paint_as_authored() {
    // no outliner: there is no geometry to clip the mesh to
    for text in [
        r##"<x:textbox x="10" y="10" width="200" height="80" font-size="20" fill="url(#w)">Mesh</x:textbox>"##,
        r##"<text x="10" y="40" font-size="20" stroke="url(#w)">Mesh</text>"##,
    ] {
        let out = compile_test(&format!("{XW}<defs>{WASH}</defs>{text}</svg>"));
        assert!(
            out.contains("mesh paint left live (no geometry to paint)"),
            "{out}"
        );
        assert!(out.contains("<text") && out.contains("url(#w)"), "{out}");
        assert!(!out.contains("x-mgc-") && !out.contains("x-mgs-"), "{out}");
    }
}

#[test]
fn runs_with_paint_of_their_own_keep_the_text_live() {
    let out = compile_outlined(&format!(
        r##"{XW}<defs>{WASH}</defs><x:textbox x="10" y="10" width="200" height="80" font-size="20" fill="url(#w)">Mesh <tspan fill="#00f">blue</tspan></x:textbox></svg>"##
    ));
    assert!(out.contains("runs with their own fill or stroke"), "{out}");
    assert!(!out.contains("x-mgc-"), "{out}");
    // a run's stroke does not stop a mesh fill, nor its fill a mesh stroke
    let out = compile_outlined(&format!(
        r##"{XW}<defs>{WASH}</defs><x:textbox x="10" y="10" width="200" height="80" font-size="20" stroke="url(#w)">Mesh <tspan fill="#00f">blue</tspan></x:textbox></svg>"##
    ));
    assert!(
        out.contains("x-mgs-") && !out.contains("left live"),
        "{out}"
    );
}

#[test]
fn an_x_text_element_does_not_inherit_a_mesh_fill() {
    // an unset fill on a textbox is black (§6), never the ancestor's
    let out = compile_outlined(&format!(
        r##"{XW}<defs>{WASH}</defs><g fill="url(#w)"><x:textbox x="10" y="10" width="200" height="80" font-size="20">Mesh</x:textbox></g></svg>"##
    ));
    assert!(!out.contains("x-mgc-"), "{out}");
}

// ---- where mesh paint must not be lowered ----

#[test]
fn elements_that_are_not_painted_by_fill_are_left_alone() {
    let before = |body: &str| compile_test(&format!("{XW}<defs>{WASH}</defs>{body}</svg>"));
    for body in [
        r##"<g fill="url(#w)"><image width="9" height="9" href="data:,"/></g>"##,
        r##"<x:pie cx="50" cy="50" r="40" fill="url(#w)"><x:slice value="1"/><x:slice value="2"/></x:pie>"##,
        r##"<rect id="a" width="9" height="9"/><rect id="b" x="90" width="9" height="9"/><x:connector from="#a" to="#b" stroke="url(#w)"/>"##,
        r##"<svg width="50" height="50" fill="url(#w)"/>"##,
    ] {
        let out = before(body);
        assert!(
            !out.contains("x-mgc-") && !out.contains("x-mgs-"),
            "{body}: {out}"
        );
        roxmltree::Document::parse(&out).unwrap();
    }
    // a paint that names something other than a mesh is as it always was
    let out = before(
        r##"<linearGradient id="lg"/><rect width="9" height="9" rx="1" fill="url(#lg)" stroke="url(#nowhere)"/>"##,
    );
    assert!(
        out.contains(
            r##"<rect width="9" height="9" rx="1" fill="url(#lg)" stroke="url(#nowhere)"/>"##
        ),
        "{out}"
    );
}

#[test]
fn a_mesh_stroke_in_a_clip_path_is_only_geometry() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}<clipPath id="c"><circle cx="50" cy="25" r="20" stroke="url(#w)" stroke-width="9"/></clipPath></defs><rect width="100" height="50" clip-path="url(#c)"/></svg>"##
    ));
    assert!(!out.contains("x-mgs-") && !out.contains("x-mgc-"), "{out}");
    assert!(
        out.contains(r#"<clipPath id="c"><circle cx="50" cy="25" r="20""#),
        "{out}"
    );
}

#[test]
fn to_path_leaves_a_mesh_paint_in_the_source() {
    let source = format!(
        r##"{XW}<defs>{WASH}</defs><x:boolean fill="url(#w)" stroke="url(#w)"><rect width="9" height="9"/><rect x="5" width="9" height="9"/></x:boolean></svg>"##
    );
    let platform = Platform::new(Mono, NoShaper, NoOutliner);
    let session = CompileSession::new(&platform, &source, "balanced", false, "").unwrap();
    let markup = session.to_path(source.find("<x:boolean").unwrap()).unwrap();
    assert!(
        markup.contains(r#"fill="url(#w)""#) && markup.contains(r#"stroke="url(#w)""#),
        "{markup}"
    );
    assert!(!markup.contains("clipPath"), "{markup}");
}

#[test]
fn one_unusable_mesh_does_not_stop_the_other_paint() {
    let out = compile_test(&format!(
        r##"{XW}<defs>{WASH}<x:mesh id="bad" points="0,0 1,1"/></defs><rect width="40" height="20" fill="url(#bad)" stroke="url(#w)"/><rect y="50" width="40" height="20" fill="url(#w)" stroke="url(#bad)"/></svg>"##
    ));
    assert_eq!(
        out.matches("x:mesh fill left live (no usable mesh)")
            .count(),
        2,
        "{out}"
    );
    assert_eq!(out.matches("<clipPath id=\"x-mgs-").count(), 1, "{out}");
    assert_eq!(out.matches("<clipPath id=\"x-mgc-").count(), 1, "{out}");
    // the unusable one is kept as written: a reference that paints nothing
    assert!(
        out.contains(r#"<path fill="url(#bad)" stroke="none" d="#),
        "{out}"
    );
    assert!(
        out.contains(r#"<path stroke="url(#bad)" fill="none" d="#),
        "{out}"
    );
}

#[test]
fn mesh_paint_lowering_survives_arbitrary_stroke_styles() {
    // a deterministic sweep: every output is well-formed, and stable
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let pick = |n: u64, of: &[&'static str]| of[(n % of.len() as u64) as usize];
    for round in 0..300 {
        let shape = pick(
            next(),
            &[
                r#"path d="M10,10 C40,-20 90,60 60,90 L20,70 Z""#,
                r#"path d="M10,10 L10,10""#,
                r#"path d="M10,10 L50,50 L10,10 L50,50""#,
                r#"path d="M0,0 H50 M60,0 H90 M0,30 Q40,90 80,30 Z M5,5""#,
                r#"path d="""#,
                r#"path d="M1e9,1e9 L-1e9,3""#,
                r#"rect x="5" y="5" width="60" height="30" rx="40""#,
                r#"rect width="0" height="9""#,
                r#"circle cx="40" cy="40" r="0.001""#,
                r#"polyline points="1,1 9,9 1,9 9,1 5,5 5,5""#,
                r#"line x1="3" y1="3" x2="3" y2="3""#,
                r#"ellipse cx="50" cy="20" rx="45" ry="2""#,
            ],
        );
        let style = format!(
            r#"stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}" stroke-miterlimit="{}" stroke-dasharray="{}" stroke-dashoffset="{}" paint-order="{}" fill-rule="{}""#,
            pick(
                next(),
                &["1", "0.01", "30", "1e4", "0", "-1", "abc", "5px", "50%"]
            ),
            pick(next(), &["butt", "round", "square", "bogus"]),
            pick(next(), &["miter", "round", "bevel", "arcs"]),
            pick(next(), &["4", "1", "0", "-9", "1e9", "x"]),
            pick(
                next(),
                &[
                    "none",
                    "3",
                    "3 1",
                    "0.001",
                    "1e-9 1e-9",
                    "1e9",
                    "5,0,5",
                    "0 9"
                ]
            ),
            pick(next(), &["0", "2.5", "-7", "1e12", "x"]),
            pick(
                next(),
                &["normal", "stroke", "markers stroke fill", "bogus"]
            ),
            pick(next(), &["nonzero", "evenodd"]),
        );
        let paints = pick(
            next(),
            &[
                r#"fill="url(#w)" stroke="url(#u)""#,
                r#"fill="none" stroke="url(#w)""#,
                r#"fill="url(#u)" stroke="red""#,
                r#"stroke="url(#u)""#,
            ],
        );
        let source = format!(
            r##"{XW}<defs>{WASH}<x:mesh id="u" units="objectBoundingBox" points="0,0 1,0 1,1 0,1"><x:face v="0 1 2 3" fill="#f00 #0f0 #00f #fff"/></x:mesh></defs><{shape} {paints} {style}/></svg>"##
        );
        let out = compile_fast(&source);
        roxmltree::Document::parse(&out)
            .unwrap_or_else(|e| panic!("round {round}: {e}\n{source}\n{out}"));
        assert_eq!(out, compile_fast(&source), "round {round}");
        assert!(
            !out.contains("NaN") && !out.contains("inf"),
            "round {round}: {source}\n{out}"
        );
    }
}
