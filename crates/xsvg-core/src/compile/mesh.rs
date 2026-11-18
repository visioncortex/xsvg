//! Extracted from the compiler core (see `compile/mod.rs`). `use super::*` pulls in
//! the shared helpers, `Ctx`, and re-exported primitives.

use super::mesh_fit::{FittedMesh, MeshKey};
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
/// Display-resolution compilation samples that same fitted field before the
/// first PNG encoding, for bitmap consumers with coarse texture interpolation.
pub(super) fn emit_mesh(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let mesh = match parse_mesh(node, out) {
        Ok(mesh) => mesh,
        Err(why) => {
            out.push_str(why);
            return;
        }
    };

    // A clipping mesh contributes its exact face polygons, irrespective of
    // paint or alpha. Reuse the parser above and avoid rasterizing any pixels.
    if ctx.clip_geometry.get() {
        out.push_str("<g");
        copy_attrs(
            node,
            out,
            &[
                "cols", "rows", "x", "y", "width", "height", "fill", "points", "units", "filter",
                "opacity",
            ],
            ctx,
        );
        out.push_str(&pos_attr(node, ctx));
        out.push('>');
        // one path: the outline of all the faces together
        let whole = mesh.region_outlines(&vec![0; mesh.faces.len()], 1);
        out.push_str("<path d=\"");
        append_outline(&mesh, &whole[0], out);
        out.push_str("\"/>");
        out.push_str("</g>");
        return;
    }

    open_effect_element(
        node,
        "g",
        out,
        &[
            "cols", "rows", "x", "y", "width", "height", "fill", "points", "units",
        ],
        ctx,
        || None,
    );
    out.push_str(&pos_attr(node, ctx));
    out.push('>');
    if !lower_mesh(&mesh, &gen_id_seed(node, ctx), node, out, ctx) {
        out.push_str("<!-- xsvg: <x:mesh> degenerate extent -->");
    }
    out.push_str("</g>");
}

/// An `<x:mesh>`'s geometry and colours, in its own coordinates: the grid
/// sugar (`cols`/`rows`) or `points` + `<x:face>`. A skipped face leaves its
/// marker in `markers`; `Err` is the marker for a mesh with nothing to draw.
/// Shared by the element itself and by a shape it fills (`fill="url(#id)"`).
pub(super) fn parse_mesh(
    node: roxmltree::Node,
    markers: &mut String,
) -> Result<crate::gradient::Mesh, &'static str> {
    use crate::gradient::Mesh;

    // ---- parse: grid sugar (cols/rows attributes) or points= + <x:face>
    let mut mesh = Mesh::default();
    let grid_sugar = node.attribute("cols").is_some() || node.attribute("rows").is_some();
    if grid_sugar {
        // <x:mesh x y width height cols rows fill="(cols+1)·(rows+1) vertex
        // colors, row-major"/> — per-VERTEX colors, smooth by construction
        // (cracks are the indexed form's job)
        let mp: super::elements::Mesh = params_of(node);
        let cols = mp.cols as usize;
        let rows = mp.rows as usize;
        let (gx, gy) = (mp.x, mp.y);
        let (gw, gh) = (mp.width, mp.height);
        let cols_ok = (1..=64).contains(&cols) && (1..=64).contains(&rows);
        let vcols: Vec<(crate::gradient::LinRgb, f32)> = node
            .attribute("fill")
            .unwrap_or("")
            .split_whitespace()
            .filter_map(parse_hex_color)
            .collect();
        if !cols_ok || gw <= 0.0 || gh <= 0.0 || vcols.len() != (cols + 1) * (rows + 1) {
            return Err(
                "<!-- xsvg: <x:mesh> grid form needs cols/rows in 1..64, positive width/height, and (cols+1)*(rows+1) fill colors -->",
            );
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
    if mesh.faces.is_empty() {
        return Err("<!-- xsvg: <x:mesh> no usable faces -->");
    }
    Ok(mesh)
}

/// The shared mesh lowering (§8.2 stage 1 + 2): fit the mesh — or take the
/// fit the platform kept from the last time this mesh was seen — and write it
/// here: one texel-aligned PNG (or plain path for a constant region) per crack
/// region, each inside its outline. Compact by default; display-resolution
/// output samples the field at its painted size. `seed` namespaces the clip
/// ids (the caller's source position). Returns `false` on a degenerate extent
/// (caller emits its marker).
pub(super) fn lower_mesh(
    mesh: &crate::gradient::Mesh,
    seed: &str,
    node: roxmltree::Node,
    out: &mut String,
    ctx: &Ctx,
) -> bool {
    let Some(fitted) = fitted_mesh(mesh, ctx) else {
        return false;
    };
    emit_fitted(&fitted, seed, node, out, ctx);
    true
}

/// The fit of `mesh` at this compile's quality: from the platform's store
/// when it has one ([`MeshFits`]), fitted here — and offered to the store —
/// when it does not.
fn fitted_mesh(mesh: &crate::gradient::Mesh, ctx: &Ctx) -> Option<std::sync::Arc<FittedMesh>> {
    use super::mesh_fit::{fit, FitClass};
    let class = FitClass::of(ctx.quality);
    if !ctx.meshes.active() {
        return fit(mesh, class).map(std::sync::Arc::new);
    }
    let key = MeshKey::of(mesh, class);
    if let Some(kept) = ctx.meshes.fitted(&key) {
        return Some(kept);
    }
    // a mesh with no extent has no fit, and nothing to keep
    let fitted = std::sync::Arc::new(fit(mesh, class)?);
    ctx.meshes.keep(key, &fitted);
    Some(fitted)
}

/// A mesh definition compiled on its own (§8.2): what an editor shows for it
/// in a palette.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSwatch {
    /// A complete SVG document: the mesh's paint, its `viewBox` the mesh's
    /// bounds. Its ids are derived from [`Self::key`], so swatches of
    /// different meshes can share one DOM.
    pub svg: String,
    /// The mesh's bounds — `x, y, width, height` — in its own coordinates
    /// (for bounding-box units, within the unit square).
    pub bounds: [f64; 4],
    /// The mesh's identity at this quality: unchanged exactly while the
    /// swatch is, so it is what a palette keys and caches its swatches by.
    pub key: MeshKey,
}

