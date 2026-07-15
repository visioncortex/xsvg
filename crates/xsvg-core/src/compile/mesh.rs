//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::*;

/// `<x:mesh>` (§8.2) — a quad-dominant mesh gradient with per-corner colors and
/// cracks, lowered by the two-stage pipeline: (1) rasterize the mesh in memory
/// (linear-light, crack-respecting region labels), (2) refit each region with a
/// seam-free shared-vertex GridField grown until the residual passes the
/// profile tolerance, then serialize each region as a **tiny PNG** placed so
/// its texel centers land on the grid vertices — the renderer's own bilinear
/// image filter reconstructs the field (a single patch is exactly a stretched
/// 2×2). Regions are clipped by the exact union of their face polygons
/// (nonzero), so cracks stay geometry-sharp regardless of raster resolution.
pub(super) fn emit_mesh(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    use crate::gradient::Mesh;

    // ---- parse: grid sugar (cols/rows attributes) or points= + <x:face>
    let mut mesh = Mesh::default();
    let mut markers = String::new();
    let grid_sugar = node.attribute("cols").is_some() || node.attribute("rows").is_some();
    if grid_sugar {
        // <x:mesh x y width height cols rows fill="(cols+1)·(rows+1) vertex
        // colors, row-major"/> — per-VERTEX colors, smooth by construction
        // (cracks are the indexed form's job)
        let cols = attr_num(node, "cols", 0.0) as usize;
        let rows = attr_num(node, "rows", 0.0) as usize;
        let (gx, gy) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
        let (gw, gh) = (attr_num(node, "width", 0.0), attr_num(node, "height", 0.0));
        let cols_ok = (1..=64).contains(&cols) && (1..=64).contains(&rows);
        let vcols: Vec<(crate::gradient::LinRgb, f32)> = node
            .attribute("fill")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(parse_hex_color)
            .collect();
        if !cols_ok || gw <= 0.0 || gh <= 0.0 || vcols.len() != (cols + 1) * (rows + 1) {
            out.push_str(
                "<!-- xsvg: <x:mesh> grid form needs cols/rows in 1..64, positive width/height, and (cols+1)*(rows+1) fill colors -->",
            );
            return;
        }
        for j in 0..=rows {
            for i in 0..=cols {
                mesh.add_vertex(
                    (gx + gw * i as f64 / cols as f64) as f32,
                    (gy + gh * j as f64 / rows as f64) as f32,
                );
            }
        }
        let vid = |i: usize, j: usize| (j * (cols + 1) + i) as u32;
        for j in 0..rows {
            for i in 0..cols {
                let q = [vid(i, j), vid(i + 1, j), vid(i + 1, j + 1), vid(i, j + 1)];
                mesh.add_quad_a(
                    q,
                    [
                        vcols[q[0] as usize].0,
                        vcols[q[1] as usize].0,
                        vcols[q[2] as usize].0,
                        vcols[q[3] as usize].0,
                    ],
                    [
                        vcols[q[0] as usize].1,
                        vcols[q[1] as usize].1,
                        vcols[q[2] as usize].1,
                        vcols[q[3] as usize].1,
                    ],
                );
            }
        }
    }
    if !grid_sugar {
        // shared vertices in SVG's own polygon `points` syntax
        let nums: Vec<f32> = node
            .attribute("points")
            .unwrap_or("")
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(|t| t.parse::<f32>().ok().filter(|v| v.is_finite()))
            .collect();
        for pair in nums.chunks_exact(2) {
            mesh.add_vertex(pair[0], pair[1]);
        }
    }
    let nv = mesh.verts.len() as u32;
    for child in node.children().filter(|c| c.is_element()) {
        if grid_sugar || child.tag_name().name() != "face" {
            continue;
        }
        let idx: Vec<u32> = child
            .attribute("v")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        let cols: Vec<(crate::gradient::LinRgb, f32)> = child
            .attribute("fill")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(parse_hex_color)
            .collect();
        let ok_idx = (idx.len() == 3 || idx.len() == 4) && idx.iter().all(|&i| i < nv);
        let ok_col = cols.len() == idx.len() || cols.len() == 1;
        if !ok_idx || !ok_col || cols.is_empty() {
            markers.push_str("<!-- xsvg: <x:face> skipped (bad indices or colors) -->");
            continue;
        }
        let col = |k: usize| if cols.len() == 1 { cols[0] } else { cols[k] };
        if idx.len() == 4 {
            mesh.add_quad_a(
                [idx[0], idx[1], idx[2], idx[3]],
                [col(0).0, col(1).0, col(2).0, col(3).0],
                [col(0).1, col(1).1, col(2).1, col(3).1],
            );
        } else {
            mesh.add_tri_a(
                [idx[0], idx[1], idx[2]],
                [col(0).0, col(1).0, col(2).0],
                [col(0).1, col(1).1, col(2).1],
            );
        }
    }
    out.push_str(&markers);
    if mesh.faces.is_empty() {
        out.push_str("<!-- xsvg: <x:mesh> no usable faces -->");
        return;
    }

    out.push_str("<g");
    copy_attrs(
        node,
        out,
        &[
            "cols", "rows", "x", "y", "width", "height", "fill", "points",
        ],
    );
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    if !lower_mesh(&mesh, node.range().start, out, ctx) {
        out.push_str("<!-- xsvg: <x:mesh> degenerate extent -->");
    }
    out.push_str("</g>");
}

