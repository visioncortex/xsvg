//! boolfuzz: the curve-preserving boolean algebra (`boolean_svg_paths_curved`,
//! linesweeper) against the flattened one (`boolean_svg_paths`, i_overlay) and
//! against the truth sampled from the operands themselves, over seeded synthetic
//! cases. Deterministic: case `seed` is the same case on every run and machine.
//!
//!   boolfuzz run   --seed 1 --count N --budget-secs T --out DIR   the campaign
//!   boolfuzz repro --seed S                                        one case, verbose
//!
//! The campaign is a parent that runs batches of cases in child processes of
//! itself: a panic is caught in the child and reported; a hang is killed by the
//! parent after a timeout and reported; either way the campaign continues at the
//! next seed. Issues go to `DIR/issues.jsonl` (one JSON object per line, with
//! the operands' path data so `repro` or a test can replay it), progress to
//! `DIR/progress.log`.

use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use xsvg_core::{boolean_svg_paths, boolean_svg_paths_curved, BoolOp, BoolOperand};

const FLAT_TOLERANCE: f64 = 0.05;
const CASE_TIMEOUT: Duration = Duration::from_secs(20);
const SLOW: Duration = Duration::from_millis(500);
const BATCH: u64 = 250;

// ---- deterministic randomness (splitmix64: identical on every platform) -----

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// uniform in [0, 1)
    fn f(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
}

// ---- the shape zoo -----------------------------------------------------------

const K: f64 = 0.552_284_749_830_793_4;

fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64, arcs: bool) -> String {
    if arcs {
        // as the compiler writes a circle: four quarter arcs
        format!(
            "M{} {cy} A{rx} {ry} 0 0 1 {cx} {} A{rx} {ry} 0 0 1 {} {cy} A{rx} {ry} 0 0 1 {cx} {} A{rx} {ry} 0 0 1 {} {cy} Z",
            cx + rx, cy + ry, cx - rx, cy - ry, cx + rx
        )
    } else {
        let (kx, ky) = (K * rx, K * ry);
        format!(
            "M{} {cy} C{} {} {} {} {cx} {} C{} {} {} {} {} {cy} C{} {} {} {} {cx} {} C{} {} {} {} {} {cy} Z",
            cx + rx, cx + rx, cy + ky, cx + kx, cy + ry, cy + ry,
            cx - kx, cy + ry, cx - rx, cy + ky, cx - rx,
            cx - rx, cy - ky, cx - kx, cy - ry, cy - ry,
            cx + kx, cy - ry, cx + rx, cy - ky, cx + rx
        )
    }
}

fn rounded_rect(x: f64, y: f64, w: f64, h: f64, r: f64) -> String {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return format!(
            "M{x} {y} L{} {y} L{} {} L{x} {} Z",
            x + w,
            x + w,
            y + h,
            y + h
        );
    }
    let k = K * r;
    let mut s = String::new();
    let _ = write!(s, "M{} {y} L{} {y} ", x + r, x + w - r);
    let _ = write!(
        s,
        "C{} {y} {} {} {} {} ",
        x + w - r + k,
        x + w,
        y + r - k,
        x + w,
        y + r
    );
    let _ = write!(s, "L{} {} ", x + w, y + h - r);
    let _ = write!(
        s,
        "C{} {} {} {} {} {} ",
        x + w,
        y + h - r + k,
        x + w - r + k,
        y + h,
        x + w - r,
        y + h
    );
    let _ = write!(s, "L{} {} ", x + r, y + h);
    let _ = write!(
        s,
        "C{} {} {x} {} {x} {} ",
        x + r - k,
        y + h,
        y + h - r + k,
        y + h - r
    );
    let _ = write!(s, "L{x} {} ", y + r);
    let _ = write!(s, "C{x} {} {} {y} {} {y} Z", y + r - k, x + r - k, x + r);
    s
}

fn polygon(pts: &[(f64, f64)]) -> String {
    let mut s = String::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(s, "{}{x} {y} ", if i == 0 { "M" } else { "L" });
    }
    s.push('Z');
    s
}

fn star(cx: f64, cy: f64, r: f64, n: usize, skip: usize) -> String {
    // a {n/skip} star polygon: self-intersecting when skip > 1
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = (i * skip) as f64 / n as f64 * std::f64::consts::TAU;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    polygon(&pts)
}

