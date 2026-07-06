//! The geometry-transform pipeline (§7): pure deformation **fields** and the **bake**
//! that lowers them — flatten → map → emit (§7.1; cubic refit is a later quality
//! upgrade). Platform-free and kurbo-backed: the wasm layer hands SVG path data in
//! and gets warped path data out, so the math is natively unit-testable.

use kurbo::simplify::{simplify_bezpath, SimplifyOptLevel, SimplifyOptions};
use kurbo::{BezPath, PathEl, Point, Rect, Shape};

/// A deformation field: a pure point map `D : ℝ² → ℝ²` (§7.2). Implementations may
/// precompute state (an envelope frame, an arc-length table) but expose only
/// per-point evaluation — the bake is field-agnostic.
pub trait Field {
    fn map(&self, p: Point) -> Point;
}

/// Which axis a warp preset bends along (Illustrator's Horizontal/Vertical radio):
/// `H` sweeps the profile across x and displaces y; `V` the transpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WarpAxis {
    #[default]
    H,
    V,
}

impl WarpAxis {
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("v") {
            Self::V
        } else {
            Self::H
        }
    }
}

/// The Envelope-Distort presets (§7.2, Transform.md §B), grouped by field family.
/// Displacement presets sweep a 1-D profile across the bend axis; the 2-D families
/// (scale / radial / rotational) evaluate over the whole normalized frame. `r̂` below
/// is the corner-normalized radius `√((nx² + ny²)/2)` — 1 at the frame's corners, so
/// every radial/rotational preset pins the corners.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    /// displacement — both edges ride one parabola: `Δ = A·(1 − u²)`
    Arch,
    /// displacement — one full sine, uniform through the height: `Δ = A·sin(π·u)`
    Flag,
    /// displacement — linear ramp (pure shear profile): `Δ = A·u`
    Rise,
    /// displacement — Flag whose phase advances by π/2 through the height:
    /// `Δ = A·sin(π·u − (π/4)·(v + 1))`
    Wave,
    /// radial — magnify about the center: `s = 1 + b·(1 − r̂²)²` (barrel; negative
    /// bend = pincushion); corners fixed. The squared profile keeps `r·s(r)`
    /// monotone for every `|b| ≤ 1`, so the field never folds the outer ring
    /// back over itself
    Fisheye,
    /// radial, axis-separable — each axis bulges by the other's parabola:
    /// `sx = 1 + (b/2)(1 − ny²)`, `sy = 1 + (b/2)(1 − nx²)`; corners fixed
    Inflate,
    /// scale — the bend axis pinches by a profile of `v` (waist at mid-height):
    /// `u′ = u·(1 − (b/2)(1 − v²))`; negative bend = barrel
    Squeeze,
    /// polar — the box bends into an annular sector spanning `Θ = b·π` (a semicircle
    /// at 100%): the midline maps to an arc of radius `R = L/Θ` (its length is
    /// preserved), perpendicular lines become radii. The whole envelope relocates —
    /// no pinned corners
    Arc,
    /// scale — the top edge stays fixed and the height scales by the parabola
    /// `s = 1 + (b/2)(1 − u²)`, so the bottom edge arcs outward (down) at the center
    ArcLower,
    /// scale — mirror of ArcLower: bottom fixed, top arcs outward (up)
    ArcUpper,
    /// scale — the height scales about the midline by `s = 1 + (b/2)(1 − u²)`:
    /// both edges bow outward symmetrically
    Bulge,
    /// scale — the top edge stays fixed and the height scales by the *inverted*
    /// parabola `s = 1 + (b/2)·u²`: the bottom center is pinned and the corners
    /// flare outward (down)
    ShellLower,
    /// scale — mirror of ShellLower: bottom fixed, top corners flare outward (up)
    ShellUpper,
    /// scale — a fish silhouette about the midline: `s = 1 + (b/2)·g(u)` with
    /// `g(u) = 1 − u² − (1+u)²/4` — the nose (u = −1) is neutral, the body bulges
    /// (peak near u ≈ −0.2), and the tail (u = 1) pinches to `1 − b/2`
    Fish,
    /// rotational — swirl about the center: `θ = b·(π/2)·(1 − r̂²)²`, center rotates
    /// most, corners fixed; angle-true (rotates the absolute offset). The eased
    /// falloff has zero angular gradient at the pinned corners (edges sweep smoothly
    /// into them instead of self-crossing in slivers) yet stays broad mid-frame, so
    /// glyphs rotate as wholes rather than shearing apart
    Twist,
}

/// An envelope-preset field over a source bounding box. The box normalizes points to
/// `(u, v) ∈ [−1, 1]²` (`u` along the bend axis). Displacement presets scale by the
/// amplitude `A = bend · L/4` (`L` = the bend-axis extent), so they grow with the
/// art they warp; positive bend displaces **up** (−y) for `axis="h"` and **right**
/// (+x) for `axis="v"`. The 2-D families (fisheye / inflate / squeeze / twist) use
/// dimensionless factors of `bend` — see each [`Profile`] variant. `bend` is clamped
/// to `[−1, 1]` (authored as −100…100 %); non-finite collapses to 0 (the identity),
/// so a degenerate box or bend can never emit NaN.
#[derive(Clone, Copy, Debug)]
pub struct EnvelopePreset {
    profile: Profile,
    bend: f64,
    axis: WarpAxis,
    bbox: Rect,
}

