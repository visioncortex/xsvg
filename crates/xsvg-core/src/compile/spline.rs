//! Convert to spline (docs/Incremental.md §11): `to_path` with every circular
//! or elliptical arc (`A`) expanded to cubic Béziers, for an editor whose path
//! model is Béziers only. An arc is split at most a quarter turn per piece and
//! each piece is the one cubic designers expect (`κ = 4/3·tan(θ/4)`, the
//! 0.5523 of a quarter circle), so a circle becomes four anchors at its
//! extremes and a rounded corner one cubic — never a flattening. Everything
//! that is not an arc passes through byte for byte.

use super::fmt;
use crate::kurbo::{Arc, Point, SvgArc, Vec2};

/// Every `A`/`a` command of `d` rewritten as absolute `C` cubics, the rest of
/// the path data untouched. Implicit repeats are honoured (an `A` with two
/// arcs' parameters converts both); a degenerate arc (its endpoints equal) is
/// dropped, one with a zero radius is the straight line SVG draws for it, and
/// radii too small for their chord are scaled up as SVG does. Path data that
/// does not parse is returned as is.
pub fn arcs_to_cubics(d: &str) -> String {
    let Some(cmds) = commands(d) else {
        return d.to_string();
    };
    if !cmds.iter().any(|c| matches!(c.cmd, 'A' | 'a')) {
        return d.to_string();
    }
    let mut out = String::with_capacity(d.len() + 64);
    // the current point and subpath start, as every command leaves them
    let (mut cur, mut start) = (Point::ZERO, Point::ZERO);
    for c in &cmds {
        if !matches!(c.cmd, 'A' | 'a') {
            out.push_str(&d[c.span.0..c.span.1]);
            step(&mut cur, &mut start, c);
            continue;
        }
        let rel = c.cmd == 'a';
        let mut cubics = String::new();
        let mut first = true;
        for a in c.nums.chunks_exact(7) {
            let mut to = Point::new(a[5], a[6]);
            if rel {
                to += cur.to_vec2();
            }
            push_arc(&mut cubics, cur, to, a, &mut first);
            cur = to;
        }
        // the cubics take the arc's place between its neighbours' separators;
        // an arc that drew nothing leaves none behind
        if !cubics.is_empty() {
            if !out.is_empty() && !out.ends_with([' ', ',', '\n', '\t']) {
                out.push(' ');
            }
            out.push_str(&cubics);
            out.push_str(&d[c.nums_end..c.span.1]);
        }
    }
    out
}

/// One cubic per piece of at most a quarter turn, or the straight line a
/// degenerate arc draws.
fn push_arc(out: &mut String, from: Point, to: Point, a: &[f64], first: &mut bool) {
    if (to - from).hypot() < 1e-12 {
        return; // SVG: an arc back to its own start draws nothing
    }
    let mut cmd = |out: &mut String, letter: char| {
        if !*first {
            out.push(' ');
        }
        *first = false;
        out.push(letter);
    };
    let svg = SvgArc {
        from,
        to,
        radii: Vec2::new(a[0].abs(), a[1].abs()),
        x_rotation: a[2].to_radians(),
        large_arc: a[3] != 0.0,
        sweep: a[4] != 0.0,
    };
    let Some(arc) = Arc::from_svg_arc(&svg) else {
        // a zero radius: the line between the endpoints
        cmd(out, 'L');
        push_pt(out, to);
        return;
    };
    let n = (arc.sweep_angle.abs() / std::f64::consts::FRAC_PI_2 - 1e-9)
        .ceil()
        .max(1.0) as usize;
    let step = arc.sweep_angle / n as f64;
    let kappa = 4.0 / 3.0 * (step / 4.0).tan();
    let (sin_r, cos_r) = arc.x_rotation.sin_cos();
    // the ellipse and its tangent at a parametric angle, rotated into place
    let point = |t: f64| -> Point {
        let (s, c) = t.sin_cos();
        let (x, y) = (arc.radii.x * c, arc.radii.y * s);
        arc.center + Vec2::new(x * cos_r - y * sin_r, x * sin_r + y * cos_r)
    };
    let tangent = |t: f64| -> Vec2 {
        let (s, c) = t.sin_cos();
        let (x, y) = (-arc.radii.x * s, arc.radii.y * c);
        Vec2::new(x * cos_r - y * sin_r, x * sin_r + y * cos_r)
    };
    let mut t0 = arc.start_angle;
    let mut p0 = from; // the authored endpoints are kept exactly
    for i in 0..n {
        let t1 = t0 + step;
        let p3 = if i + 1 == n { to } else { point(t1) };
        let p1 = p0 + tangent(t0) * kappa;
        let p2 = p3 - tangent(t1) * kappa;
        cmd(out, 'C');
        push_pt(out, p1);
        out.push(' ');
        push_pt(out, p2);
        out.push(' ');
        push_pt(out, p3);
        t0 = t1;
        p0 = p3;
    }
}

fn push_pt(out: &mut String, p: Point) {
    // a tangent's exact axis comes out as ±1e-17: that is `0`, not `-0`
    let z = |v: f64| if v.abs() < 5e-4 { 0.0 } else { v };
    out.push_str(&fmt(z(p.x)));
    out.push(' ');
    out.push_str(&fmt(z(p.y)));
}