/// Compile the mesh definition `node` — an `<x:mesh>` or a `<meshgradient>` —
/// on its own: the paint it is, under its own `transform` /
/// `gradientTransform` and nothing else, in a document of its own. The fit is
/// the one the document's compile makes for a mesh drawn or used in those
/// coordinates, so with a [`MeshFits`] store it is usually already there.
pub(super) fn mesh_swatch(node: roxmltree::Node, ctx: &Ctx) -> Result<MeshSwatch, String> {
    if !is_mesh_paint(node) {
        return Err("not a mesh: expected an <x:mesh> or a <meshgradient>".into());
    }
    let mut said = String::new();
    // no element is being painted, so there is no box: bounding-box units
    // are the unit square they are written in
    let mesh = paint_mesh(node, None, false, &mut said, ctx)
        .ok_or("the mesh has nothing to draw (no usable face, or a malformed dialect)")?;
    let fitted = fitted_mesh(&mesh, ctx).ok_or("the mesh has no extent")?;
    let key = MeshKey::of_quality(&mesh, ctx.quality);
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(x, y) in &mesh.verts {
        x0 = x0.min(x as f64);
        y0 = y0.min(y as f64);
        x1 = x1.max(x as f64);
        y1 = y1.max(y as f64);
    }
    // to the output's precision, so the bounds are the `viewBox` exactly
    // (a tessellated patch lands a few ulps off its corners)
    let bounds = [x0, y0, x1 - x0, y1 - y0].map(|v| (v * 1000.0).round() / 1000.0);
    let mut svg = format!(
        "<svg xmlns=\"{SVG_NS}\" viewBox=\"{} {} {} {}\">",
        fmt(bounds[0]),
        fmt(bounds[1]),
        fmt(bounds[2]),
        fmt(bounds[3])
    );
    emit_fitted(&fitted, &key.to_string(), node, &mut svg, ctx);
    svg.push_str("</svg>");
    Ok(MeshSwatch { svg, bounds, key })
}