impl EnvelopePreset {
    /// Build a preset field by name — all 15 Make-with-Warp styles (see [`Profile`])
    /// — over the pre-warp union bbox of the geometry it will map. `None` for
    /// unknown names. `axis` selects the bend axis for the displacement, scale, and
    /// polar families; the radial/rotational presets are symmetric and ignore it.
    pub fn new(name: &str, bend: f64, axis: WarpAxis, bbox: Rect) -> Option<Self> {
        let profile = match name {
            "arch" => Profile::Arch,
            "flag" => Profile::Flag,
            "rise" => Profile::Rise,
            "wave" => Profile::Wave,
            "fisheye" => Profile::Fisheye,
            "inflate" => Profile::Inflate,
            "squeeze" => Profile::Squeeze,
            "twist" => Profile::Twist,
            "arc" => Profile::Arc,
            "arc-lower" => Profile::ArcLower,
            "arc-upper" => Profile::ArcUpper,
            "bulge" => Profile::Bulge,
            "shell-lower" => Profile::ShellLower,
            "shell-upper" => Profile::ShellUpper,
            "fish" => Profile::Fish,
            _ => return None,
        };
        let bend = if bend.is_finite() {
            bend.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        Some(Self {
            profile,
            bend,
            axis,
            bbox,
        })
    }
}

impl Field for EnvelopePreset {
    fn map(&self, p: Point) -> Point {
        let (cx, cy) = (self.bbox.center().x, self.bbox.center().y);
        let (hw, hh) = (self.bbox.width() / 2.0, self.bbox.height() / 2.0);
        // normalized frame coords; a collapsed extent normalizes to 0 rather than
        // dividing by it
        let norm = |d: f64, half: f64| if half > 0.0 { d / half } else { 0.0 };
        let nx = norm(p.x - cx, hw);
        let ny = norm(p.y - cy, hh);
        // (u, v): u along the bend axis, v across it
        let (u, v, half_l) = match self.axis {
            WarpAxis::H => (nx, ny, hw),
            WarpAxis::V => (ny, nx, hh),
        };
        let b = self.bend;
        match self.profile {
            // displacement family: a 1-D profile Δ(u, v) pushed across the bend axis
            Profile::Arch | Profile::Flag | Profile::Rise | Profile::Wave => {
                let a = b * half_l / 2.0; // A = bend · L/4
                let d = match self.profile {
                    Profile::Arch => a * (1.0 - u * u),
                    Profile::Flag => a * (std::f64::consts::PI * u).sin(),
                    Profile::Rise => a * u,
                    _ => {
                        a * (std::f64::consts::PI * u - std::f64::consts::FRAC_PI_4 * (v + 1.0))
                            .sin()
                    }
                };
                match self.axis {
                    WarpAxis::H => Point::new(p.x, p.y - d),
                    WarpAxis::V => Point::new(p.x + d, p.y),
                }
            }
            Profile::Fisheye => {
                let r2 = (nx * nx + ny * ny) / 2.0; // r̂² — 1 at the corners
                let t = 1.0 - r2.min(1.0);
                let s = 1.0 + b * t * t;
                Point::new(cx + nx * s * hw, cy + ny * s * hh)
            }
            Profile::Inflate => {
                let sx = 1.0 + b / 2.0 * (1.0 - ny * ny);
                let sy = 1.0 + b / 2.0 * (1.0 - nx * nx);
                Point::new(cx + nx * sx * hw, cy + ny * sy * hh)
            }
            Profile::Squeeze => {
                let s = 1.0 - b / 2.0 * (1.0 - v * v);
                match self.axis {
                    WarpAxis::H => Point::new(cx + u * s * hw, p.y),
                    WarpAxis::V => Point::new(p.x, cy + u * s * hh),
                }
            }
            // scale family: the cross-axis coordinate rescales by a profile of u,
            // about an anchor — the midline, or one pinned edge
            Profile::ArcLower
            | Profile::ArcUpper
            | Profile::Bulge
            | Profile::ShellLower
            | Profile::ShellUpper
            | Profile::Fish => {
                let profile = match self.profile {
                    Profile::ArcLower | Profile::ArcUpper | Profile::Bulge => 1.0 - u * u,
                    Profile::ShellLower | Profile::ShellUpper => u * u,
                    _ => 1.0 - u * u - (1.0 + u) * (1.0 + u) / 4.0, // Fish
                };
                let s = 1.0 + b / 2.0 * profile;
                let v2 = match self.profile {
                    // "lower": the top edge (v = −1) is the pinned anchor
                    Profile::ArcLower | Profile::ShellLower => -1.0 + (v + 1.0) * s,
                    // "upper": the bottom edge (v = 1) is the pinned anchor
                    Profile::ArcUpper | Profile::ShellUpper => 1.0 - (1.0 - v) * s,
                    _ => v * s, // midline-anchored (Bulge, Fish)
                };
                match self.axis {
                    WarpAxis::H => Point::new(p.x, cy + v2 * hh),
                    WarpAxis::V => Point::new(cx + v2 * hw, p.y),
                }
            }
            Profile::Arc => {
                let theta = b * std::f64::consts::PI; // total sweep; semicircle at 100%
                if theta.abs() < 1e-4 || half_l <= 0.0 {
                    return p; // vanishing bend or degenerate frame → identity
                }
                // the midline keeps its length: it becomes an arc of radius R = L/Θ
                let r = 2.0 * half_l / theta;
                // (t, w): position along the bend axis / offset toward the bulge side
                let (t, w) = match self.axis {
                    WarpAxis::H => (p.x - cx, cy - p.y),
                    WarpAxis::V => (p.y - cy, p.x - cx),
                };
                let phi = t / half_l * (theta / 2.0);
                let (t2, w2) = ((r + w) * phi.sin(), (r + w) * phi.cos() - r);
                match self.axis {
                    WarpAxis::H => Point::new(cx + t2, cy - w2),
                    WarpAxis::V => Point::new(cx + w2, cy + t2),
                }
            }
            Profile::Twist => {
                let r2 = ((nx * nx + ny * ny) / 2.0).min(1.0); // r̂² — 1 at the corners
                let t = 1.0 - r2;
                let theta = b * std::f64::consts::FRAC_PI_2 * t * t;
                let (sin, cos) = theta.sin_cos();
                let (dx, dy) = (p.x - cx, p.y - cy);
                Point::new(cx + dx * cos - dy * sin, cy + dx * sin + dy * cos)
            }
        }
    }
}

/// Fields composed in order — the output of one feeds the next (§7.3, e.g. an
/// envelope preset followed by the distortion-slider taper).
pub struct Chain(pub Vec<Box<dyn Field>>);

impl Field for Chain {
    fn map(&self, p: Point) -> Point {
        self.0.iter().fold(p, |q, f| f.map(q))
    }
}

/// An 8-DOF projective map (§7.2 *perspective*): the envelope's corners go to four
/// authored target points; straight lines stay straight. Solved once (DLT over the
/// normalized frame for conditioning); evaluation is two dot products and a divide.
pub struct Homography {
    bbox: Rect,
    m: [f64; 8], // a b c d e f g h — bottom-right of the 3×3 fixed at 1
}

impl Homography {
    /// Solve the map taking the bbox corners — **TL, TR, BR, BL** — to `targets`.
    /// `None` for a degenerate bbox, non-finite targets, or a singular system
    /// (e.g. three collinear targets).
    pub fn new(bbox: Rect, targets: [Point; 4]) -> Option<Self> {
        if !(bbox.width() > 0.0 && bbox.height() > 0.0) {
            return None;
        }
        if targets
            .iter()
            .any(|t| !(t.x.is_finite() && t.y.is_finite()))
        {
            return None;
        }
        // sources are the corners of the normalized frame
        let src = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let mut a = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let (x, y) = src[i];
            let (tx, ty) = (targets[i].x, targets[i].y);
            a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * tx, -y * tx, tx];
            a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * ty, -y * ty, ty];
        }
        Some(Self {
            bbox,
            m: solve8(&mut a)?,
        })
    }
}

/// Gauss–Jordan elimination with partial pivoting on an 8×9 augmented system.
/// `None` when a pivot collapses (singular — degenerate corner configuration).
fn solve8(a: &mut [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let piv = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, piv);
        for row in 0..8 {
            if row != col {
                let f = a[row][col] / a[col][col];
                for k in col..9 {
                    a[row][k] -= f * a[col][k];
                }
            }
        }
    }
    let mut out = [0.0; 8];
    for (i, o) in out.iter_mut().enumerate() {
        *o = a[i][8] / a[i][i];
        if !o.is_finite() {
            return None;
        }
    }
    Some(out)
}

impl Field for Homography {
    fn map(&self, p: Point) -> Point {
        let nx = (p.x - self.bbox.center().x) / (self.bbox.width() / 2.0);
        let ny = (p.y - self.bbox.center().y) / (self.bbox.height() / 2.0);
        let [a, b, c, d, e, f, g, h] = self.m;
        // points nearing the horizon line blow up; clamp the denominator so extreme
        // corner configurations stay large-but-bounded (never NaN/inf)
        let mut w = g * nx + h * ny + 1.0;
        if w.abs() < 0.05 {
            w = if w < 0.0 { -0.05 } else { 0.05 };
        }
        Point::new((a * nx + b * ny + c) / w, (d * nx + e * ny + f) / w)
    }
}

/// A 4-corner **bilinear** distort (§7.2 *free distort* / AI Free Distort): the
/// normalized frame blends the four target corners. Cheaper than a homography and
/// makes no straight-line promise (edges shear rather than converge).
pub struct FreeDistort {
    bbox: Rect,
    targets: [Point; 4], // TL, TR, BR, BL
}

