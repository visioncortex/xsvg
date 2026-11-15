//! Nested viewports (§5.3): a nested `<svg>` — an authored crop, the baked
//! whole-file form of a cross-file `<use>` — lowers to **one `<g>`** whose
//! `transform` folds the element's own transform, the frame's position and the
//! `viewBox` fit, and whose `clip-path` is the frame mapped back into content
//! space. The compiled tree holds no nested `<svg>`: it is the authoring form
//! of a viewport (plain SVG, so an uncompiled crop still crops), and engines
//! disagree about it in the output — WebKit ignores a `transform` on a nested
//! `<svg>`, so a rotated crop sat unrotated there. A `<g>` with a clip is the
//! lowering every engine draws alike.
//!
//! The `<g>` carries `data-xsvg-viewport="x y w h"`, the frame in its own user
//! space (the clip rect), because `getBBox` of a clipped group is the unclipped
//! content: an editor maps that rect through the element's own CTM and has the
//! frame, with no parent lookup.

use super::*;
use crate::kurbo::Rect;

/// `preserveAspectRatio`, parsed (SVG 2 §8.2): `align` is `None` for `none`,
/// else the share of the slack put before the content on each axis — 0, ½ or
/// 1 for `Min`, `Mid`, `Max` — and `slice` whether the content covers the
/// frame rather than fitting inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Par {
    align: Option<(f64, f64)>,
    slice: bool,
}

impl Par {
    /// `xMidYMid meet`, the attribute's initial value.
    pub(super) const DEFAULT: Par = Par {
        align: Some((0.5, 0.5)),
        slice: false,
    };

    /// `None` for anything outside the attribute's closed grammar: a typo, or an
    /// attempt to smuggle markup, and either way not forwarded.
    pub(super) fn parse(v: &str) -> Option<Par> {
        let mut it = v.split_ascii_whitespace();
        let align = it.next()?;
        let mode = it.next();
        if it.next().is_some() {
            return None;
        }
        if align == "none" {
            return mode.is_none().then_some(Par {
                align: None,
                slice: false,
            });
        }
        let share = |k: &str| match k {
            "Min" => Some(0.0),
            "Mid" => Some(0.5),
            "Max" => Some(1.0),
            _ => None,
        };
        let (x, y) = align.strip_prefix('x')?.split_once('Y')?;
        let slice = match mode {
            None | Some("meet") => false,
            Some("slice") => true,
            _ => return None,
        };
        Some(Par {
            align: Some((share(x)?, share(y)?)),
            slice,
        })
    }
}

/// A viewport to lower: its frame in the parent's user space, the `viewBox` it
/// shows there and how (`par`), and whether it clips (`overflow`).
pub(super) struct Viewport {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) w: f64,
    pub(super) h: f64,
    pub(super) view_box: Option<[f64; 4]>,
    pub(super) par: Par,
    pub(super) clip: bool,
}

/// The attributes a viewport's source element spends on the lowering — never
/// copied to the `<g>`.
const CONSUMED: &[&str] = &[
    "x",
    "y",
    "width",
    "height",
    "viewBox",
    "preserveAspectRatio",
    "transform",
    "overflow",
    "version",
    "baseProfile",
];

/// Effects that act on the frame, not the content: a drop shadow is so many
/// frame units, not so many fractions of a unit picture. They go on a wrapper
/// in frame space, around the content `<g>`.
const FRAME_EFFECTS: &[&str] = &["filter", "mask", "clip-path"];

/// A `viewBox` with a positive extent; anything else disables it.
pub(super) fn view_box_of(node: roxmltree::Node) -> Option<[f64; 4]> {
    let p: Vec<f64> = node
        .attribute("viewBox")?
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .filter_map(parse_num)
        .collect();
    match p[..] {
        [x, y, w, h] if w > 0.0 && h > 0.0 => Some([x, y, w, h]),
        _ => None,
    }
}

/// A length attribute: a number (units stripped), or a percentage of
/// `reference` — the enclosing viewport's extent on that axis, when known.
fn length(v: &str, reference: Option<f64>) -> Option<f64> {
    let v = v.trim();
    match v.strip_suffix('%') {
        Some(p) => Some(parse_num(p)? / 100.0 * reference?),
        None => parse_num(v),
    }
}

