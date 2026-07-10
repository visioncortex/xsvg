//! CSS filter functions → SVG filter primitives (Specification.md §8): the
//! pixel-adjustment slice of Pillar 3. The author writes the standard `filter`
//! attribute with CSS function syntax — which browsers already render live —
//! and the compiler lowers it to an equivalent `<filter>` graph so static
//! renderers get the same pixels. `-x-curve(…)` extends the vocabulary with
//! Photoshop-style tone curves (monotone-cubic through control points, sampled
//! into a `feComponentTransfer` lookup table).
//!
//! Parsing and primitive generation are pure and platform-free; the wasm layer
//! wires them to the attribute.

use std::fmt::Write;

/// One parsed filter function. Amounts are normalized (percentages divided
/// through); angles are degrees.
#[derive(Clone, Debug, PartialEq)]
pub enum AdjustFn {
    Brightness(f64),
    Contrast(f64),
    Saturate(f64),
    Grayscale(f64),
    Sepia(f64),
    Invert(f64),
    HueRotate(f64),
    Opacity(f64),
    /// `-x-curve[-r|-g|-b|-a](x0 y0, x1 y1, …)` — a tone curve through control
    /// points in [0,1]², monotone-cubic interpolated (no overshoot), applied
    /// to the given channel(s).
    Curve {
        channel: CurveChannel,
        points: Vec<(f64, f64)>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurveChannel {
    Rgb,
    R,
    G,
    B,
    A,
}

/// Parse a `filter` attribute value as a list of lowerable filter functions.
/// `None` means "not ours to lower" — a `url(#…)` reference, `none`, an
/// unknown function (e.g. `blur`, deferred), or any invalid argument — and the
/// caller passes the attribute through verbatim, mirroring CSS's
/// whole-declaration-invalid behavior.
pub fn parse_filter_functions(s: &str) -> Option<Vec<AdjustFn>> {
    let mut fns = Vec::new();
    let mut rest = s.trim();
    if rest.is_empty() || rest.eq_ignore_ascii_case("none") {
        return None;
    }
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let name = rest[..open].trim();
        let close = rest[open..].find(')')? + open;
        let args = &rest[open + 1..close];
        fns.push(parse_fn(name, args)?);
        rest = rest[close + 1..].trim_start();
    }
    (!fns.is_empty()).then_some(fns)
}

fn parse_fn(name: &str, args: &str) -> Option<AdjustFn> {
    if let Some(channel) = name.strip_prefix("-x-curve") {
        let channel = match channel {
            "" => CurveChannel::Rgb,
            "-r" => CurveChannel::R,
            "-g" => CurveChannel::G,
            "-b" => CurveChannel::B,
            "-a" => CurveChannel::A,
            _ => return None,
        };
        return Some(AdjustFn::Curve {
            channel,
            points: parse_curve_points(args)?,
        });
    }
    let amount = |lo: f64, hi: f64| -> Option<f64> {
        let t = args.trim();
        let (t, scale) = match t.strip_suffix('%') {
            Some(t) => (t, 0.01),
            None => (t, 1.0),
        };
        let v: f64 = t.trim().parse().ok()?;
        let v = v * scale;
        (v.is_finite() && v >= 0.0).then(|| v.clamp(lo, hi))
    };
    Some(match name {
        "brightness" => AdjustFn::Brightness(amount(0.0, f64::MAX)?),
        "contrast" => AdjustFn::Contrast(amount(0.0, f64::MAX)?),
        "saturate" => AdjustFn::Saturate(amount(0.0, f64::MAX)?),
        "grayscale" => AdjustFn::Grayscale(amount(0.0, 1.0)?),
        "sepia" => AdjustFn::Sepia(amount(0.0, 1.0)?),
        "invert" => AdjustFn::Invert(amount(0.0, 1.0)?),
        "opacity" => AdjustFn::Opacity(amount(0.0, 1.0)?),
        "hue-rotate" => {
            let t = args.trim();
            let t = t.strip_suffix("deg").unwrap_or(t).trim();
            let v: f64 = t.parse().ok()?;
            v.is_finite().then_some(AdjustFn::HueRotate(v))?
        }
        _ => return None,
    })
}

/// `x0 y0, x1 y1, …` — at least two points, all in [0,1]², x strictly
/// increasing.
fn parse_curve_points(s: &str) -> Option<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for pair in s.split(',') {
        let mut it = pair.split_whitespace();
        let x: f64 = it.next()?.parse().ok()?;
        let y: f64 = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        let ok = (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y);
        if !ok {
            return None;
        }
        points.push((x, y));
    }
    let increasing = points.windows(2).all(|w| w[1].0 > w[0].0);
    (points.len() >= 2 && increasing).then_some(points)
}

