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
/// A cell property resolved cell › row (`<x:tr>` attrs) › column (`<x:col>`);
/// the caller supplies the header default and the table's below that.
fn cell_prop<'a, 'i>(
    cell: roxmltree::Node<'a, 'i>,
    row: roxmltree::Node<'a, 'i>,
    col: Option<roxmltree::Node<'a, 'i>>,
    name: &str,
) -> Option<&'a str> {
    cell.attribute(name)
        .or_else(|| row.attribute(name))
        .or_else(|| col.and_then(|c| c.attribute(name)))
}

pub(super) fn emit_table(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    // Create-outlines (§6.12) is all-or-nothing per element: a table whose
    // every cell traces emits paths, any other emits live text
    let tp: super::elements::Table = params_of(node);
    if tp.outline || ctx.force_outline.get() {
        let mut traced = String::new();
        if emit_table_as(node, &mut traced, ctx, true) {
            out.push_str(&traced);
            return;
        }
    }
    emit_table_as(node, out, ctx, false);
}

/// The table, live or (`outline`) traced; `false` when a cell's line could not
/// be traced — the miss recorded for `to_path`, `out` to be discarded.
fn emit_table_as(node: roxmltree::Node, out: &mut String, ctx: &Ctx, outline: bool) -> bool {
    let m = ctx.m;
    let style = style_from(node, ctx);
    let tp: super::elements::Table = params_of(node);
    let text_fill = ctx.resolve_var(&tp.fill).into_owned();
    let grid = ctx.paint(&tp.stroke);
    let grid_w = tp.stroke_width.max(0.0);
    let body_bg = tp.cell_fill.as_deref().map(|v| ctx.paint(v));
    let header_bg = ctx.paint(&tp.header_fill);
    // zebra striping: an alternate background for every other BODY row (rows with
    // no header cell); the first data row is unstriped.
    let stripe = tp.stripe.as_deref().map(|v| ctx.paint(v));
    let pad = tp.cell_padding.max(0.0);
    let default_align = tp.align.as_str();
    let default_valign = tp.valign.as_str();
    let default_rtl = tp.direction == super::elements::DirectionParam::Rtl;
    let row_min = tp.row_min_height;

    // block geometry: `in="#rect"` bbox, else x/y/width
    let (mut x, mut y, mut width) = (tp.x, tp.y, tp.width);
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
    // column styles: one <x:col> per column, in order (HTML's <col>), each a
    // set of cell defaults for that column; a cell property resolves
    // cell › row (<x:tr> attrs) › column › header default › table
    let col_styles: Vec<roxmltree::Node> = node
        .children()
        .filter(|c| c.tag_name().namespace() == Some(XSVG_NS) && c.tag_name().name() == "col")
        .collect();
    let ncols = rows
        .iter()
        .map(|r| r.children().filter(is_cell).count())
        .max()
        .unwrap_or(0);
    if ncols == 0 {
        out.push_str("<!-- xsvg: <x:table> has no rows/cells -->");
        return !outline;
    }
    let colw = resolve_cols(tp.cols.as_deref(), ncols, width);
    let mut colx = vec![x; ncols + 1];
    for i in 0..ncols {
        colx[i + 1] = colx[i] + colw[i];
    }

    // per-cell text style: the table style, with `<x:th>` defaulting to bold,
    // then the column's, the row's and the cell's own font-* in turn.
    // a cell's style: cell › row › column › a header cell's bold › the table
    // (query.rs, the same levels a query reads)
    let cell_style = |cell: roxmltree::Node, row: roxmltree::Node, _ci: usize| {
        overlay(&cell_levels(cell, row, node, Some(ctx.theme)), &style)
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
        rtl: false,
    };

    // pass 1: row heights = tallest wrapped cell + padding, floored. Each cell
    // measures with its own (possibly overridden) font metrics.
    let mut row_h = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut h = 0.0f64;
        for (ci, cell) in row.children().filter(is_cell).enumerate() {
            let cst = cell_style(cell, *row, ci);
            let cfm = m.font_metrics(&cst, cst.size);
            // a cell is a run leaf: styled runs and <tbreak/> forced breaks
            // lay out like a textbox, and the row takes the block's cap band
            // (mixed sizes make a line stand as tall as its largest run)
            let (segments, styles, _) = collect_runs(cell, &cst);
            let plain: String = segments.iter().map(|(t, _)| t.as_str()).collect();
            let bh = if plain.trim().is_empty() || content_w(ci) <= 0.0 {
                cfm.cap_height + cfm.descent
            } else {
                let lines = layout_area_runs(&segments, &styles, &measure_spec(ci), m).lines;
                match (lines.first(), lines.last()) {
                    (Some(a), Some(z)) => {
                        m.font_metrics(&cst, a.size).cap_height
                            + (z.baseline - a.baseline)
                            + m.font_metrics(&cst, z.size).descent
                    }
                    _ => cfm.cap_height + cfm.descent,
                }
            };
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
    //
    // The wrapper carries the table's own span as well: cell-level mapping is what a
    // click wants, but an incremental re-emit needs the element it replaces to be
    // findable by the range `fragment_range` reports (docs/Incremental.md §4).
    let filter = effect_attr(node, out, ctx, || None);
    out.push_str("<g");
    out.push_str(&filter);
    out.push_str(&pos_attr(node, ctx));
    // the table's own `transform` rides on the wrapper, as a textbox's does on
    // its <text>: cells are laid out in the table's user space, so rotating or
    // scaling the table is this transform (found live by the editor: a
    // transform on a table was consumed and never re-emitted — immovable)
    out.push_str(&super::transform_attr(node));
    out.push_str(&ctx.carry_at(node.range().start, false)); // `to_path`: what it keeps
    out.push('>');
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
            let bg = cell_prop(cell, *row, col_styles.get(ci).copied(), "bg")
                .map(|v| ctx.paint(v))
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

            let st = cell_style(cell, *row, ci);
            let (segments, styles, emits) = collect_runs(cell, &st);
            let plain: String = segments.iter().map(|(t, _)| t.as_str()).collect();
            // an empty cell lays its one blank line — the row an editor's
            // caret sits on, as an empty box's does (§6.4); outlined, nothing
            let blank = plain.trim().is_empty();
            if !(blank && outline) {
                let fill = level_value(&cell_levels(cell, *row, node, Some(ctx.theme)), "fill")
                    .map(|v| ctx.resolve_var(v).into_owned())
                    .unwrap_or_else(|| text_fill.clone());
                let spec = AreaSpec {
                    x: cx,
                    y: cy,
                    width: cw,
                    height: ch,
                    padding: pad,
                    align: Align::parse(
                        cell_prop(cell, *row, col_styles.get(ci).copied(), "align")
                            .unwrap_or(default_align),
                    ),
                    valign: VAlign::parse(
                        cell_prop(cell, *row, col_styles.get(ci).copied(), "valign")
                            .unwrap_or(default_valign),
                    ),
                    fit: Fit::None,
                    text_overflow: TextOverflow::Clip,
                    text_indent: 0.0,
                    // cell › row › column › table, like its alignment
                    rtl: cell_prop(cell, *row, col_styles.get(ci).copied(), "direction")
                        .map_or(default_rtl, |d| d == "rtl"),
                };
                let layout = layout_area_runs(&segments, &styles, &spec, m);
                emit_run_backdrops(
                    out,
                    &layout.lines,
                    layout.anchor.svg(),
                    layout.rtl,
                    &st,
                    layout.font_size,
                    &fill,
                    &emits,
                    Backdrops::All,
                    m,
                    ctx,
                );
                if outline {
                    let Some((paths, _)) = trace_lines(
                        &layout.lines,
                        layout.anchor,
                        layout.rtl,
                        &st,
                        layout.font_size,
                        1.0,
                        &emits,
                        m,
                        ctx.outliner,
                        ctx,
                    ) else {
                        return false;
                    };
                    push_painted_paths(out, &paths, &fill, "", "", ctx);
                    out.push_str("</g>");
                    continue;
                }
                out.push_str(&format!("<text text-anchor=\"{}\"", layout.anchor.svg()));
                if layout.rtl {
                    out.push_str(" direction=\"rtl\"");
                }
                push_font_attrs(out, &st, layout.font_size, &fill, ctx);
                // the cell's spacing (cell › row › column › table), as a box's
                if st.letter_spacing != 0.0 {
                    out.push_str(&format!(" letter-spacing=\"{}\"", fmt(st.letter_spacing)));
                }
                if st.word_spacing != 0.0 {
                    out.push_str(&format!(" word-spacing=\"{}\"", fmt(st.word_spacing)));
                }
                out.push('>');
                for line in &layout.lines {
                    emit_line(out, line, &st, layout.font_size, 1.0, m, &emits, ctx);
                }
                out.push_str("</text>");
            }
            out.push_str("</g>");
        }
    }
    out.push_str("</g>");
    true
}
