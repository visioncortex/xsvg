//! The browser-bilinear hack: place a tiny image so its **texel centers** land
//! exactly on the fitted grid's vertices, and the renderer's smooth image
//! filter becomes the gradient interpolator.
//!
//! An image sampler reads texel `k` (of `n` across) at fraction `(k + 0.5)/n`
//! of the image span, clamp-padding the outer half-texels. So to make the `n`
//! texel centers coincide with `n` grid vertices spread across a pixel bbox of
//! span `s` (distance between the first and last vertex, i.e. between the
//! corner pixel *centers*), the `<image>` must be **wider than the bbox**:
//!
//! ```text
//! width  = n·s/(n−1)              (n=2: exactly 2× the span)
//! offset = lo + 0.5 − s/(2(n−1))  (n=2: half a span before the bbox)
//! ```
//!
//! Interpolating between real samples of a (bi)linear field is exact — naive
//! bbox placement instead runs the ramp through the clamp plateaus and renders
//! every gradient as an S-curve. The overhang is cut by the region's clipPath.
//! Ported from vtracer's fit-eval `svg_roundtrip` (2×2 case), generalized to
//! n×m grids.

/// Placement of one axis: the image edge coordinate and length in pixel units,
/// for `n` texels whose centers must land on `lo + 0.5 … lo + 0.5 + span`
/// (pixel-center convention; `span = hi − lo` between corner pixel indices).
/// A degenerate axis (`span == 0`) pins a 1-texel-wide image on the pixel.
pub fn texel_axis(lo: f64, span: f64, n: usize) -> (f64, f64) {
    if span <= 0.0 || n < 2 {
        return (lo, 1.0);
    }
    let n = n as f64;
    let len = n * span / (n - 1.0);
    let offset = lo + 0.5 - 0.5 * span / (n - 1.0);
    (offset, len)
}

/// Full 2D placement for an `nx × ny`-texel image over the pixel bbox
/// `(x0, y0)..=(x1, y1)` (inclusive pixel indices): returns `(x, y, w, h)` for
/// the `<image>` element, in the raster's pixel coordinate space.
pub fn texel_placement(
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    nx: usize,
    ny: usize,
) -> (f64, f64, f64, f64) {
    let (ix, iw) = texel_axis(x0 as f64, (x1 - x0) as f64, nx);
    let (iy, ih) = texel_axis(y0 as f64, (y1 - y0) as f64, ny);
    (ix, iy, iw, ih)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_texels_span_twice_the_bbox() {
        // the original 2×2 construction: image spans 2× the bbox, offset by half
        let (ix, iy, iw, ih) = texel_placement(10, 20, 40, 40, 2, 2);
        let (sx, sy) = (30.0, 20.0);
        assert_eq!(iw, 2.0 * sx);
        assert_eq!(ih, 2.0 * sy);
        assert_eq!(ix, 10.0 + 0.5 - 0.5 * sx);
        assert_eq!(iy, 20.0 + 0.5 - 0.5 * sy);
        // texel centers at 25% / 75% of the image land on the corner pixel centers
        assert!((ix + 0.25 * iw - 10.5).abs() < 1e-9);
        assert!((ix + 0.75 * iw - 40.5).abs() < 1e-9);
    }

    #[test]
    fn n_texel_centers_land_on_grid_vertices() {
        for n in [2usize, 3, 5, 9] {
            let (lo, hi) = (7u32, 91u32);
            let (off, len) = texel_axis(lo as f64, (hi - lo) as f64, n);
            for k in 0..n {
                let center = off + (k as f64 + 0.5) * len / n as f64;
                let vertex = lo as f64 + 0.5 + k as f64 * (hi - lo) as f64 / (n as f64 - 1.0);
                assert!(
                    (center - vertex).abs() < 1e-9,
                    "n={n} k={k}: {center} vs {vertex}"
                );
            }
        }
    }

    #[test]
    fn degenerate_axis_pins_one_texel() {
        let (off, len) = texel_axis(5.0, 0.0, 2);
        assert_eq!((off, len), (5.0, 1.0));
    }
}
