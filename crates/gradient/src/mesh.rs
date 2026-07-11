//! The mesh: shared vertices, quad/tri faces with per-corner colors, crack
//! derivation, and a CPU rasterizer (stage 1 of the lowering pipeline).
//!
//! Model and math ported from vtracer's `quadmesh` crate: faces wind CCW, a
//! quad's corners map to local coordinates `0→(0,0) 1→(1,0) 2→(1,1) 3→(0,1)`,
//! bilinear color is `mix(mix(c0,c1,u), mix(c3,c2,u), v)` evaluated in
//! **linear-light RGB** (inverse-bilinear for arbitrary quads — Íñigo Quílez's
//! invBilinear — barycentric for triangles). A **crack** is an edge whose two
//! incident faces disagree on color at either shared endpoint; a **region** is a
//! maximal set of faces connected through non-crack edges. Regions are what get
//! one fitted field + one clip path each in the SVG lowering.

use crate::color::LinRgb;

/// Sentinel for a triangle's unused fourth vertex slot.
pub const NONE: u32 = u32::MAX;

/// One face: 3 or 4 CCW vertex indices (`v[3] == NONE` for a triangle) and the
/// corner colors in linear-light RGB.
#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub v: [u32; 4],
    pub colors: [LinRgb; 4],
    /// per-corner straight alpha in [0,1] — feathering; 1.0 = opaque
    pub alpha: [f32; 4],
}

impl Face {
    pub fn arity(&self) -> usize {
        if self.v[3] == NONE {
            3
        } else {
            4
        }
    }
}

/// A quad-dominant mesh with per-face-corner colors.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub verts: Vec<(f32, f32)>,
    pub faces: Vec<Face>,
}

/// A rasterized mesh: per-pixel linear-light RGB plus the region label (or
/// [`NONE`] outside every face). Pixel `(x, y)` samples the mesh at the pixel
/// center `(x + 0.5, y + 0.5)` offset by `origin`.
pub struct Raster {
    pub w: usize,
    pub h: usize,
    /// linear-light RGB, `w*h*3`, row-major
    pub lin: Vec<f32>,
    /// region label per pixel; [`NONE`] where no face covers the pixel
    pub labels: Vec<u32>,
    /// number of regions (labels are `0..regions`)
    pub regions: usize,
    /// region label per face (the labeling the pixels were painted with) —
    /// callers build exact per-region clip geometry from the face polygons
    pub face_regions: Vec<u32>,
    /// straight alpha per pixel, [0,1]
    pub alpha: Vec<f32>,
}

impl Raster {
    /// The linear-light image encoded to interleaved sRGB8 RGB (`w*h*3`).
    /// Uncovered pixels encode as black; consult `labels` for coverage.
    pub fn to_srgb8(&self) -> Vec<u8> {
        self.lin
            .iter()
            .map(|&v| crate::color::linear_to_srgb8(v))
            .collect()
    }

    /// Interleaved sRGB8 + straight alpha (`w*h*4`).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for i in 0..self.w * self.h {
            for c in 0..3 {
                out.push(crate::color::linear_to_srgb8(self.lin[i * 3 + c]));
            }
            out.push((self.alpha[i] * 255.0 + 0.5).clamp(0.0, 255.0) as u8);
        }
        out
    }

    /// Whether every covered pixel is fully opaque (within 1/255).
    pub fn fully_opaque(&self) -> bool {
        self.labels
            .iter()
            .zip(&self.alpha)
            .all(|(&l, &a)| l == NONE || a >= 254.5 / 255.0)
    }
}

impl Mesh {
    pub fn add_vertex(&mut self, x: f32, y: f32) -> u32 {
        self.verts.push((x, y));
        (self.verts.len() - 1) as u32
    }

    pub fn add_quad(&mut self, v: [u32; 4], colors: [LinRgb; 4]) {
        self.add_quad_a(v, colors, [1.0; 4]);
    }

    pub fn add_quad_a(&mut self, v: [u32; 4], colors: [LinRgb; 4], alpha: [f32; 4]) {
        self.faces.push(Face { v, colors, alpha });
    }

