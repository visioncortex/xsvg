//! Coons patches — the SVG 2 / Inkscape `<meshgradient>` primitive: a quad
//! whose four edges are **cubic Béziers**, with one color per corner. The
//! interior is the standard Coons surface (the bilinear blend of the four
//! boundary curves minus the bilinear corner blend), and color interpolates
//! bilinearly from the corners (SVG 2's default; `type="bicubic"` is a
//! smoothing refinement we treat as bilinear in v1).
//!
//! A patch tessellates into the straight-quad [`Mesh`](crate::mesh::Mesh) —
//! polycurve → points — so the whole render→refit lowering pipeline applies
//! unchanged. Adjacent patches share edge curves (the SVG 2 inheritance
//! rules), so tessellating both sides at the same density lands the same
//! boundary points; a quantized dedup map makes them the *same vertices*, and
//! matching corner colors then join the patches into one smooth region.

use crate::color::LinRgb;
use crate::mesh::Mesh;
use std::collections::HashMap;

/// Evaluate a cubic Bézier at `t`.
pub fn cubic(p: [(f32, f32); 4], t: f32) -> (f32, f32) {
    let s = 1.0 - t;
    let (a, b, c, d) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
    (
        a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0,
        a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1,
    )
}

/// One Coons patch. Edges are stored **clockwise from the top-left corner**,
/// each as 4 cubic control points in walking order:
/// `top TL→TR, right TR→BR, bottom BR→BL, left BL→TL`.
/// Colors are the corner colors `[TL, TR, BR, BL]` (linear-light).
#[derive(Clone, Copy, Debug)]
pub struct CoonsPatch {
    pub edges: [[(f32, f32); 4]; 4],
    pub colors: [LinRgb; 4],
    /// per-corner straight alpha [TL, TR, BR, BL] — SVG 2 `stop-opacity`
    pub alpha: [f32; 4],
    /// SVG 2 `type="bicubic"` approximation: ease `(u, v)` with smoothstep
    /// before the corner blend. The tangential derivative is then zero at
    /// every patch boundary — both sides — so adjacent patches meet C¹ and
    /// the bilinear Mach bands at the seams disappear.
    pub eased: bool,
}

impl CoonsPatch {
    /// The Coons surface point at patch-local `(u, v)` (`u` rightward, `v`
    /// downward, both in [0,1]).
    pub fn point(&self, u: f32, v: f32) -> (f32, f32) {
        let t = cubic(self.edges[0], u); // top, TL→TR
        let r = cubic(self.edges[1], v); // right, TR→BR
        let b = cubic(self.edges[2], 1.0 - u); // bottom stored BR→BL
        let l = cubic(self.edges[3], 1.0 - v); // left stored BL→TL
        let tl = self.edges[0][0];
        let tr = self.edges[1][0];
        let br = self.edges[2][0];
        let bl = self.edges[3][0];
        let blend = |e: f32, f: f32, g: f32, h: f32| (1.0 - v) * e + v * f + (1.0 - u) * g + u * h;
        let corner = |cx: fn((f32, f32)) -> f32| {
            (1.0 - u) * (1.0 - v) * cx(tl)
                + u * (1.0 - v) * cx(tr)
                + u * v * cx(br)
                + (1.0 - u) * v * cx(bl)
        };
        (
            blend(t.0, b.0, l.0, r.0) - corner(|p| p.0),
            blend(t.1, b.1, l.1, r.1) - corner(|p| p.1),
        )
    }

    #[inline]
    fn ease(&self, t: f32) -> f32 {
        if self.eased {
            t * t * (3.0 - 2.0 * t)
        } else {
            t
        }
    }

    /// Corner-color interpolation at `(u, v)` — bilinear, or smoothstep-eased
    /// when [`Self::eased`] (the `type="bicubic"` approximation).
    pub fn color(&self, u: f32, v: f32) -> LinRgb {
        let (u, v) = (self.ease(u), self.ease(v));
        let [tl, tr, br, bl] = self.colors;
        let top = tl.lerp(tr, u);
        let bottom = bl.lerp(br, u);
        top.lerp(bottom, v)
    }

    /// Corner-alpha interpolation at `(u, v)` (same easing as [`Self::color`]).
    pub fn alpha_at(&self, u: f32, v: f32) -> f32 {
        let (u, v) = (self.ease(u), self.ease(v));
        let [tl, tr, br, bl] = self.alpha;
        let top = tl + (tr - tl) * u;
        let bottom = bl + (br - bl) * u;
        top + (bottom - top) * v
    }

    /// Tessellate into `n × n` straight quads appended to `mesh`, deduplicating
    /// vertices through `dedup` (position quantized to 1e-3) so adjacent
    /// patches sharing an edge curve share the mesh vertices too.
    pub fn tessellate_into(&self, mesh: &mut Mesh, n: usize, dedup: &mut HashMap<(i64, i64), u32>) {
        let n = n.max(1);
        let vid = |mesh: &mut Mesh, dedup: &mut HashMap<(i64, i64), u32>, x: f32, y: f32| -> u32 {
            let key = ((x * 1000.0).round() as i64, (y * 1000.0).round() as i64);
            *dedup.entry(key).or_insert_with(|| mesh.add_vertex(x, y))
        };
        let mut ids = vec![0u32; (n + 1) * (n + 1)];
        for j in 0..=n {
            for i in 0..=n {
                let (x, y) = self.point(i as f32 / n as f32, j as f32 / n as f32);
                ids[j * (n + 1) + i] = vid(mesh, dedup, x, y);
            }
        }
        for j in 0..n {
            for i in 0..n {
                let uv = |i: usize, j: usize| (i as f32 / n as f32, j as f32 / n as f32);
                let c = |(u, v): (f32, f32)| self.color(u, v);
                let a = |(u, v): (f32, f32)| self.alpha_at(u, v);
                mesh.add_quad_a(
                    [
                        ids[j * (n + 1) + i],
                        ids[j * (n + 1) + i + 1],
                        ids[(j + 1) * (n + 1) + i + 1],
                        ids[(j + 1) * (n + 1) + i],
                    ],
                    [
                        c(uv(i, j)),
                        c(uv(i + 1, j)),
                        c(uv(i + 1, j + 1)),
                        c(uv(i, j + 1)),
                    ],
                    [
                        a(uv(i, j)),
                        a(uv(i + 1, j)),
                        a(uv(i + 1, j + 1)),
                        a(uv(i, j + 1)),
                    ],
                );
            }
        }
    }
}