/// Sample a monotone cubic (Fritsch–Carlson) through `points` at `n` uniform
/// positions across [0,1]. Outside the control range the curve extends flat.
/// Monotone segments never overshoot, so a non-decreasing point set yields a
/// non-decreasing table.
pub fn sample_monotone_curve(points: &[(f64, f64)], n: usize) -> Vec<f64> {
    let k = points.len();
    // secants and Fritsch–Carlson tangents
    let d: Vec<f64> = points
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0))
        .collect();
    let mut m = vec![0.0; k];
    m[0] = d[0];
    m[k - 1] = d[k - 2];
    for i in 1..k - 1 {
        m[i] = if d[i - 1] * d[i] <= 0.0 {
            0.0
        } else {
            (d[i - 1] + d[i]) / 2.0
        };
    }
    for i in 0..k - 1 {
        if d[i] == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else {
            let (a, b) = (m[i] / d[i], m[i + 1] / d[i]);
            let s = a * a + b * b;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m[i] = t * a * d[i];
                m[i + 1] = t * b * d[i];
            }
        }
    }
    (0..n)
        .map(|j| {
            let x = j as f64 / (n - 1) as f64;
            if x <= points[0].0 {
                return points[0].1;
            }
            if x >= points[k - 1].0 {
                return points[k - 1].1;
            }
            let i = points.partition_point(|p| p.0 <= x) - 1;
            let (x0, y0) = points[i];
            let (x1, y1) = points[i + 1];
            let h = x1 - x0;
            let t = (x - x0) / h;
            let (t2, t3) = (t * t, t * t * t);
            let v = y0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                + m[i] * h * (t3 - 2.0 * t2 + t)
                + y1 * (-2.0 * t3 + 3.0 * t2)
                + m[i + 1] * h * (t3 - t2);
            v.clamp(0.0, 1.0)
        })
        .collect()
}

/// Samples per curve table: piecewise-linear interpolation between 64 uniform
/// samples keeps the error of any monotone-cubic curve far below one 8-bit
/// step, at a fraction of a 256-entry table's weight.
const CURVE_SAMPLES: usize = 64;

fn fmt(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".into()
    } else {
        s.into()
    }
}

/// Emit the SVG filter primitives equivalent to the function list, in order.
/// Primitives chain implicitly (each consumes the previous result). The
/// numeric mappings are the ones the Filter Effects spec defines for the CSS
/// shorthand functions, so lowered output matches live browser rendering.
pub fn filter_primitives(fns: &[AdjustFn]) -> String {
    let mut out = String::new();
    for f in fns {
        match f {
            AdjustFn::Brightness(k) => rgb_linear(&mut out, *k, 0.0),
            AdjustFn::Contrast(k) => rgb_linear(&mut out, *k, 0.5 - 0.5 * k),
            AdjustFn::Saturate(k) => {
                let _ = write!(
                    out,
                    "<feColorMatrix type=\"saturate\" values=\"{}\"/>",
                    fmt(*k)
                );
            }
            AdjustFn::Grayscale(k) => {
                let _ = write!(
                    out,
                    "<feColorMatrix type=\"saturate\" values=\"{}\"/>",
                    fmt(1.0 - k)
                );
            }
            AdjustFn::Sepia(k) => {
                // lerp(identity, full-sepia, k), the spec's definition
                let full = [
                    [0.393, 0.769, 0.189],
                    [0.349, 0.686, 0.168],
                    [0.272, 0.534, 0.131],
                ];
                let mut values = String::new();
                for (r, row) in full.iter().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        let id = if r == c { 1.0 } else { 0.0 };
                        let _ = write!(values, "{} ", fmt(id * (1.0 - k) + cell * k));
                    }
                    values.push_str("0 0 ");
                }
                values.push_str("0 0 0 1 0");
                let _ = write!(out, "<feColorMatrix type=\"matrix\" values=\"{values}\"/>");
            }
            AdjustFn::Invert(k) => {
                let table = format!("{} {}", fmt(*k), fmt(1.0 - k));
                let _ = write!(
                    out,
                    "<feComponentTransfer>{}</feComponentTransfer>",
                    ["R", "G", "B"]
                        .map(|ch| format!("<feFunc{ch} type=\"table\" tableValues=\"{table}\"/>"))
                        .join("")
                );
            }
            AdjustFn::HueRotate(deg) => {
                let _ = write!(
                    out,
                    "<feColorMatrix type=\"hueRotate\" values=\"{}\"/>",
                    fmt(*deg)
                );
            }
            AdjustFn::Opacity(k) => {
                let _ = write!(
                    out,
                    "<feComponentTransfer><feFuncA type=\"table\" tableValues=\"0 {}\"/></feComponentTransfer>",
                    fmt(*k)
                );
            }
            AdjustFn::Curve { channel, points } => {
                let table = sample_monotone_curve(points, CURVE_SAMPLES)
                    .iter()
                    .map(|v| fmt(*v))
                    .collect::<Vec<_>>()
                    .join(" ");
                let chs: &[&str] = match channel {
                    CurveChannel::Rgb => &["R", "G", "B"],
                    CurveChannel::R => &["R"],
                    CurveChannel::G => &["G"],
                    CurveChannel::B => &["B"],
                    CurveChannel::A => &["A"],
                };
                out.push_str("<feComponentTransfer>");
                for ch in chs {
                    let _ = write!(out, "<feFunc{ch} type=\"table\" tableValues=\"{table}\"/>");
                }
                out.push_str("</feComponentTransfer>");
            }
        }
    }
    out
}