    pub fn add_tri(&mut self, v: [u32; 3], colors: [LinRgb; 3]) {
        self.add_tri_a(v, colors, [1.0; 3]);
    }

    pub fn add_tri_a(&mut self, v: [u32; 3], colors: [LinRgb; 3], alpha: [f32; 3]) {
        self.faces.push(Face {
            v: [v[0], v[1], v[2], NONE],
            colors: [colors[0], colors[1], colors[2], LinRgb::default()],
            alpha: [alpha[0], alpha[1], alpha[2], 1.0],
        });
    }

    fn pos(&self, v: u32) -> (f32, f32) {
        self.verts[v as usize]
    }

    /// Region id per face: union-find over **smooth** shared edges. An edge is
    /// smooth iff both incident faces agree (within `eps`, linear-light) on the
    /// color at each shared endpoint; a mismatch is a crack and separates
    /// regions. This is `quadmesh::build_edges`' classification folded directly
    /// into the component labeling.
    pub fn face_regions(&self, eps: f32) -> (Vec<u32>, usize) {
        let nf = self.faces.len();
        let mut parent: Vec<u32> = (0..nf as u32).collect();
        fn find(p: &mut [u32], mut x: u32) -> u32 {
            while p[x as usize] != x {
                p[x as usize] = p[p[x as usize] as usize];
                x = p[x as usize];
            }
            x
        }
        let close = |a: (LinRgb, f32), b: (LinRgb, f32)| {
            (a.0.r - b.0.r).abs() <= eps
                && (a.0.g - b.0.g).abs() <= eps
                && (a.0.b - b.0.b).abs() <= eps
                && (a.1 - b.1).abs() <= eps
        };

        // edge key -> (face, corner color+alpha at lo end, at hi end)
        type Corner = (LinRgb, f32);
        let mut seen: std::collections::HashMap<(u32, u32), (u32, Corner, Corner)> =
            std::collections::HashMap::new();
        for (f, face) in self.faces.iter().enumerate() {
            let n = face.arity();
            for c in 0..n {
                let a = face.v[c];
                let b = face.v[(c + 1) % n];
                let ca = (face.colors[c], face.alpha[c]);
                let cb = (face.colors[(c + 1) % n], face.alpha[(c + 1) % n]);
                let (lo, hi, c_lo, c_hi) = if a < b {
                    (a, b, ca, cb)
                } else {
                    (b, a, cb, ca)
                };
                match seen.get(&(lo, hi)) {
                    None => {
                        seen.insert((lo, hi), (f as u32, c_lo, c_hi));
                    }
                    Some(&(g, g_lo, g_hi)) => {
                        if close(c_lo, g_lo) && close(c_hi, g_hi) {
                            let (ra, rb) = (find(&mut parent, f as u32), find(&mut parent, g));
                            if ra != rb {
                                parent[ra as usize] = rb;
                            }
                        }
                    }
                }
            }
        }
        // ---- smooth-interior T-junction pass: an edge with no exact partner
        // may face a CHAIN of finer edges whose vertices lie on it (hanging
        // nodes). If the coarse side's interpolated color+alpha agrees with the
        // fine side at both of the finer edge's endpoints, the junction is
        // smooth and the faces join.
        struct Side {
            face: u32,
            a: (f32, f32),
            b: (f32, f32),
            c_a: (LinRgb, f32),
            c_b: (LinRgb, f32),
        }
        let unpartnered: Vec<Side> = {
            let mut count: std::collections::HashMap<(u32, u32), u32> =
                std::collections::HashMap::new();
            for face in &self.faces {
                let n = face.arity();
                for c in 0..n {
                    let (a, b) = (face.v[c], face.v[(c + 1) % n]);
                    let key = if a < b { (a, b) } else { (b, a) };
                    *count.entry(key).or_insert(0) += 1;
                }
            }
            let mut out = Vec::new();
            for (f, face) in self.faces.iter().enumerate() {
                let n = face.arity();
                for c in 0..n {
                    let (a, b) = (face.v[c], face.v[(c + 1) % n]);
                    let key = if a < b { (a, b) } else { (b, a) };
                    if count[&key] == 1 {
                        out.push(Side {
                            face: f as u32,
                            a: self.pos(a),
                            b: self.pos(b),
                            c_a: (face.colors[c], face.alpha[c]),
                            c_b: (face.colors[(c + 1) % n], face.alpha[(c + 1) % n]),
                        });
                    }
                }
            }
            out
        };
        let lerp_corner =
            |a: (LinRgb, f32), b: (LinRgb, f32), t: f32| (a.0.lerp(b.0, t), a.1 + (b.1 - a.1) * t);
        // is p on segment (a,b), strictly interior-ish? returns the parameter
        let on_segment = |a: (f32, f32), b: (f32, f32), p: (f32, f32)| -> Option<f32> {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            if len2 < 1e-12 {
                return None;
            }
            let t = ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2;
            if !(-1e-4..=1.0001).contains(&t) {
                return None;
            }
            let (px, py) = (a.0 + t * dx, a.1 + t * dy);
            let d2 = (p.0 - px).powi(2) + (p.1 - py).powi(2);
            (d2 < 1e-4 * len2.max(1.0)).then_some(t)
        };
        for coarse in &unpartnered {
            for fine in &unpartnered {
                if coarse.face == fine.face {
                    continue;
                }
                // both of the finer edge's endpoints must lie on the coarse edge
                // (and it must genuinely be finer, not the same span)
                let (Some(ta), Some(tb)) = (
                    on_segment(coarse.a, coarse.b, fine.a),
                    on_segment(coarse.a, coarse.b, fine.b),
                ) else {
                    continue;
                };
                if (tb - ta).abs() >= 0.9999 {
                    continue;
                }
                let want_a = lerp_corner(coarse.c_a, coarse.c_b, ta);
                let want_b = lerp_corner(coarse.c_a, coarse.c_b, tb);
                if close(want_a, fine.c_a) && close(want_b, fine.c_b) {
                    let (ra, rb) = (find(&mut parent, coarse.face), find(&mut parent, fine.face));
                    if ra != rb {
                        parent[ra as usize] = rb;
                    }
                }
            }
        }

        // compact roots to 0..count
        let mut label = vec![0u32; nf];
        let mut next = 0u32;
        let mut root_label: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for f in 0..nf {
            let r = find(&mut parent, f as u32);
            let l = *root_label.entry(r).or_insert_with(|| {
                let l = next;
                next += 1;
                l
            });
            label[f] = l;
        }
        (label, next as usize)
    }