/// Write a fit into the document: the half of the lowering that depends on
/// where the mesh is drawn — the clip ids (`seed`) and, at display resolution,
/// the density it is seen at.
fn emit_fitted(
    fitted: &FittedMesh,
    seed: &str,
    node: roxmltree::Node,
    out: &mut String,
    ctx: &Ctx,
) {
    use super::mesh_fit::RegionPaint;
    // one answer for the whole mesh: every region shares its placements
    let density = super::mesh_texture::density(node, ctx);
    let ch = fitted.channels;
    for region in &fitted.regions {
        let (r, clip_d) = (region.index, &region.clip_d);
        match &region.paint {
            RegionPaint::Solid { rgb, alpha } => {
                let opacity = if *alpha < 254.5 / 255.0 {
                    format!(" fill-opacity=\"{}\"", fmt(*alpha as f64))
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "<path fill=\"#{:02x}{:02x}{:02x}\"{opacity} d=\"{clip_d}\"/>",
                    rgb[0], rgb[1], rgb[2]
                ));
            }
            RegionPaint::Texture {
                texels,
                size,
                place,
                compact,
            } => {
                let (ux, uy, uw, uh) = *place;
                // Keep the fitted grid's placement regardless of output texture
                // density. The compact grid is the fit's own stored PNG; a
                // resampled texture is megapixels and is compressed.
                let (pw, ph) = super::mesh_texture::dimensions(density, (uw, uh), *size);
                let resampled;
                let href: &str = if (pw, ph) == *size {
                    compact
                } else {
                    let pixels = super::mesh_texture::interpolate(texels, *size, (pw, ph), ch);
                    resampled = crate::gradient::base64::encode(&super::mesh_texture::encode(
                        pw, ph, &pixels, ch,
                    ));
                    &resampled
                };
                let cid = format!("x-mesh-{seed}-{r}");
                // The texel grid is placed so that *bilinear* reconstruction reproduces the
                // fitted field exactly, so the image must ask for bilinear by name. `auto` is
                // the renderer's choice, and resvg chooses bicubic: measured, that drifts up to
                // ~9/255 off the field, while bilinear lands within 0.5/255 (8-bit rounding).
                //
                // Both spellings, because they reach different consumers. resvg honours only
                // the CSS form (the attribute is parsed and dropped); the attribute survives
                // tools that strip or flatten `style`, and SVG 2 reads it as a CSS value. Where
                // both apply, CSS wins and they agree. A renderer that does not know `smooth`
                // treats it as invalid and keeps `auto` — Chromium renders identically with or
                // without it.
                out.push_str(&format!(
                    "<clipPath id=\"{cid}\"><path d=\"{clip_d}\"/></clipPath><image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" image-rendering=\"smooth\" style=\"image-rendering:smooth\" clip-path=\"url(#{cid})\" href=\"data:image/png;base64,{href}\"/>",
                    fmt(ux),
                    fmt(uy),
                    fmt(uw),
                    fmt(uh),
                ));
            }
        }
    }
}

/// Whether `node` is a mesh a fill may name (§8.2): an SVG 2 `<meshgradient>`
/// — the Inkscape dialect no browser renders — or xsvg's own `<x:mesh>`.
/// Either way the reference is compiled, never live.
pub(super) fn is_mesh_paint(node: roxmltree::Node) -> bool {
    matches!(
        (node.tag_name().namespace(), node.tag_name().name()),
        (Some(SVG_NS), "meshgradient") | (Some(XSVG_NS), "mesh")
    )
}

/// The id a paint value names — `url(#id)`, quotes and a fallback colour
/// tolerated (`url("#id") #888`).
pub(super) fn paint_ref(value: &str) -> Option<&str> {
    let (inner, _fallback) = value.trim().strip_prefix("url(")?.split_once(')')?;
    inner
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .strip_prefix('#')
        .filter(|id| !id.is_empty())
}

/// How an element that may be painted with a mesh gives up its geometry.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Bearer {
    /// a basic shape: its own path, and `fill-rule` is its winding
    Shape,
    /// compiled output read back as one outline: a boolean or an offset
    Outline,
    /// glyph outlines: `<text>`, `<x:textbox>`, `<x:textpath>`
    Text,
}