impl FreeDistort {
    pub fn new(bbox: Rect, targets: [Point; 4]) -> Option<Self> {
        if targets
            .iter()
            .any(|t| !(t.x.is_finite() && t.y.is_finite()))
        {
            return None;
        }
        Some(Self { bbox, targets })
    }
}

impl Field for FreeDistort {
    fn map(&self, p: Point) -> Point {
        let norm = |d: f64, half: f64| if half > 0.0 { d / half } else { 0.0 };
        let nx = norm(p.x - self.bbox.center().x, self.bbox.width() / 2.0);
        let ny = norm(p.y - self.bbox.center().y, self.bbox.height() / 2.0);
        let [tl, tr, br, bl] = self.targets;
        let (wtl, wtr) = ((1.0 - nx) * (1.0 - ny) / 4.0, (1.0 + nx) * (1.0 - ny) / 4.0);
        let (wbr, wbl) = ((1.0 + nx) * (1.0 + ny) / 4.0, (1.0 - nx) * (1.0 + ny) / 4.0);
        Point::new(
            tl.x * wtl + tr.x * wtr + br.x * wbr + bl.x * wbl,
            tl.y * wtl + tr.y * wtr + br.y * wbr + bl.y * wbl,
        )
    }
}

/// The Warp-Options **distortion sliders** (§7.3 `distort-h`/`distort-v`): a
/// center-anchored projective taper composed after a preset. Every point's offset
/// from the frame center divides by `w = 1 − (dh/2)·nx − (dv/2)·ny`, clamped away
/// from zero — a constrained homography whose horizon stays outside the frame for
/// moderate slider values. Positive `dh` grows the **right** side (and shrinks the
/// left); positive `dv` grows the **bottom**. Inputs clamp to `[−1, 1]`; non-finite
/// collapses to 0.
pub struct Taper {
    bbox: Rect,
    dh: f64,
    dv: f64,
}

impl Taper {
    pub fn new(bbox: Rect, dh: f64, dv: f64) -> Self {
        let clamp = |v: f64| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        Self {
            bbox,
            dh: clamp(dh),
            dv: clamp(dv),
        }
    }
}

impl Field for Taper {
    fn map(&self, p: Point) -> Point {
        let (cx, cy) = (self.bbox.center().x, self.bbox.center().y);
        let norm = |d: f64, half: f64| if half > 0.0 { d / half } else { 0.0 };
        let nx = norm(p.x - cx, self.bbox.width() / 2.0);
        let ny = norm(p.y - cy, self.bbox.height() / 2.0);
        let w = (1.0 - self.dh / 2.0 * nx - self.dv / 2.0 * ny).max(0.05);
        Point::new(cx + (p.x - cx) / w, cy + (p.y - cy) / w)
    }
}

/// Where a run of width `w` begins within a path extent `extent` (§6.13):
/// `align` distributes the slack, `start` adds an absolute head-start. Slack may be
/// negative (run longer than the path) — `middle`/`end` then shift before the
/// path's start, symmetric with the past-the-end overshoot.
pub fn run_offset(extent: f64, w: f64, align: &str, start: f64) -> f64 {
    start
        + match align {
            "middle" => (extent - w) / 2.0,
            "end" => extent - w,
            _ => 0.0,
        }
}

/// A reference path flattened to an arc-length-parameterized polyline — the shared
/// geometry behind the §6.13 text-on-path fields. `at(s, off)` walks `s` user units
/// of arc length from the path's start and steps `off` along the local normal (the
/// tangent rotated +90° in y-down coords: for a left→right path positive `off`
/// points down, so negative glyph y rises above the curve); past either end it
/// extends straight along the end tangent. `y_at(x)` reads the same polyline as a
/// height field (§6.13.1 skew — first crossing wins, clamped outside the x-extent).
/// Only the first subpath is used. `None` for unparsable or zero-length paths.
pub struct PathFrame {
    pts: Vec<Point>,
    cum: Vec<f64>, // cumulative arc length per vertex; cum[0] = 0
}

impl PathFrame {
    pub fn new(path_d: &str, tolerance: f64) -> Option<Self> {
        let path = BezPath::from_svg(path_d).ok()?;
        let mut pts: Vec<Point> = Vec::new();
        // flatten finer than the output tolerance so the frame isn't the bottleneck
        let tol = (tolerance / 2.0).clamp(1e-3, 0.25);
        let mut done = false;
        kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
            PathEl::MoveTo(p) => {
                if pts.is_empty() {
                    pts.push(p);
                } else {
                    done = true; // first subpath only
                }
            }
            PathEl::LineTo(p) if !done => pts.push(p),
            PathEl::ClosePath if !done => {
                if let Some(&first) = pts.first() {
                    pts.push(first);
                }
                done = true;
            }
            _ => {}
        });
        if pts.len() < 2 || pts.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
            return None;
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut acc = 0.0;
        cum.push(0.0);
        for w in pts.windows(2) {
            acc += w[0].distance(w[1]);
            cum.push(acc);
        }
        (acc > 1e-9).then_some(Self { pts, cum })
    }

    /// Total arc length — the run-placement extent under rainbow.
    pub fn len(&self) -> f64 {
        *self.cum.last().unwrap()
    }

    /// The path's start / end x — its extent for run placement under skew.
    pub fn x0(&self) -> f64 {
        self.pts[0].x
    }
    pub fn x1(&self) -> f64 {
        self.pts[self.pts.len() - 1].x
    }

    /// Unit tangent of the polyline segment starting at vertex `i` (degenerate → +x).
    fn tangent(&self, i: usize) -> (f64, f64) {
        let d = self.pts[i + 1] - self.pts[i];
        let len = d.hypot();
        if len > 0.0 {
            (d.x / len, d.y / len)
        } else {
            (1.0, 0.0)
        }
    }

    /// Point `off` units along the local normal from the path point at arc length
    /// `s`; straight extrapolation past the ends.
    pub fn at(&self, s: f64, off: f64) -> Point {
        let len = self.len();
        let sc = s.clamp(0.0, len);
        let over = s - sc; // straight-line overshoot past the path's ends
                           // binary search for the segment containing sc
        let i = match self.cum.binary_search_by(|c| c.partial_cmp(&sc).unwrap()) {
            Ok(i) => i.min(self.pts.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.pts.len() - 2),
        };
        let seg = self.cum[i + 1] - self.cum[i];
        let frac = if seg > 0.0 {
            (sc - self.cum[i]) / seg
        } else {
            0.0
        };
        let (tx, ty) = self.tangent(if over < 0.0 {
            0
        } else if over > 0.0 {
            self.pts.len() - 2
        } else {
            i
        });
        let p = self.pts[i].lerp(self.pts[i + 1], frac);
        Point::new(p.x + over * tx - off * ty, p.y + over * ty + off * tx)
    }

    /// The path's height profile: its y at horizontal position `x` (first crossing
    /// in path order), clamped to the endpoint y outside the x-extent.
    pub fn y_at(&self, x: f64) -> f64 {
        let first = self.pts[0];
        let last = self.pts[self.pts.len() - 1];
        if x <= first.x.min(last.x) {
            return if first.x <= last.x { first.y } else { last.y };
        }
        if x >= first.x.max(last.x) {
            return if first.x <= last.x { last.y } else { first.y };
        }
        for w in self.pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if (x >= a.x && x <= b.x) || (x <= a.x && x >= b.x) {
                let t = if b.x != a.x {
                    (x - a.x) / (b.x - a.x)
                } else {
                    0.0
                };
                return a.y + t * (b.y - a.y);
            }
        }
        last.y
    }
}