fn blob(rng: &mut Rng, cx: f64, cy: f64, r: f64, n: usize) -> String {
    // a closed loop of random cubics around a centre, handles wild enough to
    // self-intersect now and then
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let rr = r * rng.range(0.5, 1.3);
            (cx + rr * a.cos(), cy + rr * a.sin())
        })
        .collect();
    let mut s = format!("M{} {} ", pts[0].0, pts[0].1);
    for i in 0..n {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % n];
        let h = rng.range(0.1, 1.2) * r;
        let (a0, a1) = (
            rng.range(0.0, std::f64::consts::TAU),
            rng.range(0.0, std::f64::consts::TAU),
        );
        let _ = write!(
            s,
            "C{} {} {} {} {x1} {y1} ",
            x0 + h * a0.cos(),
            y0 + h * a0.sin(),
            x1 + h * a1.cos(),
            y1 + h * a1.sin()
        );
    }
    s.push('Z');
    s
}

fn quad_blob(rng: &mut Rng, cx: f64, cy: f64, r: f64, n: usize) -> String {
    // a glyph-like loop of quadratics (TrueType outlines are quads)
    let pts: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            let rr = r * rng.range(0.6, 1.2);
            (cx + rr * a.cos(), cy + rr * a.sin())
        })
        .collect();
    let mut s = format!("M{} {} ", pts[0].0, pts[0].1);
    for i in 0..n {
        let (x1, y1) = pts[(i + 1) % n];
        let a = (i as f64 + 0.5) / n as f64 * std::f64::consts::TAU;
        let rr = r * rng.range(0.3, 1.5);
        let _ = write!(s, "Q{} {} {x1} {y1} ", cx + rr * a.cos(), cy + rr * a.sin());
    }
    s.push('Z');
    s
}

/// A shape as path data plus a human label.
fn shape(rng: &mut Rng, cx: f64, cy: f64, size: f64) -> (String, &'static str) {
    match rng.below(12) {
        0 => (
            rounded_rect(cx - size, cy - size * 0.6, size * 2.0, size * 1.2, 0.0),
            "rect",
        ),
        1 => (
            rounded_rect(
                cx - size,
                cy - size * 0.6,
                size * 2.0,
                size * 1.2,
                size * rng.range(0.05, 0.6),
            ),
            "rrect",
        ),
        2 => (
            ellipse(cx, cy, size, size * rng.range(0.3, 1.0), false),
            "ellipse-cubics",
        ),
        3 => (ellipse(cx, cy, size, size, true), "circle-arcs"),
        4 => {
            let n = 3 + rng.below(9) as usize;
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let a = i as f64 / n as f64 * std::f64::consts::TAU;
                    (cx + size * a.cos(), cy + size * a.sin())
                })
                .collect();
            (polygon(&pts), "regular")
        }
        5 => {
            let n = 5 + 2 * rng.below(4) as usize;
            (
                star(
                    cx,
                    cy,
                    size,
                    n,
                    2 + rng.below(((n - 1) / 2) as u64) as usize,
                ),
                "star",
            )
        }
        6 => {
            let n = 3 + rng.below(10) as usize;
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|_| {
                    (
                        cx + size * rng.range(-1.0, 1.0),
                        cy + size * rng.range(-1.0, 1.0),
                    )
                })
                .collect();
            (polygon(&pts), "random-polygon")
        }
        7 => {
            let n = 3 + rng.below(8) as usize;
            (blob(rng, cx, cy, size, n), "cubic-blob")
        }
        8 => {
            let n = 4 + rng.below(10) as usize;
            (quad_blob(rng, cx, cy, size, n), "quad-blob")
        }
        9 => {
            // a ring: outer circle plus a reversed inner one (a hole)
            let outer = ellipse(cx, cy, size, size, false);
            let inner = ellipse(
                cx,
                cy,
                size * rng.range(0.2, 0.8),
                size * rng.range(0.2, 0.8),
                false,
            );
            let rev = reverse_d(&inner);
            (format!("{outer} {rev}"), "ring")
        }
        10 => {
            // a bowtie (self-crossing quad)
            (
                polygon(&[
                    (cx - size, cy - size),
                    (cx + size, cy + size),
                    (cx + size, cy - size),
                    (cx - size, cy + size),
                ]),
                "bowtie",
            )
        }
        _ => {
            // degenerate-ish: collinear / repeated points / zero height
            match rng.below(3) {
                0 => (
                    polygon(&[
                        (cx, cy),
                        (cx + size, cy),
                        (cx + 2.0 * size, cy),
                        (cx + size, cy + size),
                    ]),
                    "collinear",
                ),
                1 => (
                    polygon(&[
                        (cx, cy),
                        (cx, cy),
                        (cx + size, cy),
                        (cx + size, cy + size),
                        (cx + size, cy + size),
                    ]),
                    "repeated-points",
                ),
                _ => (
                    polygon(&[(cx, cy), (cx + size, cy), (cx + size, cy), (cx, cy)]),
                    "zero-area",
                ),
            }
        }
    }
}

