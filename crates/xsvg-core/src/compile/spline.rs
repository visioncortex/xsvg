//! The compile-side half of convert-to-spline: the converter itself lives in
//! [`crate::spline`] (no feature gate, so the boolean algebra can use it too).

pub use crate::spline::arcs_to_cubics;

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
