//! Texture serialization at the requested output density. The fitted field,
//! clip geometry and texel placement are identical in compact and raster modes.
use super::{element_transform, Ctx, Viewport};
use crate::kurbo::Affine;

/// Root user coordinates, including nested viewport fits. Translation is
/// irrelevant to density, but retaining the full affine keeps composition exact.
pub(super) fn display_transform(node: roxmltree::Node, ctx: &Ctx) -> Affine {
    let mut ancestors: Vec<_> = node.ancestors().filter(|n| n.is_element()).collect();
    ancestors.reverse();
    ancestors.into_iter().fold(ctx.raster_parent, |matrix, n| {
        let mut local = element_transform(n);
        if n.parent_element().is_some() && n.tag_name().name() == "svg" {
            if let Some(viewport) = Viewport::of(n) {
                let (sx, sy, _, _) = viewport.fit();
                local *= Affine::scale_non_uniform(sx, sy);
            }
        }
        matrix * local
    })
}

pub(super) fn dimensions(
    node: roxmltree::Node,
    ctx: &Ctx,
    size: (f64, f64),
    grid: (usize, usize),
) -> (usize, usize) {
    let Some(density) = ctx.rasterize_at_display_resolution else {
        return grid;
    };
    let [a, b, c, d, _, _] = display_transform(node, ctx).as_coeffs();
    bounded_dimensions(
        (size.0 * a.hypot(b) * density, size.1 * c.hypot(d) * density),
        grid,
    )
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