/// §6.13.1 **skew**: vertical displacement by the frame's height profile — glyphs
/// stay upright. `base_x` is the absolute x where the run begins (placement already
/// applied); `shift` is the baseline-shift (positive = above the path).
pub struct SkewField<'a> {
    pub frame: &'a PathFrame,
    pub base_x: f64,
    pub shift: f64,
}

impl Field for SkewField<'_> {
    fn map(&self, p: Point) -> Point {
        let x = self.base_x + p.x;
        Point::new(x, self.frame.y_at(x) + p.y - self.shift)
    }
}

/// §6.13.2 **rainbow**: arc-length follow + normal offset — glyphs rotate and
/// deform along the curve. `s0` is the arc length where the run begins.
pub struct RainbowField<'a> {
    pub frame: &'a PathFrame,
    pub s0: f64,
    pub shift: f64,
}

impl Field for RainbowField<'_> {
    fn map(&self, p: Point) -> Point {
        self.frame.at(self.s0 + p.x, p.y - self.shift)
    }
}

/// The native text-on-path bake (§6.13): warp a flat-baseline outlined run onto a
/// reference path. `outline_d` is the run outlined at the origin (baseline y = 0,
/// advancing in +x); `advance` is its advance width (align placement); `fx` selects
/// and places the field. Runs the §7.1 bake (+ refit per quality). `None` when the
/// effect is unknown or either path is degenerate — the caller degrades (§6.13.3).
pub fn warp_text_on_path(
    outline_d: &str,
    ref_path_d: &str,
    fx: &crate::PathEffect,
    advance: f64,
    tolerance: f64,
    do_refit: bool,
) -> Option<String> {
    if !advance.is_finite() {
        return None;
    }
    let frame = PathFrame::new(ref_path_d, tolerance)?;
    match fx.effect {
        "skew" => {
            let base_x =
                frame.x0() + run_offset(frame.x1() - frame.x0(), advance, fx.align, fx.start);
            let field = SkewField {
                frame: &frame,
                base_x,
                shift: fx.baseline_shift,
            };
            warp_svg_path(outline_d, &field, tolerance, do_refit)
        }
        "rainbow" => {
            let s0 = run_offset(frame.len(), advance, fx.align, fx.start);
            let field = RainbowField {
                frame: &frame,
                s0,
                shift: fx.baseline_shift,
            };
            warp_svg_path(outline_d, &field, tolerance, do_refit)
        }
        _ => None,
    }
}

/// How many times a mapped segment may halve during adaptive subdivision (2^10 ≈
/// 1000 pieces per input segment) — a hard cap so a pathological field terminates.
const MAX_SPLIT_DEPTH: u8 = 10;

/// The bake (§7.1): **flatten** curves to chords within `tolerance` (kurbo), **map**
/// every vertex through `field`, and subdivide each chord adaptively until the
/// mapped midpoint deviates from the mapped chord by at most `tolerance` — so long
/// straight edges curve smoothly under a nonlinear field instead of staying chords.
/// Output is a polyline `BezPath` (`M`/`L`/`Z` only); cubic refit is a later quality
/// tier. Closing edges are subdivided like any other segment before the `Z`.
pub fn bake(path: &BezPath, field: &dyn Field, tolerance: f64) -> BezPath {
    let tol = tolerance.max(1e-3);
    let mut out = BezPath::new();
    let mut cur = Point::ZERO;
    let mut start = Point::ZERO;
    kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
        PathEl::MoveTo(p) => {
            out.move_to(field.map(p));
            cur = p;
            start = p;
        }
        PathEl::LineTo(p) => {
            subdivide(&mut out, field, cur, p, field.map(cur), field.map(p), tol);
            cur = p;
        }
        PathEl::ClosePath => {
            if cur != start {
                subdivide(
                    &mut out,
                    field,
                    cur,
                    start,
                    field.map(cur),
                    field.map(start),
                    tol,
                );
            }
            out.close_path();
            cur = start;
        }
        // flatten only emits Move/Line/Close
        _ => {}
    });
    out
}

fn subdivide(
    out: &mut BezPath,
    field: &dyn Field,
    a: Point,
    b: Point,
    fa: Point,
    fb: Point,
    tol: f64,
) {
    fn rec(
        out: &mut BezPath,
        field: &dyn Field,
        a: Point,
        b: Point,
        fa: Point,
        fb: Point,
        tol: f64,
        depth: u8,
    ) {
        if depth > 0 {
            let m = a.midpoint(b);
            let fm = field.map(m);
            // Geometric (Hausdorff-style) error: distance from mapped probes to the
            // mapped chord segment. Probing the quarter points as well as the
            // midpoint catches antisymmetric profiles (flag's sine passes through
            // the chord at the midpoint); measuring against the segment — not the
            // chord's midpoint — keeps line-preserving fields (perspective) from
            // subdividing geometry that is already exactly straight.
            let err = seg_dist(fm, fa, fb)
                .max(seg_dist(field.map(a.lerp(b, 0.25)), fa, fb))
                .max(seg_dist(field.map(a.lerp(b, 0.75)), fa, fb));
            if err > tol {
                rec(out, field, a, m, fa, fm, tol, depth - 1);
                rec(out, field, m, b, fm, fb, tol, depth - 1);
                return;
            }
        }
        out.line_to(fb);
    }
    rec(out, field, a, b, fa, fb, tol, MAX_SPLIT_DEPTH);
}

/// Distance from `p` to the segment `[a, b]`.
fn seg_dist(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// Tight bounding box of an SVG path `d` string, or `None` if it doesn't parse or
/// is empty. Used to build the pre-warp envelope frame.
pub fn svg_path_bbox(d: &str) -> Option<Rect> {
    let path = BezPath::from_svg(d).ok()?;
    (!path.elements().is_empty()).then(|| path.bounding_box())
}

/// Refit a baked polyline to cubic Béziers (§7.1's third step): kurbo's simplify is
/// corner-aware and error-bounded, so the result stays within the same `tolerance`
/// budget while dropping most vertices. The angle threshold separates the small
/// turning angles of adaptive-subdivision joins (a few degrees — fused into curves)
/// from genuine mapped corners (kept sharp); kurbo's default (~1 mrad) would treat
/// every sampled vertex as a corner and fuse nothing. The `Optimize` level matters:
/// the default subdivision fitter degrades badly on right-to-left runs (kurbo
/// 0.13.1 — a symmetric parabola fits to 2 cubics forward but ~16 backward).
pub fn refit(path: &BezPath, tolerance: f64) -> BezPath {
    let options = SimplifyOptions::default()
        .angle_thresh(0.25) // ≈ 14°
        .opt_level(SimplifyOptLevel::Optimize);
    simplify_bezpath(
        path.elements().iter().copied(),
        tolerance.max(1e-3),
        &options,
    )
}

/// Bake an SVG path `d` string through `field` and re-serialize it (2-decimal
/// coordinates). With `refit`, the mapped polyline is refit to cubics at the same
/// tolerance (the `balanced`/`highest` output; `fast` keeps the raw polyline).
/// `None` if the input doesn't parse, produces nothing, or the field leaks a
/// non-finite coordinate — the caller keeps the original geometry (§4 totality:
/// a warp degrades, it never emits NaN).
pub fn warp_svg_path(d: &str, field: &dyn Field, tolerance: f64, do_refit: bool) -> Option<String> {
    let path = BezPath::from_svg(d).ok()?;
    let mut baked = bake(&path, field, tolerance);
    if do_refit {
        baked = refit(&baked, tolerance);
    }
    let mut s = String::new();
    for el in baked.elements() {
        match el {
            PathEl::MoveTo(p) => push_cmd(&mut s, 'M', &[*p])?,
            PathEl::LineTo(p) => push_cmd(&mut s, 'L', &[*p])?,
            PathEl::QuadTo(p1, p2) => push_cmd(&mut s, 'Q', &[*p1, *p2])?,
            PathEl::CurveTo(p1, p2, p3) => push_cmd(&mut s, 'C', &[*p1, *p2, *p3])?,
            PathEl::ClosePath => s.push('Z'),
        }
    }
    (!s.is_empty()).then_some(s)
}

fn push_cmd(s: &mut String, cmd: char, pts: &[Point]) -> Option<()> {
    if pts.iter().any(|p| !(p.x.is_finite() && p.y.is_finite())) {
        return None;
    }
    s.push(cmd);
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&round2(p.x));
        s.push(',');
        s.push_str(&round2(p.y));
    }
    Some(())
}