    /// Face color + alpha at point `p` (linear RGB, straight alpha):
    /// inverse-bilinear `uv` for quads, barycentric for triangles.
    pub fn eval_face(&self, f: usize, p: (f32, f32)) -> (LinRgb, f32) {
        let face = &self.faces[f];
        let c = &face.colors;
        let al = &face.alpha;
        if face.arity() == 4 {
            let (u, v) = inverse_bilinear(
                self.pos(face.v[0]),
                self.pos(face.v[1]),
                self.pos(face.v[2]),
                self.pos(face.v[3]),
                p,
            );
            let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
            let bottom = c[0].lerp(c[1], u);
            let top = c[3].lerp(c[2], u);
            let ab = al[0] + (al[1] - al[0]) * u;
            let at = al[3] + (al[2] - al[3]) * u;
            (bottom.lerp(top, v), ab + (at - ab) * v)
        } else {
            let (w0, w1, w2) = barycentric(
                self.pos(face.v[0]),
                self.pos(face.v[1]),
                self.pos(face.v[2]),
                p,
            );
            (
                LinRgb::new(
                    c[0].r * w0 + c[1].r * w1 + c[2].r * w2,
                    c[0].g * w0 + c[1].g * w1 + c[2].g * w2,
                    c[0].b * w0 + c[1].b * w1 + c[2].b * w2,
                ),
                al[0] * w0 + al[1] * w1 + al[2] * w2,
            )
        }
    }

