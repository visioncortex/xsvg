//! The unified **continuous color field**: one representation for every
//! topology-safe fill. Ported from vtracer's `gradient::field` (the `Cluster`
//! parameter generalized to a rect + pixel-index slice), and widened to
//! **RGBA** — alpha is a fourth fitted channel, which is what makes mesh
//! *feathering* serialize (the tiny PNGs carry straight alpha and the
//! renderer's sampler interpolates it like any channel).
//!
//! A field is 4 corner colors over an axis-aligned patch, bilinearly
//! interpolated (matching the mesh corner winding `0→(0,0) 1→(1,0) 2→(1,1)
//! 3→(0,1)`). Solid and linear are *degeneracies of the same 4 corners*, not
//! separate types. We always fit bilinear (the most general single patch) and
//! report the collapsed [`Dof`]. **A large residual means subdivide** — fit a
//! finer [`GridField`] — never a new primitive type.
//!
//! All pixel buffers are interleaved RGBA, 4 bytes per pixel, sRGB-encoded
//! color + straight (unpremultiplied) alpha.

/// An axis-aligned pixel rect, `left/top` inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn new(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            width,
            height,
        }
    }

    /// Bounding rect of a pixel-index set over a `w`-wide image.
    pub fn of_indices(indices: &[u32], w: usize) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for &idx in indices {
            let (x, y) = ((idx as usize % w) as i32, (idx as usize / w) as i32);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
        Self {
            left: x0,
            top: y0,
            width: x1 - x0 + 1,
            height: y1 - y0 + 1,
        }
    }
}

/// The collapsed degrees of freedom of a fitted field (topology-safe reductions only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dof {
    Solid,
    Linear,
    Bilinear,
}

/// A continuous RGBA field over one patch: 4 corner colors (sRGB channel
/// values 0..255, alpha 0..255 straight) bilinearly interpolated, plus the
/// collapsed DOF and the fit residual. Fitting happens in the **encoded
/// (sRGB) domain** on purpose: the SVG `<image>` sampler that reconstructs
/// the field interpolates raw texels, so fitting encoded values is what makes
/// the round trip exact.
#[derive(Debug, Clone, Copy)]
pub struct ColorField {
    pub rect: Rect,
    /// corner RGBA, quad-local order `0→(0,0) 1→(1,0) 2→(1,1) 3→(0,1)`
    pub corners: [[f32; 4]; 4],
    pub dof: Dof,
    /// RMS residual of this field over the patch's pixels (0..255, over RGBA)
    pub rmse: f32,
}

impl ColorField {
    /// Evaluate the bilinear field at an image pixel (RGBA, 0..255), uv-clamped.
    pub fn eval(&self, x: i32, y: i32) -> [f32; 4] {
        let dx = (self.rect.width - 1).max(1) as f32;
        let dy = (self.rect.height - 1).max(1) as f32;
        let u = ((x - self.rect.left) as f32 / dx).clamp(0.0, 1.0);
        let v = ((y - self.rect.top) as f32 / dy).clamp(0.0, 1.0);
        let mut out = [0f32; 4];
        for c in 0..4 {
            let bottom = self.corners[0][c] * (1.0 - u) + self.corners[1][c] * u;
            let top = self.corners[3][c] * (1.0 - u) + self.corners[2][c] * u;
            out[c] = bottom * (1.0 - v) + top * v;
        }
        out
    }
}

