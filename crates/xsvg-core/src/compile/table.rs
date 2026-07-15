//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// Resolve `<x:table cols>` into `ncols` pixel widths spanning `total`. Each token
/// is a fixed length (`80`) or a flex weight (`*` = 1, `2*` = 2); flex columns
/// split whatever remains after the fixed ones. Missing/blank tokens are flex-1,
/// so no `cols` at all yields even columns.
pub(super) fn resolve_cols(spec: Option<&str>, ncols: usize, total: f64) -> Vec<f64> {
    let mut fixed = vec![0.0f64; ncols];
    let mut flex = vec![0.0f64; ncols];
    if let Some(s) = spec {
        for (i, tok) in s.split_whitespace().take(ncols).enumerate() {
            if let Some(w) = tok.strip_suffix('*') {
                flex[i] = if w.is_empty() {
                    1.0
                } else {
                    parse_num(w).unwrap_or(1.0).max(0.0)
                };
            } else if let Some(px) = parse_num(tok) {
                fixed[i] = px.max(0.0);
            } else {
                flex[i] = 1.0;
            }
        }
    }
    for i in 0..ncols {
        if fixed[i] == 0.0 && flex[i] == 0.0 {
            flex[i] = 1.0; // unspecified column → flex 1 (even split)
        }
    }
    let fixed_sum: f64 = fixed.iter().sum();
    let flex_sum: f64 = flex.iter().sum();
    let remaining = (total - fixed_sum).max(0.0);
    (0..ncols)
        .map(|i| {
            if flex[i] > 0.0 && flex_sum > 0.0 {
                remaining * flex[i] / flex_sum
            } else {
                fixed[i]
            }
        })
        .collect()
}