/// The elements a mesh can paint (§8.2): everything with geometry of its own
/// whose `fill` and `stroke` are ordinary paints.
pub(super) fn mesh_bearer(node: roxmltree::Node) -> Option<Bearer> {
    match (node.tag_name().namespace(), node.tag_name().name()) {
        (
            Some(SVG_NS),
            "path" | "rect" | "circle" | "ellipse" | "polygon" | "polyline" | "line",
        ) => Some(Bearer::Shape),
        (Some(SVG_NS), "text") => Some(Bearer::Text),
        (Some(XSVG_NS), "boolean" | "offset") => Some(Bearer::Outline),
        (Some(XSVG_NS), "textbox" | "textpath") => Some(Bearer::Text),
        _ => None,
    }
}

/// A presentation attribute as the element sees it: its own, else the nearest
/// ancestor's — SVG inheritance, over attributes. The `x:` text elements do
/// not inherit paint (an unset `fill` is black, §6), so theirs is their own.
fn presentation<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    let own_only = matches!(name, "fill" | "stroke")
        && node.tag_name().namespace() == Some(XSVG_NS)
        && mesh_bearer(node) == Some(Bearer::Text);
    if own_only {
        return node.attribute(name);
    }
    node.ancestors().find_map(|a| a.attribute(name))
}

/// The id of the paint `node` is painted with in `prop` (`fill` / `stroke`),
/// when the element is one a mesh can paint. Says nothing about what the id
/// names: [`mesh_paint_target`] and the `dependents` scan each look it up.
pub(super) fn bearer_paint_id<'a>(node: roxmltree::Node<'a, 'a>, prop: &str) -> Option<&'a str> {
    mesh_bearer(node)?;
    paint_ref(presentation(node, prop)?)
}

/// The mesh `node` is painted with in `prop`, if its paint names one.
fn mesh_paint_target<'a>(
    node: roxmltree::Node<'a, 'a>,
    prop: &str,
    ctx: &Ctx,
) -> Option<roxmltree::Node<'a, 'a>> {
    let target = ctx.resolve_ref(node, bearer_paint_id(node, prop)?)?;
    is_mesh_paint(target).then_some(target)
}

/// Why a mesh paint was not lowered.
const NO_GEOMETRY: &str = "<!-- xsvg: mesh paint left live (no geometry to paint) -->";
const OWN_PAINT: &str = "<!-- xsvg: mesh paint left live (runs with their own fill or stroke) -->";
const NON_SCALING: &str = "<!-- xsvg: mesh stroke left live (non-scaling-stroke) -->";
const TOO_MANY_DASHES: &str = "<!-- xsvg: mesh stroke left live (dash pattern too fine) -->";

/// How many dashes a mesh stroke may be outlined into.
const MAX_DASHES: f64 = 20_000.0;

/// The attributes that say how a shape is painted. They go on the one path
/// that carries what is left of the element's own paint; everything else
/// (identity, transform, opacity, clipping, effects) goes on the group.
const PAINT_ATTRS: &[&str] = &[
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-opacity",
    "paint-order",
    "vector-effect",
    "shape-rendering",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "style",
];

/// A basic shape's own geometry attributes: consumed into the path.
const SHAPE_GEOMETRY: &[&str] = &[
    "d", "x", "y", "width", "height", "cx", "cy", "r", "rx", "ry", "points", "x1", "y1", "x2", "y2",
];

/// What the group keeps of an element that is not a basic shape — the rest of
/// its attributes are that element's own vocabulary, already spent on the
/// geometry.
const GROUP_ATTRS: &[&str] = &[
    "id",
    "class",
    "transform",
    "opacity",
    "clip-path",
    "mask",
    "display",
    "visibility",
    "pointer-events",
];

