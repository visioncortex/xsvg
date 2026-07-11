//! Fidelity + performance bench for the grid-fit solver and the alpha
//! pipeline — the numbers behind docs/Bench.md. Run release:
//!
//!     cargo run --release -p gradient --example bench_fit
//!
//! 1. SOLVER: banded conjugate gradient (shipped) vs a dense Gauss–Jordan
//!    reference, on a hard radial field. Fidelity must match (same normal
//!    equations); time must not cliff at large grids.
//! 2. ALPHA: fitting straight vs premultiplied RGBA on a feathered field,
//!    error measured in *premultiplied* space (what compositing shows over
//!    any background).

use gradient::{fit_grid, GridField};
use std::time::Instant;

/// Dense Gauss–Jordan reference (the solver the CG replaced), assembled from
/// the same bilinear footprint — used only to check fidelity/time here.
fn fit_grid_dense(indices: &[u32], w: usize, rgba: &[u8], gx: usize, gy: usize) -> GridField {
    let rect = gradient::Rect::of_indices(indices, w);
    let (dx, dy) = ((rect.width - 1).max(1) as f32, (rect.height - 1).max(1) as f32);
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
        let vs = [cj * g + ci, cj * g + ci + 1, (cj + 1) * g + ci, (cj + 1) * g + ci + 1];
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
    let ridge = 1e-6 * indices.len().max(1) as f64;
    for (i, row) in mat.iter_mut().enumerate() {
        row[i] += ridge;
    }
    // Gauss–Jordan
    let n = nv;
    for col in 0..n {
        let mut piv = col;
        for r in (col + 1)..n {
            if mat[r][col].abs() > mat[piv][col].abs() {
                piv = r;
            }
        }
        mat.swap(col, piv);
        rhs.swap(col, piv);
        let d = mat[col][col];
        if d.abs() < 1e-12 {
            continue;
        }
        for k in col..n {
            mat[col][k] /= d;
        }
        for k in 0..4 {
            rhs[col][k] /= d;
        }
        for r in 0..n {
            if r == col || mat[r][col] == 0.0 {
                continue;
            }
            let f = mat[r][col];
            for k in col..n {
                mat[r][k] -= f * mat[col][k];
            }
            for k in 0..4 {
                rhs[r][k] -= f * rhs[col][k];
            }
        }
    }
    let verts = rhs
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
    let mut field = GridField { rect, gx, gy, verts, rmse: 0.0 };
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
    field.rmse = ((sse / (indices.len().max(1) as f64 * 4.0)).max(0.0)).sqrt() as f32;
    field
}

fn psnr(rmse: f64) -> f64 {
    if rmse <= 1e-9 {
        f64::INFINITY
    } else {
        20.0 * (255.0 / rmse).log10()
    }
}

