//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// `<x:list list="bullet|number|none">` (§6.14): a vertical stack of `<x:li>`
/// items flowed top-down from the block's `x`/`y` (or the bbox of `in="#rect"`),
/// each wrapped to the content width with a HANGING indent — the marker sits in
/// the gutter and continuation lines align to the text column. `indent="N"` on an
/// item sets its nesting level; each level steps the text column right by `indent`
/// (default 1.5em) and cycles the marker style (bullet •◦▪ / number 1. a. i.).
/// Numbered items keep an outline counter per level, restarted when nesting pops.
/// Compiles to plain positioned `<tspan>`s — live and selectable — with **one
/// `<text>` per item**, keyed to its `<x:li>` via [`pos_attr`] (the same
/// per-source-node tagging paragraphs and table cells get): an editor can
/// address each item's rendered glyphs individually, and the item's text
/// content is exactly its own source text. Generated markers ("1.", "▸") live
/// in a separate untagged `<text>` so they never pollute an item's char stream.
pub(super) fn emit_list(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node);
    let lp: super::elements::List = params_of(node);
    let fill = lp.fill.as_str();
    let pos = pos_attr(node, ctx);
    let size = style.size;

    // Block geometry: an `in="#rect"` reference (its bbox), else x/y/width; the
    // height (rect bbox or explicit `height`) is only used for vertical align.
    let (mut x, mut y, mut width) = (lp.x, lp.y, lp.width);
    let mut avail_h = dim_attr(node, "height");
    if let Some(r) = node.attribute("in") {
        if let Some(bb) = ref_geometry(node, r, ctx)
            .ok()
            .and_then(|d| svg_path_bbox(&d))
        {
            x = bb.x0;
            y = bb.y0;
            width = bb.width();
            avail_h = Some(bb.height());
        }
    }
    let indent_step = lp.indent.unwrap_or(size * 1.5).max(0.0);
    let marker_gap = lp.marker_gap.unwrap_or(size * 0.5);
    let item_spacing = lp.item_spacing.unwrap_or(size * 0.35);
    let list_kind = lp.list.as_str();
    let marker_scale = lp.marker_size.max(0.0);
    let marker_fill = resolve_var(lp.marker_fill.as_deref().unwrap_or(fill)).into_owned();
    let valign = VAlign::parse(lp.valign.as_str());

    let fm = m.font_metrics(&style, size);
    let mut counters: Vec<u32> = Vec::new(); // one running number per nesting level

    // Pass 1: resolve each item's marker and wrap its text, with baselines laid
    // out relative to a zero origin (0, advance, 2·advance, …). The absolute
    // vertical offset is applied in pass 2 once the block height is known.
    struct Item<'n> {
        col: f64,
        marker: Marker,
        lines: Vec<PlacedLine>,
        baseline: f64, // the item's first-line baseline (relative frame) —
        // kept OUTSIDE `lines` so an EMPTY item (whose text laid out to
        // nothing) still knows its row: it takes vertical space like any
        // item, and its marker must sit on that row, not at the block top
        size: f64, // per-item font size (defaults to the list's)
        src: roxmltree::Node<'n, 'n>, // the <x:li>, for per-item source keys
    }
    let mut items: Vec<Item> = Vec::new();
    let mut last_baseline = 0.0; // baseline of the last line placed (relative frame)
    let mut first = true;
    for li in node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "li")
    {
        let level = attr_num(li, "indent", 0.0).max(0.0) as usize;
        let kind = li.attribute("list").unwrap_or(list_kind);
        // collect through the run walker so a <tbreak/> becomes '\n' — items
        // still lay out single-style, but forced breaks are honored below
        let text: String = collect_runs(li, &style)
            .0
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        let text_x = x + (level as f64 + 1.0) * indent_step;
        let max_w = (x + width - text_x).max(1.0);
        let col = text_x - marker_gap; // markers' right-edge column
                                       // an <x:li> may override the font size (e.g. a smaller sub-point); it
                                       // inherits the list's size otherwise, keeping the list's line-height
        let item_size = attr_pos(li, "font-size", size);
        let item_style = TextStyle {
            size: item_size,
            ..style.clone()
        };
        let item_advance = item_size * style.line_height;

        // outline counters: extend to this level, drop any deeper ones (a pop
        // restarts sublists), then advance this level's number
        if counters.len() <= level {
            counters.resize(level + 1, 0);
        }
        counters.truncate(level + 1);
        counters[level] += 1;

        // resolve the marker: an explicit `marker` (item, then list) wins — a
        // named shape (disc/circle/square/dash) draws a shape, anything else is
        // a literal text marker; otherwise the list kind decides.
        let marker_attr = li.attribute("marker").or(lp.marker.as_deref());
        let marker = match marker_attr {
            Some(mk) => match shape_token(mk) {
                Some(tok) => Marker::Shape(tok),
                None => Marker::Text(mk.to_string()),
            },
            None => match kind {
                "none" => Marker::None,
                "number" => Marker::Text(number_marker(level, counters[level])),
                _ => Marker::Shape(bullet_shape(level)),
            },
        };

        // the gap INTO this item uses THIS item's leading (item_advance), so a
        // smaller item tucks up under its parent instead of inheriting the
        // previous — larger — item's line height
        let first_baseline = if first {
            0.0
        } else {
            last_baseline + item_advance + item_spacing
        };
        // forced breaks ('\n' from <tbreak/>) split the item into chunks that
        // wrap independently and step by the item's own advance — a blank
        // chunk (consecutive breaks) occupies one row, like everywhere else
        let mut lines: Vec<PlacedLine> = Vec::new();
        let mut item_last_b = first_baseline;
        if !text.trim().is_empty() {
            let mut next_b = first_baseline;
            for chunk in text.split('\n') {
                if chunk.trim().is_empty() {
                    item_last_b = next_b;
                    next_b += item_advance;
                    continue;
                }
                let ls = layout_flow(chunk, &item_style, text_x, next_b, max_w, m);
                item_last_b = ls.last().map(|l| l.baseline).unwrap_or(next_b);
                next_b = item_last_b + item_advance;
                lines.extend(ls);
            }
        }
        last_baseline = item_last_b;
        items.push(Item {
            col,
            marker,
            lines,
            baseline: first_baseline,
            size: item_size,
            src: li,
        });
        first = false;
    }

    // Block height = first cap-top → last descent. In the relative frame the first
    // baseline is 0 (so cap-top is −cap_height) and the last baseline is
    // `last_baseline`. Place per `valign` within the available height (top else).
    let block_h = if items.is_empty() {
        0.0
    } else {
        last_baseline + fm.cap_height + fm.descent
    };
    let block_top = match (valign, avail_h) {
        (VAlign::Middle, Some(h)) => y + (h - block_h) / 2.0,
        (VAlign::Bottom, Some(h)) => y + (h - block_h),
        _ => y, // top, or no height to align within
    };
    let offset = block_top + fm.cap_height; // relative baseline 0 → this y

    // Pass 2: emit, shifting every relative baseline by `offset`. Markers and text
    // scale with the item's own size; a per-line `font-size` overrides the <text>
    // base only when the item differs from the list size.
    let mut shapes = String::new(); // drawn bullet markers (siblings of the text)
    let mut markers = String::new(); // generated text markers, one shared <text>
    let mut body = String::new(); // one keyed <text> per item: its OWN source text
    for it in &items {
        let item_baseline = it.baseline + offset;
        let item_fm = m.font_metrics(
            &TextStyle {
                size: it.size,
                ..style.clone()
            },
            it.size,
        );
        let item_r = it.size * 0.16 * marker_scale;
        let fs = if (it.size - size).abs() > 1e-6 {
            format!(" font-size=\"{}\"", fmt(it.size))
        } else {
            String::new()
        };
        match &it.marker {
            Marker::None => {}
            Marker::Shape(tok) => {
                // centre on the middle of lowercase (x-height), right edge at `col`
                let cy = item_baseline - item_fm.x_height * 0.5;
                shapes.push_str(&shape_marker(tok, it.col, cy, item_r, &marker_fill));
            }
            Marker::Text(mk) => {
                // right-aligned into the gutter so "1." and "10." share a column edge
                markers.push_str(&format!(
                    "<tspan x=\"{}\" y=\"{}\" text-anchor=\"end\" fill=\"{}\"{}>",
                    fmt(it.col),
                    fmt(item_baseline),
                    marker_fill,
                    fs
                ));
                push_escaped(&mut markers, mk, false);
                markers.push_str("</tspan>");
            }
        }
        // list items are single-style plain text — one positioned <tspan> per
        // line, wrapped in the item's OWN keyed <text> (empty items emit none)
        if !it.lines.is_empty() {
            body.push_str("<text text-anchor=\"start\"");
            push_font_attrs(&mut body, &style, size, fill);
            body.push_str(&pos_attr(it.src, ctx)); // per-item source map
            body.push('>');
            for line in &it.lines {
                body.push_str(&format!(
                    "<tspan x=\"{}\" y=\"{}\"{}>",
                    fmt(line.x),
                    fmt(line.baseline + offset),
                    fs
                ));
                push_escaped(&mut body, &line.text, false);
                body.push_str("</tspan>");
            }
            body.push_str("</text>");
        }
    }

    // wrap in a <g> so the block position/transform covers the drawn markers,
    // the generated-marker text, and the per-item texts
    out.push_str("<g");
    out.push_str(&pos);
    out.push('>');
    out.push_str(&shapes);
    if !markers.is_empty() {
        out.push_str("<text text-anchor=\"start\"");
        push_font_attrs(out, &style, size, fill);
        out.push('>');
        out.push_str(&markers);
        out.push_str("</text>");
    }
    out.push_str(&body);
    out.push_str("</g>");
}