/// The user-space extent a percentage inside `node` refers to: the nearest
/// enclosing viewport's `viewBox`, else its frame. The root's comes from its
/// `viewBox` or numeric `width`/`height`; a root sized by its window has none.
fn enclosing_extent(node: roxmltree::Node) -> (Option<f64>, Option<f64>) {
    for a in node.ancestors().skip(1).filter(|a| a.is_element()) {
        if !matches!(a.tag_name().name(), "svg" | "xsvg") {
            continue;
        }
        if let Some([_, _, w, h]) = view_box_of(a) {
            return (Some(w), Some(h));
        }
        if a.parent_element().is_none() {
            let dim = |n: &str| {
                a.attribute(n)
                    .and_then(|v| length(v, None))
                    .filter(|v| *v > 0.0)
            };
            return (dim("width"), dim("height"));
        }
        return match Viewport::of(a) {
            Some(vp) => (Some(vp.w), Some(vp.h)),
            None => (None, None),
        };
    }
    (None, None)
}

impl Viewport {
    /// The viewport a nested `<svg>` declares. `None` when its frame cannot be
    /// resolved here — a percentage or missing size with no enclosing extent to
    /// take it of, or a non-positive size, which renders nothing.
    pub(super) fn of(node: roxmltree::Node) -> Option<Viewport> {
        let (pw, ph) = enclosing_extent(node);
        let side = |name: &str, reference: Option<f64>| match node.attribute(name) {
            Some(v) => length(v, reference),
            None => reference, // the attribute's initial value is 100%
        };
        let w = side("width", pw).filter(|w| *w > 0.0)?;
        let h = side("height", ph).filter(|h| *h > 0.0)?;
        let at = |name: &str, reference: Option<f64>| {
            node.attribute(name)
                .and_then(|v| length(v, reference))
                .unwrap_or(0.0)
        };
        Some(Viewport {
            x: at("x", pw),
            y: at("y", ph),
            w,
            h,
            view_box: view_box_of(node),
            par: node
                .attribute("preserveAspectRatio")
                .and_then(Par::parse)
                .unwrap_or(Par::DEFAULT),
            // `auto` is `visible` on a viewport element
            clip: !matches!(node.attribute("overflow"), Some("visible") | Some("auto")),
        })
    }

    /// The `viewBox` fit (SVG 2 §8.2): the scale of content to frame on each
    /// axis, and the offset of the content inside the frame.
    pub(super) fn fit(&self) -> (f64, f64, f64, f64) {
        let Some([_, _, vw, vh]) = self.view_box else {
            return (1.0, 1.0, 0.0, 0.0);
        };
        let (sx, sy) = (self.w / vw, self.h / vh);
        match self.par.align {
            None => (sx, sy, 0.0, 0.0),
            Some((fx, fy)) => {
                let s = if self.par.slice {
                    sx.max(sy)
                } else {
                    sx.min(sy)
                };
                (s, s, (self.w - vw * s) * fx, (self.h - vh * s) * fy)
            }
        }
    }

    /// The frame in content space: the clip, and the box the editor reads. For
    /// a crop (`preserveAspectRatio="none"`) it is the `viewBox` itself.
    pub(super) fn frame_in_content(&self) -> Rect {
        let (sx, sy, ax, ay) = self.fit();
        let [vx, vy, ..] = self.view_box.unwrap_or([0.0; 4]);
        let (x0, y0) = (vx - ax / sx, vy - ay / sy);
        Rect::new(x0, y0, x0 + self.w / sx, y0 + self.h / sy)
    }

    /// The transform placing the frame's origin in the parent's space, after the
    /// element's own.
    fn frame_transform(&self, own: Option<&str>) -> String {
        compose(own, self.x, self.y, 1.0, 1.0, 0.0, 0.0)
    }

    /// The transform placing content in the parent's space — the element's
    /// own, the frame's position, the fit — or, `from_frame`, only the fit.
    fn content_transform(&self, own: Option<&str>, from_frame: bool) -> String {
        let (sx, sy, ax, ay) = self.fit();
        let [vx, vy, ..] = self.view_box.unwrap_or([0.0; 4]);
        if from_frame {
            compose(None, ax, ay, sx, sy, vx, vy)
        } else {
            compose(own, self.x + ax, self.y + ay, sx, sy, vx, vy)
        }
    }
}