/// The shared mesh lowering (§8.2 stage 1 + 2): rasterize at a profile-graded
/// resolution, fit each crack region with a grown grid field, and emit one
/// texel-aligned tiny PNG (or plain path for constant regions) per region.
/// `seed` namespaces the clip ids (the caller's source position). Returns
/// `false` on a degenerate extent (caller emits its marker).
pub(super) fn lower_mesh(mesh: &crate::gradient::Mesh, seed: usize, out: &mut String, ctx: &Ctx) -> bool {
    use crate::gradient;
    use crate::gradient::{fit_field, fit_grid, texel_placement, Dof};

    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &mesh.verts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if !(x1 > x0 && y1 > y0) {
        return false;
    }
    let (max_px, min_px) = match ctx.quality {
        QualityProfile::Fast => (64.0f32, 24.0f32),
        QualityProfile::Balanced => (128.0, 32.0),
        QualityProfile::Highest | QualityProfile::Raster => (384.0, 48.0),
    };
    // resolution from the LONG axis, but never starve the short one — a thin
    // strip still needs rows for its cross-axis fit
    let (dim_max, dim_min) = ((x1 - x0).max(y1 - y0), (x1 - x0).min(y1 - y0));
    let scale = (dim_max / max_px).min(dim_min / min_px).max(1e-6);
    let (w, h) = (
        (((x1 - x0) / scale).ceil() as usize).max(1),
        (((y1 - y0) / scale).ceil() as usize).max(1),
    );
    let raster = mesh.rasterize(w, h, (x0, y0), scale, 1e-3);
    let rgba = raster.to_rgba8();
    let opaque = raster.fully_opaque();

    // per-region pixel index lists + pixel bboxes
    let mut region_px: Vec<Vec<u32>> = vec![Vec::new(); raster.regions];
    let mut bbox: Vec<(u32, u32, u32, u32)> = vec![(u32::MAX, u32::MAX, 0, 0); raster.regions];
    for (i, &l) in raster.labels.iter().enumerate() {
        if l == gradient::mesh::NONE {
            continue;
        }
        region_px[l as usize].push(i as u32);
        let (px, py) = ((i % w) as u32, (i / w) as u32);
        let b = &mut bbox[l as usize];
        b.0 = b.0.min(px);
        b.1 = b.1.min(py);
        b.2 = b.2.max(px);
        b.3 = b.3.max(py);
    }

    // rmse tolerance (sRGB units) and grid cap by profile
    let (tol, cap) = match ctx.quality {
        QualityProfile::Fast => (4.0f32, 10usize),
        QualityProfile::Balanced => (1.5, 24),
        QualityProfile::Highest | QualityProfile::Raster => (0.5, 48),
    };

    for r in 0..raster.regions {
        if region_px[r].is_empty() {
            continue;
        }
        // exact clip geometry: union of the region's face polygons (nonzero)
        let mut clip_d = String::new();
        for (f, face) in mesh.faces.iter().enumerate() {
            if raster.face_regions[f] != r as u32 {
                continue;
            }
            let n = face.arity();
            for c in 0..n {
                let (px, py) = mesh.verts[face.v[c] as usize];
                clip_d.push(if c == 0 { 'M' } else { 'L' });
                clip_d.push_str(&format!("{},{}", fmt(px as f64), fmt(py as f64)));
            }
            clip_d.push('Z');
        }

        let single = fit_field(&region_px[r], w, &rgba, 2.0);
        if single.dof == Dof::Solid {
            let c = single.corners[0];
            let a = (c[3] / 255.0).clamp(0.0, 1.0);
            let opacity = if a < 254.5 / 255.0 {
                format!(" fill-opacity=\"{}\"", fmt(a as f64))
            } else {
                String::new()
            };
            out.push_str(&format!(
                "<path fill=\"#{:02x}{:02x}{:02x}\"{opacity} d=\"{clip_d}\"/>",
                c[0].round().clamp(0.0, 255.0) as u8,
                c[1].round().clamp(0.0, 255.0) as u8,
                c[2].round().clamp(0.0, 255.0) as u8
            ));
            continue;
        }

        // grow the shared-vertex grid until the residual passes the tolerance.
        // The gx:gy aspect comes from the FIELD's measured directional
        // variation (sum of |∂/∂x| vs |∂/∂y| over the region), not the bbox —
        // a wide region with purely vertical structure gets rows, not
        // stretched-wide column padding, so texels stay meaningful. (A greedy
        // per-axis search stalls here: row counts that straddle an interior
        // color kink are transiently WORSE, which lets no-op column growth win
        // ties all the way to the cap.)
        let (bx0, by0, bx1, by1) = bbox[r];
        let (mut vx, mut vy) = (1e-6f64, 1e-6f64);
        for &idx in &region_px[r] {
            let i = idx as usize;
            let (px, py) = (i % w, i / w);
            if px + 1 < w && raster.labels[i + 1] == r as u32 {
                for c in 0..4 {
                    vx += (rgba[(i + 1) * 4 + c] as f64 - rgba[i * 4 + c] as f64).abs();
                }
            }
            if py + 1 < h && raster.labels[i + w] == r as u32 {
                for c in 0..4 {
                    vy += (rgba[(i + w) * 4 + c] as f64 - rgba[i * 4 + c] as f64).abs();
                }
            }
        }
        // blend the field ratio with the bbox aspect: the field says where the
        // STRUCTURE is, the bbox keeps enough of the other axis that localized
        // cross-structure (a rim hugging a wide region's end caps) still gets
        // texels the global sum can't see
        let field_s = (vx / vy).sqrt();
        let bbox_s = (((bx1 - bx0).max(1) as f64) / ((by1 - by0).max(1) as f64)).sqrt();
        let s = ((field_s * bbox_s).sqrt().clamp(1.0 / 6.0, 6.0)) as f32;
        let mut best = None;
        for g in [1usize, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48] {
            let gx = ((g as f32 * s).round() as usize).clamp(1, cap);
            let gy = ((g as f32 / s).round() as usize).clamp(1, cap);
            let grid = fit_grid(&region_px[r], w, &rgba, gx, gy);
            let done = grid.rmse <= tol || (gx >= cap && gy >= cap);
            best = Some(grid);
            if done {
                break;
            }
        }
        let grid = best.unwrap();

        // tiny PNG: (gx+1)×(gy+1) texels, one per grid vertex; opaque meshes
        // stay RGB, feathered ones carry the alpha channel
        let (tw, th) = (grid.gx + 1, grid.gy + 1);
        let ch = if opaque { 3 } else { 4 };
        let mut px = Vec::with_capacity(tw * th * ch);
        for vert in &grid.verts {
            for c in 0..ch {
                px.push(vert[c].round().clamp(0.0, 255.0) as u8);
            }
        }
        let png = if opaque {
            gradient::png::encode_rgb_png(tw as u32, th as u32, &px)
        } else {
            gradient::png::encode_rgba_png(tw as u32, th as u32, &px)
        };
        let (ix, iy, iw, ih) = texel_placement(bx0, by0, bx1, by1, tw, th);
        // raster pixel space -> user units
        let (ux, uy) = (x0 as f64 + ix * scale as f64, y0 as f64 + iy * scale as f64);
        let (uw, uh) = (iw * scale as f64, ih * scale as f64);
        let cid = format!("x-mesh-{seed}-{r}");
        out.push_str(&format!(
            "<clipPath id=\"{cid}\"><path d=\"{clip_d}\"/></clipPath><image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" clip-path=\"url(#{cid})\" href=\"data:image/png;base64,{}\"/>",
            fmt(ux),
            fmt(uy),
            fmt(uw),
            fmt(uh),
            gradient::base64::encode(&png)
        ));
    }
    true
}