/// A resolved `<x:li>` marker: a drawn shape token, a literal text string
/// (numbers or a custom character), or nothing.
pub(super) enum Marker {
    None,
    Shape(&'static str),
    Text(String),
}

/// Map a `marker` attribute to a drawn-shape token, or `None` if it should be
/// taken as a literal text marker (e.g. "▸", "—", "★").
pub(super) fn shape_token(s: &str) -> Option<&'static str> {
    match s {
        "disc" | "dot" | "bullet" => Some("disc"),
        "circle" | "ring" | "hollow" | "open" => Some("ring"),
        "square" => Some("square"),
        "dash" => Some("dash"),
        _ => None,
    }
}

/// The default bullet shape for a nesting level: filled disc, hollow ring, then
/// filled square — cycling every three levels.
pub(super) fn bullet_shape(level: usize) -> &'static str {
    match level % 3 {
        0 => "disc",
        1 => "ring",
        _ => "square",
    }
}

/// One drawn bullet marker, its right edge at `col` and centre at `cy`. `r` is
/// the disc radius; the square is sized to equal the disc's AREA (side = r·√π,
/// so its diagonal is smaller than the disc's diameter — optical compensation),
/// and the ring's stroke straddles so its outer edge matches the disc.
pub(super) fn shape_marker(tok: &str, col: f64, cy: f64, r: f64, fill: &str) -> String {
    match tok {
        "ring" => {
            // a hollow ring reads lighter/smaller than a filled disc of equal
            // diameter, so enlarge its outer edge ~12% to match the disc's weight
            let outer = r * 1.12;
            let sw = r * 0.45;
            let pr = outer - sw / 2.0; // outer edge = pr + sw/2 = outer
            format!(
                "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{}\"/>",
                fmt(col - outer),
                fmt(cy),
                fmt(pr),
                fill,
                fmt(sw)
            )
        }
        "square" => {
            let s = std::f64::consts::PI.sqrt() * r; // equal area to the disc
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                fmt(col - s),
                fmt(cy - s / 2.0),
                fmt(s),
                fmt(s),
                fill
            )
        }
        "dash" => {
            let (w, t) = (r * 2.8, r * 0.55);
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{}\" fill=\"{}\"/>",
                fmt(col - w),
                fmt(cy - t / 2.0),
                fmt(w),
                fmt(t),
                fmt(t / 2.0),
                fill
            )
        }
        _ => format!(
            "<circle cx=\"{}\" cy=\"{}\" r=\"{}\" fill=\"{}\"/>",
            fmt(col - r),
            fmt(cy),
            fmt(r),
            fill
        ),
    }
}

/// The `<x:list list="number">` marker for a 1-based counter at a nesting level:
/// decimal, lower-alpha, then lower-roman, cycling every three levels.
pub(super) fn number_marker(level: usize, n: u32) -> String {
    match level % 3 {
        0 => format!("{n}."),
        1 => format!("{}.", alpha_lower(n)),
        _ => format!("{}.", roman_lower(n)),
    }
}

/// Bijective base-26 letters: 1→a, 26→z, 27→aa (spreadsheet-column style).
pub(super) fn alpha_lower(mut n: u32) -> String {
    let mut s = String::new();
    while n > 0 {
        n -= 1;
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        n /= 26;
    }
    if s.is_empty() {
        s.push('0'); // n == 0 shouldn't occur, but never emit an empty marker
    }
    s
}

/// Lowercase Roman numerals for a positive counter (0 falls back to "0").
pub(super) fn roman_lower(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, sym) in TABLE {
        while n >= v {
            s.push_str(sym);
            n -= v;
        }
    }
    s
}