fn reverse_d(d: &str) -> String {
    let p = BezPath::from_svg(d).unwrap().reverse_subpaths();
    p.to_svg()
}

fn transformed(d: &str, a: Affine) -> String {
    let mut p = BezPath::from_svg(&xsvg_core::spline::arcs_to_cubics(d)).unwrap();
    p.apply_affine(a);
    p.to_svg()
}

struct Case {
    seed: u64,
    op: BoolOp,
    operands: Vec<(Vec<String>, bool)>, // (paths, even_odd)
    label: String,
}

fn gen_case(seed: u64) -> Case {
    let mut rng = Rng::new(seed);
    let op = match rng.below(4) {
        0 => BoolOp::Union,
        1 => BoolOp::Intersect,
        2 => BoolOp::Subtract,
        _ => BoolOp::Exclude,
    };
    // the coordinate regime: ordinary, tiny, huge, far from the origin
    let (scale, offset) = match rng.below(8) {
        0 => (0.01, 0.0),
        1 => (1000.0, 0.0),
        2 => (1.0, 1.0e6),
        _ => (1.0, 0.0),
    };
    let n = 2 + rng.below(3) as usize * (rng.chance(0.3) as usize);
    let mut operands = Vec::new();
    let mut labels = Vec::new();
    let (cx, cy, size) = (
        rng.range(-50.0, 50.0),
        rng.range(-50.0, 50.0),
        rng.range(5.0, 60.0),
    );
    let (first, first_label) = shape(&mut rng, cx, cy, size);
    let first_d = transformed(
        &first,
        Affine::scale(scale) * Affine::translate((offset, offset)),
    );
    operands.push((vec![first_d.clone()], rng.chance(0.2)));
    labels.push(first_label.to_string());
    for _ in 1..n {
        // the relation of this operand to the first: independent, a copy, a
        // copy nudged by epsilon, an edge/point contact, nested inside
        let relation = rng.below(7);
        let (d, label) = match relation {
            0 => {
                let (dx, dy, k) = (
                    rng.range(-size, size),
                    rng.range(-size, size),
                    rng.range(0.3, 1.5),
                );
                let (d, l) = shape(&mut rng, cx + dx, cy + dy, size * k);
                (d, l.to_string())
            }
            1 => (first.clone(), "copy".to_string()),
            2 => {
                let eps = 10f64.powf(rng.range(-9.0, -2.0));
                (
                    transformed(&first, Affine::translate((eps, eps * rng.range(-1.0, 1.0)))),
                    format!("copy+{eps:e}"),
                )
            }
            3 => {
                // rotated copy: its curves cross the original's
                (
                    transformed(
                        &first,
                        Affine::rotate_about(rng.range(0.1, 3.0), Point::new(cx, cy)),
                    ),
                    "rotated-copy".to_string(),
                )
            }
            4 => {
                // a rect sharing the first's bounding-box edge exactly
                let bb = BezPath::from_svg(&xsvg_core::spline::arcs_to_cubics(&first))
                    .unwrap()
                    .bounding_box();
                (
                    rounded_rect(bb.x1, bb.y0, size, bb.height(), 0.0),
                    "shared-edge".to_string(),
                )
            }
            5 => {
                // a circle tangent to the first's bbox right edge at its middle
                let bb = BezPath::from_svg(&xsvg_core::spline::arcs_to_cubics(&first))
                    .unwrap()
                    .bounding_box();
                let r = size * 0.5;
                (
                    ellipse(bb.x1 + r, (bb.y0 + bb.y1) / 2.0, r, r, rng.chance(0.5)),
                    "tangent".to_string(),
                )
            }
            _ => {
                let k = rng.range(0.2, 0.6);
                let (d, l) = shape(&mut rng, cx, cy, size * k);
                (d, format!("nested-{l}"))
            }
        };
        let d = transformed(
            &d,
            Affine::scale(scale) * Affine::translate((offset, offset)),
        );
        operands.push((vec![d], rng.chance(0.2)));
        labels.push(label);
    }
    // sometimes an operand of several paths (an outlined textbox is one)
    if rng.chance(0.15) {
        let (extra, l) = shape(&mut rng, cx + size, cy, size * 0.5);
        operands[0].0.push(transformed(
            &extra,
            Affine::scale(scale) * Affine::translate((offset, offset)),
        ));
        labels[0] = format!("{}+{l}", labels[0]);
    }
    Case {
        seed,
        op,
        operands,
        label: format!("{op:?} {} s{scale} o{offset}", labels.join(" / ")),
    }
}