/// Lower an element painted with a mesh — its `fill`, its `stroke`, or both
/// (§8.2). The element becomes a group carrying its identity, transform and
/// effects; inside, each mesh paint is the mesh's own output clipped to the
/// region that paint covers — the element's geometry for a fill, its stroke
/// outline for a stroke — and one path carries whatever paint is left (a plain
/// fill or stroke, markers). Pieces stack in `paint-order`.
///
/// A `<meshgradient>` tessellates its Coons patches into the straight-quad
/// mesh (polycurve → points), under `gradientTransform` and `gradientUnits`.
/// An `<x:mesh>` is that mesh already: its coordinates are the element's user
/// space — or fractions of its bounding box, `units="objectBoundingBox"` —
/// under the mesh's own `transform`. Nothing above the mesh applies, as with
/// any paint server.
///
/// Returns `false` (the caller emits the element as it always did, the paint
/// unrendered as in any browser) when no mesh paint could be lowered.
pub(super) fn emit_mesh_paint(node: roxmltree::Node, out: &mut String, ctx: &Ctx) -> bool {
    // a harvest wants geometry, a clip ignores paint, `to_path` writes source
    if ctx.clip_geometry.get() || ctx.paint_live.get() || ctx.to_path.get() {
        return false;
    }
    let Some(bearer) = mesh_bearer(node) else {
        return false;
    };
    let fill_paint = mesh_paint_target(node, "fill", ctx);
    let mut stroke_paint = mesh_paint_target(node, "stroke", ctx);
    if fill_paint.is_none() && stroke_paint.is_none() {
        return false;
    }
    // A run's own paint is drawn by the text, not by the element's geometry;
    // one mesh over every glyph would repaint it.
    if bearer == Bearer::Text
        && node.descendants().skip(1).any(|n| {
            (fill_paint.is_some() && n.attribute("fill").is_some())
                || (stroke_paint.is_some() && n.attribute("stroke").is_some())
        })
    {
        out.push_str(OWN_PAINT);
        return false;
    }
    let geometry = match bearer {
        Bearer::Shape => shape_to_path_d(node, ctx),
        // scratch-compiled, glyphs outlined; the harvest keeps paint live,
        // which is what keeps this function out of it
        Bearer::Outline | Bearer::Text => x_output_geometry(node, ctx).ok(),
    };
    let Some(mut shape_d) = geometry else {
        out.push_str(NO_GEOMETRY);
        return false;
    };
    // A polyline is open: its geometry is closed for the operations that need
    // an area, and fills the same either way, but its stroke has two ends.
    if node.tag_name().name() == "polyline" && corner_spec(node, ctx.theme).is_none() {
        if let Some(open) = shape_d.strip_suffix('Z') {
            shape_d = open.trim_end().to_string();
        }
    }
    let bbox = svg_path_bbox(&shape_d);

    // the stroke's own region: its outline, as fill geometry
    let mut stroke_d = None;
    if stroke_paint.is_some() {
        match stroke_outline(node, &shape_d, ctx) {
            Ok(d) => stroke_d = d,
            Err(why) => {
                out.push_str(why);
                stroke_paint = None;
            }
        }
    }
    let fill_mesh = fill_paint.and_then(|p| paint_mesh(p, bbox, true, out, ctx));
    let stroke_mesh = stroke_paint.and_then(|p| paint_mesh(p, bbox, true, out, ctx));
    if fill_mesh.is_none() && stroke_mesh.is_none() {
        return false;
    }

    let pos = gen_id_seed(node, ctx);
    let filter = effect_attr(node, out, ctx, || bbox);
    out.push_str("<g");
    if bearer == Bearer::Shape {
        let mut skip = [SHAPE_GEOMETRY, PAINT_ATTRS].concat();
        skip.push("filter");
        copy_attrs(node, out, &skip, ctx);
    } else {
        for a in node.attributes() {
            if a.namespace().is_none() && GROUP_ATTRS.contains(&a.name()) {
                push_attr(out, a.name(), &ctx.resolve_var(a.value()));
            }
        }
    }
    out.push_str(&ctx.carry_x(node));
    out.push_str(&filter);
    out.push_str(&pos_attr(node, ctx));
    out.push('>');

    // what is left for the element's own path: a paint that is not a mesh
    let painted = |prop: &str| presentation(node, prop).is_some_and(|v| v.trim() != "none");
    let plain_fill =
        fill_mesh.is_none() && presentation(node, "fill").is_none_or(|v| v.trim() != "none");
    let plain_stroke = stroke_mesh.is_none() && painted("stroke");
    let markers = ["marker", "marker-start", "marker-mid", "marker-end"]
        .iter()
        .any(|m| painted(m));
    let rest = |out: &mut String| {
        out.push_str("<path");
        for a in node.attributes() {
            let name = a.name();
            let lowered = (name == "fill" && fill_mesh.is_some())
                || (name == "stroke" && stroke_mesh.is_some());
            // an outline's `fill-rule` was its operands' winding, now resolved
            let spent = name == "fill-rule" && bearer != Bearer::Shape;
            if a.namespace().is_none() && PAINT_ATTRS.contains(&name) && !lowered && !spent {
                push_attr(out, name, &ctx.resolve_var(a.value()));
            }
        }
        if fill_mesh.is_some() {
            out.push_str(" fill=\"none\"");
        }
        if stroke_mesh.is_some() {
            out.push_str(" stroke=\"none\"");
        }
        out.push_str(&format!(" d=\"{shape_d}\"/>"));
    };
    let above = |name: &str| node.ancestors().skip(1).find_map(|a| a.attribute(name));
    let mut inherited_reset = String::new();
    if above("stroke").is_some_and(|v| v.trim() != "none") {
        inherited_reset.push_str(" stroke=\"none\"");
    }
    if above("fill-opacity").is_some() {
        inherited_reset.push_str(" fill-opacity=\"1\"");
    }
    let piece = |out: &mut String,
                 mesh: &crate::gradient::Mesh,
                 clip_d: &str,
                 tag: &str,
                 rule: &str,
                 opacity: Option<&str>| {
        out.push_str(&format!(
            "<clipPath id=\"x-{tag}-{pos}\"><path{rule} d=\"{clip_d}\"/></clipPath><g clip-path=\"url(#x-{tag}-{pos})\""
        ));
        if let Some(o) = opacity {
            push_attr(out, "opacity", &ctx.resolve_var(o));
        }
        // The piece is the mesh's own markup, and its solid regions are
        // plain paths: what the element inherits must not paint them again.
        out.push_str(&inherited_reset);
        out.push('>');
        // the stroke's regions get clip ids of their own
        let seed = if tag == "mgc" {
            pos.clone()
        } else {
            format!("{pos}s")
        };
        if !lower_mesh(mesh, &seed, node, out, ctx) {
            out.push_str("<!-- xsvg: mesh fill degenerate extent -->");
        }
        out.push_str("</g>");
    };
    let even_odd = bearer == Bearer::Shape
        && presentation(node, "fill-rule").map(str::trim) == Some("evenodd");
    let fill_piece = |out: &mut String| match &fill_mesh {
        Some(mesh) => piece(
            out,
            mesh,
            &shape_d,
            "mgc",
            if even_odd {
                " clip-rule=\"evenodd\""
            } else {
                ""
            },
            presentation(node, "fill-opacity"),
        ),
        None => rest(out),
    };
    let stroke_piece = |out: &mut String| match (&stroke_mesh, &stroke_d) {
        (Some(mesh), Some(d)) => piece(
            out,
            mesh,
            d,
            "mgs",
            "",
            presentation(node, "stroke-opacity"),
        ),
        (Some(_), None) => {} // a zero-width stroke paints nothing
        (None, _) => rest(out),
    };
    // `paint-order` (SVG 2): the stroke goes under the fill when it is named
    // first, or when markers are named first and the fill is not second
    let order: Vec<&str> = presentation(node, "paint-order")
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let stroke_first = matches!(order.as_slice(), ["stroke", ..] | ["markers", "stroke", ..]);
    let both = fill_mesh.is_some() && stroke_mesh.is_some();
    if both {
        // neither piece is the path, so it follows with whatever it has left
        if stroke_first {
            stroke_piece(out);
            fill_piece(out);
        } else {
            fill_piece(out);
            stroke_piece(out);
        }
        if markers {
            rest(out);
        }
    } else if fill_mesh.is_some() {
        // the path holds the plain stroke and the markers
        let path = plain_stroke || markers;
        if stroke_first && path {
            rest(out);
        }
        fill_piece(out);
        if !stroke_first && path {
            rest(out);
        }
    } else {
        // the path holds the plain fill and the markers
        let path = plain_fill || markers;
        if stroke_first {
            stroke_piece(out);
        }
        if path {
            rest(out);
        }
        if !stroke_first {
            stroke_piece(out);
        }
    }
    out.push_str("</g>");
    true
}