/// The `fill="url(#id)"` target, when it is an SVG 2 `<meshgradient>` — the
/// Inkscape mesh dialect no browser renders; xsvg compiles it (§8.2).
pub(super) fn mesh_fill_target<'a>(
    node: roxmltree::Node<'a, 'a>,
) -> Option<roxmltree::Node<'a, 'a>> {
    let fill = node.attribute("fill")?;
    let id = fill.strip_prefix("url(#")?.strip_suffix(')')?;
    let target = resolve_ref(node, id)?;
    (target.tag_name().name() == "meshgradient").then_some(target)
}

/// Lower a shape whose fill references an SVG 2 `<meshgradient>`: tessellate
/// the Coons patches into the straight-quad mesh (polycurve → points), lower
/// it through the standard pipeline, clip by the shape's geometry, and re-emit
/// the stroke (if any) on top. Returns `false` (caller falls through to normal
/// emission, mesh unrendered as in any browser) when the dialect doesn't parse.
pub(super) fn emit_meshgradient_fill(
    node: roxmltree::Node,
    mg: roxmltree::Node,
    out: &mut String,
    ctx: &Ctx,
) -> bool {
    let Some(shape_d) = shape_to_path_d(node) else {
        return false;
    };
    let tess = match ctx.quality {
        QualityProfile::Fast => 8usize,
        QualityProfile::Balanced => 12,
        QualityProfile::Highest | QualityProfile::Raster => 20,
    };
    // objectBoundingBox units: patch coordinates live in the unit square of
    // the shape's bbox
    let unit_map = if mg.attribute("gradientUnits") == Some("objectBoundingBox") {
        let Some(bb) = svg_path_bbox(&shape_d) else {
            return false;
        };
        crate::kurbo::Affine::translate((bb.x0, bb.y0))
            * crate::kurbo::Affine::scale_non_uniform(bb.width(), bb.height())
    } else {
        crate::kurbo::Affine::IDENTITY
    };
    let Some(mesh) = parse_meshgradient(mg, tess, unit_map) else {
        out.push_str("<!-- xsvg: meshgradient fill left live (malformed dialect) -->");
        return false;
    };
    let pos = node.range().start;
    out.push_str(&format!(
        "<clipPath id=\"x-mgc-{pos}\"><path d=\"{shape_d}\"/></clipPath><g clip-path=\"url(#x-mgc-{pos})\""
    ));
    if let Some(t) = node.attribute("transform") {
        out.push_str(" transform=\"");
        push_escaped(out, t, true);
        out.push('"');
    }
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    if !lower_mesh(&mesh, pos, out, ctx) {
        out.push_str("<!-- xsvg: meshgradient degenerate extent -->");
    }
    out.push_str("</g>");
    // the stroke still paints over the mesh fill
    if node.attribute("stroke").is_some() {
        out.push_str("<path");
        copy_attrs(
            node,
            out,
            &[
                "fill", "d", "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "points",
                "x1", "y1", "x2", "y2",
            ],
        );
        out.push_str(&format!(" fill=\"none\" d=\"{shape_d}\"/>"));
    }
    true
}

