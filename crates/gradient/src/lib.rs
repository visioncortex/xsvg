//! Continuous color fields for xsvg's mesh-gradient lowering (Pillar 3).
//!
//! Extracted from vtracer's `gradient` and `quadmesh` crates (the originals keep
//! the image-tracing pipeline; this is the standalone subset xsvg needs), in
//! pipeline order:
//!
//! 1. **[`mesh`]** — the authorable mesh: shared vertices, quad/tri faces with
//!    per-corner colors (linear-light working space), crack derivation (regions =
//!    faces connected through color-agreeing edges), and a CPU rasterizer.
//! 2. **[`field`]** — continuous color fields over axis-aligned rects: a single
//!    bilinear patch ([`field::ColorField`], 4 corners, DOF-collapsed) and the
//!    seam-free shared-vertex subdivision ([`field::GridField`]), both fitted by
//!    least squares. The doctrine: a high residual means *subdivide* (more grid),
//!    never a new primitive type.
//! 3. **[`contour`]** — pixel-exact region boundaries as closed loops, for
//!    `clipPath` serialization of crack regions.
//! 4. **[`emit`]** — the browser-bilinear hack: place a tiny (gx+1)×(gy+1) image
//!    so its **texel centers** land on the grid vertices; the renderer's smooth
//!    image filter then interpolates the same tensor-product basis as the field —
//!    a bilinear patch serializes *exactly* as a stretched 2×2 PNG.
//! 5. **[`png`]** + [`base64`] — dependency-free encoders for the data URIs
//!    (stored-DEFLATE PNG; the images are a handful of texels, compression is
//!    irrelevant).
//!
//! Everything is pure and platform-free (wasm-safe, no_std-adjacent).

pub mod base64;
pub mod color;
pub mod contour;
pub mod coons;
pub mod emit;
pub mod field;
pub mod mesh;
pub mod png;

pub use color::{linear_to_srgb8, srgb8_to_linear, LinRgb, RgbColor};
pub use contour::{region_contours, Loop};
pub use coons::{cubic, line_edge, reverse_edge, CoonsPatch};
pub use emit::texel_placement;
pub use field::{fit_field, fit_grid, fit_grid_lin, ColorField, Dof, GridField, Rect};
pub use mesh::{Face, Mesh, Raster};