/// `own translate(tx ty) scale(sx sy) translate(-vx -vy)`, identity parts left out.
fn compose(own: Option<&str>, tx: f64, ty: f64, sx: f64, sy: f64, vx: f64, vy: f64) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = own.map(str::trim).filter(|t| !t.is_empty()) {
        parts.push(t.to_string());
    }
    if tx != 0.0 || ty != 0.0 {
        parts.push(format!("translate({} {})", fmt(tx), fmt(ty)));
    }
    if sx != 1.0 || sy != 1.0 {
        parts.push(if sx == sy {
            format!("scale({})", fmt(sx))
        } else {
            format!("scale({} {})", fmt(sx), fmt(sy))
        });
    }
    if vx != 0.0 || vy != 0.0 {
        parts.push(format!("translate({} {})", fmt(-vx), fmt(-vy)));
    }
    parts.join(" ")
}

/// Open the `<g>` a viewport lowers to, reading identity, transform, effects
/// and the remaining presentation attributes from `source` (the nested
/// `<svg>`, or the `<use>` a whole-file link bakes); `skip` names further
/// attributes of `source` not to copy. Writes the defs first (a filter's, the
/// clip's), then the frame-space wrapper when an effect needs one, then the
/// keyed content `<g>` with the folded transform, the clip, the viewport
/// marker and the copied attributes. Returns the closing markup.
pub(super) fn open_viewport(
    out: &mut String,
    vp: &Viewport,
    source: roxmltree::Node,
    skip: &[&str],
    ctx: &Ctx,
) -> &'static str {
    let own = source.attribute("transform");
    let effects = FRAME_EFFECTS.iter().any(|a| source.attribute(*a).is_some());
    // a filter on the frame: its region is the frame, exactly
    let filter = if effects {
        lower_filter_attr(source, out, ctx, || Some(Rect::new(0.0, 0.0, vp.w, vp.h)))
    } else {
        None
    };
    let clip_id = vp
        .clip
        .then(|| format!("x-vp-{}", gen_id_seed(source, ctx)));
    let frame = vp.frame_in_content();
    if let Some(id) = &clip_id {
        out.push_str(&format!(
            "<clipPath id=\"{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>",
            fmt(frame.x0),
            fmt(frame.y0),
            fmt(frame.width()),
            fmt(frame.height())
        ));
    }
    let push_transform = |out: &mut String, t: String| {
        if !t.is_empty() {
            out.push_str(" transform=\"");
            push_escaped(out, &t, true);
            out.push('"');
        }
    };
    let mut close = "</g>";
    if effects {
        out.push_str("<g");
        push_transform(out, vp.frame_transform(own));
        for name in FRAME_EFFECTS {
            if *name == "filter" {
                if let Some(f) = &filter {
                    out.push_str(f);
                    continue;
                }
            }
            if let Some(v) = source.attribute(*name) {
                out.push_str(&format!(" {name}=\""));
                push_escaped(out, &ctx.resolve_var(v), true);
                out.push('"');
            }
        }
        out.push('>');
        close = "</g></g>";
    }
    out.push_str("<g");
    out.push_str(&pos_attr(source, ctx));
    push_transform(out, vp.content_transform(own, effects));
    if let Some(id) = &clip_id {
        out.push_str(&format!(" clip-path=\"url(#{id})\""));
    }
    out.push_str(&format!(
        " data-xsvg-viewport=\"{} {} {} {}\"",
        fmt(frame.x0),
        fmt(frame.y0),
        fmt(frame.width()),
        fmt(frame.height())
    ));
    let mut skips: Vec<&str> = CONSUMED.to_vec();
    skips.extend_from_slice(skip);
    if effects {
        skips.extend_from_slice(FRAME_EFFECTS);
    }
    copy_attrs(source, out, &skips, ctx);
    out.push('>');
    close
}