// ---- oracles -------------------------------------------------------------------

#[derive(Default)]
struct Verdict {
    issues: Vec<(String, String)>, // (kind, detail)
    elapsed: Duration,
    curved_len: usize,
    flat_len: usize,
}

fn inside(path: &BezPath, p: Point, even_odd: bool) -> bool {
    let w = path.winding(p);
    if even_odd {
        w % 2 != 0
    } else {
        w != 0
    }
}

fn combine(op: BoolOp, ins: &[bool]) -> bool {
    match op {
        BoolOp::Union => ins.iter().any(|&b| b),
        BoolOp::Intersect => ins.iter().all(|&b| b),
        BoolOp::Subtract => ins[0] && !ins[1..].iter().any(|&b| b),
        BoolOp::Exclude => ins.iter().filter(|&&b| b).count() % 2 == 1,
    }
}

fn parse_closed(d: &str) -> BezPath {
    let p = BezPath::from_svg(&xsvg_core::spline::arcs_to_cubics(d)).unwrap_or_default();
    // close every subpath (fill semantics), as the engines do
    let mut out = BezPath::new();
    let mut open = false;
    for el in p.elements() {
        if let kurbo::PathEl::MoveTo(_) = el {
            if open {
                out.close_path();
            }
            open = true;
        }
        if let kurbo::PathEl::ClosePath = el {
            open = false;
        }
        out.push(*el);
    }
    if open {
        out.close_path();
    }
    out
}