/// ` name="value"`, escaped.
fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    push_escaped(out, value, true);
    out.push('"');
}

/// The region a stroke covers, as fill geometry (nonzero): kurbo's
/// stroke-to-fill under the element's `stroke-*` properties, inherited as SVG
/// inherits them. `Ok(None)` is a stroke with no width — nothing to paint;
/// `Err` is the marker for a stroke whose width is not in user space.
fn stroke_outline(
    node: roxmltree::Node,
    d: &str,
    ctx: &Ctx,
) -> Result<Option<String>, &'static str> {
    use crate::kurbo::{stroke, Cap, Join, Stroke, StrokeOpts};
    let prop = |name: &str| presentation(node, name).map(|v| ctx.resolve_var(v).trim().to_string());
    if prop("vector-effect").as_deref() == Some("non-scaling-stroke") {
        return Err(NON_SCALING);
    }
    let num = |name: &str, default: f64| {
        prop(name)
            .and_then(|v| parse_num(&v))
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    };
    let width = num("stroke-width", 1.0);
    if width <= 0.0 {
        return Ok(None);
    }
    let Ok(path) = crate::kurbo::BezPath::from_svg(d) else {
        return Ok(None);
    };
    {
        use crate::kurbo::Shape;
        let bb = path.bounding_box();
        if !(bb.is_finite() && width.is_finite()) {
            return Ok(None);
        }
    }
    let cap = match prop("stroke-linecap").as_deref() {
        Some("round") => Cap::Round,
        Some("square") => Cap::Square,
        _ => Cap::Butt,
    };
    let join = match prop("stroke-linejoin").as_deref() {
        Some("round") => Join::Round,
        Some("bevel") => Join::Bevel,
        _ => Join::Miter,
    };
    let mut style = Stroke::new(width)
        .with_caps(cap)
        .with_join(join)
        .with_miter_limit(num("stroke-miterlimit", 4.0).max(1.0));
    if let Some(dashes) = prop("stroke-dasharray") {
        let tokens: Vec<&str> = dashes
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .collect();
        let mut pattern: Vec<f64> = tokens.iter().filter_map(|t| parse_num(t)).collect();
        // SVG: a negative or unreadable entry voids the list (solid), all zero
        // is solid, and an odd count repeats to make it even
        let valid = !pattern.is_empty()
            && pattern.len() == tokens.len()
            && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)
            && pattern.iter().any(|v| *v > 0.0);
        if valid {
            if pattern.len() % 2 == 1 {
                pattern.extend_from_within(..);
            }
            // Every dash is geometry here, where a renderer only paints it: a
            // pattern far finer than the path is long would be millions of
            // them. Past the budget the stroke is left to the renderer.
            use crate::kurbo::Shape;
            let period: f64 = pattern.iter().sum();
            let dashes = path.perimeter(1.0) / period * (pattern.len() / 2) as f64;
            // (a count that is not a number is over budget too)
            if dashes.is_nan() || dashes > MAX_DASHES {
                return Err(TOO_MANY_DASHES);
            }
            let offset = num("stroke-dashoffset", 0.0);
            // the pattern repeats, so the offset matters only within a period
            style = style.with_dashes(offset.rem_euclid(period), pattern);
        }
    }
    let outline = stroke(
        path,
        &style,
        &StrokeOpts::default(),
        ctx.quality.tolerance().max(0.01),
    );
    // kurbo writes full f64 digits; three decimals is the output's precision
    let mut d = String::new();
    let pt = |p: crate::kurbo::Point| format!("{},{}", fmt(p.x), fmt(p.y));
    for el in outline.elements() {
        use crate::kurbo::PathEl::*;
        match *el {
            MoveTo(p) => d.push_str(&format!("M{}", pt(p))),
            LineTo(p) => d.push_str(&format!("L{}", pt(p))),
            QuadTo(a, p) => d.push_str(&format!("Q{} {}", pt(a), pt(p))),
            CurveTo(a, b, p) => d.push_str(&format!("C{} {} {}", pt(a), pt(b), pt(p))),
            ClosePath => d.push('Z'),
        }
    }
    if d.is_empty() {
        return Ok(None);
    }
    Ok(Some(d))
}

