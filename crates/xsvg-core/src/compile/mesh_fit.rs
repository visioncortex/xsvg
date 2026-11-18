//! Fitting a mesh, apart from writing it (§8.2 stage 1 + 2).
//!
//! [`fit`] is the expensive half of the mesh lowering and a pure function of
//! the mesh and the quality profile: rasterize, label the crack regions, fit
//! each with a grown grid field. What it returns — a [`FittedMesh`] — carries
//! no ids, no source position and no display density, so one fit serves every
//! place the same mesh is drawn, in this compile and the next. `mesh.rs`
//! writes a fit into a document; a [`MeshFits`] store on the platform keeps
//! fits between compiles, keyed by a fingerprint of the mesh ([`MeshKey`]).

use crate::gradient::Mesh;
use crate::QualityProfile;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Bumped whenever [`fit`] would answer differently for the same mesh, so a
/// key minted by one fitter never names another's result.
const FIT_VERSION: u64 = 2;

/// The most elongated a mesh is fitted at. The raster gives the short axis a
/// minimum of rows, so the long axis of a sliver would otherwise cost pixels
/// without limit; past this ratio the long axis is squeezed for the fit and
/// the result stretched back (an affine map, which the field survives).
const MAX_FIT_ASPECT: f32 = 256.0;

/// What of a quality profile the fit depends on: `highest` and `raster` fit
/// alike, and share their fits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum FitClass {
    Fast,
    Balanced,
    Highest,
}

impl FitClass {
    pub(super) fn of(quality: QualityProfile) -> Self {
        match quality {
            QualityProfile::Fast => Self::Fast,
            QualityProfile::Balanced => Self::Balanced,
            QualityProfile::Highest | QualityProfile::Raster => Self::Highest,
        }
    }

    /// Raster resolution: pixels along the long axis, and the least the short
    /// axis is given.
    fn raster(self) -> (f32, f32) {
        match self {
            Self::Fast => (64.0, 24.0),
            Self::Balanced => (128.0, 32.0),
            Self::Highest => (384.0, 48.0),
        }
    }

    /// The rmse tolerance (sRGB units) a region's grid grows to, and its cap.
    fn grid(self) -> (f32, usize) {
        match self {
            Self::Fast => (4.0, 10),
            Self::Balanced => (1.5, 24),
            Self::Highest => (0.5, 48),
        }
    }
}

/// The identity of a fit: a 128-bit fingerprint of everything [`fit`] reads —
/// every vertex, face, corner colour and alpha of the mesh, bit for bit, the
/// fit class of the quality profile, and the fitter's version. Equal keys mean
/// the same fit; a mesh that changed in any way has another key, which is the
/// whole of a store's invalidation.
///
/// It is a fingerprint, not a cryptographic digest: collisions are not a
/// practical event among one user's documents, but a store shared between
/// parties that do not trust each other should not take this as its address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MeshKey([u64; 2]);

impl MeshKey {
    pub(super) fn of_quality(mesh: &Mesh, quality: QualityProfile) -> Self {
        Self::of(mesh, FitClass::of(quality))
    }

    pub(super) fn of(mesh: &Mesh, class: FitClass) -> Self {
        let mut f = Fingerprint::new();
        f.word(FIT_VERSION);
        f.word(class as u64);
        // counts first, so no run of vertices can be read as faces
        f.word(mesh.verts.len() as u64);
        f.word(mesh.faces.len() as u64);
        let pair = |a: u32, b: u32| (a as u64) << 32 | b as u64;
        for &(x, y) in &mesh.verts {
            f.word(pair(x.to_bits(), y.to_bits()));
        }
        for face in &mesh.faces {
            f.word(pair(face.v[0], face.v[1]));
            f.word(pair(face.v[2], face.v[3]));
            for c in &face.colors {
                f.word(pair(c.r.to_bits(), c.g.to_bits()));
                f.word(pair(c.b.to_bits(), 0));
            }
            f.word(pair(face.alpha[0].to_bits(), face.alpha[1].to_bits()));
            f.word(pair(face.alpha[2].to_bits(), face.alpha[3].to_bits()));
        }
        Self(f.finish())
    }