fn evaluate(case: &Case) -> Verdict {
    let mut v = Verdict::default();
    let ops: Vec<BoolOperand> = case
        .operands
        .iter()
        .map(|(paths, eo)| BoolOperand {
            paths: paths.iter().map(String::as_str).collect(),
            even_odd: *eo,
        })
        .collect();
    let t = Instant::now();
    let curved = boolean_svg_paths_curved(&ops, case.op);
    v.elapsed = t.elapsed();
    if v.elapsed > SLOW {
        v.issues.push(("slow".into(), format!("{:?}", v.elapsed)));
    }
    let flat = boolean_svg_paths(&ops, case.op, FLAT_TOLERANCE);
    let curved = match curved {
        Err(e) => {
            v.issues.push(("engine-error".into(), e));
            return v;
        }
        Ok(c) => c,
    };
    v.curved_len = curved.as_ref().map_or(0, String::len);
    v.flat_len = flat.as_ref().map_or(0, String::len);
    let curved_path = curved.as_deref().map(parse_closed).unwrap_or_default();
    let flat_path = flat.as_deref().map(parse_closed).unwrap_or_default();
    if let Some(c) = &curved {
        if c.contains("NaN") || c.contains("inf") {
            v.issues.push(("nan".into(), c.clone()));
        }
    }
    // zero-area slivers and unclosed contours in the output
    let mut sub = BezPath::new();
    let mut slivers = 0;
    for el in curved_path.elements() {
        if let kurbo::PathEl::MoveTo(_) = el {
            if !sub.elements().is_empty() && sub.area().abs() < 1e-9 {
                slivers += 1;
            }
            sub = BezPath::new();
        }
        sub.push(*el);
    }
    if !sub.elements().is_empty() && sub.area().abs() < 1e-9 {
        slivers += 1;
    }
    if slivers > 0 {
        v.issues
            .push(("sliver".into(), format!("{slivers} zero-area contour(s)")));
    }
    // a node repeated in place (a zero-length segment) is a node the editor
    // would have to delete
    let mut dups = 0;
    let mut cur = Point::ZERO;
    for el in curved_path.elements() {
        match *el {
            kurbo::PathEl::MoveTo(p) => cur = p,
            kurbo::PathEl::LineTo(p)
            | kurbo::PathEl::QuadTo(_, p)
            | kurbo::PathEl::CurveTo(_, _, p) => {
                // zero chord with the handles at the point too: nothing drawn
                let handles_home = match *el {
                    kurbo::PathEl::QuadTo(a, _) => (a - cur).hypot() < 1e-3,
                    kurbo::PathEl::CurveTo(a, b, _) => {
                        (a - cur).hypot() < 1e-3 && (b - cur).hypot() < 1e-3
                    }
                    _ => true,
                };
                if (p - cur).hypot() < 1e-9 && handles_home {
                    dups += 1;
                }
                cur = p;
            }
            kurbo::PathEl::ClosePath => {}
        }
    }
    if dups > 0 {
        v.issues.push((
            "dup-points".into(),
            format!("{dups} zero-length segment(s)"),
        ));
    }
    // the truth from the operands; both engines sampled on one grid
    let operand_paths: Vec<(BezPath, bool)> = case
        .operands
        .iter()
        .map(|(paths, eo)| {
            let mut p = BezPath::new();
            for d in paths {
                p.extend(parse_closed(d).elements().iter().copied());
            }
            (p, *eo)
        })
        .collect();
    let mut bb: Option<Rect> = None;
    for (p, _) in &operand_paths {
        if !p.elements().is_empty() {
            let b = p.bounding_box();
            bb = Some(bb.map_or(b, |a| a.union(b)));
        }
    }
    let Some(bb) = bb else { return v };
    if !(bb.width().is_finite() && bb.height().is_finite())
        || bb.width() <= 0.0
        || bb.height() <= 0.0
    {
        return v;
    }
    let n = 64;
    let (mut mism_curved, mut mism_flat, mut both_wrong, mut total, mut truth_in) = (0, 0, 0, 0, 0);
    let (sx, sy) = (bb.width() / n as f64, bb.height() / n as f64);
    // samples jittered within their cells (seeded), so a grid never lines up
    // with an edge — a 45° diamond's edges run through the cell centres
    let mut jit = Rng::new(case.seed ^ 0xA5A5);
    for j in 0..n {
        for i in 0..n {
            let p = Point::new(
                bb.x0 + (i as f64 + jit.range(0.2, 0.8)) * sx,
                bb.y0 + (j as f64 + jit.range(0.2, 0.8)) * sy,
            );
            let ins: Vec<bool> = operand_paths
                .iter()
                .map(|(path, eo)| inside(path, p, *eo))
                .collect();
            let truth = combine(case.op, &ins);
            let c = inside(&curved_path, p, false);
            let f = inside(&flat_path, p, false);
            total += 1;
            truth_in += truth as usize;
            if c != truth {
                mism_curved += 1;
            }
            if f != truth {
                mism_flat += 1;
            }
            if c != truth && f != truth {
                both_wrong += 1;
            }
        }
    }
    // a curved result may miss the truth only where the flat one does too (the
    // sampled truth is itself fuzzy on boundaries): beyond that, and beyond a
    // floor for the grid's own aliasing, it is an issue
    let floor = (total / 100).max(4);
    if mism_curved > mism_flat + floor {
        v.issues.push((
            "pixel".into(),
            format!(
                "curved wrong at {mism_curved}/{total} samples, flat at {mism_flat} (both {both_wrong}); truth covers {truth_in}"
            ),
        ));
    }
    // area parity with the flat engine
    let (ac, af) = (curved_path.area().abs(), flat_path.area().abs());
    let scale = ac.max(af).max(bb.area() * 1e-6);
    if (ac - af).abs() > 0.02 * scale + FLAT_TOLERANCE * 4.0 * (bb.width() + bb.height()) {
        v.issues.push((
            "area".into(),
            format!(
                "curved {ac:.4} flat {af:.4} (bbox {:.1}×{:.1})",
                bb.width(),
                bb.height()
            ),
        ));
    }
    // handle preservation: a union of operands that never meet keeps every
    // input segment verbatim
    if case.op == BoolOp::Union && case.operands.len() == 2 {
        let (a, b) = (&operand_paths[0].0, &operand_paths[1].0);
        let (ba, bb2) = (a.bounding_box(), b.bounding_box());
        let disjoint = ba.x1 < bb2.x0 || bb2.x1 < ba.x0 || ba.y1 < bb2.y0 || bb2.y1 < ba.y0;
        let simple = case.label.contains("rrect")
            || case.label.contains("ellipse-cubics")
            || case.label.contains("rect ");
        if disjoint && simple && !case.operands.iter().any(|(_, eo)| *eo) {
            let want = handles(a)
                .into_iter()
                .chain(handles(b))
                .collect::<std::collections::BTreeSet<_>>();
            let got = handles(&curved_path)
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>();
            let missing: Vec<_> = want.difference(&got).take(3).cloned().collect();
            if !missing.is_empty() {
                v.issues.push((
                    "preserve".into(),
                    format!("input handles missing from the result: {missing:?}"),
                ));
            }
        }
    }
    v
}

