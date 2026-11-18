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
        // A hanging node is judged at the resolution the format can SAY:
        // corner colours are 8-bit, so the best any author can write is
        // the coarse edge's interpolation ROUNDED to 8 bits — and near
        // white half a quantum is ~4.5e-3 in linear light, past `eps`
        // (a pinned node on a pale skin field cracked at 1.05e-3). Within
        // half a quantum per channel (alpha in 1/255 steps) the hex IS
        // the interpolation, so the junction is smooth.
        let within_8bit = |want: (LinRgb, f32), got: (LinRgb, f32)| {
            const HALF: f32 = 0.5 + 1e-3; // a rounding tie, with f32 slack
            let q = crate::color::linear_to_srgb_quanta;
            (q(want.0.r) - q(got.0.r)).abs() <= HALF
                && (q(want.0.g) - q(got.0.g)).abs() <= HALF
                && (q(want.0.b) - q(got.0.b)).abs() <= HALF
                && ((want.1 - got.1) * 255.0).abs() <= HALF
        };
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
                let agree = |want, got| close(want, got) || within_8bit(want, got);
                if agree(want_a, fine.c_a) && agree(want_b, fine.c_b) {
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

    /// The outline of every region: closed loops of vertex indices whose
    /// nonzero fill is the union of the region's faces. `face_regions` labels
    /// each face `0..regions` (as [`Self::face_regions`] and
    /// [`Raster::face_regions`] do); a face labelled outside that range, or
    /// naming a vertex that does not exist, belongs to no outline.
    ///
    /// Each face is taken with positive signed area, and its edges are summed
    /// as directed pairs: an edge two faces of a region share runs once each
    /// way and cancels, so what is left is the boundary — for an `n×n` grid,
    /// `4n` edges rather than `4n²`. The sum is a cycle (every face is one, and
    /// cancelling a pair keeps every vertex balanced), so it always splits
    /// into closed loops, and its winding number at any point is the number of
    /// faces covering that point — which is why the fill is exact, not an
    /// approximation: holes come out wound the other way, an edge that meets
    /// several shorter ones (a T-junction) leaves a sliver of zero area, and
    /// faces that overlap stay filled.
    ///
    /// Vertices at the same position are one vertex here even when the faces
    /// index them separately, so patches that were tessellated one by one
    /// still cancel along their seams. The output is deterministic: loops
    /// start in face order and follow the order edges were met in.
    pub fn region_outlines(&self, face_regions: &[u32], regions: usize) -> Vec<Vec<Vec<u32>>> {
        // the first index at each position stands for all of them
        let mut at: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        let canon: Vec<u32> = self
            .verts
            .iter()
            .enumerate()
            // `+ 0.0` folds -0.0 into 0.0, which compare equal but differ in bits
            .map(|(i, &(x, y))| {
                *at.entry(((x + 0.0).to_bits(), (y + 0.0).to_bits()))
                    .or_insert(i as u32)
            })
            .collect();
        let mut edges: Vec<Vec<(u32, u32)>> = vec![Vec::new(); regions];
        for (f, face) in self.faces.iter().enumerate() {
            let Some(&region) = face_regions.get(f) else {
                continue;
            };
            let n = face.arity();
            if region as usize >= regions
                || face.v[..n].iter().any(|&v| v as usize >= self.verts.len())
            {
                continue;
            }
            let forward = self.signed_area2(face) >= 0.0;
            for c in 0..n {
                let (a, b) = (
                    canon[face.v[c] as usize],
                    canon[face.v[(c + 1) % n] as usize],
                );
                if a != b {
                    edges[region as usize].push(if forward { (a, b) } else { (b, a) });
                }
            }
        }
        edges.into_iter().map(trace_loops).collect()
    }

    /// Twice the signed area of a face (shoelace), in `f64` so that the sign —
    /// all [`Self::region_outlines`] reads — is stable for thin faces.
    fn signed_area2(&self, face: &Face) -> f64 {
        let n = face.arity();
        (0..n)
            .map(|c| {
                let (ax, ay) = self.verts[face.v[c] as usize];
                let (bx, by) = self.verts[face.v[(c + 1) % n] as usize];
                ax as f64 * by as f64 - bx as f64 * ay as f64
            })
            .sum()
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

/// Cancel opposite directed edges and chain the rest into closed loops.
fn trace_loops(edges: Vec<(u32, u32)>) -> Vec<Vec<u32>> {
    // net multiplicity per direction: an edge meeting its reverse cancels it
    let mut net: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for &(a, b) in &edges {
        match net.get_mut(&(b, a)) {
            Some(n) if *n > 0 => *n -= 1,
            _ => *net.entry((a, b)).or_insert(0) += 1,
        }
    }
    // the survivors, in the order they were met (never the map's)
    let mut boundary: Vec<(u32, u32)> = Vec::new();
    for &edge in &edges {
        if let Some(n) = net.get_mut(&edge) {
            if *n > 0 {
                *n -= 1;
                boundary.push(edge);
            }
        }
    }
    // outgoing edges grouped by start vertex; the stable sort keeps met-order
    boundary.sort_by_key(|e| e.0);
    let mut used = vec![false; boundary.len()];
    // per group (keyed by its first slot): how far its edges are spent
    let mut spent = vec![0usize; boundary.len()];
    let mut next_from = |v: u32, used: &mut [bool]| -> Option<usize> {
        let first = boundary.partition_point(|e| e.0 < v);
        let mut i = first + *spent.get(first)?;
        while i < boundary.len() && boundary[i].0 == v {
            if !used[i] {
                used[i] = true;
                spent[first] = i + 1 - first;
                return Some(i);
            }
            i += 1;
        }
        None
    };
    let mut loops = Vec::new();
    for seed in 0..boundary.len() {
        if used[seed] {
            continue;
        }
        used[seed] = true;
        let start = boundary[seed].0;
        let mut ring = vec![start];
        let mut at = boundary[seed].1;
        // balanced at every vertex, so the walk can only run out back at `start`
        while at != start {
            let Some(i) = next_from(at, &mut used) else {
                break;
            };
            ring.push(at);
            at = boundary[i].1;
        }
        loops.push(ring);
    }
    loops
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
    fn an_8bit_hanging_node_on_a_pale_field_is_smooth() {
        // The authored case: every corner is a HEX colour, so the hanging
        // node can only be the coarse edge's interpolation rounded to
        // 8 bits. Near white that misses the exact lerp by more than eps
        // in linear light (here 1.05e-3) — still the same colour, at the
        // only resolution the format has.
        let hex = |r, g, b| RgbColor::new(r, g, b).to_linear();
        let (tr, br) = (hex(244, 210, 200), hex(236, 200, 214));
        let mid8 = tr.lerp(br, 0.5).to_srgb8().to_linear();
        let mut m = Mesh::default();
        let v = [
            m.add_vertex(0.0, 0.0),
            m.add_vertex(64.0, 0.0),
            m.add_vertex(64.0, 64.0),
            m.add_vertex(0.0, 64.0),
            m.add_vertex(64.0, 32.0), // hanging on 1-2
            m.add_vertex(96.0, 0.0),
            m.add_vertex(96.0, 32.0),
            m.add_vertex(96.0, 64.0),
        ];
        let (tl, bl) = (hex(250, 228, 214), hex(248, 220, 206));
        let (e, f, g) = (hex(238, 214, 196), hex(232, 206, 210), hex(240, 190, 200));
        m.add_quad([v[0], v[1], v[2], v[3]], [tl, tr, br, bl]);
        m.add_quad([v[1], v[5], v[6], v[4]], [tr, e, f, mid8]);
        m.add_quad([v[4], v[6], v[7], v[2]], [mid8, f, g, br]);
        let (_, count) = m.face_regions(1e-3);
        assert_eq!(count, 1, "an 8-bit-exact hanging node is smooth");

        // two quanta off is a real disagreement: still a crack
        let off = RgbColor::new(
            mid8.to_srgb8().r.saturating_sub(2),
            mid8.to_srgb8().g,
            mid8.to_srgb8().b,
        )
        .to_linear();
        let mut m2 = m.clone();
        m2.faces[1].colors[3] = off;
        m2.faces[2].colors[0] = off;
        let (_, count) = m2.face_regions(1e-3);
        assert!(count >= 2, "a 2-quanta hanging-node miss must crack");
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

#[cfg(test)]
mod outline_tests {
    use super::*;

    const C: LinRgb = LinRgb::new(0.5, 0.5, 0.5);

    /// A `w×h` grid of unit quads over shared vertices, row-major.
    fn grid(w: usize, h: usize) -> Mesh {
        let mut m = Mesh::default();
        for y in 0..=h {
            for x in 0..=w {
                m.add_vertex(x as f32, y as f32);
            }
        }
        let at = |x: usize, y: usize| (y * (w + 1) + x) as u32;
        for y in 0..h {
            for x in 0..w {
                m.add_quad(
                    [at(x, y), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)],
                    [C; 4],
                );
            }
        }
        m
    }

    fn one_region(m: &Mesh) -> Vec<Vec<u32>> {
        let mut all = m.region_outlines(&vec![0; m.faces.len()], 1);
        all.remove(0)
    }

    fn points(m: &Mesh, ring: &[u32]) -> Vec<(f32, f32)> {
        ring.iter().map(|&v| m.verts[v as usize]).collect()
    }

    /// Twice the signed area enclosed by a set of loops.
    fn area2(m: &Mesh, loops: &[Vec<u32>]) -> f64 {
        loops
            .iter()
            .map(|ring| {
                let p = points(m, ring);
                (0..p.len())
                    .map(|i| {
                        let (a, b) = (p[i], p[(i + 1) % p.len()]);
                        a.0 as f64 * b.1 as f64 - b.0 as f64 * a.1 as f64
                    })
                    .sum::<f64>()
            })
            .sum()
    }

    /// Winding number of a closed polygon about `p`. Each edge is evaluated
    /// from its lower endpoint whichever way it runs, so an edge and its
    /// reverse contribute exact opposites — sums over faces and over outlines
    /// are then comparable as integers, with no tolerance.
    fn wind(poly: &[(f32, f32)], p: (f32, f32)) -> i32 {
        let mut w = 0;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (lo, hi, sign) = if (a.1, a.0) < (b.1, b.0) {
                (a, b, 1)
            } else {
                (b, a, -1)
            };
            if lo.1 <= p.1 && p.1 < hi.1 {
                let x = lo.0 as f64
                    + (p.1 as f64 - lo.1 as f64) / (hi.1 as f64 - lo.1 as f64)
                        * (hi.0 as f64 - lo.0 as f64);
                if x > p.0 as f64 {
                    w += sign;
                }
            }
        }
        w
    }

    fn face_points(m: &Mesh, face: &Face) -> Vec<(f32, f32)> {
        points(m, &face.v[..face.arity()])
    }

    /// Faces of `region` covering `p`, each counted with positive area.
    fn faces_wind(m: &Mesh, labels: &[u32], region: u32, p: (f32, f32)) -> i32 {
        m.faces
            .iter()
            .zip(labels)
            .filter(|(_, &l)| l == region)
            .map(|(face, _)| {
                let w = wind(&face_points(m, face), p);
                if m.signed_area2(face) >= 0.0 {
                    w
                } else {
                    -w
                }
            })
            .sum()
    }

    fn outline_wind(m: &Mesh, loops: &[Vec<u32>], p: (f32, f32)) -> i32 {
        loops.iter().map(|ring| wind(&points(m, ring), p)).sum()
    }

    /// xorshift64*, so the randomized tests replay exactly.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    #[test]
    fn a_single_face_is_its_own_outline() {
        let m = grid(1, 1);
        let loops = one_region(&m);
        assert_eq!(loops, vec![vec![0, 1, 3, 2]]);
        let mut t = Mesh::default();
        for (x, y) in [(0.0, 0.0), (4.0, 0.0), (0.0, 3.0)] {
            t.add_vertex(x, y);
        }
        t.add_tri([0, 1, 2], [C; 3]);
        assert_eq!(one_region(&t), vec![vec![0, 1, 2]]);
        assert_eq!(area2(&t, &one_region(&t)), 12.0);
    }

    #[test]
    fn a_grid_collapses_to_its_perimeter() {
        for (w, h) in [(1, 2), (2, 2), (3, 5), (8, 8), (20, 20)] {
            let m = grid(w, h);
            let loops = one_region(&m);
            assert_eq!(loops.len(), 1, "{w}x{h}");
            // 4n edges, not 4n²
            assert_eq!(loops[0].len(), 2 * (w + h), "{w}x{h}");
            assert_eq!(area2(&m, &loops), 2.0 * (w * h) as f64);
            // every vertex on the loop is on the rim
            for &v in &loops[0] {
                let (x, y) = m.verts[v as usize];
                assert!(x == 0.0 || y == 0.0 || x == w as f32 || y == h as f32);
            }
        }
    }

    #[test]
    fn a_large_grid_stays_linear() {
        let m = grid(240, 240); // 57,600 faces: the case this exists for
        let loops = one_region(&m);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 960);
        assert_eq!(area2(&m, &loops), 2.0 * 57_600.0);
    }

    #[test]
    fn winding_order_of_the_input_does_not_matter() {
        let mut m = grid(3, 3);
        // reverse every other face, rotate the rest
        for (f, face) in m.faces.iter_mut().enumerate() {
            if f % 2 == 0 {
                face.v.reverse();
            } else {
                face.v.rotate_left(f % 4);
            }
        }
        let loops = one_region(&m);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 12);
        assert_eq!(area2(&m, &loops), 18.0);
    }

    #[test]
    fn regions_get_separate_outlines_that_partition_the_mesh() {
        let m = grid(4, 4);
        // left half / right half
        let halves: Vec<u32> = (0..16).map(|f| (f % 4 >= 2) as u32).collect();
        let out = m.region_outlines(&halves, 2);
        assert_eq!(out.len(), 2);
        for region in &out {
            assert_eq!(region.len(), 1);
            assert_eq!(region[0].len(), 12);
            assert_eq!(area2(&m, region), 16.0);
        }
        // a checkerboard: every face is its own island (corners only touch)
        let checker: Vec<u32> = (0..16).map(|f| ((f % 4 + f / 4) % 2) as u32).collect();
        let out = m.region_outlines(&checker, 2);
        for region in &out {
            assert_eq!(area2(&m, region), 16.0);
            assert_eq!(region.iter().map(Vec::len).sum::<usize>(), 32);
            for cx in 0..4 {
                for cy in 0..4 {
                    let p = (cx as f32 + 0.5, cy as f32 + 0.5);
                    let label = checker[cy * 4 + cx];
                    let inside = outline_wind(&m, region, p) != 0;
                    assert_eq!(inside, std::ptr::eq(region, &out[label as usize]));
                }
            }
        }
    }

    #[test]
    fn a_hole_is_a_second_loop_wound_the_other_way() {
        let mut m = grid(3, 3);
        m.faces.remove(4); // the middle
        let loops = one_region(&m);
        assert_eq!(loops.len(), 2);
        assert_eq!(area2(&m, &loops), 16.0);
        let signs: Vec<bool> = loops
            .iter()
            .map(|l| area2(&m, std::slice::from_ref(l)) > 0.0)
            .collect();
        assert_ne!(signs[0], signs[1]);
        assert_eq!(
            outline_wind(&m, &loops, (1.5, 1.5)),
            0,
            "the hole stays open"
        );
        assert_eq!(outline_wind(&m, &loops, (0.5, 1.5)), 1);
        assert_eq!(outline_wind(&m, &loops, (3.5, 1.5)), 0);
    }

    #[test]
    fn unshared_vertices_at_one_position_still_cancel() {
        // two quads built separately: the seam's vertices are duplicated
        let mut m = Mesh::default();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            m.add_vertex(x, y);
        }
        for (x, y) in [(1.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0)] {
            m.add_vertex(x, y);
        }
        m.add_quad([0, 1, 2, 3], [C; 4]);
        m.add_quad([4, 5, 6, 7], [C; 4]);
        let loops = one_region(&m);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 6);
        assert_eq!(area2(&m, &loops), 4.0);
        // negative zero is the same place as zero
        m.verts[4] = (1.0, -0.0);
        assert_eq!(one_region(&m).len(), 1);
        assert_eq!(one_region(&m)[0].len(), 6);
    }

    #[test]
    fn a_t_junction_keeps_the_area_exact() {
        // one tall quad beside two stacked ones: the tall quad's right edge
        // meets two shorter edges and cancels neither
        let mut m = Mesh::default();
        for (x, y) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (1.0, 2.0),
            (0.0, 2.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (1.0, 1.0),
            (2.0, 2.0),
        ] {
            m.add_vertex(x, y);
        }
        m.add_quad([0, 1, 2, 3], [C; 4]);
        m.add_quad([1, 4, 5, 6], [C; 4]);
        m.add_quad([6, 5, 7, 2], [C; 4]);
        let loops = one_region(&m);
        assert_eq!(area2(&m, &loops), 8.0);
        for p in [(0.5, 0.5), (1.5, 0.5), (1.5, 1.5), (0.5, 1.9), (1.01, 1.0)] {
            assert_eq!(outline_wind(&m, &loops, p), 1, "{p:?}");
        }
        assert_eq!(outline_wind(&m, &loops, (2.5, 1.0)), 0);
    }

    #[test]
    fn faces_touching_at_a_corner_close_properly() {
        let mut m = grid(2, 2);
        m.faces.remove(1);
        m.faces.remove(1); // keep (0,0) and (1,1): a pinch at the centre
        let loops = one_region(&m);
        assert_eq!(area2(&m, &loops), 4.0);
        assert_eq!(loops.iter().map(Vec::len).sum::<usize>(), 8);
        assert_eq!(outline_wind(&m, &loops, (0.5, 0.5)), 1);
        assert_eq!(outline_wind(&m, &loops, (1.5, 1.5)), 1);
        assert_eq!(outline_wind(&m, &loops, (1.5, 0.5)), 0);
        assert_eq!(outline_wind(&m, &loops, (0.5, 1.5)), 0);
    }

    #[test]
    fn degenerate_faces_add_nothing() {
        let mut m = grid(2, 1);
        let before = one_region(&m);
        // a quad collapsed onto an edge, a repeated-vertex triangle, a point
        m.add_quad([0, 1, 1, 0], [C; 4]);
        m.add_tri([2, 2, 5], [C; 3]);
        m.add_quad([4, 4, 4, 4], [C; 4]);
        let after = one_region(&m);
        assert_eq!(area2(&m, &after), area2(&m, &before));
        for p in [(0.5, 0.5), (1.5, 0.5), (2.5, 0.5), (0.5, -0.5)] {
            assert_eq!(outline_wind(&m, &after, p), outline_wind(&m, &before, p));
        }
    }

    #[test]
    fn overlapping_faces_stay_filled() {
        let mut m = grid(1, 1);
        // the same square again, wound the other way: today's per-face
        // emission would cancel it to a hole under nonzero
        m.add_quad([2, 3, 1, 0], [C; 4]);
        let loops = one_region(&m);
        assert_eq!(outline_wind(&m, &loops, (0.5, 0.5)), 2);
        assert_eq!(loops.len(), 2);
    }

    #[test]
    fn bad_labels_and_indices_are_ignored() {
        let mut m = grid(2, 1);
        assert!(Mesh::default().region_outlines(&[], 0).is_empty());
        assert_eq!(m.region_outlines(&[], 2), vec![Vec::<Vec<u32>>::new(); 2]);
        // a label past `regions`, and fewer labels than faces
        let out = m.region_outlines(&[7], 1);
        assert!(out[0].is_empty());
        let out = m.region_outlines(&[0], 1);
        assert_eq!(out[0], vec![vec![0, 1, 4, 3]]);
        // a face naming a vertex that is not there
        m.add_quad([0, 1, 99, 3], [C; 4]);
        let out = m.region_outlines(&[0, 0, 0], 1);
        assert_eq!(area2(&m, &out[0]), 4.0);
        // a region nobody is in
        let out = m.region_outlines(&[0, 0, 0], 3);
        assert!(out[1].is_empty() && out[2].is_empty());
    }

    #[test]
    fn outlines_follow_the_regions_the_rasterizer_painted() {
        // two quads with a colour crack between them → two regions, each its
        // own square; then smooth → one region, one outline
        let mut m = Mesh::default();
        for (x, y) in [
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (0.0, 1.0),
            (1.0, 1.0),
            (2.0, 1.0),
        ] {
            m.add_vertex(x, y);
        }
        let (red, blue) = (LinRgb::new(1.0, 0.0, 0.0), LinRgb::new(0.0, 0.0, 1.0));
        m.add_quad([0, 1, 4, 3], [red; 4]);
        m.add_quad([1, 2, 5, 4], [blue; 4]);
        let raster = m.rasterize(8, 4, (0.0, 0.0), 0.25, 1e-3);
        let out = m.region_outlines(&raster.face_regions, raster.regions);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|r| r.len() == 1 && r[0].len() == 4));
        m.faces[1].colors = [red; 4];
        let raster = m.rasterize(8, 4, (0.0, 0.0), 0.25, 1e-3);
        let out = m.region_outlines(&raster.face_regions, raster.regions);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], vec![vec![0, 1, 2, 5, 4, 3]]);
    }

    #[test]
    fn the_same_mesh_always_gives_the_same_loops() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut m = grid(9, 7);
        let labels: Vec<u32> = (0..m.faces.len()).map(|_| rng.below(3) as u32).collect();
        for _ in 0..20 {
            let i = rng.below(m.faces.len());
            m.faces[i].v.reverse();
        }
        let first = m.region_outlines(&labels, 3);
        for _ in 0..8 {
            assert_eq!(m.region_outlines(&labels, 3), first);
        }
    }

    /// Any mesh at all — random indices over a coarse lattice, so faces
    /// overlap, fold, repeat vertices, share edges three ways and collapse:
    /// the outline's winding number equals the faces' at every point, exactly.
    #[test]
    fn fuzz_arbitrary_meshes_keep_the_winding_number() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for round in 0..4000 {
            let mut m = Mesh::default();
            let side = 2 + rng.below(5);
            for _ in 0..3 + rng.below(14) {
                // a lattice makes coincident vertices and collinear edges common
                let (x, y) = (rng.below(side) as f32, rng.below(side) as f32);
                m.add_vertex(if rng.below(9) == 0 { -x } else { x }, y);
            }
            let nv = m.verts.len();
            for _ in 0..1 + rng.below(24) {
                let tri = rng.below(3) == 0;
                let mut v = || rng.below(nv) as u32;
                if tri {
                    m.add_tri([v(), v(), v()], [C; 3]);
                } else {
                    m.add_quad([v(), v(), v(), v()], [C; 4]);
                }
            }
            let regions = 1 + rng.below(3);
            let labels: Vec<u32> = (0..m.faces.len())
                .map(|_| rng.below(regions) as u32)
                .collect();
            let out = m.region_outlines(&labels, regions);
            assert_eq!(out.len(), regions);
            for (r, loops) in out.iter().enumerate() {
                for ring in loops {
                    assert!(ring.len() >= 3, "round {round}: {ring:?}");
                    assert!(ring.windows(2).all(|w| w[0] != w[1]));
                }
                for _ in 0..40 {
                    let p = (
                        rng.unit() * (2 * side) as f32 - side as f32,
                        rng.unit() * side as f32 - 0.25,
                    );
                    assert_eq!(
                        outline_wind(&m, loops, p),
                        faces_wind(&m, &labels, r as u32, p),
                        "round {round}, region {r}, at {p:?}\n{m:?}\n{labels:?}"
                    );
                }
                // and on lattice lines, where edges and vertices actually are
                for _ in 0..20 {
                    let p = (
                        rng.below(side) as f32 + 0.5 * rng.below(2) as f32,
                        rng.below(side) as f32,
                    );
                    assert_eq!(
                        outline_wind(&m, loops, p),
                        faces_wind(&m, &labels, r as u32, p),
                        "round {round}, region {r}, at {p:?}"
                    );
                }
            }
        }
    }

    /// Meshes like the ones the compiler builds — a jittered grid whose faces
    /// do not overlap, with holes, mixed winding, split quads, duplicated
    /// vertices and random region labels: the outline fills exactly what the
    /// faces cover, which is also what emitting every face used to fill.
    #[test]
    fn fuzz_valid_meshes_fill_exactly_what_their_faces_cover() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for round in 0..600 {
            let (w, h) = (1 + rng.below(7), 1 + rng.below(7));
            let mut m = Mesh::default();
            let mut jitter = Vec::new();
            for y in 0..=h {
                for x in 0..=w {
                    // under a third of a cell: quads stay convex and disjoint
                    let (dx, dy) = (rng.unit() * 0.6 - 0.3, rng.unit() * 0.6 - 0.3);
                    jitter.push((x as f32 + dx, y as f32 + dy));
                }
            }
            let shared = rng.below(2) == 0;
            if shared {
                for &(x, y) in &jitter {
                    m.add_vertex(x, y);
                }
            }
            let regions = 1 + rng.below(4);
            let mut labels = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    if rng.below(6) == 0 {
                        continue; // a hole, or a notch in the rim
                    }
                    let corner = |m: &mut Mesh, cx: usize, cy: usize| {
                        let i = cy * (w + 1) + cx;
                        if shared {
                            i as u32
                        } else {
                            m.add_vertex(jitter[i].0, jitter[i].1)
                        }
                    };
                    let mut v = [
                        corner(&mut m, x, y),
                        corner(&mut m, x + 1, y),
                        corner(&mut m, x + 1, y + 1),
                        corner(&mut m, x, y + 1),
                    ];
                    v.rotate_left(rng.below(4));
                    if rng.below(2) == 0 {
                        v.reverse();
                    }
                    // blocks of faces share a label, so regions have real seams
                    let label = ((x / 2 + y / 2 * 3 + round) % regions) as u32;
                    if rng.below(5) == 0 {
                        m.add_tri([v[0], v[1], v[2]], [C; 3]);
                        m.add_tri([v[0], v[2], v[3]], [C; 3]);
                        labels.extend([label, label]);
                    } else {
                        m.add_quad(v, [C; 4]);
                        labels.push(label);
                    }
                }
            }
            let out = m.region_outlines(&labels, regions);
            for (r, loops) in out.iter().enumerate() {
                let faces: f64 = m
                    .faces
                    .iter()
                    .zip(&labels)
                    .filter(|(_, &l)| l as usize == r)
                    .map(|(f, _)| m.signed_area2(f).abs())
                    .sum();
                assert!(
                    (area2(&m, loops) - faces).abs() < 1e-3,
                    "round {round} region {r}: {} vs {faces}",
                    area2(&m, loops)
                );
                for _ in 0..60 {
                    let p = (
                        rng.unit() * (w as f32 + 1.0) - 0.5,
                        rng.unit() * (h as f32 + 1.0) - 0.5,
                    );
                    let mine = m
                        .faces
                        .iter()
                        .zip(&labels)
                        .filter(|(_, &l)| l as usize == r);
                    let covered = mine.clone().any(|(f, _)| wind(&face_points(&m, f), p) != 0);
                    // every face as its own subpath of one nonzero path
                    let per_face: i32 = mine.map(|(f, _)| wind(&face_points(&m, f), p)).sum();
                    let outline = outline_wind(&m, loops, p);
                    assert_eq!(outline != 0, covered, "round {round} region {r} at {p:?}");
                    assert!(outline == 0 || outline == 1, "simple cover: {outline}");
                    // the old emission agrees wherever faces share a winding
                    // direction; where they do not it could only under-fill
                    assert!(per_face == 0 || covered);
                }
            }
        }
    }
}