    /// Rasterize into a `w×h` grid whose pixel `(x, y)` samples mesh coordinates
    /// `(origin.0 + (x+0.5)·scale, origin.1 + (y+0.5)·scale)`. Faces paint in
    /// order via per-face scanline fill (a watertight mesh double-paints only
    /// the measure-zero shared edges, harmlessly). `eps` is the crack tolerance
    /// for [`Self::face_regions`], in linear-light units.
    pub fn rasterize(
        &self,
        w: usize,
        h: usize,
        origin: (f32, f32),
        scale: f32,
        eps: f32,
    ) -> Raster {
        let (face_region, regions) = self.face_regions(eps);
        let mut lin = vec![0f32; w * h * 3];
        let mut alpha = vec![0f32; w * h];
        let mut labels = vec![NONE; w * h];
        let face_regions_out = face_region.clone();

        for (f, face) in self.faces.iter().enumerate() {
            let n = face.arity();
            let poly: Vec<(f32, f32)> = (0..n).map(|c| self.pos(face.v[c])).collect();
            let ys: Vec<f32> = poly.iter().map(|p| p.1).collect();
            let (min_y, max_y) = (
                ys.iter().cloned().fold(f32::MAX, f32::min),
                ys.iter().cloned().fold(f32::MIN, f32::max),
            );
            let row0 = (((min_y - origin.1) / scale - 0.5).floor().max(0.0)) as usize;
            let row1 = ((((max_y - origin.1) / scale - 0.5).ceil()) as isize).min(h as isize - 1);
            if row1 < 0 {
                continue;
            }
            for y in row0..=(row1 as usize) {
                let my = origin.1 + (y as f32 + 0.5) * scale;
                // even-odd crossings of the horizontal line at my
                let mut xs: Vec<f32> = Vec::with_capacity(4);
                for i in 0..n {
                    let (x0, y0) = poly[i];
                    let (x1, y1) = poly[(i + 1) % n];
                    if (y0 <= my && my < y1) || (y1 <= my && my < y0) {
                        xs.push(x0 + (my - y0) / (y1 - y0) * (x1 - x0));
                    }
                }
                xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let mut k = 0;
                while k + 1 < xs.len() {
                    let px0 = (((xs[k] - origin.0) / scale - 0.5).ceil().max(0.0)) as usize;
                    let px1 = ((((xs[k + 1] - origin.0) / scale - 0.5).floor()) as isize)
                        .min(w as isize - 1);
                    let mut x = px0 as isize;
                    while x <= px1 {
                        let mx = origin.0 + (x as f32 + 0.5) * scale;
                        let (c, a) = self.eval_face(f, (mx, my));
                        let o = (y * w + x as usize) * 3;
                        lin[o] = c.r;
                        lin[o + 1] = c.g;
                        lin[o + 2] = c.b;
                        alpha[y * w + x as usize] = a;
                        labels[y * w + x as usize] = face_region[f];
                        x += 1;
                    }
                    k += 2;
                }
            }
        }
        Raster {
            w,
            h,
            lin,
            labels,
            regions,
            face_regions: face_regions_out,
            alpha,
        }
    }
}

#[inline]
fn cross(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    ax * by - ay * bx
}

/// Barycentric weights `(w0, w1, w2)` of `p` wrt triangle `(a, b, c)`.
pub fn barycentric(a: (f32, f32), b: (f32, f32), c: (f32, f32), p: (f32, f32)) -> (f32, f32, f32) {
    let v0 = (b.0 - a.0, b.1 - a.1);
    let v1 = (c.0 - a.0, c.1 - a.1);
    let v2 = (p.0 - a.0, p.1 - a.1);
    let den = cross(v0.0, v0.1, v1.0, v1.1);
    if den.abs() < 1e-12 {
        return (1.0, 0.0, 0.0);
    }
    let w1 = cross(v2.0, v2.1, v1.0, v1.1) / den;
    let w2 = cross(v0.0, v0.1, v2.0, v2.1) / den;
    (1.0 - w1 - w2, w1, w2)
}