/// Whether a `<use>` in `node`'s document sizes it: `width`/`height` on a `<use>`
/// resize an `<svg>` target and are ignored on a `<g>`, the one place the
/// lowering would change what renders.
fn sized_by_a_use(node: roxmltree::Node) -> bool {
    let Some(id) = node.attribute("id") else {
        return false;
    };
    node.document().descendants().any(|u| {
        u.is_element()
            && u.tag_name().name() == "use"
            && u.attribute("href")
                .or_else(|| u.attribute((XLINK_NS, "href")))
                .and_then(|h| h.strip_prefix('#'))
                == Some(id)
            && (u.attribute("width").is_some() || u.attribute("height").is_some())
    })
}

/// Why a nested `<svg>` stays the viewport element it is, when it does.
fn kept_because(node: roxmltree::Node) -> Option<&'static str> {
    if sized_by_a_use(node) {
        Some("a <use> sizes it")
    } else if Viewport::of(node).is_none() {
        Some("its frame is not resolvable here")
    } else {
        None
    }
}

/// The `transform` a same-document `<use>` must take on to draw its instance
/// unchanged when its target is a kept viewport whose own `transform` was
/// hoisted: a `<use>` deep-clones its target, that transform included, so the
/// instance would lose it. With the instance's CTM being
/// `Tu · translate(ux uy) · T · …`, the `<use>` carries
/// `Tu · translate(ux uy) · T · translate(−ux −uy)` instead and nothing moves.
/// `None` for every other `<use>`.
pub(super) fn use_compensation(node: roxmltree::Node) -> Option<String> {
    let id = local_href_id(node)?;
    let target = node
        .document()
        .descendants()
        .find(|n| n.is_element() && n.attribute("id") == Some(id))?;
    if target.tag_name().namespace() != Some(SVG_NS)
        || target.tag_name().name() != "svg"
        || target.parent_element().is_none()
        || kept_because(target).is_none()
    {
        return None;
    }
    let t = target
        .attribute("transform")
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    let (ux, uy) = (attr_num(node, "x", 0.0), attr_num(node, "y", 0.0));
    let tu = node
        .attribute("transform")
        .map(parse_transform)
        .unwrap_or_default();
    let at = crate::kurbo::Affine::translate((ux, uy));
    let m = (tu * at * parse_transform(t) * at.inverse()).as_coeffs();
    Some(format!(
        "matrix({})",
        m.iter().map(|c| fmt(*c)).collect::<Vec<_>>().join(" ")
    ))
}

/// A nested `<svg>` (§5.3): lowered to a `<g>`, or — when a `<use>` sizes it,
/// or its frame cannot be resolved — kept as the viewport element it is, with
/// its `transform` hoisted onto a wrapping `<g>` so that no nested `<svg>` in
/// the output ever carries one (the `<use>`s of it are compensated,
/// [`use_compensation`]).
pub(super) fn emit_nested_svg(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    if let Some(why) = kept_because(node) {
        keep(node, out, ctx, why);
        return;
    }
    let Some(vp) = Viewport::of(node) else {
        return; // unreachable: an unresolvable frame is a kept one
    };
    let close = open_viewport(out, &vp, node, &[], ctx);
    emit_children(node, out, ctx);
    out.push_str(close);
}

fn keep(node: roxmltree::Node, out: &mut String, ctx: &Ctx, why: &str) {
    out.push_str(&format!("<!-- xsvg: nested <svg> kept: {why} -->"));
    let hoisted = transform_attr(node);
    if !hoisted.is_empty() {
        out.push_str("<g");
        out.push_str(&hoisted);
        out.push('>');
    }
    emit_passthrough(node, "svg", out, false, &["transform"], ctx);
    if !hoisted.is_empty() {
        out.push_str("</g>");
    }
}

/// The `x y w h` a lowered viewport carries, for measuring compiled output.
pub(super) fn viewport_marker(node: roxmltree::Node) -> Option<Rect> {
    let p: Vec<f64> = node
        .attribute("data-xsvg-viewport")?
        .split(' ')
        .filter_map(parse_num)
        .collect();
    match p[..] {
        [x, y, w, h] => Some(Rect::new(x, y, x + w, y + h)),
        _ => None,
    }
}