/// Parse the SVG 2 / Inkscape `<meshgradient>` dialect into a tessellated
/// mesh: `meshrow`s of `meshpatch`es whose `<stop>`s carry one cubic (`c`/`C`)
/// or line (`l`/`L`) edge each plus a corner color, with the standard
/// inheritance — a patch after the first in a row inherits its left edge
/// (reversed) from the neighbour, rows after the first inherit top edges from
/// the row above, and inherited corners keep their colors. Colors attach to
/// each stop edge's START corner on the very first patch, and to the END
/// corner elsewhere (inherited corners win). `gradientTransform` is honored;
/// `gradientUnits="objectBoundingBox"` and `type="bicubic"` degrade (bicubic
/// renders as bilinear). Returns `None` if anything fails to parse.
pub(super) fn parse_meshgradient(
    mg: roxmltree::Node,
    tess: usize,
    unit_map: crate::kurbo::Affine,
) -> Option<crate::gradient::Mesh> {
    use crate::gradient::{reverse_edge, CoonsPatch, LinRgb, Mesh};
    let eased = mg.attribute("type") == Some("bicubic");
    let origin = (attr_num(mg, "x", 0.0) as f32, attr_num(mg, "y", 0.0) as f32);
    let affine = unit_map
        * mg.attribute("gradientTransform")
            .map(parse_transform)
            .unwrap_or(crate::kurbo::Affine::IDENTITY);

    let mut rows: Vec<Vec<CoonsPatch>> = Vec::new();
    for row_el in mg
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "meshrow")
    {
        let r = rows.len();
        let mut row: Vec<CoonsPatch> = Vec::new();
        for patch_el in row_el
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "meshpatch")
        {
            let i = row.len();
            let above = rows.last().and_then(|pr| pr.get(i)).copied();
            let prev = row.last().copied();
            if r > 0 && above.is_none() {
                return None; // ragged rows are not meshes
            }
            let top = (r > 0).then(|| reverse_edge(above.unwrap().edges[2]));
            let left = (i > 0).then(|| reverse_edge(prev.unwrap().edges[1]));

            let mut edges: [Option<[(f32, f32); 4]>; 4] = [top, None, None, left];
            // corners [TL, TR, BR, BL]; inherited ones are fixed
            let mut colors: [Option<(LinRgb, f32)>; 4] = [None; 4];
            if let Some(p) = above {
                colors[0] = Some((p.colors[3], p.alpha[3])); // our TL = above BL
                colors[1] = Some((p.colors[2], p.alpha[2])); // our TR = above BR
            }
            if let Some(p) = prev {
                colors[0] = Some((p.colors[1], p.alpha[1])); // our TL = prev TR
                colors[3] = Some((p.colors[2], p.alpha[2])); // our BL = prev BR
            }

            let missing: Vec<usize> = (0..4).filter(|&k| edges[k].is_none()).collect();
            let stops: Vec<roxmltree::Node> = patch_el
                .children()
                .filter(|c| c.is_element() && c.tag_name().name() == "stop")
                .collect();
            if stops.len() < missing.len() {
                return None;
            }
            // walking start: TL for a parsed top edge, else TR (top inherited)
            let mut cur = match (r > 0, i > 0) {
                (false, false) => origin,
                (false, true) => prev.unwrap().edges[1][0], // prev TR
                (true, _) => top.unwrap()[3],               // TR, after the inherited top
            };
            for (stop, &k) in stops.iter().zip(missing.iter()) {
                let (edge, next) = parse_stop_edge(stop.attribute("path")?, cur)?;
                edges[k] = Some(edge);
                cur = next;
                // this stop's color belongs to the edge's END corner — except on
                // the very first patch, where it is the START corner
                let corner = if r == 0 && i == 0 { k } else { (k + 1) % 4 };
                if colors[corner].is_none() {
                    colors[corner] = Some(stop_color(*stop)?);
                }
            }
            // first patch: the 4th stop's color lands on BL via the ends rule
            // never firing — fill any hole from the stop list in corner order
            if r == 0 && i == 0 {
                for (stop, &k) in stops.iter().zip(missing.iter()) {
                    if colors[k].is_none() {
                        colors[k] = Some(stop_color(*stop)?);
                    }
                }
            }
            if colors.iter().any(|c| c.is_none()) {
                return None;
            }
            row.push(CoonsPatch {
                edges: [edges[0]?, edges[1]?, edges[2]?, edges[3]?],
                colors: [colors[0]?.0, colors[1]?.0, colors[2]?.0, colors[3]?.0],
                alpha: [colors[0]?.1, colors[1]?.1, colors[2]?.1, colors[3]?.1],
                eased,
            });
        }
        if row.is_empty() {
            return None;
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return None;
    }

    let mut mesh = Mesh::default();
    let mut dedup = std::collections::HashMap::new();
    for row in &rows {
        for patch in row {
            let mut p = *patch;
            if affine != crate::kurbo::Affine::IDENTITY {
                for e in &mut p.edges {
                    for pt in e.iter_mut() {
                        let m = affine * crate::kurbo::Point::new(pt.0 as f64, pt.1 as f64);
                        *pt = (m.x as f32, m.y as f32);
                    }
                }
            }
            p.tessellate_into(&mut mesh, tess, &mut dedup);
        }
    }
    Some(mesh)
}