/// `<x:table>` (§6.15): a grid of `<x:tr>` rows of `<x:td>`/`<x:th>` cells. Column
/// widths are author-set (`cols`, else even); **row heights grow to fit content** —
/// each cell wraps its text to its column and the tallest cell sets the row — the
/// presentation-table model (Slides/Canva), baked to plain `<rect>`s + `<text>`.
pub(super) fn emit_table(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let m = ctx.m;
    let style = style_from(node);
    let text_fill = resolve_var(node.attribute("fill").unwrap_or("#0f172a")).into_owned();
    let grid = resolve_var(node.attribute("stroke").unwrap_or("#cbd5e1")).into_owned();
    let grid_w = attr_num(node, "stroke-width", 1.0).max(0.0);
    let body_bg = node
        .attribute("cell-fill")
        .map(|v| resolve_var(v).into_owned());
    let header_bg = resolve_var(node.attribute("header-fill").unwrap_or("#f1f5f9")).into_owned();
    // zebra striping: an alternate background for every other BODY row (rows with
    // no header cell); the first data row is unstriped.
    let stripe = node
        .attribute("stripe")
        .map(|v| resolve_var(v).into_owned());
    let pad = attr_num(node, "cell-padding", 8.0).max(0.0);
    let default_align = node.attribute("align").unwrap_or("start");
    let default_valign = node.attribute("valign").unwrap_or("middle");
    let row_min = attr_num(node, "row-min-height", 0.0);

    // block geometry: `in="#rect"` bbox, else x/y/width
    let (mut x, mut y, mut width) = (
        attr_num(node, "x", 0.0),
        attr_num(node, "y", 0.0),
        attr_num(node, "width", 400.0),
    );
    if let Some(r) = node.attribute("in") {
        if let Some(bb) = ref_geometry(node, r, ctx)
            .ok()
            .and_then(|d| svg_path_bbox(&d))
        {
            x = bb.x0;
            y = bb.y0;
            width = bb.width();
        }
    }

    let is_cell = |c: &roxmltree::Node| {
        c.tag_name().namespace() == Some(XSVG_NS) && matches!(c.tag_name().name(), "td" | "th")
    };
    let rows: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "tr")
        .collect();
    let ncols = rows
        .iter()
        .map(|r| r.children().filter(is_cell).count())
        .max()
        .unwrap_or(0);
    if ncols == 0 {
        out.push_str("<!-- xsvg: <x:table> has no rows/cells -->");
        return;
    }
    let colw = resolve_cols(node.attribute("cols"), ncols, width);
    let mut colx = vec![x; ncols + 1];
    for i in 0..ncols {
        colx[i + 1] = colx[i] + colw[i];
    }

    // per-cell text style: bold for header cells, else the table style
    // per-cell text style: the table style, with `<x:th>` defaulting to bold and
    // any explicit font-* on the cell overriding the table.
    let cell_style = |cell: roxmltree::Node| {
        let mut st = style.clone();
        if cell.tag_name().name() == "th" {
            st.weight = "700".to_string();
        }
        if let Some(v) = cell.attribute("font-family") {
            st.family = v.to_string();
        }
        if let Some(v) = cell
            .attribute("font-size")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.size = v;
        }
        if let Some(v) = cell.attribute("font-weight") {
            st.weight = v.to_string();
        }
        if let Some(v) = cell.attribute("font-style") {
            st.style = v.to_string();
        }
        if let Some(v) = cell
            .attribute("line-height")
            .and_then(parse_num)
            .filter(|n| *n > 0.0)
        {
            st.line_height = v;
        }
        st
    };
    // content width available for a cell's text
    let content_w = |ci: usize| (colw[ci] - 2.0 * pad).max(0.0);
    let measure_spec = |ci: usize| AreaSpec {
        x: 0.0,
        y: 0.0,
        width: colw[ci],
        height: 1e5, // tall enough that nothing clips → true line count
        padding: pad,
        align: Align::Start,
        valign: VAlign::Top,
        fit: Fit::None,
        text_overflow: TextOverflow::Clip,
        text_indent: 0.0,
    };

    // pass 1: row heights = tallest wrapped cell + padding, floored. Each cell
    // measures with its own (possibly overridden) font metrics.
    let mut row_h = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut h = 0.0f64;
        for (ci, cell) in row.children().filter(is_cell).enumerate() {
            let cst = cell_style(cell);
            let cfm = m.font_metrics(&cst, cst.size);
            let text = collect_text(cell);
            let lines = if text.trim().is_empty() || content_w(ci) <= 0.0 {
                1
            } else {
                layout_area(&text, &cst, &measure_spec(ci), m)
                    .lines
                    .len()
                    .max(1)
            };
            let bh =
                cfm.cap_height + cfm.descent + (lines - 1) as f64 * (cst.size * cst.line_height);
            h = h.max(bh + 2.0 * pad);
        }
        row_h.push(h.max(row_min));
    }
    let mut rowy = vec![y; rows.len() + 1];
    for i in 0..rows.len() {
        rowy[i + 1] = rowy[i] + row_h[i];
    }

    // Pass 2: one `<g>` per cell (background rect + its text), so the source map
    // resolves a click to the individual `<x:td>`/`<x:th>` — not the whole table.
    // Cells don't overlap, so emitting each cell's bg then text keeps the paint
    // order correct without a separate backgrounds pass.
    out.push_str("<g>");
    let base = [EmitAttrs::default()];
    let mut body_row = 0usize;
    for (ri, row) in rows.iter().enumerate() {
        let header_row = row
            .children()
            .filter(is_cell)
            .any(|c| c.tag_name().name() == "th");
        let striped = stripe.is_some() && !header_row && body_row % 2 == 1;
        if !header_row {
            body_row += 1;
        }
        for (ci, cell) in row.children().filter(is_cell).enumerate() {
            let header = cell.tag_name().name() == "th";
            let (cx, cw, cy, ch) = (colx[ci], colw[ci], rowy[ri], row_h[ri]);
            let bg = cell
                .attribute("bg")
                .map(|v| resolve_var(v).into_owned())
                .or_else(|| {
                    if header {
                        Some(header_bg.clone())
                    } else if striped {
                        stripe.clone()
                    } else {
                        body_bg.clone()
                    }
                });
            out.push_str("<g");
            out.push_str(&pos_attr(cell, ctx));
            out.push('>');
            out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"",
                fmt(cx),
                fmt(cy),
                fmt(cw),
                fmt(ch),
                bg.as_deref().unwrap_or("none")
            ));
            if grid_w > 0.0 {
                out.push_str(&format!(
                    " stroke=\"{grid}\" stroke-width=\"{}\"",
                    fmt(grid_w)
                ));
            }
            out.push_str("/>");

            let text = collect_text(cell);
            if !text.trim().is_empty() {
                let st = cell_style(cell);
                let fill = cell
                    .attribute("fill")
                    .map(|v| resolve_var(v).into_owned())
                    .unwrap_or_else(|| text_fill.clone());
                let spec = AreaSpec {
                    x: cx,
                    y: cy,
                    width: cw,
                    height: ch,
                    padding: pad,
                    align: Align::parse(cell.attribute("align").unwrap_or(default_align)),
                    valign: VAlign::parse(cell.attribute("valign").unwrap_or(default_valign)),
                    fit: Fit::None,
                    text_overflow: TextOverflow::Clip,
                    text_indent: 0.0,
                };
                let layout = layout_area(&text, &st, &spec, m);
                out.push_str(&format!("<text text-anchor=\"{}\"", layout.anchor.svg()));
                push_font_attrs(out, &st, layout.font_size, &fill);
                out.push('>');
                for line in &layout.lines {
                    emit_line(out, line, &st, layout.font_size, 1.0, m, &base);
                }
                out.push_str("</text>");
            }
            out.push_str("</g>");
        }
    }
    out.push_str("</g>");
}