/// Every cubic control point of a path, formatted as the compiler writes it.
fn handles(p: &BezPath) -> Vec<String> {
    let f = |v: f64| {
        if v.fract() == 0.0 {
            format!("{}", v as i64)
        } else {
            format!("{v:.3}")
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        }
    };
    p.elements()
        .iter()
        .filter_map(|el| match el {
            kurbo::PathEl::CurveTo(a, b, _) => {
                Some(format!("{} {} / {} {}", f(a.x), f(a.y), f(b.x), f(b.y)))
            }
            _ => None,
        })
        .collect()
}

// ---- the child: a batch of cases, each under catch_unwind ---------------------

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn case_json(case: &Case) -> String {
    let ops: Vec<String> = case
        .operands
        .iter()
        .map(|(paths, eo)| {
            format!(
                "{{\"even_odd\":{eo},\"paths\":[{}]}}",
                paths
                    .iter()
                    .map(|p| json_str(p))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect();
    format!(
        "\"op\":{},\"label\":{},\"operands\":[{}]",
        json_str(&format!("{:?}", case.op)),
        json_str(&case.label),
        ops.join(",")
    )
}

fn child(start: u64, count: u64) {
    use std::sync::Mutex;
    static LAST_PANIC: Mutex<String> = Mutex::new(String::new());
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        *LAST_PANIC.lock().unwrap() = format!(
            "{msg} at {}",
            info.location().map(|l| l.to_string()).unwrap_or_default()
        );
    }));
    let out = std::io::stdout();
    for seed in start..start + count {
        let mut o = out.lock();
        let _ = writeln!(o, "S {seed}");
        let _ = o.flush();
        drop(o);
        let case = gen_case(seed);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| evaluate(&case)));
        let mut o = out.lock();
        match result {
            Ok(v) => {
                let issues: Vec<String> = v
                    .issues
                    .iter()
                    .map(|(k, d)| {
                        format!("{{\"kind\":{},\"detail\":{}}}", json_str(k), json_str(d))
                    })
                    .collect();
                let _ = writeln!(
                    o,
                    "R {seed} {{\"ms\":{},\"curved_len\":{},\"flat_len\":{},\"issues\":[{}]}}",
                    v.elapsed.as_secs_f64() * 1000.0,
                    v.curved_len,
                    v.flat_len,
                    issues.join(",")
                );
            }
            Err(_) => {
                let msg = LAST_PANIC.lock().unwrap().clone();
                let _ = writeln!(
                    o,
                    "R {seed} {{\"issues\":[{{\"kind\":\"panic\",\"detail\":{}}}]}}",
                    json_str(&msg)
                );
            }
        }
        let _ = o.flush();
    }
}

// ---- the parent: batches in child processes, timeouts, the issue log ------------