fn main() {
    // ---- 1. solver: hard radial field, 256×256 region
    let (w, h) = (256usize, 256usize);
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let rmax = (cx * cx + cy * cy).sqrt();
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt() / rmax;
            let v = 250.0 - 220.0 * d;
            let o = (y * w + x) * 4;
            rgba[o] = v as u8;
            rgba[o + 1] = (v * 0.6) as u8;
            rgba[o + 2] = (255.0 - v) as u8;
            rgba[o + 3] = 255;
        }
    }
    let idx: Vec<u32> = (0..(w * h) as u32).collect();

    println!("SOLVER — radial 256x256, square grids (times in ms)");
    println!("{:>6} {:>10} {:>10} {:>10} {:>10} {:>12}", "grid", "cg ms", "dense ms", "cg rmse", "dense rmse", "rmse delta");
    for g in [8usize, 16, 24, 32, 48] {
        let t0 = Instant::now();
        let a = fit_grid(&idx, w, &rgba, g, g);
        let t_cg = t0.elapsed().as_secs_f64() * 1000.0;
        let (t_dense, b_rmse) = if g <= 32 {
            let t1 = Instant::now();
            let b = fit_grid_dense(&idx, w, &rgba, g, g);
            (t1.elapsed().as_secs_f64() * 1000.0, b.rmse)
        } else {
            (f64::NAN, f32::NAN) // dense at 48² (2401³ flops) is the cliff itself
        };
        println!(
            "{:>4}x{:<2} {:>10.1} {:>10.1} {:>10.4} {:>10.4} {:>12.6}",
            g, g, t_cg, t_dense, a.rmse, b_rmse, (a.rmse - b_rmse).abs()
        );
    }

    // ---- 2. alpha: feathered field, error in premultiplied space
    let (w2, h2) = (128usize, 64usize);
    let mut straight = vec![0u8; w2 * h2 * 4];
    for y in 0..h2 {
        for x in 0..w2 {
            // color varies horizontally, alpha fades vertically — the layered
            // gloss case
            let cxf = x as f64 / (w2 - 1) as f64;
            let a = 1.0 - y as f64 / (h2 - 1) as f64;
            let o = (y * w2 + x) * 4;
            straight[o] = (200.0 + 55.0 * cxf) as u8;
            straight[o + 1] = (230.0 - 80.0 * cxf) as u8;
            straight[o + 2] = 255;
            straight[o + 3] = (255.0 * a * a) as u8; // nonlinear fade
        }
    }
    let idx2: Vec<u32> = (0..(w2 * h2) as u32).collect();
    let mut premul = straight.clone();
    for i in 0..w2 * h2 {
        let a = premul[i * 4 + 3] as f64 / 255.0;
        for c in 0..3 {
            premul[i * 4 + c] = (premul[i * 4 + c] as f64 * a + 0.5) as u8;
        }
    }
    // The browser premultiplies texels BEFORE interpolating (Skia et al.), so
    // the honest recon model is: take the STRAIGHT texel values the PNG would
    // carry, premultiply them per-vertex, interpolate that, and compare with
    // the premultiplied truth.
    let browser_recon_err = |straight_verts: &GridField| -> f64 {
        let mut pm = straight_verts.clone();
        for v in &mut pm.verts {
            let a = v[3] as f64 / 255.0;
            for c in 0..3 {
                v[c] = (v[c] as f64 * a) as f32;
            }
        }
        let mut sse = 0f64;
        for i in 0..w2 * h2 {
            let (x, y) = ((i % w2) as i32, (i / w2) as i32);
            let p = pm.eval(x, y);
            let ra = straight[i * 4 + 3] as f64 / 255.0;
            for c in 0..3 {
                let want = straight[i * 4 + c] as f64 * ra;
                sse += (want - p[c] as f64) * (want - p[c] as f64);
            }
            let wa = straight[i * 4 + 3] as f64;
            sse += (wa - p[3] as f64) * (wa - p[3] as f64);
        }
        (sse / (w2 * h2 * 4) as f64).sqrt()
    };
    for g in [3usize, 6, 12] {
        println!("\nALPHA — feathered 128x64, grid {g}x{g}, browser-compositing error");
        // straight pipeline: fit straight, emit straight
        let fs = fit_grid(&idx2, w2, &straight, g, g);
        // premul pipeline: fit premultiplied, emit straight (= premul / alpha)
        let fp = fit_grid(&idx2, w2, &premul, g, g);
        let mut fp_straight = fp.clone();
        for v in &mut fp_straight.verts {
            let a = (v[3] as f64 / 255.0).max(1.0 / 255.0);
            for c in 0..3 {
                v[c] = ((v[c] as f64 / a).min(255.0)) as f32;
            }
        }
        let es = browser_recon_err(&fs);
        let ep = browser_recon_err(&fp_straight);
        println!("  straight-fit : rmse {es:.3}  psnr {:.1} dB", psnr(es));
        println!("  premul-fit   : rmse {ep:.3}  psnr {:.1} dB", psnr(ep));
    }
}
