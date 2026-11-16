//! Texture serialization at the requested output density. The fitted field,
//! clip geometry and texel placement are identical in compact and raster modes.
use super::{
    attr_num, element_transform, parse_num, view_box_of, Ctx, Par, Viewport, SVG_NS, XLINK_NS,
};
use crate::kurbo::Affine;

/// The placement with the largest scale: what a linked document, which takes
/// one parent transform, should be rasterized for.
pub(super) fn display_transform(node: roxmltree::Node, ctx: &Ctx) -> Affine {
    display_transforms(node, ctx)
        .into_iter()
        .max_by(|a, b| a.determinant().abs().total_cmp(&b.determinant().abs()))
        .unwrap_or(ctx.raster_parent)
}

/// Root user coordinates of every place `node` is drawn, including nested
/// viewport fits: where it stands, and once per `<use>` that instances it or
/// an ancestor. Translation is irrelevant to density, but retaining the full
/// affine keeps composition exact.
pub(super) fn display_transforms(node: roxmltree::Node, ctx: &Ctx) -> Vec<Affine> {
    let uses: Vec<(&str, roxmltree::Node)> = node
        .document()
        .descendants()
        .filter(|n| n.tag_name().name() == "use" && n.tag_name().namespace() == Some(SVG_NS))
        .filter_map(|n| {
            let href = n.attribute("href").or(n.attribute((XLINK_NS, "href")))?;
            Some((href.strip_prefix('#')?, n))
        })
        .collect();
    let mut found = Vec::new();
    placements(node, ctx, &uses, Affine::IDENTITY, 0, &mut found);
    if found.is_empty() {
        // Drawn by no `<use>` and standing somewhere that is not rendered in
        // place (a `<pattern>`, a `<marker>`): its own coordinates are the
        // only evidence there is.
        found.push(
            node.ancestors()
                .filter(|n| n.is_element())
                .fold(Affine::IDENTITY, |below, n| local(n, None) * below),
        );
        found[0] = ctx.raster_parent * found[0];
    }
    found
}

/// Bounds for instancing that feeds on itself: `<use>` chains deeper than
/// this, or fanning out wider, stop adding placements.
const MAX_USE_DEPTH: usize = 8;
const MAX_PLACEMENTS: usize = 64;

/// Walk from `node` to the root, carrying `below`: the transform from `node`'s
/// content up to (not including) the element in hand.
fn placements(
    node: roxmltree::Node,
    ctx: &Ctx,
    uses: &[(&str, roxmltree::Node)],
    mut below: Affine,
    depth: usize,
    found: &mut Vec<Affine>,
) {
    for n in node.ancestors().filter(|n| n.is_element()) {
        if depth < MAX_USE_DEPTH && n.parent_element().is_some() {
            if let Some(id) = n.attribute("id") {
                for (_, instance) in uses.iter().filter(|(href, _)| *href == id) {
                    if found.len() >= MAX_PLACEMENTS {
                        return;
                    }
                    // the referenced element is drawn at the `<use>`'s x/y,
                    // inside the `<use>`'s own transform (the walk applies it)
                    let at = Affine::translate((
                        attr_num(*instance, "x", 0.0),
                        attr_num(*instance, "y", 0.0),
                    ));
                    let inner = at * local(n, Some(*instance)) * below;
                    placements(*instance, ctx, uses, inner, depth + 1, found);
                }
            }
        }
        // never rendered where it stands: only its instances count
        if n.tag_name().namespace() == Some(SVG_NS)
            && matches!(
                n.tag_name().name(),
                "defs" | "symbol" | "clipPath" | "mask" | "pattern" | "marker"
            )
        {
            return;
        }
        below = local(n, None) * below;
    }
    found.push(ctx.raster_parent * below);
}

/// An element's own contribution: its transform, and for a viewport element
/// the `viewBox` fit — sized by the instancing `<use>` when there is one,
/// whose `width`/`height` override the target's.
fn local(n: roxmltree::Node, instance: Option<roxmltree::Node>) -> Affine {
    let name = n.tag_name().name();
    let viewport = match instance {
        _ if n.tag_name().namespace() != Some(SVG_NS) => None,
        None if name == "svg" && n.parent_element().is_some() => Viewport::of(n),
        Some(u) if matches!(name, "svg" | "symbol") => {
            let side = |a: &str| {
                let own = |e: roxmltree::Node| e.attribute(a).and_then(parse_num);
                own(u).or_else(|| own(n)).filter(|v| *v > 0.0)
            };
            match (side("width"), side("height")) {
                (Some(w), Some(h)) => Some(Viewport {
                    x: 0.0,
                    y: 0.0,
                    w,
                    h,
                    view_box: view_box_of(n),
                    par: n
                        .attribute("preserveAspectRatio")
                        .and_then(Par::parse)
                        .unwrap_or(Par::DEFAULT),
                    clip: false,
                }),
                _ => None,
            }
        }
        _ => None,
    };
    // a `<symbol>` takes no transform of its own
    let mut matrix = if name == "symbol" && n.tag_name().namespace() == Some(SVG_NS) {
        Affine::IDENTITY
    } else {
        element_transform(n)
    };
    if let Some(viewport) = viewport {
        let (sx, sy, _, _) = viewport.fit();
        matrix *= Affine::scale_non_uniform(sx, sy);
    }
    matrix
}