/// One `<stop path>` edge: a single `c`/`C` cubic or `l`/`L` line from `cur`.
/// Returns the edge (4 control points, walking order) and the new current point.
pub(super) fn parse_stop_edge(d: &str, cur: (f32, f32)) -> Option<([(f32, f32); 4], (f32, f32))> {
    let d = d.trim();
    let (cmd, rest) = d.split_at(1);
    let nums: Vec<f32> = rest
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<f32>>>()?;
    let rel = |k: usize| (cur.0 + nums[k], cur.1 + nums[k + 1]);
    let abs = |k: usize| (nums[k], nums[k + 1]);
    match (cmd, nums.len()) {
        ("c", 6) => {
            let e = [cur, rel(0), rel(2), rel(4)];
            Some((e, e[3]))
        }
        ("C", 6) => {
            let e = [cur, abs(0), abs(2), abs(4)];
            Some((e, e[3]))
        }
        ("l", 2) => {
            let e = crate::gradient::line_edge(cur, rel(0));
            Some((e, e[3]))
        }
        ("L", 2) => {
            let e = crate::gradient::line_edge(cur, abs(0));
            Some((e, e[3]))
        }
        _ => None,
    }
}

/// A mesh `<stop>`'s color + alpha: `stop-color` (attribute or `style`
/// declaration — Inkscape's habit), with `stop-opacity` multiplied in.
pub(super) fn stop_color(stop: roxmltree::Node) -> Option<(crate::gradient::LinRgb, f32)> {
    let style_prop = |name: &str| {
        stop.attribute("style")?.split(';').find_map(|kv| {
            let (k, v) = kv.split_once(':')?;
            (k.trim() == name).then(|| v.trim().to_string())
        })
    };
    let color = stop
        .attribute("stop-color")
        .map(str::to_string)
        .or_else(|| style_prop("stop-color"))?;
    let color = resolve_var(color.trim()); // §4.1 color token
    let (rgb, mut a) = parse_hex_color(color.trim())?;
    if let Some(op) = stop
        .attribute("stop-opacity")
        .map(str::to_string)
        .or_else(|| style_prop("stop-opacity"))
    {
        let v: f32 = op.trim().parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        a *= v.clamp(0.0, 1.0);
    }
    Some((rgb, a))
}