/// `<feComponentTransfer>` applying the same linear map to R, G, and B.
fn rgb_linear(out: &mut String, slope: f64, intercept: f64) {
    out.push_str("<feComponentTransfer>");
    for ch in ["R", "G", "B"] {
        let _ = write!(
            out,
            "<feFunc{ch} type=\"linear\" slope=\"{}\" intercept=\"{}\"/>",
            fmt(slope),
            fmt(intercept)
        );
    }
    out.push_str("</feComponentTransfer>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_css_vocabulary() {
        let fns =
            parse_filter_functions("brightness(1.2) contrast(105%) hue-rotate(90deg)").unwrap();
        assert_eq!(
            fns,
            vec![
                AdjustFn::Brightness(1.2),
                AdjustFn::Contrast(1.05),
                AdjustFn::HueRotate(90.0),
            ]
        );
        // clamped-to-one family accepts >100% by clamping, like CSS
        assert_eq!(
            parse_filter_functions("sepia(250%)").unwrap(),
            vec![AdjustFn::Sepia(1.0)]
        );
    }

    #[test]
    fn rejects_what_it_must_not_lower() {
        assert_eq!(parse_filter_functions("url(#f)"), None);
        assert_eq!(parse_filter_functions("none"), None);
        assert_eq!(parse_filter_functions("blur(3px)"), None); // deferred
        assert_eq!(parse_filter_functions("brightness(-1)"), None);
        assert_eq!(parse_filter_functions("brightness(1e999)"), None);
        assert_eq!(parse_filter_functions("brightness(1.2) url(#f)"), None);
        assert_eq!(parse_filter_functions("brightness("), None);
        assert_eq!(parse_filter_functions("-x-curve(9 9)"), None);
        assert_eq!(parse_filter_functions("-x-curve(0 0)"), None); // one point
        assert_eq!(parse_filter_functions("-x-curve(0 0, 0 1)"), None); // dup x
    }

    #[test]
    fn primitive_mappings_match_the_spec() {
        let p = filter_primitives(&[AdjustFn::Brightness(1.2)]);
        assert!(p.contains("slope=\"1.2\" intercept=\"0\""), "{p}");
        let p = filter_primitives(&[AdjustFn::Contrast(1.5)]);
        assert!(p.contains("slope=\"1.5\" intercept=\"-0.25\""), "{p}");
        let p = filter_primitives(&[AdjustFn::Grayscale(0.75)]);
        assert!(p.contains("type=\"saturate\" values=\"0.25\""), "{p}");
        let p = filter_primitives(&[AdjustFn::Invert(1.0)]);
        assert!(p.contains("tableValues=\"1 0\""), "{p}");
        let p = filter_primitives(&[AdjustFn::Opacity(0.4)]);
        assert!(
            p.contains("feFuncA type=\"table\" tableValues=\"0 0.4\""),
            "{p}"
        );
        // sepia(1) row one is the full sepia constant row
        let p = filter_primitives(&[AdjustFn::Sepia(1.0)]);
        assert!(p.contains("values=\"0.393 0.769 0.189 0 0"), "{p}");
        // functions chain in authored order
        let p = filter_primitives(&[AdjustFn::Brightness(1.1), AdjustFn::Saturate(0.5)]);
        let b = p.find("feComponentTransfer").unwrap();
        let s = p.find("feColorMatrix").unwrap();
        assert!(b < s, "{p}");
    }

    #[test]
    fn curves_are_monotone_and_hit_their_endpoints() {
        let pts = [(0.0, 0.0), (0.25, 0.15), (0.75, 0.9), (1.0, 1.0)];
        let t = sample_monotone_curve(&pts, 64);
        assert_eq!(t.len(), 64);
        assert!((t[0] - 0.0).abs() < 1e-9 && (t[63] - 1.0).abs() < 1e-9);
        assert!(t.windows(2).all(|w| w[1] >= w[0]), "not monotone: {t:?}");
        // identity control points sample to the identity ramp
        let id = sample_monotone_curve(&[(0.0, 0.0), (1.0, 1.0)], 64);
        for (j, v) in id.iter().enumerate() {
            assert!((v - j as f64 / 63.0).abs() < 1e-9);
        }
        // a partial-range curve extends flat outside its control points
        let flat = sample_monotone_curve(&[(0.25, 0.5), (0.75, 0.5)], 64);
        assert!(flat.iter().all(|v| (v - 0.5).abs() < 1e-9));
    }

    #[test]
    fn curve_tables_reach_the_markup() {
        let fns = parse_filter_functions("-x-curve-r(0 0, 0.5 0.7, 1 1)").unwrap();
        let p = filter_primitives(&fns);
        assert!(p.contains("<feFuncR type=\"table\""), "{p}");
        assert!(!p.contains("feFuncG"), "single channel only: {p}");
    }
}