/// Texels per user unit on each axis for `node`, or `None` when textures stay
/// compact. Where the node is drawn more than once, the densest placement
/// decides: every instance shares the one texture.
pub(super) fn density(node: roxmltree::Node, ctx: &Ctx) -> Option<(f64, f64)> {
    let density = ctx.rasterize_at_display_resolution?;
    Some(
        display_transforms(node, ctx)
            .iter()
            .fold((0.0_f64, 0.0_f64), |(x, y), matrix| {
                let [a, b, c, d, _, _] = matrix.as_coeffs();
                (x.max(a.hypot(b) * density), y.max(c.hypot(d) * density))
            }),
    )
}

pub(super) fn dimensions(
    density: Option<(f64, f64)>,
    size: (f64, f64),
    grid: (usize, usize),
) -> (usize, usize) {
    match density {
        Some((x, y)) => bounded_dimensions((size.0 * x, size.1 * y), grid),
        None => grid,
    }
}

fn bounded_dimensions(size: (f64, f64), grid: (usize, usize)) -> (usize, usize) {
    if !size.0.is_finite() || !size.1.is_finite() {
        return grid;
    }
    let (w, h) = (
        size.0.ceil().max(grid.0 as f64),
        size.1.ceil().max(grid.1 as f64),
    );
    // Bound allocations for giant, mostly off-artboard artwork.
    let limit = 1.0_f64
        .min(8192.0 / w)
        .min(8192.0 / h)
        .min((16_777_216.0 / (w * h)).sqrt());
    let target = (
        (w * limit).floor().max(1.0) as usize,
        (h * limit).floor().max(1.0) as usize,
    );
    if target.0 <= grid.0 && target.1 <= grid.1 {
        grid
    } else {
        target
    }
}

/// A smooth field filters down to near-constant residuals, so a fast DEFLATE
/// level already removes almost all of it; higher levels cost time for bytes
/// that are not there.
pub(super) fn encode(w: usize, h: usize, pixels: &[u8], channels: usize) -> Vec<u8> {
    crate::gradient::png::encode_png_with(w as u32, h as u32, pixels, channels, |raw| {
        miniz_oxide::deflate::compress_to_vec_zlib(raw, DEFLATE_LEVEL)
    })
}

const DEFLATE_LEVEL: u8 = 3;

/// Bilinear sampling of the grid's quantized vertex colours, in premultiplied
/// sRGB like native image compositing. No PNG is encoded/decoded on the way.
pub(super) fn interpolate(
    input: &[u8],
    source: (usize, usize),
    target: (usize, usize),
    channels: usize,
) -> Vec<u8> {
    let (sw, sh) = source;
    let (w, h) = target;
    let mut output = vec![0; w * h * channels];
    for y in 0..h {
        let sy = (((y as f64 + 0.5) * sh as f64 / h as f64) - 0.5).clamp(0.0, (sh - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f64;
        for x in 0..w {
            let sx = (((x as f64 + 0.5) * sw as f64 / w as f64) - 0.5).clamp(0.0, (sw - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f64;
            let offsets = [
                (y0 * sw + x0) * channels,
                (y0 * sw + x1) * channels,
                (y1 * sw + x0) * channels,
                (y1 * sw + x1) * channels,
            ];
            let mut weights = [
                (1.0 - fx) * (1.0 - fy),
                fx * (1.0 - fy),
                (1.0 - fx) * fy,
                fx * fy,
            ];
            for i in 0..4 {
                weights[i] *= if channels == 4 {
                    input[offsets[i] + 3] as f64
                } else {
                    255.0
                };
            }
            let alpha: f64 = weights.iter().sum();
            let out = (y * w + x) * channels;
            if alpha > 0.0 {
                for c in 0..3 {
                    output[out + c] = (offsets
                        .iter()
                        .zip(weights)
                        .map(|(&i, weight)| input[i + c] as f64 * weight)
                        .sum::<f64>()
                        / alpha)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
            if channels == 4 {
                output[out + 3] = alpha.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bilinear_colour_and_alpha() {
        let pixels = interpolate(
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
            (2, 2),
            (3, 3),
            3,
        );
        assert_eq!(&pixels[12..15], &[128, 128, 128]);
        assert_eq!(&pixels[..3], &[255, 0, 0]);
        let feather = interpolate(&[255, 0, 0, 255, 0, 0, 255, 0], (2, 1), (3, 1), 4);
        assert_eq!(&feather[4..8], &[255, 0, 0, 128]);
        assert_eq!(&feather[8..], &[0, 0, 0, 0]);
    }
    #[test]
    fn bounds_texture_memory_and_keeps_compact_grids() {
        assert_eq!(bounded_dimensions((300.2, 199.1), (13, 13)), (301, 200));
        assert_eq!(bounded_dimensions((1.0, 1.0), (13, 13)), (13, 13));
        let (w, h) = bounded_dimensions((1e9, 1e9), (13, 13));
        assert!(w <= 8192 && h <= 8192 && w * h <= 16_777_216);
    }
}