    /// The key as bytes, for a store that is not a `HashMap`.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[..8].copy_from_slice(&self.0[0].to_be_bytes());
        out[8..].copy_from_slice(&self.0[1].to_be_bytes());
        out
    }
}

/// 32 lowercase hex digits.
impl std::fmt::Display for MeshKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}{:016x}", self.0[0], self.0[1])
    }
}

/// Two independent 64-bit lanes over a stream of words. Each lane is a
/// multiply-rotate mix with its own constant and its own order of operations,
/// finished with the word count through an avalanche, so the lanes do not
/// collide together.
struct Fingerprint {
    a: u64,
    b: u64,
    words: u64,
}

impl Fingerprint {
    fn new() -> Self {
        Self {
            a: 0x243F_6A88_85A3_08D3,
            b: 0x1319_8A2E_0370_7344,
            words: 0,
        }
    }

    fn word(&mut self, w: u64) {
        self.a = (self.a ^ w)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(29);
        self.b = (self.b.rotate_left(31) ^ w.wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
            .wrapping_mul(0x1656_67B1_9E37_79F9);
        self.words += 1;
    }

    fn finish(self) -> [u64; 2] {
        // splitmix64's finalizer
        let avalanche = |mut z: u64| {
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        [
            avalanche(self.a ^ self.words),
            avalanche(self.b ^ self.words.rotate_left(32)),
        ]
    }
}

/// A fitted mesh: per crack region, its exact outline and its paint — a flat
/// colour, or the texel grid whose bilinear reconstruction is the fitted
/// field. Everything about a mesh's lowering that does not depend on where it
/// is drawn. Opaque: made by the compiler, kept by a [`MeshFits`] store.
#[derive(Debug)]
pub struct FittedMesh {
    pub(super) regions: Vec<FittedRegion>,
    /// 3 when every corner is opaque, 4 when the textures carry alpha
    pub(super) channels: usize,
}

#[derive(Debug)]
pub(super) struct FittedRegion {
    /// the region's label in the raster — its number in the clip ids
    pub(super) index: usize,
    /// the region's outline, as path data in the mesh's coordinates
    pub(super) clip_d: String,
    pub(super) paint: RegionPaint,
}

#[derive(Debug)]
pub(super) enum RegionPaint {
    /// One colour throughout: sRGB8, and its straight alpha in [0, 1].
    Solid { rgb: [u8; 3], alpha: f32 },
    /// A grid of texels, one per grid vertex, placed so their centers land on
    /// the vertices.
    Texture {
        /// interleaved sRGB8 (+ alpha), row-major, `size.0 * size.1` texels
        texels: Vec<u8>,
        size: (usize, usize),
        /// x, y, width, height of the image, in the mesh's coordinates
        place: (f64, f64, f64, f64),
        /// the compact output — those texels as a stored PNG, base64
        compact: String,
    },
}

impl FittedMesh {
    /// Roughly what keeping this fit costs, in bytes: what a bounded store
    /// counts.
    pub fn weight(&self) -> usize {
        let regions: usize = self
            .regions
            .iter()
            .map(|r| {
                r.clip_d.len()
                    + match &r.paint {
                        RegionPaint::Solid { .. } => 0,
                        RegionPaint::Texture {
                            texels, compact, ..
                        } => texels.len() + compact.len(),
                    }
            })
            .sum();
        regions + self.regions.len() * std::mem::size_of::<FittedRegion>()
    }
}

/// Fit `mesh` (§8.2 stage 1 + 2): rasterize at the class's resolution, fit
/// each crack region with a grid field grown until its residual passes the
/// class's tolerance. `None` for a mesh with no extent.
pub(super) fn fit(mesh: &Mesh, class: FitClass) -> Option<FittedMesh> {
    use crate::gradient::{fit_field, fit_grid, texel_placement, Dof};

    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &mesh.verts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    // fit space = user space ÷ (kx, ky); all of it but a sliver is 1:1
    let (kx, ky) = {
        let (ex, ey) = (x1 - x0, y1 - y0);
        if ex > ey * MAX_FIT_ASPECT {
            (ex / (ey * MAX_FIT_ASPECT), 1.0)
        } else if ey > ex * MAX_FIT_ASPECT {
            (1.0, ey / (ex * MAX_FIT_ASPECT))
        } else {
            (1.0f32, 1.0f32)
        }
    };
    if !(kx.is_finite() && ky.is_finite()) {
        return None;
    }
    let squeezed;
    let (user, mesh) = (mesh, {
        if (kx, ky) == (1.0, 1.0) {
            mesh
        } else {
            let mut m = mesh.clone();
            for v in &mut m.verts {
                *v = (v.0 / kx, v.1 / ky);
            }
            squeezed = m;
            &squeezed
        }
    });
    let (x0, y0, x1, y1) = (x0 / kx, y0 / ky, x1 / kx, y1 / ky);
    let (max_px, min_px) = class.raster();
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
        if l == crate::gradient::mesh::NONE {
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

    let (tol, cap) = class.grid();
    let channels = if opaque { 3 } else { 4 };
    // Exact clip geometry: each region's outline. Naming every face instead
    // costs a polygon per tessellated quad — megabytes for a fine mesh, all of
    // it interior edges that cancel (crates/gradient `region_outlines`).
    let outlines = user.region_outlines(&raster.face_regions, raster.regions);
    let mut regions = Vec::new();
    for r in 0..raster.regions {
        if region_px[r].is_empty() {
            continue;
        }
        let mut clip_d = String::new();
        super::mesh::append_outline(user, &outlines[r], &mut clip_d);

        let single = fit_field(&region_px[r], w, &rgba, 2.0);
        if single.dof == Dof::Solid {
            let c = single.corners[0];
            regions.push(FittedRegion {
                index: r,
                clip_d,
                paint: RegionPaint::Solid {
                    rgb: [0, 1, 2].map(|k| c[k].round().clamp(0.0, 255.0) as u8),
                    alpha: (c[3] / 255.0).clamp(0.0, 1.0),
                },
            });
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
        let mut texels = Vec::with_capacity(tw * th * channels);
        for vert in &grid.verts {
            for v in &vert[..channels] {
                texels.push(v.round().clamp(0.0, 255.0) as u8);
            }
        }
        let (ix, iy, iw, ih) = texel_placement(bx0, by0, bx1, by1, tw, th);
        // back out of fit space: the grid's placement in the mesh's own units
        let (kx, ky) = (kx as f64, ky as f64);
        let place = (
            (x0 as f64 + ix * scale as f64) * kx,
            (y0 as f64 + iy * scale as f64) * ky,
            iw * scale as f64 * kx,
            ih * scale as f64 * ky,
        );
        // The compact grid stays byte-for-byte what it always was: a few
        // texels, stored.
        let png = if opaque {
            crate::gradient::png::encode_rgb_png(tw as u32, th as u32, &texels)
        } else {
            crate::gradient::png::encode_rgba_png(tw as u32, th as u32, &texels)
        };
        regions.push(FittedRegion {
            index: r,
            clip_d,
            paint: RegionPaint::Texture {
                texels,
                size: (tw, th),
                place,
                compact: crate::gradient::base64::encode(&png),
            },
        });
    }
    Some(FittedMesh { regions, channels })
}

/// Where fits are kept between compiles — the platform's seam for it, as
/// [`Measurer`](crate::Measurer) is for font metrics. The compiler asks before
/// fitting and offers what it fitted; the store decides what to keep and for
/// how long. Nothing is ever invalidated: a [`MeshKey`] names one fit for
/// good, so a store only has to bound itself.
///
/// Transparent by contract — a store returns what it was given for that key —
/// so output is byte-identical with or without one.
pub trait MeshFits {
    /// The fit kept under `key`, if there is one.
    fn fitted(&self, key: &MeshKey) -> Option<Arc<FittedMesh>>;

    /// A fit the compiler just made. Called on every miss.
    fn keep(&self, key: MeshKey, fit: &Arc<FittedMesh>);

    /// Whether anything is kept at all. `false` only for [`NoMeshCache`],
    /// which lets the compiler skip fingerprinting meshes nobody will look up.
    fn active(&self) -> bool {
        true
    }
}

impl<T: MeshFits + ?Sized> MeshFits for &T {
    fn fitted(&self, key: &MeshKey) -> Option<Arc<FittedMesh>> {
        (**self).fitted(key)
    }
    fn keep(&self, key: MeshKey, fit: &Arc<FittedMesh>) {
        (**self).keep(key, fit)
    }
    fn active(&self) -> bool {
        (**self).active()
    }
}

/// The inert default: every mesh is fitted where it is drawn, every time.
pub struct NoMeshCache;

impl MeshFits for NoMeshCache {
    fn fitted(&self, _: &MeshKey) -> Option<Arc<FittedMesh>> {
        None
    }
    fn keep(&self, _: MeshKey, _: &Arc<FittedMesh>) {}
    fn active(&self) -> bool {
        false
    }
}

/// A [`MeshFits`] store in memory: every fit made while it lives, by key. An
/// editor recompiles a shape on every move, and the mesh that paints it has
/// not changed; a mesh that paints ten shapes is the same mesh ten times.
/// Behind this, each is fitted once.
///
/// Bounded: past [`MeshCache::CAPACITY`] bytes of fits or
/// [`MeshCache::ENTRIES`] of them it starts over rather than grow without
/// limit — dragging a mesh's vertex mints a key per frame, and none of them is
/// asked for again. Owned by the embedder and handed to
/// `Platform::with_mesh_cache`, by value or by reference; `Send + Sync`, so one
/// may serve every thread.
#[derive(Default)]
pub struct MeshCache {
    fits: Mutex<Kept>,
    hits: AtomicU64,
    misses: AtomicU64,
}

#[derive(Default)]
struct Kept {
    map: HashMap<MeshKey, Arc<FittedMesh>>,
    weight: usize,
}

impl MeshCache {
    /// Bytes of fits kept before the cache is cleared and refilled. A fit is a
    /// few kilobytes; the largest, a fine mesh's outline, some tens.
    pub const CAPACITY: usize = 32 << 20;
    /// Fits kept before the cache is cleared and refilled.
    pub const ENTRIES: usize = 4096;

    pub fn new() -> Self {
        Self::default()
    }

    /// Lookups answered from the cache since it was made.
    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Lookups that found nothing — each one a fit the compiler then made.
    pub fn misses(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }

    /// Fits currently kept.
    pub fn len(&self) -> usize {
        self.kept().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Forget every fit. Never needed for correctness — keys do not go stale —
    /// only to give the memory back, say when a document closes.
    pub fn clear(&self) {
        let mut kept = self.kept();
        kept.map.clear();
        kept.weight = 0;
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        // A poisoned lock means a panic mid-insert elsewhere; the map is still
        // a valid cache, so take it rather than fail every later lookup.
        self.fits.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl MeshFits for MeshCache {
    fn fitted(&self, key: &MeshKey) -> Option<Arc<FittedMesh>> {
        let hit = self.kept().map.get(key).cloned();
        let counter = if hit.is_some() {
            &self.hits
        } else {
            &self.misses
        };
        counter.fetch_add(1, Ordering::Relaxed);
        hit
    }

    // The lock is never held across a fit: the compiler fits between the two
    // calls, and two threads missing the same mesh simply both fit it.
    fn keep(&self, key: MeshKey, fit: &Arc<FittedMesh>) {
        let weight = fit.weight();
        let mut kept = self.kept();
        // the same key again replaces its fit: it is not a second entry
        if let Some(old) = kept.map.remove(&key) {
            kept.weight -= old.weight();
        }
        if kept.map.len() >= Self::ENTRIES || kept.weight + weight > Self::CAPACITY {
            kept.map.clear();
            kept.weight = 0;
        }
        kept.map.insert(key, fit.clone());
        kept.weight += weight;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gradient::LinRgb;

    fn quad() -> Mesh {
        let mut m = Mesh::default();
        for p in [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 6.0),
            (0.0, 6.0),
            (20.0, 0.0),
            (20.0, 6.0),
        ] {
            m.add_vertex(p.0, p.1);
        }
        let c = |r: f32, g: f32, b: f32| LinRgb { r, g, b };
        m.add_quad_a(
            [0, 1, 2, 3],
            [
                c(1.0, 0.0, 0.0),
                c(0.0, 1.0, 0.0),
                c(0.0, 0.0, 1.0),
                c(1.0, 1.0, 1.0),
            ],
            [1.0; 4],
        );
        m.add_tri_a(
            [1, 4, 5],
            [c(0.2, 0.3, 0.4), c(0.5, 0.6, 0.7), c(0.8, 0.9, 1.0)],
            [1.0, 0.5, 0.25],
        );
        m
    }

    fn key(m: &Mesh) -> MeshKey {
        MeshKey::of(m, FitClass::Balanced)
    }

    #[test]
    fn equal_meshes_have_one_key_and_it_is_stable() {
        assert_eq!(key(&quad()), key(&quad().clone()));
        // a key is a value: the same in every process, on every platform
        assert_eq!(key(&quad()).to_string().len(), 32);
        assert_eq!(key(&quad()).to_string(), key(&quad()).to_string());
        let bytes = key(&quad()).to_bytes();
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, key(&quad()).to_string());
    }

    #[test]
    fn everything_the_fit_reads_is_in_the_key() {
        let base = key(&quad());
        let mut seen = std::collections::HashSet::from([base]);
        let mut differs = |what: &str, edit: &dyn Fn(&mut Mesh)| {
            let mut m = quad();
            edit(&mut m);
            assert!(
                seen.insert(key(&m)),
                "{what} is not in the key, or collides"
            );
        };
        differs("a vertex x", &|m| m.verts[2].0 += 0.001);
        differs("a vertex y", &|m| m.verts[2].1 += 0.001);
        differs("the smallest move", &|m| {
            m.verts[0].0 = f32::from_bits(m.verts[0].0.to_bits() + 1)
        });
        differs("an unused vertex", &|m| {
            m.add_vertex(3.0, 3.0);
        });
        differs("a vertex index", &|m| m.faces[0].v = [1, 2, 3, 0]);
        differs("a face's arity", &|m| {
            m.faces[0].v[3] = crate::gradient::mesh::NONE
        });
        differs("red", &|m| m.faces[0].colors[1].r += 0.01);
        differs("green", &|m| m.faces[0].colors[1].g += 0.01);
        differs("blue", &|m| m.faces[0].colors[3].b -= 0.01);
        differs("alpha", &|m| m.faces[1].alpha[2] = 0.3);
        differs("the order of faces", &|m| m.faces.swap(0, 1));
        differs("two vertices swapped", &|m| m.verts.swap(0, 1));
        differs("a face removed", &|m| {
            m.faces.pop();
        });
        differs("a face repeated", &|m| {
            let f = m.faces[0];
            m.faces.push(f);
        });
        // swapping values between fields of one face is a different mesh
        differs("colour channels exchanged", &|m| {
            let c = &mut m.faces[0].colors[0];
            std::mem::swap(&mut c.r, &mut c.g);
        });
        // the fit class is part of it; `raster` fits as `highest` does
        let m = quad();
        let keys: Vec<MeshKey> = [FitClass::Fast, FitClass::Balanced, FitClass::Highest]
            .map(|c| MeshKey::of(&m, c))
            .to_vec();
        assert!(keys[0] != keys[1] && keys[1] != keys[2] && keys[0] != keys[2]);
        assert_eq!(
            FitClass::of(QualityProfile::Raster),
            FitClass::of(QualityProfile::Highest)
        );
        assert_ne!(
            FitClass::of(QualityProfile::Fast),
            FitClass::of(QualityProfile::Balanced)
        );
    }

    #[test]
    fn vertices_cannot_be_read_as_faces() {
        // the same words, split differently between the two lists
        let mut a = Mesh::default();
        let mut b = Mesh::default();
        for i in 0..12 {
            a.add_vertex(i as f32, 1.0);
        }
        for i in 0..2 {
            b.add_vertex(i as f32, 1.0);
        }
        assert_ne!(key(&a), key(&b));
        assert_ne!(key(&Mesh::default()), key(&b));
    }

    #[test]
    fn a_single_bit_anywhere_moves_both_lanes() {
        // 4000 one-bit edits of one mesh: no two keys alike, and neither half
        // of any key left where it was
        let base = quad();
        let k0 = key(&base);
        let mut seen = std::collections::HashSet::from([k0]);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut flips = 0u32;
        for _ in 0..4000 {
            let mut m = base.clone();
            let bit = 1u32 << (next() % 32);
            match next() % 4 {
                0 => {
                    let v = &mut m.verts[(next() % 6) as usize];
                    v.0 = f32::from_bits(v.0.to_bits() ^ bit);
                }
                1 => {
                    let v = &mut m.verts[(next() % 6) as usize];
                    v.1 = f32::from_bits(v.1.to_bits() ^ bit);
                }
                2 => {
                    let c = &mut m.faces[(next() % 2) as usize].colors[(next() % 4) as usize];
                    c.g = f32::from_bits(c.g.to_bits() ^ bit);
                }
                _ => {
                    let a = &mut m.faces[(next() % 2) as usize].alpha[(next() % 4) as usize];
                    *a = f32::from_bits(a.to_bits() ^ bit);
                }
            }
            let k = key(&m);
            assert!(k.0[0] != k0.0[0] && k.0[1] != k0.0[1], "a lane stood still");
            seen.insert(k);
            flips += (k.0[0] ^ k0.0[0]).count_ones() + (k.0[1] ^ k0.0[1]).count_ones();
        }
        // 6 vertices × 64 bits + 8 colours × 32 + 8 alphas × 32 distinct edits
        // exist; every one drawn has a key of its own
        assert!(seen.len() > 600, "{}", seen.len());
        // and a one-bit edit flips about half of the 128 bits
        let mean = flips as f64 / 4000.0;
        assert!((54.0..74.0).contains(&mean), "{mean}");
    }

    #[test]
    fn a_fit_is_a_function_of_the_mesh() {
        let (a, b) = (
            fit(&quad(), FitClass::Balanced).unwrap(),
            fit(&quad(), FitClass::Balanced).unwrap(),
        );
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
        assert_eq!(a.channels, 4, "the triangle is feathered");
        assert!(a.weight() > 0);
        // no extent, no fit
        let mut flat = quad();
        for v in &mut flat.verts {
            v.1 = 0.0;
        }
        assert!(fit(&flat, FitClass::Balanced).is_none());
        assert!(fit(&Mesh::default(), FitClass::Fast).is_none());
    }

    fn blob(bytes: usize) -> Arc<FittedMesh> {
        Arc::new(FittedMesh {
            regions: vec![FittedRegion {
                index: 0,
                clip_d: "M".repeat(bytes),
                paint: RegionPaint::Solid {
                    rgb: [0; 3],
                    alpha: 1.0,
                },
            }],
            channels: 3,
        })
    }

    fn nth(n: u64) -> MeshKey {
        MeshKey([n, !n])
    }

    #[test]
    fn the_cache_returns_what_it_was_given_and_counts() {
        let cache = MeshCache::new();
        assert!(cache.is_empty() && cache.active());
        assert!(cache.fitted(&nth(1)).is_none());
        let fit = blob(10);
        cache.keep(nth(1), &fit);
        let hit = cache.fitted(&nth(1)).unwrap();
        assert!(Arc::ptr_eq(&hit, &fit), "the very fit, not a copy");
        assert!(cache.fitted(&nth(2)).is_none());
        assert_eq!((cache.hits(), cache.misses(), cache.len()), (1, 2, 1));
        // through a reference it is the same store
        let by_ref: &dyn MeshFits = &&cache;
        assert!(by_ref.fitted(&nth(1)).is_some() && by_ref.active());
        assert_eq!(cache.hits(), 2);
        cache.clear();
        assert!(cache.is_empty() && cache.fitted(&nth(1)).is_none());
        // the inert default keeps nothing and says so
        NoMeshCache.keep(nth(1), &fit);
        assert!(NoMeshCache.fitted(&nth(1)).is_none() && !NoMeshCache.active());
    }

    #[test]
    fn the_cache_starts_over_at_its_entry_limit() {
        let cache = MeshCache::new();
        for n in 0..MeshCache::ENTRIES as u64 {
            cache.keep(nth(n), &blob(1));
        }
        assert_eq!(cache.len(), MeshCache::ENTRIES);
        assert!(cache.fitted(&nth(0)).is_some());
        // one more: everything before it goes, and it is kept
        cache.keep(nth(u64::MAX), &blob(1));
        assert_eq!(cache.len(), 1);
        assert!(cache.fitted(&nth(0)).is_none() && cache.fitted(&nth(u64::MAX)).is_some());
    }

    #[test]
    fn the_cache_starts_over_at_its_byte_limit() {
        let cache = MeshCache::new();
        let big = MeshCache::CAPACITY / 4;
        for n in 0..3 {
            cache.keep(nth(n), &blob(big));
        }
        assert_eq!(cache.len(), 3);
        // the same key again replaces its fit and is counted once
        for _ in 0..10 {
            cache.keep(nth(2), &blob(big));
        }
        assert_eq!(cache.len(), 3);
        // a fourth quarter (plus overhead) does not fit beside the other three
        cache.keep(nth(3), &blob(big));
        assert_eq!(cache.len(), 1);
        assert!(cache.kept().weight < MeshCache::CAPACITY);
        // a fit larger than the whole capacity is still kept, alone
        cache.keep(nth(9), &blob(MeshCache::CAPACITY + 1));
        assert_eq!(cache.len(), 1);
        assert!(cache.fitted(&nth(9)).is_some());
        cache.keep(nth(10), &blob(1));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_poisoned_lock_is_still_a_cache() {
        let cache = Arc::new(MeshCache::new());
        cache.keep(nth(1), &blob(1));
        let held = cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.fits.lock().unwrap();
            panic!("mid-insert");
        })
        .join();
        assert!(cache.fits.is_poisoned());
        assert!(cache.fitted(&nth(1)).is_some());
        cache.keep(nth(2), &blob(1));
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn one_cache_serves_every_thread() {
        fn shared<T: Send + Sync>() {}
        shared::<MeshCache>();
        shared::<MeshKey>();
        shared::<Arc<FittedMesh>>();
        let cache = Arc::new(MeshCache::new());
        let threads: Vec<_> = (0..8u64)
            .map(|t| {
                let cache = cache.clone();
                std::thread::spawn(move || {
                    for n in 0..200 {
                        let key = nth(n % 50);
                        if cache.fitted(&key).is_none() {
                            cache.keep(key, &blob((t + 1) as usize));
                        }
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(cache.len(), 50);
        assert_eq!(cache.hits() + cache.misses(), 1600);
    }
}