/// Invert the bilinear map for corners `p0..p3` (uv `00,10,11,01`) at `p`,
/// returning `(u, v)`. (Íñigo Quílez's invBilinear, ported via quadmesh.)
pub fn inverse_bilinear(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    p: (f32, f32),
) -> (f32, f32) {
    let e = (p1.0 - p0.0, p1.1 - p0.1);
    let f = (p3.0 - p0.0, p3.1 - p0.1);
    let g = (p0.0 - p1.0 + p2.0 - p3.0, p0.1 - p1.1 + p2.1 - p3.1);
    let h = (p.0 - p0.0, p.1 - p0.1);

    let k2 = cross(g.0, g.1, f.0, f.1);
    let k1 = cross(e.0, e.1, f.0, f.1) + cross(h.0, h.1, g.0, g.1);
    let k0 = cross(h.0, h.1, e.0, e.1);

    let v = if k2.abs() < 1e-9 {
        if k1.abs() < 1e-12 {
            0.0
        } else {
            -k0 / k1
        }
    } else {
        let disc = (k1 * k1 - 4.0 * k0 * k2).max(0.0).sqrt();
        let v1 = (-k1 + disc) / (2.0 * k2);
        let v2 = (-k1 - disc) / (2.0 * k2);
        if (0.0..=1.0).contains(&v1) || (v1 - 0.5).abs() <= (v2 - 0.5).abs() {
            v1
        } else {
            v2
        }
    };
    let denx = e.0 + g.0 * v;
    let deny = e.1 + g.1 * v;
    let u = if denx.abs() >= deny.abs() {
        if denx.abs() < 1e-12 {
            0.0
        } else {
            (h.0 - f.0 * v) / denx
        }
    } else if deny.abs() < 1e-12 {
        0.0
    } else {
        (h.1 - f.1 * v) / deny
    };
    (u, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::RgbColor;

    fn lin(r: u8, g: u8, b: u8) -> LinRgb {
        RgbColor::new(r, g, b).to_linear()
    }

    #[test]
    fn inverse_bilinear_round_trips_on_a_skewed_quad() {
        let (p0, p1, p2, p3) = ((0.0, 0.0), (10.0, 1.0), (12.0, 9.0), (-1.0, 8.0));
        for &(u, v) in &[(0.25, 0.25), (0.5, 0.5), (0.9, 0.1), (0.0, 1.0)] {
            // forward bilinear map
            let bx = |a: (f32, f32), b: (f32, f32), t: f32| {
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            };
            let bot = bx(p0, p1, u);
            let top = bx(p3, p2, u);
            let p = bx(bot, top, v);
            let (ru, rv) = inverse_bilinear(p0, p1, p2, p3, p);
            assert!(
                (ru - u).abs() < 1e-4 && (rv - v).abs() < 1e-4,
                "({u},{v}) -> ({ru},{rv})"
            );
        }
    }

    #[test]
    fn cracks_split_regions_and_smooth_edges_join_them() {
        // two quads sharing an edge with AGREEING colors -> one region;
        // flip one shared corner -> crack -> two regions
        let mut m = Mesh::default();
        let v = [
            m.add_vertex(0.0, 0.0),
            m.add_vertex(10.0, 0.0),
            m.add_vertex(20.0, 0.0),
            m.add_vertex(0.0, 10.0),
            m.add_vertex(10.0, 10.0),
            m.add_vertex(20.0, 10.0),
        ];
        let (red, green) = (lin(255, 0, 0), lin(0, 255, 0));
        m.add_quad([v[0], v[1], v[4], v[3]], [red, green, green, red]);
        m.add_quad([v[1], v[2], v[5], v[4]], [green, red, red, green]);
        let (_, count) = m.face_regions(1e-4);
        assert_eq!(count, 1, "agreeing shared edge is smooth");

        let mut m2 = m.clone();
        m2.faces[1].colors[0] = red; // disagree at the shared edge's top end
        let (_, count) = m2.face_regions(1e-4);
        assert_eq!(count, 2, "color mismatch is a crack");
    }

    #[test]
    fn smooth_t_junctions_join_regions_and_cracked_ones_split() {
        // one coarse quad on the left; TWO stacked finer quads on the right
        // sharing its right edge with a hanging node at the midpoint
        let mut m = Mesh::default();
        let v = [
            m.add_vertex(0.0, 0.0),   // 0
            m.add_vertex(10.0, 0.0),  // 1
            m.add_vertex(10.0, 10.0), // 2
            m.add_vertex(0.0, 10.0),  // 3
            m.add_vertex(10.0, 5.0),  // 4: hanging node on edge 1-2
            m.add_vertex(20.0, 0.0),  // 5
            m.add_vertex(20.0, 5.0),  // 6
            m.add_vertex(20.0, 10.0), // 7
        ];
        let (black, white) = (
            RgbColor::new(0, 0, 0).to_linear(),
            RgbColor::new(255, 255, 255).to_linear(),
        );
        let mid = black.lerp(white, 0.5);
        // coarse: black at top-right corner, white at bottom-right
        m.add_quad([v[0], v[1], v[2], v[3]], [black, black, white, white]);
        // fine top: matches coarse's lerp at the hanging node (mid)
        m.add_quad([v[1], v[5], v[6], v[4]], [black, black, mid, mid]);
        // fine bottom
        m.add_quad([v[4], v[6], v[7], v[2]], [mid, mid, white, white]);
        let (_, count) = m.face_regions(1e-3);
        assert_eq!(count, 1, "color-consistent T-junction is smooth");

        // flip the hanging-node color: the junction cracks
        let mut m2 = m.clone();
        m2.faces[1].colors[3] = black; // fine-top's corner at the hanging node
        let (_, count) = m2.face_regions(1e-3);
        assert!(count >= 2, "mismatched hanging node must split");
    }

    #[test]
    fn raster_samples_face_colors_and_labels_coverage() {
        let mut m = Mesh::default();
        let v = [
            m.add_vertex(0.0, 0.0),
            m.add_vertex(16.0, 0.0),
            m.add_vertex(16.0, 8.0),
            m.add_vertex(0.0, 8.0),
        ];
        let (black, white) = (lin(0, 0, 0), lin(255, 255, 255));
        // horizontal ramp black -> white
        m.add_quad([v[0], v[1], v[2], v[3]], [black, white, white, black]);
        let r = m.rasterize(16, 8, (0.0, 0.0), 1.0, 1e-4);
        assert_eq!(r.regions, 1);
        assert!(r.labels.iter().all(|&l| l == 0), "full coverage");
        // linear-light ramp: left pixel near 0, right pixel near 1, monotone
        let row = 4;
        let at = |x: usize| r.lin[(row * 16 + x) * 3];
        assert!(at(0) < 0.05 && at(15) > 0.9, "{} {}", at(0), at(15));
        assert!((0..15).all(|x| at(x) <= at(x + 1) + 1e-6), "monotone ramp");
    }

    #[test]
    fn triangles_rasterize_with_barycentric_color() {
        let mut m = Mesh::default();
        let v = [
            m.add_vertex(0.0, 0.0),
            m.add_vertex(12.0, 0.0),
            m.add_vertex(0.0, 12.0),
        ];
        m.add_tri(
            [v[0], v[1], v[2]],
            [lin(255, 0, 0), lin(0, 255, 0), lin(0, 0, 255)],
        );
        let r = m.rasterize(12, 12, (0.0, 0.0), 1.0, 1e-4);
        // covered near the right-angle corner, uncovered past the hypotenuse
        assert_eq!(r.labels[0], 0);
        assert_eq!(r.labels[11 * 12 + 11], NONE);
        // corner pixel is dominated by corner 0's red
        let o = 0;
        assert!(r.lin[o] > r.lin[o + 1] && r.lin[o] > r.lin[o + 2]);
    }
}