/// Fit the most general single-patch bilinear RGBA field to the pixels
/// `indices` (into a `w`-wide interleaved RGBA image) by least squares, then
/// collapse to the lowest DOF the data supports (`eps` is the slope/twist
/// tolerance in 0..255 units). A large `rmse` ⇒ subdivide.
pub fn fit_field(indices: &[u32], w: usize, rgba: &[u8], eps: f32) -> ColorField {
    let rect = Rect::of_indices(indices, w);
    let dx = (rect.width - 1).max(1) as f64;
    let dy = (rect.height - 1).max(1) as f64;

    // normal equations for the basis [1, u, v, uv], channels normalised to [0,1]
    let mut g = [[0f64; 4]; 4];
    let mut rhs = [[0f64; 4]; 4];
    for &idx in indices {
        let i = idx as usize;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let u = (x - rect.left) as f64 / dx;
        let v = (y - rect.top) as f64 / dy;
        let phi = [1.0, u, v, u * v];
        for a in 0..4 {
            for b in 0..4 {
                g[a][b] += phi[a] * phi[b];
            }
        }
        for c in 0..4 {
            let col = rgba[i * 4 + c] as f64 / 255.0;
            for a in 0..4 {
                rhs[a][c] += phi[a] * col;
            }
        }
    }
    // tiny ridge so an unsupported DOF (e.g. a 1px-wide sliver ⇒ no u-variation)
    // resolves to 0 rather than blowing up — the sliver clamp, for free.
    let n = indices.len().max(1) as f64;
    for a in 0..4 {
        g[a][a] += 1e-6 * n;
    }
    let coef = solve4(g, rhs);

    // observed per-channel range — the corner clamp below
    let mut lo = [f32::INFINITY; 4];
    let mut hi = [f32::NEG_INFINITY; 4];
    for &idx in indices {
        let i = idx as usize;
        for c in 0..4 {
            let v = rgba[i * 4 + c] as f32;
            lo[c] = lo[c].min(v);
            hi[c] = hi[c].max(v);
        }
    }

    let mut corners = [[0f32; 4]; 4];
    let (mut slope, mut twist) = (0f32, 0f32);
    for c in 0..4 {
        let (a, b, cc, d) = (coef[0][c], coef[1][c], coef[2][c], coef[3][c]);
        corners[0][c] = (a * 255.0) as f32;
        corners[1][c] = ((a + b) * 255.0) as f32;
        corners[2][c] = ((a + b + cc + d) * 255.0) as f32;
        corners[3][c] = ((a + cc) * 255.0) as f32;
        // Bilinear eval is a convex combination of the corners, so clamping the
        // corners to the observed range bounds every eval to values the region
        // actually holds — killing overfit extrapolation on thin cores.
        for k in 0..4 {
            corners[k][c] = corners[k][c].clamp(lo[c], hi[c]);
        }
        slope = slope.max((b.abs().max(cc.abs()) * 255.0) as f32);
        twist = twist.max((d.abs() * 255.0) as f32);
    }
    let dof = if twist < eps && slope < eps {
        Dof::Solid
    } else if twist < eps {
        Dof::Linear
    } else {
        Dof::Bilinear
    };

    let mut field = ColorField {
        rect,
        corners,
        dof,
        rmse: 0.0,
    };
    let mut sse = 0f64;
    for &idx in indices {
        let i = idx as usize;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let pred = field.eval(x, y);
        for c in 0..4 {
            let e = rgba[i * 4 + c] as f64 - pred[c] as f64;
            sse += e * e;
        }
    }
    field.rmse = ((sse / (n * 4.0)).max(0.0)).sqrt() as f32;
    field
}