fn run(seed: u64, count: u64, budget: Duration, dir: &str) {
    std::fs::create_dir_all(dir).unwrap();
    let mut issues = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}/issues.jsonl"))
        .unwrap();
    let mut progress = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(format!("{dir}/progress.log"))
        .unwrap();
    let exe = std::env::current_exe().unwrap();
    let started = Instant::now();
    let mut next = seed;
    let end = seed + count;
    let mut done = 0u64;
    let mut counts: std::collections::BTreeMap<String, u64> = Default::default();
    let mut last_report = Instant::now();
    let record = |issues: &mut std::fs::File,
                  seed: u64,
                  kind: &str,
                  detail: &str,
                  counts: &mut std::collections::BTreeMap<String, u64>| {
        let case = gen_case(seed);
        let _ = writeln!(
            issues,
            "{{\"seed\":{seed},\"kind\":{},\"detail\":{},{}}}",
            json_str(kind),
            json_str(detail),
            case_json(&case)
        );
        *counts.entry(kind.to_string()).or_default() += 1;
    };
    while next < end && started.elapsed() < budget {
        let batch = BATCH.min(end - next);
        let mut child = Command::new(&exe)
            .args(["child", &next.to_string(), &batch.to_string()])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut in_flight: Option<u64> = None;
        let mut last_done = next;
        loop {
            match rx.recv_timeout(CASE_TIMEOUT) {
                Ok(line) => {
                    if let Some(s) = line.strip_prefix("S ") {
                        in_flight = s.trim().parse().ok();
                    } else if let Some(rest) = line.strip_prefix("R ") {
                        let (s, json) = rest.split_once(' ').unwrap_or((rest, "{}"));
                        let seed: u64 = s.parse().unwrap_or(0);
                        in_flight = None;
                        last_done = seed + 1;
                        done += 1;
                        // pull the issues out of the child's line
                        for part in json.split("{\"kind\":").skip(1) {
                            let kind = part.split('"').nth(1).unwrap_or("?").to_string();
                            let detail = part
                                .split("\"detail\":")
                                .nth(1)
                                .map(|d| d.trim_end_matches(['}', ']']).to_string())
                                .unwrap_or_default();
                            let detail = detail.trim_matches('"').replace("\\\"", "\"");
                            record(&mut issues, seed, &kind, &detail, &mut counts);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let _ = child.kill();
                    if let Some(seed) = in_flight {
                        record(
                            &mut issues,
                            seed,
                            "timeout",
                            &format!("no result within {CASE_TIMEOUT:?}"),
                            &mut counts,
                        );
                        last_done = seed + 1;
                        done += 1;
                    }
                    break;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = child.wait().ok();
        if let Some(seed) = in_flight {
            // the child died mid-case without reporting (an abort, a stack overflow)
            if status.is_none_or(|s| !s.success()) && last_done <= seed {
                record(
                    &mut issues,
                    seed,
                    "crash",
                    &format!("child exited {status:?}"),
                    &mut counts,
                );
                last_done = seed + 1;
                done += 1;
            }
        }
        next = last_done.max(next + 1);
        if last_report.elapsed() > Duration::from_secs(60) {
            last_report = Instant::now();
            let line = format!(
                "{:>7.0}s  cases {done}  next seed {next}  {:.0}/s  issues {:?}",
                started.elapsed().as_secs_f64(),
                done as f64 / started.elapsed().as_secs_f64(),
                counts
            );
            let _ = writeln!(progress, "{line}");
            println!("{line}");
        }
    }
    let line = format!(
        "DONE {:.0}s  cases {done}  seeds {seed}..{next}  issues {:?}",
        started.elapsed().as_secs_f64(),
        counts
    );
    let _ = writeln!(progress, "{line}");
    println!("{line}");
}

fn repro(seed: u64) {
    let mut case = gen_case(seed);
    // `--subset 0,2`: only those operands, to bisect a multi-operand case
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--subset") {
        let keep: Vec<usize> = args[i + 1]
            .split(',')
            .filter_map(|t| t.parse().ok())
            .collect();
        case.operands = keep
            .iter()
            .filter_map(|&k| case.operands.get(k).cloned())
            .collect();
        case.label = format!("{} [subset {:?}]", case.label, keep);
    }
    // `--operand I D`: replace operand I's path data, to test a hypothesis
    let mut k = 0;
    while let Some(i) = args[k..].iter().position(|a| a == "--operand") {
        let idx: usize = args[k + i + 1].parse().unwrap();
        case.operands[idx].0 = vec![args[k + i + 2].clone()];
        case.label = format!("{} [operand {idx} replaced]", case.label);
        k += i + 3;
    }
    if let Some(i) = args.iter().position(|a| a == "--op") {
        case.op = BoolOp::parse(&args[i + 1]).expect("union|intersect|subtract|exclude");
        case.label = format!("{} [op {:?}]", case.label, case.op);
    }

    println!("seed {seed}: {}", case.label);
    for (i, (paths, eo)) in case.operands.iter().enumerate() {
        println!("  operand {i} even_odd={eo}");
        for d in paths {
            println!("    d=\"{d}\"");
        }
    }
    let ops: Vec<BoolOperand> = case
        .operands
        .iter()
        .map(|(paths, eo)| BoolOperand {
            paths: paths.iter().map(String::as_str).collect(),
            even_odd: *eo,
        })
        .collect();
    println!("curved: {:?}", boolean_svg_paths_curved(&ops, case.op));
    if args.iter().any(|a| a == "--raw") {
        println!(
            "raw:    {:?}",
            xsvg_core::boolean_svg_paths_curved_raw(&ops, case.op)
        );
    }
    println!(
        "flat:   {:?}",
        boolean_svg_paths(&ops, case.op, FLAT_TOLERANCE)
    );
    let v = evaluate(&case);
    println!("issues: {:?} in {:?}", v.issues, v.elapsed);
    if v.issues.iter().any(|(k, _)| k == "pixel") {
        // a map of the samples: '.' agree, 'c' curved wrong, 'f' flat wrong, 'x' both
        let ops_paths: Vec<(BezPath, bool)> = case
            .operands
            .iter()
            .map(|(paths, eo)| {
                let mut p = BezPath::new();
                for d in paths {
                    p.extend(parse_closed(d).elements().iter().copied());
                }
                (p, *eo)
            })
            .collect();
        let curved = boolean_svg_paths_curved(&ops, case.op)
            .ok()
            .flatten()
            .as_deref()
            .map(parse_closed)
            .unwrap_or_default();
        let flat = boolean_svg_paths(&ops, case.op, FLAT_TOLERANCE)
            .as_deref()
            .map(parse_closed)
            .unwrap_or_default();
        let mut bb: Option<Rect> = None;
        for (p, _) in &ops_paths {
            if !p.elements().is_empty() {
                let b = p.bounding_box();
                bb = Some(bb.map_or(b, |a| a.union(b)));
            }
        }
        let bb = bb.unwrap();
        let n = 64;
        let mut jit = Rng::new(case.seed ^ 0xA5A5);
        for j in 0..n {
            let mut row = String::new();
            for i in 0..n {
                let p = Point::new(
                    bb.x0 + (i as f64 + jit.range(0.2, 0.8)) * bb.width() / n as f64,
                    bb.y0 + (j as f64 + jit.range(0.2, 0.8)) * bb.height() / n as f64,
                );
                let ins: Vec<bool> = ops_paths
                    .iter()
                    .map(|(path, eo)| inside(path, p, *eo))
                    .collect();
                let t = combine(case.op, &ins);
                let (c, f) = (inside(&curved, p, false), inside(&flat, p, false));
                row.push(match (c != t, f != t) {
                    (false, false) => {
                        if t {
                            '#'
                        } else {
                            '.'
                        }
                    }
                    (true, false) => 'c',
                    (false, true) => 'f',
                    (true, true) => 'x',
                });
            }
            println!("{row}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str, default: &str| -> String {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
            .unwrap_or_else(|| default.to_string())
    };
    match args.get(1).map(String::as_str) {
        Some("child") => child(args[2].parse().unwrap(), args[3].parse().unwrap()),
        Some("repro") => repro(flag("--seed", "1").parse().unwrap()),
        Some("run") => run(
            flag("--seed", "1").parse().unwrap(),
            flag("--count", "1000000").parse().unwrap(),
            Duration::from_secs(flag("--budget-secs", "3600").parse().unwrap()),
            &flag("--out", "target/boolfuzz"),
        ),
        _ => eprintln!("usage: boolfuzz run [--seed S] [--count N] [--budget-secs T] [--out DIR] | repro --seed S"),
    }
}
