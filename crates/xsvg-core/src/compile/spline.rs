//! The compile-side half of convert-to-spline: the converter itself lives in
//! [`crate::spline`] (no feature gate, so the boolean algebra can use it too).

pub use crate::spline::arcs_to_cubics;

/// `to_path` markup with every `d` attribute's arcs expanded to cubics.
pub(super) fn markup_to_spline(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut last = 0;
    for (a, b) in d_attr_ranges(markup) {
        out.push_str(&markup[last..a]);
        out.push_str(&arcs_to_cubics(&markup[a..b]));
        last = b;
    }
    out.push_str(&markup[last..]);
    out
}

/// Byte ranges of the value of every `d` attribute in `markup`, read as XML:
/// only inside a start tag, never in character data or a comment. A converted
/// `<g>` carries authored children through verbatim, so `d="…"` may appear in
/// a `<text>`'s words or a comment, where it is not path data.
pub(super) fn d_attr_ranges(markup: &str) -> Vec<(usize, usize)> {
    let bytes = markup.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(lt) = markup[i..].find('<').map(|p| i + p) {
        let rest = &markup[lt..];
        // comments, CDATA and processing instructions hold no attributes
        let skip = [("<!--", "-->"), ("<![CDATA[", "]]>"), ("<?", "?>")]
            .iter()
            .find(|(open, _)| rest.starts_with(open));
        if let Some((open, close)) = skip {
            i = match rest[open.len()..].find(close) {
                Some(p) => lt + open.len() + p + close.len(),
                None => markup.len(),
            };
            continue;
        }
        // a tag: walk its attributes, quotes respected, to the closing `>`
        let mut j = lt + 1;
        while j < bytes.len() && bytes[j] != b'>' {
            if bytes[j] == b'"' || bytes[j] == b'\'' {
                let quote = bytes[j];
                let start = j + 1;
                let end = markup[start..]
                    .find(quote as char)
                    .map_or(markup.len(), |p| start + p);
                // the name before `=`: exactly `d`, after white space
                let named_d = j >= lt + 3
                    && bytes[j - 1] == b'='
                    && bytes[j - 2] == b'd'
                    && bytes[j - 3].is_ascii_whitespace();
                if named_d {
                    out.push((start, end));
                }
                j = end + 1;
            } else {
                j += 1;
            }
        }
        i = (j + 1).min(markup.len());
    }
    out
}