/// Solve `A x = B` for a 4×4 `A` and 4×4 `B` via Gauss–Jordan with partial pivoting.
fn solve4(mut a: [[f64; 4]; 4], mut b: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    for col in 0..4 {
        let mut piv = col;
        for r in (col + 1)..4 {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for k in col..4 {
            a[col][k] /= d;
        }
        for k in 0..4 {
            b[col][k] /= d;
        }
        for r in 0..4 {
            if r == col {
                continue;
            }
            let f = a[r][col];
            if f == 0.0 {
                continue;
            }
            for k in col..4 {
                a[r][k] -= f * a[col][k];
            }
            for k in 0..4 {
                b[r][k] -= f * b[col][k];
            }
        }
    }
    b
}

/// A **subdivided** continuous RGBA field: a `gx × gy` grid of bilinear cells
/// whose corner colors are stored **once per shared vertex** ((gx+1)×(gy+1)).
/// Adjacent cells read the same vertex colors, so the field is C⁰ across every
/// internal edge **by construction** — no seams. `gx = gy = 1` is exactly one
/// [`ColorField`] patch; higher grids reduce the residual.
#[derive(Debug, Clone)]
pub struct GridField {
    pub rect: Rect,
    pub gx: usize,
    pub gy: usize,
    /// shared vertex RGBA, row-major `vy*(gx+1)+vx`, 0..255
    pub verts: Vec<[f32; 4]>,
    pub rmse: f32,
}

impl GridField {
    pub fn eval(&self, x: i32, y: i32) -> [f32; 4] {
        let dx = (self.rect.width - 1).max(1) as f32;
        let dy = (self.rect.height - 1).max(1) as f32;
        let fx = ((x - self.rect.left) as f32 / dx * self.gx as f32).clamp(0.0, self.gx as f32);
        let fy = ((y - self.rect.top) as f32 / dy * self.gy as f32).clamp(0.0, self.gy as f32);
        let ci = (fx.floor() as usize).min(self.gx - 1);
        let cj = (fy.floor() as usize).min(self.gy - 1);
        let (s, t) = (fx - ci as f32, fy - cj as f32);
        let g = self.gx + 1;
        let v = [
            cj * g + ci,
            cj * g + ci + 1,
            (cj + 1) * g + ci,
            (cj + 1) * g + ci + 1,
        ];
        let wt = [(1.0 - s) * (1.0 - t), s * (1.0 - t), (1.0 - s) * t, s * t];
        let mut out = [0f32; 4];
        for c in 0..4 {
            for k in 0..4 {
                out[c] += wt[k] * self.verts[v[k]][c];
            }
        }
        out
    }
}

/// Fit a seam-free subdivided RGBA field: solve for the **shared** grid-vertex
/// colors that best reproduce the pixels (one global least squares, so adjacent
/// cells agree at shared vertices → C⁰). Never fit sub-cells independently —
/// that seams.
pub fn fit_grid(indices: &[u32], w: usize, rgba: &[u8], gx: usize, gy: usize) -> GridField {
    let (gx, gy) = (gx.max(1), gy.max(1));
    let rect = Rect::of_indices(indices, w);
    let (dx, dy) = (
        (rect.width - 1).max(1) as f32,
        (rect.height - 1).max(1) as f32,
    );
    let g = gx + 1;
    let nv = (gx + 1) * (gy + 1);

    let mut mat = vec![vec![0f64; nv]; nv];
    let mut rhs = vec![[0f64; 4]; nv];
    for &idx in indices {
        let i = idx as usize;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let fx = ((x - rect.left) as f32 / dx * gx as f32).clamp(0.0, gx as f32);
        let fy = ((y - rect.top) as f32 / dy * gy as f32).clamp(0.0, gy as f32);
        let ci = (fx.floor() as usize).min(gx - 1);
        let cj = (fy.floor() as usize).min(gy - 1);
        let (s, t) = ((fx - ci as f32) as f64, (fy - cj as f32) as f64);
        let vs = [
            cj * g + ci,
            cj * g + ci + 1,
            (cj + 1) * g + ci,
            (cj + 1) * g + ci + 1,
        ];
        let wt = [(1.0 - s) * (1.0 - t), s * (1.0 - t), (1.0 - s) * t, s * t];
        for a in 0..4 {
            for b in 0..4 {
                mat[vs[a]][vs[b]] += wt[a] * wt[b];
            }
            for c in 0..4 {
                rhs[vs[a]][c] += wt[a] * (rgba[i * 4 + c] as f64 / 255.0);
            }
        }
    }
    // ridge keeps vertices with no incident pixels (empty cells outside the
    // region) finite; they don't affect any pixel's reconstruction.
    let ridge = 1e-6 * indices.len().max(1) as f64;
    for i in 0..nv {
        mat[i][i] += ridge;
    }
    let sol = solve_dense(mat, rhs);
    let verts: Vec<[f32; 4]> = sol
        .iter()
        .map(|c| {
            [
                (c[0] * 255.0) as f32,
                (c[1] * 255.0) as f32,
                (c[2] * 255.0) as f32,
                (c[3] * 255.0) as f32,
            ]
        })
        .collect();

    let mut field = GridField {
        rect,
        gx,
        gy,
        verts,
        rmse: 0.0,
    };
    let mut sse = 0f64;
    for &idx in indices {
        let i = idx as usize;
        let (x, y) = ((i % w) as i32, (i / w) as i32);
        let pred = field.eval(x, y);
        for c in 0..4 {
            let e = rgba[i * 4 + c] as f64 - pred[c] as f64;
            sse += e * e;
        }
    }
    let n = indices.len().max(1) as f64;
    field.rmse = ((sse / (n * 4.0)).max(0.0)).sqrt() as f32;
    field
}

/// Fit a seam-free `gx×gy` shared-vertex bilinear field over the **whole**
/// `w×h` domain of a **linear-light** RGB image `lin` (`w*h*3`, values in
/// [0,1]), pixel-center parameterised. Returns `(gx+1)*(gy+1)` linear vertex
/// colors, row-major. (Three channels — the fit-quality harness's domain.)
pub fn fit_grid_lin(w: usize, h: usize, lin: &[f32], gx: usize, gy: usize) -> Vec<[f32; 3]> {
    let (gx, gy) = (gx.max(1), gy.max(1));
    let g = gx + 1;
    let nv = (gx + 1) * (gy + 1);
    let mut mat = vec![vec![0f64; nv]; nv];
    let mut rhs = vec![[0f64; 4]; nv];
    for y in 0..h {
        for x in 0..w {
            let fx = ((x as f32 + 0.5) / w as f32 * gx as f32).clamp(0.0, gx as f32);
            let fy = ((y as f32 + 0.5) / h as f32 * gy as f32).clamp(0.0, gy as f32);
            let ci = (fx.floor() as usize).min(gx - 1);
            let cj = (fy.floor() as usize).min(gy - 1);
            let (s, t) = ((fx - ci as f32) as f64, (fy - cj as f32) as f64);
            let vs = [
                cj * g + ci,
                cj * g + ci + 1,
                (cj + 1) * g + ci,
                (cj + 1) * g + ci + 1,
            ];
            let wt = [(1.0 - s) * (1.0 - t), s * (1.0 - t), (1.0 - s) * t, s * t];
            let base = (y * w + x) * 3;
            for a in 0..4 {
                for b in 0..4 {
                    mat[vs[a]][vs[b]] += wt[a] * wt[b];
                }
                for c in 0..3 {
                    rhs[vs[a]][c] += wt[a] * lin[base + c] as f64;
                }
            }
        }
    }
    let ridge = 1e-6 * (w * h) as f64;
    for i in 0..nv {
        mat[i][i] += ridge;
    }
    solve_dense(mat, rhs)
        .iter()
        .map(|c| [c[0] as f32, c[1] as f32, c[2] as f32])
        .collect()
}

/// Gauss–Jordan solve of `A x = B` (`A` is `n×n`, `B` is `n×4`) with partial pivoting.
fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<[f64; 4]>) -> Vec<[f64; 4]> {
    let n = a.len();
    for col in 0..n {
        let mut piv = col;
        for r in (col + 1)..n {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let d = a[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for k in col..n {
            a[col][k] /= d;
        }
        for k in 0..4 {
            b[col][k] /= d;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r][col];
            if f == 0.0 {
                continue;
            }
            for k in col..n {
                a[r][k] -= f * a[col][k];
            }
            for k in 0..4 {
                b[r][k] -= f * b[col][k];
            }
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img<F: Fn(usize, usize) -> [u8; 4]>(w: usize, h: usize, f: F) -> Vec<u8> {
        let mut v = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let c = f(x, y);
                let p = (y * w + x) * 4;
                v[p..p + 4].copy_from_slice(&c);
            }
        }
        v
    }

    fn all(w: usize, h: usize) -> Vec<u32> {
        (0..(w * h) as u32).collect()
    }

    #[test]
    fn solid_collapses_to_solid() {
        let (w, h) = (32, 32);
        let rgba = img(w, h, |_, _| [100, 150, 200, 255]);
        let f = fit_field(&all(w, h), w, &rgba, 2.0);
        assert_eq!(f.dof, Dof::Solid);
        assert!(f.rmse < 1.0, "rmse {}", f.rmse);
    }

    #[test]
    fn plane_collapses_to_linear() {
        let (w, h) = (32, 32);
        let rgba = img(w, h, |x, y| {
            let r = 20.0 + 200.0 * x as f32 / (w - 1) as f32;
            let g = 30.0 + 150.0 * y as f32 / (h - 1) as f32;
            [r as u8, g as u8, 90, 255]
        });
        let f = fit_field(&all(w, h), w, &rgba, 2.0);
        assert_eq!(f.dof, Dof::Linear, "corners {:?}", f.corners);
        assert!(f.rmse < 2.0, "rmse {}", f.rmse);
    }

    #[test]
    fn twist_stays_bilinear() {
        let (w, h) = (32, 32);
        let rgba = img(w, h, |x, y| {
            let u = x as f32 / (w - 1) as f32;
            let v = y as f32 / (h - 1) as f32;
            [(255.0 * u * v) as u8, 40, 40, 255]
        });
        let f = fit_field(&all(w, h), w, &rgba, 2.0);
        assert_eq!(f.dof, Dof::Bilinear);
        assert!(f.rmse < 2.0, "rmse {}", f.rmse);
    }

    #[test]
    fn alpha_is_a_fitted_channel() {
        // an alpha ramp with constant color: the fit recovers the fade — this
        // is mesh FEATHERING's engine
        let (w, h) = (32, 16);
        let rgba = img(w, h, |x, _| {
            let a = 255.0 * x as f32 / (w - 1) as f32;
            [200, 60, 60, a as u8]
        });
        let f = fit_field(&all(w, h), w, &rgba, 2.0);
        assert_eq!(f.dof, Dof::Linear, "{:?}", f.corners);
        assert!(f.rmse < 2.0, "rmse {}", f.rmse);
        assert!(
            f.corners[0][3] < 5.0 && f.corners[1][3] > 250.0,
            "{:?}",
            f.corners
        );
        // grid fit carries alpha too
        let g = fit_grid(&all(w, h), w, &rgba, 2, 1);
        assert!(g.rmse < 2.0, "rmse {}", g.rmse);
        assert!(
            g.verts[0][3] < 8.0 && g.verts[2][3] > 247.0,
            "{:?}",
            g.verts
        );
    }

    #[test]
    fn radial_is_not_fit_by_one_patch_and_subdivision_converges() {
        let (w, h) = (48, 48);
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let rmax = (cx * cx + cy * cy).sqrt();
        let rgba = img(w, h, |x, y| {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / rmax;
            let val = 250.0 - 220.0 * d;
            [val as u8, (val * 0.5) as u8, 120, 255]
        });
        let idx = all(w, h);
        let single = fit_field(&idx, w, &rgba, 2.0).rmse;
        assert!(
            single > 12.0,
            "radial should not fit one patch; rmse {single}"
        );
        let r1 = fit_grid(&idx, w, &rgba, 1, 1).rmse;
        let r4 = fit_grid(&idx, w, &rgba, 4, 4).rmse;
        let r8 = fit_grid(&idx, w, &rgba, 8, 8).rmse;
        assert!(
            (r1 - single).abs() < 0.5,
            "1x1 grid = single patch: {r1} vs {single}"
        );
        assert!(
            r1 > r4 && r4 > r8,
            "subdivision reduces residual: {r1} > {r4} > {r8}"
        );
        assert!(r8 < r1 * 0.35, "8x8 cuts the residual hard: {r8} vs {r1}");
    }

    #[test]
    fn grid_fit_on_linear_light_recovers_a_bilinear_patch() {
        let (w, h) = (24, 16);
        let corners = [
            [0.0f32, 0.1, 0.9],
            [1.0, 0.2, 0.0],
            [0.5, 0.9, 0.3],
            [0.2, 0.4, 0.7],
        ];
        let mut lin = vec![0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let u = (x as f32 + 0.5) / w as f32;
                let v = (y as f32 + 0.5) / h as f32;
                for c in 0..3 {
                    let bottom = corners[0][c] * (1.0 - u) + corners[1][c] * u;
                    let top = corners[3][c] * (1.0 - u) + corners[2][c] * u;
                    lin[(y * w + x) * 3 + c] = bottom * (1.0 - v) + top * v;
                }
            }
        }
        let verts = fit_grid_lin(w, h, &lin, 1, 1);
        let expect = [corners[0], corners[1], corners[3], corners[2]];
        for (v, e) in verts.iter().zip(expect.iter()) {
            for c in 0..3 {
                assert!((v[c] - e[c]).abs() < 0.02, "{v:?} vs {e:?}");
            }
        }
    }
}