/// A straight edge from `a` to `b` as a cubic (controls at the third points).
pub fn line_edge(a: (f32, f32), b: (f32, f32)) -> [(f32, f32); 4] {
    let lerp = |t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    [a, lerp(1.0 / 3.0), lerp(2.0 / 3.0), b]
}

/// Reverse an edge (for the SVG 2 inheritance rules: a shared edge is the
/// neighbour's edge walked backwards).
pub fn reverse_edge(e: [(f32, f32); 4]) -> [(f32, f32); 4] {
    [e[3], e[2], e[1], e[0]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::RgbColor;

    fn lin(r: u8, g: u8, b: u8) -> LinRgb {
        RgbColor::new(r, g, b).to_linear()
    }

    fn flat_patch(w: f32, h: f32) -> CoonsPatch {
        let (tl, tr, br, bl) = ((0.0, 0.0), (w, 0.0), (w, h), (0.0, h));
        CoonsPatch {
            edges: [
                line_edge(tl, tr),
                line_edge(tr, br),
                line_edge(br, bl),
                line_edge(bl, tl),
            ],
            colors: [
                lin(255, 0, 0),
                lin(0, 255, 0),
                lin(0, 0, 255),
                lin(255, 255, 0),
            ],
            alpha: [1.0; 4],
            eased: false,
        }
    }

    #[test]
    fn flat_patch_is_the_bilinear_quad() {
        let p = flat_patch(100.0, 50.0);
        for &(u, v) in &[(0.0, 0.0), (1.0, 0.0), (0.5, 0.5), (0.25, 0.75), (1.0, 1.0)] {
            let (x, y) = p.point(u, v);
            assert!((x - 100.0 * u).abs() < 1e-3, "u={u} v={v}: x={x}");
            assert!((y - 50.0 * v).abs() < 1e-3, "u={u} v={v}: y={y}");
        }
    }

    #[test]
    fn curved_edge_bulges_the_surface() {
        // top edge bows upward: interior points near the top must lift above
        // the chord, and the mid-top point matches the cubic itself
        let mut p = flat_patch(100.0, 50.0);
        p.edges[0] = [(0.0, 0.0), (33.0, -30.0), (67.0, -30.0), (100.0, 0.0)];
        let (top_x, top_y) = p.point(0.5, 0.0);
        let on_curve = cubic(p.edges[0], 0.5);
        assert!((top_x - on_curve.0).abs() < 1e-3 && (top_y - on_curve.1).abs() < 1e-3);
        assert!(top_y < -10.0, "bulge: {top_y}");
        // corners never move
        assert_eq!(p.point(0.0, 0.0), (0.0, 0.0));
        assert_eq!(p.point(1.0, 1.0), (100.0, 50.0));
    }

    #[test]
    fn eased_patches_flatten_the_gradient_at_their_edges() {
        let mut p = flat_patch(100.0, 50.0);
        // bilinear: color changes linearly near the edge; eased: derivative ~0
        let lin_step = p.color(0.1, 0.0).r - p.color(0.0, 0.0).r;
        p.eased = true;
        let eased_step = p.color(0.1, 0.0).r - p.color(0.0, 0.0).r;
        assert!(
            eased_step.abs() < lin_step.abs() * 0.4,
            "eased edge must be flatter: {eased_step} vs {lin_step}"
        );
        // center value unchanged (smoothstep(0.5) = 0.5)
        p.eased = false;
        let mid_lin = p.color(0.5, 0.5);
        p.eased = true;
        let mid_eased = p.color(0.5, 0.5);
        assert!((mid_lin.r - mid_eased.r).abs() < 1e-6);
    }

    #[test]
    fn tessellation_dedups_shared_edges_between_patches() {
        // two flat patches side by side sharing the vertical edge; the second
        // stores the shared edge REVERSED (the inheritance convention)
        let a = flat_patch(10.0, 10.0);
        let shared = a.edges[1]; // TR→BR of a = left edge of b walked BL→TL reversed
        let (tr, br) = ((20.0, 0.0), (20.0, 10.0));
        let b = CoonsPatch {
            edges: [
                line_edge((10.0, 0.0), tr),
                line_edge(tr, br),
                line_edge(br, (10.0, 10.0)),
                reverse_edge(shared),
            ],
            colors: a.colors,
            alpha: [1.0; 4],
            eased: false,
        };
        let mut mesh = Mesh::default();
        let mut dedup = HashMap::new();
        a.tessellate_into(&mut mesh, 4, &mut dedup);
        b.tessellate_into(&mut mesh, 4, &mut dedup);
        // 2 patches × 5×5 grids sharing one 5-vertex column
        assert_eq!(mesh.verts.len(), 25 + 25 - 5);
        assert_eq!(mesh.faces.len(), 32);
    }
}