/// The mesh a paint server stands for, in the user space of the element it
/// paints (`bbox` is that element's fill bounding box, for bounding-box
/// units) — or, when it is not `painting` anything, as it is written. `None`
/// leaves its marker and the paint as authored.
fn paint_mesh(
    paint: roxmltree::Node,
    bbox: Option<Rect>,
    painting: bool,
    out: &mut String,
    ctx: &Ctx,
) -> Option<crate::gradient::Mesh> {
    use crate::kurbo::Affine;
    let is_x = paint.tag_name().name() == "mesh";
    let boxed = if is_x {
        let declared: super::elements::Mesh = params_of(paint);
        declared.units == super::elements::MeshUnits::ObjectBoundingBox
    } else {
        paint.attribute("gradientUnits").map(str::trim) == Some("objectBoundingBox")
    };
    // objectBoundingBox units: coordinates live in the unit square of the
    // element's bbox. A box with no width or height has no such square, and
    // the paint is then nothing at all (SVG's rule for every paint server).
    let unit_map = if boxed && painting {
        match bbox.filter(|bb| bb.width() > 0.0 && bb.height() > 0.0) {
            Some(bb) => {
                Affine::translate((bb.x0, bb.y0))
                    * Affine::scale_non_uniform(bb.width(), bb.height())
            }
            None => {
                out.push_str("<!-- xsvg: mesh paint left live (no bounding box for objectBoundingBox units) -->");
                return None;
            }
        }
    } else {
        Affine::IDENTITY
    };
    if is_x {
        // its own markers (a skipped face) are said where the mesh stands
        let Ok(mut mesh) = parse_mesh(paint, &mut String::new()) else {
            out.push_str("<!-- xsvg: x:mesh fill left live (no usable mesh) -->");
            return None;
        };
        let placed = unit_map * element_transform(paint);
        if placed != Affine::IDENTITY {
            for v in &mut mesh.verts {
                let p = placed * crate::kurbo::Point::new(v.0 as f64, v.1 as f64);
                *v = (p.x as f32, p.y as f32);
            }
        }
        return Some(mesh);
    }
    let tess = match ctx.quality {
        QualityProfile::Fast => 8usize,
        QualityProfile::Balanced => 12,
        QualityProfile::Highest | QualityProfile::Raster => 20,
    };
    let mesh = parse_meshgradient(paint, tess, unit_map, ctx);
    if mesh.is_none() {
        out.push_str("<!-- xsvg: meshgradient fill left live (malformed dialect) -->");
    }
    mesh
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
    ctx: &Ctx,
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
                    colors[corner] = Some(stop_color(*stop, ctx)?);
                }
            }
            // first patch: the 4th stop's color lands on BL via the ends rule
            // never firing — fill any hole from the stop list in corner order
            if r == 0 && i == 0 {
                for (stop, &k) in stops.iter().zip(missing.iter()) {
                    if colors[k].is_none() {
                        colors[k] = Some(stop_color(*stop, ctx)?);
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

/// One `<stop path>` edge: its four control points in walking order, and the new
/// current point.
pub(super) type StopEdge = ([(f32, f32); 4], (f32, f32));

/// One `<stop path>` edge: a single `c`/`C` cubic or `l`/`L` line from `cur`.
/// Returns the edge (4 control points, walking order) and the new current point.
pub(super) fn parse_stop_edge(d: &str, cur: (f32, f32)) -> Option<StopEdge> {
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
pub(super) fn stop_color(
    stop: roxmltree::Node,
    ctx: &Ctx,
) -> Option<(crate::gradient::LinRgb, f32)> {
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
    let color = ctx.resolve_var(color.trim()); // §4.1 color token
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

/// A set of closed vertex loops as path data, one subpath each — the outline
/// of a colour region, or of an authored clipping mesh. Filled nonzero it is
/// exactly the faces it was traced from.
pub(super) fn append_outline(mesh: &crate::gradient::Mesh, loops: &[Vec<u32>], out: &mut String) {
    for ring in loops {
        for (i, &v) in ring.iter().enumerate() {
            let (x, y) = mesh.verts[v as usize];
            out.push(if i == 0 { 'M' } else { 'L' });
            out.push_str(&fmt(x as f64));
            out.push(',');
            out.push_str(&fmt(y as f64));
        }
        out.push('Z');
    }
}