/// One command of path data: its letter, its numbers, and the bytes it spans
/// (from its letter to the next command's), copied verbatim when it passes.
struct Cmd {
    cmd: char,
    nums: Vec<f64>,
    span: (usize, usize),
    /// Where the numbers end: `span.1` less the trailing separators.
    nums_end: usize,
}

/// Split path data into commands. `None` when it does not parse (a stray
/// character, a number where a flag belongs, a wrong parameter count).
fn commands(d: &str) -> Option<Vec<Cmd>> {
    let b = d.as_bytes();
    let mut cmds: Vec<Cmd> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() || b[i] == b',' {
            i += 1;
            continue;
        }
        let cmd = b[i] as char;
        if !cmd.is_ascii_alphabetic() {
            return None;
        }
        let begin = i;
        i += 1;
        let arity = arity(cmd)?;
        let mut nums = Vec::new();
        loop {
            // one parameter set, or none for Z; a set may repeat implicitly
            let mut set = Vec::with_capacity(arity);
            let save = i;
            for k in 0..arity {
                skip_sep(b, &mut i);
                let flag = matches!(cmd, 'A' | 'a') && (k == 3 || k == 4);
                let Some(v) = (if flag {
                    read_flag(b, &mut i)
                } else {
                    read_num(d, &mut i)
                }) else {
                    if k == 0 && !nums.is_empty() {
                        i = save;
                        break;
                    }
                    return None;
                };
                set.push(v);
            }
            if set.len() < arity {
                break;
            }
            nums.extend(set);
            if arity == 0 {
                break;
            }
            // another set follows only if the next token is a number
            let mut peek = i;
            skip_sep(b, &mut peek);
            if peek >= b.len()
                || !(b[peek].is_ascii_digit() || matches!(b[peek], b'-' | b'+' | b'.'))
            {
                break;
            }
        }
        // the span runs to the next command letter, trailing separators included
        let mut end = i;
        while end < b.len() && (b[end].is_ascii_whitespace() || b[end] == b',') {
            end += 1;
        }
        cmds.push(Cmd {
            cmd,
            nums,
            span: (begin, end),
            nums_end: i,
        });
        i = end;
    }
    Some(cmds)
}

fn arity(cmd: char) -> Option<usize> {
    Some(match cmd.to_ascii_uppercase() {
        'M' | 'L' | 'T' => 2,
        'H' | 'V' => 1,
        'C' => 6,
        'S' | 'Q' => 4,
        'A' => 7,
        'Z' => 0,
        _ => return None,
    })
}

fn skip_sep(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
        *i += 1;
    }
}

fn read_flag(b: &[u8], i: &mut usize) -> Option<f64> {
    let v = match b.get(*i)? {
        b'0' => 0.0,
        b'1' => 1.0,
        _ => return None,
    };
    *i += 1;
    Some(v)
}

/// A number as the path grammar lexes it: sign, digits, one `.`, an exponent;
/// glued forms (`1-2`, `.5.5`) stop where the next number starts.
fn read_num(d: &str, i: &mut usize) -> Option<f64> {
    let b = d.as_bytes();
    let start = *i;
    let mut j = *i;
    if j < b.len() && matches!(b[j], b'-' | b'+') {
        j += 1;
    }
    let mut digits = 0;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < b.len() && b[j] == b'.' {
        j += 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if j < b.len() && matches!(b[j], b'e' | b'E') {
        let mut k = j + 1;
        if k < b.len() && matches!(b[k], b'-' | b'+') {
            k += 1;
        }
        let e0 = k;
        while k < b.len() && b[k].is_ascii_digit() {
            k += 1;
        }
        if k > e0 {
            j = k;
        }
    }
    let v: f64 = d[start..j].parse().ok()?;
    *i = j;
    Some(v)
}

/// Advance the current point (and subpath start) past a non-arc command.
fn step(cur: &mut Point, start: &mut Point, c: &Cmd) {
    let rel = c.cmd.is_ascii_lowercase();
    let up = c.cmd.to_ascii_uppercase();
    if up == 'Z' {
        *cur = *start;
        return;
    }
    let ar = arity(c.cmd).unwrap_or(0);
    for (k, set) in c.nums.chunks_exact(ar).enumerate() {
        let base = if rel { cur.to_vec2() } else { Vec2::ZERO };
        match up {
            'M' => {
                *cur = Point::new(set[0], set[1]) + base;
                if k == 0 {
                    *start = *cur;
                }
            }
            'H' => cur.x = set[0] + base.x,
            'V' => cur.y = set[0] + base.y,
            _ => *cur = Point::new(set[ar - 2], set[ar - 1]) + base,
        }
    }
}

/// `to_path` markup with every `d` attribute's arcs expanded to cubics.
pub(super) fn markup_to_spline(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    while let Some(i) = rest.find(" d=\"") {
        let (head, tail) = rest.split_at(i + 4);
        out.push_str(head);
        let Some(end) = tail.find('"') else {
            out.push_str(tail);
            return out;
        };
        out.push_str(&arcs_to_cubics(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}