/// `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa` → linear-light RGB + straight
/// alpha in [0,1] (the CSS Color 4 hex-alpha forms — mesh feathering's syntax).
pub(super) fn parse_hex_color(s: &str) -> Option<(crate::gradient::LinRgb, f32)> {
    let hex = s.strip_prefix('#')?;
    let byte = |a: u8, b: u8| {
        let hi = (a as char).to_digit(16)?;
        let lo = (b as char).to_digit(16)?;
        Some((hi * 16 + lo) as u8)
    };
    let b = hex.as_bytes();
    let (r, g, bl, a) = match b.len() {
        3 => (byte(b[0], b[0])?, byte(b[1], b[1])?, byte(b[2], b[2])?, 255),
        4 => (
            byte(b[0], b[0])?,
            byte(b[1], b[1])?,
            byte(b[2], b[2])?,
            byte(b[3], b[3])?,
        ),
        6 => (byte(b[0], b[1])?, byte(b[2], b[3])?, byte(b[4], b[5])?, 255),
        8 => (
            byte(b[0], b[1])?,
            byte(b[2], b[3])?,
            byte(b[4], b[5])?,
            byte(b[6], b[7])?,
        ),
        _ => return None,
    };
    Some((
        crate::gradient::RgbColor::new(r, g, bl).to_linear(),
        a as f32 / 255.0,
    ))
}