/// Format with at most 2 decimals, trimming trailing zeros (`1.50` → `1.5`,
/// `2.00` → `2`).
fn round2(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path() -> BezPath {
        BezPath::from_svg("M0,0 L100,0 L100,40 L0,40 Z").unwrap()
    }

    fn arch(bend: f64, bbox: Rect) -> EnvelopePreset {
        EnvelopePreset::new("arch", bend, WarpAxis::H, bbox).unwrap()
    }

    fn vertices(p: &BezPath) -> Vec<Point> {
        p.elements()
            .iter()
            .filter_map(|el| match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn zero_bend_is_identity() {
        let path = rect_path();
        let baked = bake(&path, &arch(0.0, path.bounding_box()), 0.25);
        // identity map, but the implicit closing edge becomes an explicit LineTo
        // back to the subpath start (so a field could subdivide it)
        let mut expected = vertices(&path);
        expected.push(expected[0]);
        assert_eq!(vertices(&baked), expected);
        assert_eq!(baked.elements().last(), Some(&PathEl::ClosePath));
    }

    #[test]
    fn arch_lifts_the_center_and_pins_the_ends() {
        // a baseline: bbox 100 wide → A = bend·L/4 = 0.8·25 = 20 up at the center
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(0.8, Rect::new(0.0, 0.0, 100.0, 0.0));
        let baked = bake(&line, &f, 0.05);
        let vs = vertices(&baked);
        assert!((vs.first().unwrap().y - 0.0).abs() < 1e-9);
        assert!((vs.last().unwrap().y - 0.0).abs() < 1e-9);
        let min_y = vs.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        assert!((min_y + 20.0).abs() < 0.1, "apex {min_y}, want ≈ −20");
        // the straight source line must have been subdivided to follow the parabola
        assert!(vs.len() > 8, "no adaptive subdivision: {} pts", vs.len());
    }

    #[test]
    fn flag_bows_a_straight_edge_despite_midpoint_symmetry() {
        // flag's sine is zero at both endpoints AND the midpoint of a full-width
        // edge — a midpoint-only error test never splits it (regression: the box
        // stayed a straight rectangle). A = 25: the edge must swing ≈ ±25.
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f =
            EnvelopePreset::new("flag", 1.0, WarpAxis::H, Rect::new(0.0, 0.0, 100.0, 0.0)).unwrap();
        let vs = vertices(&bake(&line, &f, 0.25));
        assert!(vs.len() > 8, "sine edge not subdivided: {} pts", vs.len());
        let min_y = vs.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_y = vs.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        assert!((min_y + 25.0).abs() < 0.5, "trough {min_y}, want ≈ −25");
        assert!((max_y - 25.0).abs() < 0.5, "crest {max_y}, want ≈ +25");
    }

    #[test]
    fn tighter_tolerance_means_more_segments() {
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(1.0, Rect::new(0.0, 0.0, 100.0, 0.0));
        let coarse = vertices(&bake(&line, &f, 1.0)).len();
        let fine = vertices(&bake(&line, &f, 0.05)).len();
        assert!(fine > coarse, "fine {fine} !> coarse {coarse}");
    }

    #[test]
    fn subdivision_meets_the_tolerance() {
        // dense-sample the source line through the field; every mapped sample must
        // sit within ~tol of the baked polyline
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let f = arch(1.0, Rect::new(0.0, 0.0, 100.0, 0.0));
        let tol = 0.25;
        let vs = vertices(&bake(&line, &f, tol));
        for i in 0..=1000 {
            let p = f.map(Point::new(i as f64 * 0.1, 0.0));
            let dist = vs
                .windows(2)
                .map(|w| dist_to_segment(p, w[0], w[1]))
                .fold(f64::INFINITY, f64::min);
            assert!(
                dist <= tol * 1.5,
                "sample {p:?} is {dist} from the polyline"
            );
        }
    }

    fn dist_to_segment(p: Point, a: Point, b: Point) -> f64 {
        let ab = b - a;
        let len2 = ab.hypot2();
        let t = if len2 > 0.0 {
            ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(a + ab * t)
    }

    #[test]
    fn rise_on_axis_v_displaces_x() {
        let line = BezPath::from_svg("M0,0 L0,100").unwrap();
        let f =
            EnvelopePreset::new("rise", 1.0, WarpAxis::V, Rect::new(0.0, 0.0, 0.0, 100.0)).unwrap();
        let baked = bake(&line, &f, 0.25);
        let vs = vertices(&baked);
        // A = 25; rise is linear in u: x goes −25 → +25 while y is untouched
        assert!((vs.first().unwrap().x + 25.0).abs() < 1e-9);
        assert!((vs.last().unwrap().x - 25.0).abs() < 1e-9);
        assert!(vs.iter().all(|p| p.y.is_finite()));
    }

    #[test]
    fn wave_curves_the_closing_edge() {
        // wave depends on v, so the rect's (straight, implicitly closed) left edge
        // must be subdivided — more vertices than the 4 corners, still closed
        let path = rect_path();
        let f = EnvelopePreset::new("wave", 1.0, WarpAxis::H, path.bounding_box()).unwrap();
        let baked = bake(&path, &f, 0.05);
        assert!(vertices(&baked).len() > 8);
        assert_eq!(baked.elements().last(), Some(&PathEl::ClosePath));
    }

    /// All preset names, across every field family.
    const PRESETS: [&str; 15] = [
        "arch",
        "flag",
        "rise",
        "wave",
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
        "arc",
        "arc-lower",
        "arc-upper",
        "bulge",
        "shell-lower",
        "shell-upper",
        "fish",
    ];

    #[test]
    fn degenerate_bbox_and_bend_never_leak_nan() {
        let path = rect_path();
        for name in PRESETS {
            for bbox in [Rect::ZERO, Rect::new(5.0, 5.0, 5.0, 5.0)] {
                for bend in [0.0, 1.0, -1.0, f64::NAN, f64::INFINITY] {
                    let f = EnvelopePreset::new(name, bend, WarpAxis::H, bbox).unwrap();
                    let baked = bake(&path, &f, 0.25);
                    assert!(
                        vertices(&baked)
                            .iter()
                            .all(|p| p.x.is_finite() && p.y.is_finite()),
                        "{name} bend={bend} bbox={bbox:?}"
                    );
                }
            }
        }
    }

    /// The centered frame used by the 2-D family tests: 100×40 about the origin.
    fn centered() -> Rect {
        Rect::new(-50.0, -20.0, 50.0, 20.0)
    }

    fn preset(name: &str, bend: f64) -> EnvelopePreset {
        EnvelopePreset::new(name, bend, WarpAxis::H, centered()).unwrap()
    }

    #[test]
    fn fisheye_magnifies_the_center_and_pins_the_corners() {
        let f = preset("fisheye", 1.0);
        // nx = 0.2, ny = 0 → r̂² = 0.02 → s = 1 + 0.98² = 1.9604 → x: 10 → 19.604
        let m = f.map(Point::new(10.0, 0.0));
        assert!((m.x - 19.604).abs() < 1e-9 && m.y.abs() < 1e-9, "{m:?}");
        // corners are at r̂ = 1 → fixed
        let c = f.map(Point::new(50.0, 20.0));
        assert!(
            (c.x - 50.0).abs() < 1e-9 && (c.y - 20.0).abs() < 1e-9,
            "{c:?}"
        );
    }

    #[test]
    fn fisheye_is_radially_monotone_at_full_bend() {
        // the eased profile keeps r·s(r) strictly increasing — the outer ring never
        // folds back over the interior (regression: the linear profile folded past
        // bend = 50%)
        for bend in [1.0, -1.0] {
            let f = preset("fisheye", bend);
            let mut prev = -1.0;
            for i in 0..=100 {
                let x = 50.0 * i as f64 / 100.0; // a center→corner ray
                let y = 20.0 * i as f64 / 100.0;
                let d = f.map(Point::new(x, y)).distance(Point::ZERO);
                assert!(d > prev, "fold at step {i} (bend {bend}): {d} <= {prev}");
                prev = d;
            }
        }
    }

    #[test]
    fn arc_preserves_the_midline_and_wraps_a_semicircle_at_full_bend() {
        // bend 100% → Θ = π; the centered() midline (length 100) → R = 100/π
        let f = preset("arc", 1.0);
        // the midline's center is invariant
        assert!(f.map(Point::ZERO).distance(Point::ZERO) < 1e-9);
        // its endpoints land at (±R, R): φ = ±90° on the circle centered at (0, R)
        let r = 100.0 / std::f64::consts::PI;
        let e = f.map(Point::new(50.0, 0.0));
        assert!((e.x - r).abs() < 1e-9 && (e.y - r).abs() < 1e-9, "{e:?}");
        let w = f.map(Point::new(-50.0, 0.0));
        assert!((w.x + r).abs() < 1e-9 && (w.y - r).abs() < 1e-9, "{w:?}");
        // a point above the midline sits on a larger radius (center at (0, R))
        let top = f.map(Point::new(0.0, -20.0));
        assert!((top.distance(Point::new(0.0, r)) - (r + 20.0)).abs() < 1e-9);
    }

    #[test]
    fn arc_vanishing_bend_is_the_identity() {
        let p = Point::new(37.0, 11.0);
        assert!(preset("arc", 0.0).map(p).distance(p) < 1e-9);
        assert!(preset("arc", 1e-9).map(p).distance(p) < 1e-6);
    }

    #[test]
    fn arc_lower_pins_the_top_and_arcs_the_bottom_center() {
        let f = preset("arc-lower", 1.0);
        for x in [-50.0, 0.0, 30.0] {
            let p = Point::new(x, -20.0); // the whole top edge is the anchor
            assert!(f.map(p).distance(p) < 1e-9, "{p:?}");
        }
        // bottom center pushed a half-height further down: s = 1.5 → y: 20 → 40
        let m = f.map(Point::new(0.0, 20.0));
        assert!((m.y - 40.0).abs() < 1e-9 && m.x.abs() < 1e-9, "{m:?}");
        // bottom corners unchanged (s = 1 at u = ±1)
        assert!(
            f.map(Point::new(50.0, 20.0))
                .distance(Point::new(50.0, 20.0))
                < 1e-9
        );
        // arc-upper mirrors: bottom pinned, top center pushed up
        let up = preset("arc-upper", 1.0);
        assert!(
            up.map(Point::new(0.0, 20.0))
                .distance(Point::new(0.0, 20.0))
                < 1e-9
        );
        assert!((up.map(Point::new(0.0, -20.0)).y + 40.0).abs() < 1e-9);
    }

    #[test]
    fn shell_lower_pins_the_bottom_center_and_flares_the_corners() {
        let f = preset("shell-lower", 1.0);
        // bottom center pinned (s = 1 at u = 0), top edge is the anchor
        assert!(f.map(Point::new(0.0, 20.0)).distance(Point::new(0.0, 20.0)) < 1e-9);
        assert!(
            f.map(Point::new(50.0, -20.0))
                .distance(Point::new(50.0, -20.0))
                < 1e-9
        );
        // bottom corners flare down: s = 1.5 at u = ±1 → y: 20 → 40
        assert!((f.map(Point::new(50.0, 20.0)).y - 40.0).abs() < 1e-9);
    }

    #[test]
    fn bulge_bows_both_edges_and_fish_pinches_the_tail() {
        let bulge = preset("bulge", 1.0);
        assert!((bulge.map(Point::new(0.0, -20.0)).y + 30.0).abs() < 1e-9);
        assert!((bulge.map(Point::new(0.0, 20.0)).y - 30.0).abs() < 1e-9);

        let fish = preset("fish", 1.0);
        // nose (u = −1) neutral; tail (u = 1) pinches to half height; body bulges
        assert!(
            fish.map(Point::new(-50.0, 20.0))
                .distance(Point::new(-50.0, 20.0))
                < 1e-9
        );
        assert!((fish.map(Point::new(50.0, 20.0)).y - 10.0).abs() < 1e-9);
        assert!(fish.map(Point::new(-10.0, 20.0)).y > 20.0);
    }

    #[test]
    fn twist_never_self_crosses_at_the_pinned_corners() {
        // near a pinned corner the eased falloff moves edge points much less than
        // their distance to the corner, so the outline cannot double back into a
        // sliver (regression: the linear falloff swept ~66% of the corner distance)
        let f = preset("twist", 1.0);
        let corner = Point::new(50.0, -20.0);
        for d in [1.0, 2.0, 5.0, 10.0] {
            let p = Point::new(50.0 - d, -20.0); // on the top edge, d from the corner
            let moved = f.map(p).distance(p);
            assert!(moved < 0.5 * d, "edge point {d} from corner moved {moved}");
        }
        assert!(f.map(corner).distance(corner) < 1e-9);
    }

    #[test]
    fn inflate_bulges_mid_edges_and_pins_the_corners() {
        let f = preset("inflate", 1.0);
        // right-edge midpoint (nx=1, ny=0): sx = 1.5 → x: 50 → 75, y untouched
        let m = f.map(Point::new(50.0, 0.0));
        assert!((m.x - 75.0).abs() < 1e-9 && m.y.abs() < 1e-9, "{m:?}");
        // top-edge midpoint (nx=0, ny=−1): sy = 1.5 → y: −20 → −30
        let t = f.map(Point::new(0.0, -20.0));
        assert!(t.x.abs() < 1e-9 && (t.y + 30.0).abs() < 1e-9, "{t:?}");
        let c = f.map(Point::new(-50.0, 20.0));
        assert!(
            (c.x + 50.0).abs() < 1e-9 && (c.y - 20.0).abs() < 1e-9,
            "{c:?}"
        );
    }

    #[test]
    fn squeeze_pinches_the_waist_and_pins_the_top_and_bottom() {
        let f = preset("squeeze", 1.0);
        // right-edge midpoint (u=1, v=0): s = 0.5 → x: 50 → 25
        let m = f.map(Point::new(50.0, 0.0));
        assert!((m.x - 25.0).abs() < 1e-9 && m.y.abs() < 1e-9, "{m:?}");
        // corners (v = ±1) stay put
        let c = f.map(Point::new(50.0, -20.0));
        assert!(
            (c.x - 50.0).abs() < 1e-9 && (c.y + 20.0).abs() < 1e-9,
            "{c:?}"
        );
    }

    #[test]
    fn twist_swirls_the_center_and_pins_the_corners() {
        let f = preset("twist", 1.0);
        // near the center θ → 90°: (ε, 0) rotates to ≈ (0, ε)
        let m = f.map(Point::new(0.5, 0.0));
        assert!(m.x.abs() < 0.02 && (m.y - 0.5).abs() < 0.02, "{m:?}");
        // corners are at r̂ = 1 → θ = 0 → fixed
        let c = f.map(Point::new(50.0, 20.0));
        assert!(
            (c.x - 50.0).abs() < 1e-9 && (c.y - 20.0).abs() < 1e-9,
            "{c:?}"
        );
        // the swirl is angle-true: distance from the center is preserved
        let p = Point::new(20.0, 5.0);
        assert!((f.map(p).distance(Point::ZERO) - p.distance(Point::ZERO)).abs() < 1e-9);
    }

    #[test]
    fn homography_maps_corners_exactly_and_keeps_lines_straight() {
        let bbox = Rect::new(0.0, 0.0, 100.0, 40.0);
        let targets = [
            Point::new(10.0, 5.0),
            Point::new(90.0, 0.0),
            Point::new(100.0, 45.0),
            Point::new(-5.0, 40.0),
        ];
        let h = Homography::new(bbox, targets).unwrap();
        for (src, t) in [
            (Point::new(0.0, 0.0), targets[0]),
            (Point::new(100.0, 0.0), targets[1]),
            (Point::new(100.0, 40.0), targets[2]),
            (Point::new(0.0, 40.0), targets[3]),
        ] {
            assert!(h.map(src).distance(t) < 1e-6, "{src:?} → {:?}", h.map(src));
        }
        // projective maps preserve straightness: the mapped top edge is collinear
        let (a, m, b) = (
            h.map(Point::new(0.0, 0.0)),
            h.map(Point::new(37.0, 0.0)),
            h.map(Point::new(100.0, 0.0)),
        );
        let cross = (m - a).cross(b - a);
        assert!(cross.abs() < 1e-6, "not collinear: {cross}");
        // …and the bake therefore does NOT subdivide a straight edge
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        assert_eq!(vertices(&bake(&line, &h, 0.25)).len(), 2);
    }

    #[test]
    fn homography_identity_and_degenerate_inputs() {
        let bbox = Rect::new(0.0, 0.0, 100.0, 40.0);
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 40.0),
            Point::new(0.0, 40.0),
        ];
        let h = Homography::new(bbox, corners).unwrap();
        let p = Point::new(33.0, 17.0);
        assert!(h.map(p).distance(p) < 1e-9);
        // degenerate bbox / non-finite targets → None
        assert!(Homography::new(Rect::ZERO, corners).is_none());
        let mut bad = corners;
        bad[2] = Point::new(f64::NAN, 0.0);
        assert!(Homography::new(bbox, bad).is_none());
        // coincident targets: either rejected as singular, or total (never NaN)
        if let Some(h) = Homography::new(bbox, [Point::new(5.0, 5.0); 4]) {
            let m = h.map(Point::new(50.0, 20.0));
            assert!(m.x.is_finite() && m.y.is_finite());
        }
    }

    #[test]
    fn free_distort_blends_corners() {
        let bbox = Rect::new(0.0, 0.0, 100.0, 40.0);
        let targets = [
            Point::new(0.0, 10.0),
            Point::new(100.0, -10.0),
            Point::new(110.0, 50.0),
            Point::new(-10.0, 30.0),
        ];
        let f = FreeDistort::new(bbox, targets).unwrap();
        assert!(f.map(Point::new(0.0, 0.0)).distance(targets[0]) < 1e-9);
        assert!(f.map(Point::new(100.0, 40.0)).distance(targets[2]) < 1e-9);
        // the center blends all four corners equally
        let c = f.map(Point::new(50.0, 20.0));
        assert!((c.x - 50.0).abs() < 1e-9 && (c.y - 20.0).abs() < 1e-9);
        assert!(FreeDistort::new(bbox, [Point::new(f64::INFINITY, 0.0); 4]).is_none());
    }

    #[test]
    fn taper_tapers_and_never_divides_by_zero() {
        let t = Taper::new(centered(), 1.0, 0.0);
        // positive dh: the right side (nx = 1) grows (w = 0.5), the left shrinks
        let r = t.map(Point::new(50.0, 20.0));
        assert!((r.x - 100.0).abs() < 1e-9 && (r.y - 40.0).abs() < 1e-9);
        let l = t.map(Point::new(-50.0, 20.0));
        assert!((l.x + 50.0 / 1.5).abs() < 1e-9 && (l.y - 20.0 / 1.5).abs() < 1e-9);
        // extreme sliders + far-outside points stay finite (clamped denominator)
        let x = Taper::new(centered(), -1.0, -1.0);
        let m = x.map(Point::new(500.0, 500.0));
        assert!(m.x.is_finite() && m.y.is_finite());
        // garbage sliders collapse to the identity
        let id = Taper::new(centered(), f64::NAN, f64::INFINITY);
        assert!(
            id.map(Point::new(30.0, 10.0))
                .distance(Point::new(30.0, 10.0))
                < 1e-9
        );
    }

    #[test]
    fn chain_composes_in_order() {
        let bbox = Rect::new(0.0, 0.0, 100.0, 40.0);
        let chain = Chain(vec![
            Box::new(EnvelopePreset::new("rise", 1.0, WarpAxis::H, bbox).unwrap()),
            Box::new(Taper::new(bbox, 1.0, 0.0)),
        ]);
        let p = Point::new(100.0, 20.0);
        let step1 = EnvelopePreset::new("rise", 1.0, WarpAxis::H, bbox)
            .unwrap()
            .map(p);
        let expect = Taper::new(bbox, 1.0, 0.0).map(step1);
        assert!(chain.map(p).distance(expect) < 1e-12);
    }

    #[test]
    fn bend_is_clamped() {
        // 500% behaves exactly like 100%
        let line = BezPath::from_svg("M0,0 L100,0").unwrap();
        let bbox = Rect::new(0.0, 0.0, 100.0, 0.0);
        let big = bake(&line, &arch(5.0, bbox), 0.05);
        let one = bake(&line, &arch(1.0, bbox), 0.05);
        assert_eq!(vertices(&big), vertices(&one));
    }

    #[test]
    fn unknown_preset_is_none() {
        assert!(EnvelopePreset::new("twirl", 0.5, WarpAxis::H, Rect::ZERO).is_none());
    }

    // ---- text-on-path (§6.13, native) ----

    use crate::PathEffect;

    fn fx<'a>(effect: &'a str, shift: f64, align: &'a str, start: f64) -> PathEffect<'a> {
        PathEffect {
            effect,
            baseline_shift: shift,
            align,
            start,
        }
    }

    #[test]
    fn path_frame_measures_and_samples() {
        let f = PathFrame::new("M0,0 L100,0", 0.25).unwrap();
        assert!((f.len() - 100.0).abs() < 1e-9);
        assert_eq!((f.x0(), f.x1()), (0.0, 100.0));
        // on-path, normal offset, and straight extrapolation past both ends
        assert!(f.at(50.0, 0.0).distance(Point::new(50.0, 0.0)) < 1e-9);
        assert!(f.at(50.0, -10.0).distance(Point::new(50.0, -10.0)) < 1e-9);
        assert!(f.at(150.0, 0.0).distance(Point::new(150.0, 0.0)) < 1e-9);
        assert!(f.at(-50.0, 0.0).distance(Point::new(-50.0, 0.0)) < 1e-9);
        // height field on a slope, clamped outside the extent
        let g = PathFrame::new("M0,0 L100,100", 0.25).unwrap();
        assert!((g.y_at(50.0) - 50.0).abs() < 1e-9);
        assert!((g.y_at(-10.0) - 0.0).abs() < 1e-9);
        assert!((g.y_at(200.0) - 100.0).abs() < 1e-9);
        // a curve's arc length exceeds its chord
        let c = PathFrame::new("M0,40 C40,0 80,80 120,40", 0.25).unwrap();
        assert!(c.len() > 120.0, "{}", c.len());
        // degenerate inputs
        assert!(PathFrame::new("", 0.25).is_none());
        assert!(PathFrame::new("garbage", 0.25).is_none());
        assert!(PathFrame::new("M5,5 L5,5", 0.25).is_none());
    }

    #[test]
    fn warp_text_on_path_places_and_shifts() {
        // a 10×5 box outline on the baseline, on a flat reference path at y = 20
        let outline = "M0,0 L10,0 L10,-5 L0,-5 Z";
        let flat = "M0,20 L120,20";
        // skew, defaults: run starts at the path's start x; baseline lands on y=20
        let d = warp_text_on_path(
            outline,
            flat,
            &fx("skew", 0.0, "start", 0.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M0,20"), "{d}");
        // baseline-shift lifts it; start pushes it along; align=end right-aligns
        let d = warp_text_on_path(
            outline,
            flat,
            &fx("skew", 8.0, "start", 0.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M0,12"), "{d}");
        let d = warp_text_on_path(
            outline,
            flat,
            &fx("skew", 0.0, "start", 15.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M15,20"), "{d}");
        let d = warp_text_on_path(
            outline,
            flat,
            &fx("skew", 0.0, "end", 0.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M110,20"), "{d}");
        // rainbow on the same flat path behaves identically at s0 = 0
        let d = warp_text_on_path(
            outline,
            flat,
            &fx("rainbow", 0.0, "middle", 0.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M55,20"), "{d}");
        // rainbow on a vertical path: the run follows it downward, rotated 90°
        let vert = "M40,0 L40,120";
        let d = warp_text_on_path(
            outline,
            vert,
            &fx("rainbow", 0.0, "start", 0.0),
            10.0,
            0.25,
            false,
        )
        .unwrap();
        assert!(d.starts_with("M40,0"), "{d}");
        assert!(d.contains("40,10"), "advance runs down the path: {d}");
        // unknown effect / degenerate inputs degrade to None, never NaN
        assert!(warp_text_on_path(
            outline,
            flat,
            &fx("stair", 0.0, "start", 0.0),
            10.0,
            0.25,
            false
        )
        .is_none());
        assert!(warp_text_on_path(
            outline,
            "M5,5 L5,5",
            &fx("skew", 0.0, "start", 0.0),
            10.0,
            0.25,
            false
        )
        .is_none());
        assert!(warp_text_on_path(
            outline,
            flat,
            &fx("skew", 0.0, "start", 0.0),
            f64::NAN,
            0.25,
            false
        )
        .is_none());
    }

    #[test]
    fn warp_text_on_path_refits_curved_output() {
        // a long baseline box over a curved path: polyline subdivides, refit fuses
        let outline = "M0,0 L100,0 L100,-10 L0,-10 Z";
        let wave = "M0,40 C40,0 80,80 120,40";
        let poly = warp_text_on_path(
            outline,
            wave,
            &fx("skew", 0.0, "start", 0.0),
            100.0,
            0.25,
            false,
        )
        .unwrap();
        let fitted = warp_text_on_path(
            outline,
            wave,
            &fx("skew", 0.0, "start", 0.0),
            100.0,
            0.25,
            true,
        )
        .unwrap();
        assert!(!poly.contains('C'));
        assert!(fitted.contains('C'), "{fitted}");
        assert!(fitted.len() < poly.len());
    }

    #[test]
    fn warp_svg_path_round_trips_and_rejects_garbage() {
        let f = arch(0.5, Rect::new(0.0, 0.0, 100.0, 40.0));
        let d = warp_svg_path("M0,0 L100,0 L100,40 L0,40 Z", &f, 0.25, false).unwrap();
        assert!(d.starts_with('M') && d.ends_with('Z'), "{d}");
        assert!(!d.contains("NaN") && !d.contains("inf"), "{d}");
        assert!(warp_svg_path("not a path", &f, 0.25, false).is_none());
        assert!(warp_svg_path("", &f, 0.25, false).is_none());
    }

    #[test]
    fn refit_shrinks_the_polyline_and_stays_within_tolerance() {
        let tol = 0.25;
        let f = arch(1.0, Rect::new(0.0, 0.0, 100.0, 40.0));
        let path = BezPath::from_svg("M0,0 L100,0 L100,40 L0,40 Z").unwrap();
        let poly = bake(&path, &f, tol);
        let fitted = refit(&poly, tol);
        assert!(
            fitted.elements().len() < poly.elements().len() / 2,
            "refit {} !<< polyline {}",
            fitted.elements().len(),
            poly.elements().len()
        );
        assert!(fitted
            .elements()
            .iter()
            .any(|e| matches!(e, PathEl::CurveTo(..))));
        // every polyline vertex must lie within ~tol of the fitted curve
        use kurbo::ParamCurveNearest;
        for v in vertices(&poly) {
            let d2 = fitted
                .segments()
                .map(|seg| seg.nearest(v, 1e-3).distance_sq)
                .fold(f64::INFINITY, f64::min);
            assert!(d2.sqrt() <= tol * 2.0, "vertex {v:?} is {} away", d2.sqrt());
        }
        // corners survive: the mapped rect corners stay on the fitted outline
        for src in [
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 40.0),
            Point::new(0.0, 40.0),
        ] {
            let c = f.map(src);
            let d2 = fitted
                .segments()
                .map(|seg| seg.nearest(c, 1e-3).distance_sq)
                .fold(f64::INFINITY, f64::min);
            assert!(d2.sqrt() <= tol * 2.0, "corner {c:?} drifted {}", d2.sqrt());
        }
    }

    #[test]
    fn warp_svg_path_refit_emits_cubics() {
        let f = arch(0.8, Rect::new(0.0, 0.0, 100.0, 40.0));
        let poly = warp_svg_path("M0,0 L100,0 L100,40 L0,40 Z", &f, 0.25, false).unwrap();
        let refitted = warp_svg_path("M0,0 L100,0 L100,40 L0,40 Z", &f, 0.25, true).unwrap();
        assert!(!poly.contains('C'));
        assert!(refitted.contains('C'), "{refitted}");
        assert!(
            refitted.len() < poly.len(),
            "{} !< {}",
            refitted.len(),
            poly.len()
        );
        assert!(refitted.ends_with('Z'), "{refitted}");
    }

    #[test]
    fn svg_path_bbox_parses_and_rejects() {
        let b = svg_path_bbox("M10,20 L110,20 L110,60 Z").unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (10.0, 20.0, 110.0, 60.0));
        assert!(svg_path_bbox("").is_none());
        assert!(svg_path_bbox("garbage").is_none());
    }
}
