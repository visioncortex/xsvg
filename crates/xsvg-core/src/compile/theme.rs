//! Theming (§4.1): the compile-time colour and type tokens of `<x:theme>`.
//!
//! Loaded once per parsed document into a [`Theme`] the [`Ctx`] carries; emitters
//! resolve `var(name)` paints and `x:font="name"` bundles through it.

use super::*;

/// A `<x:theme>`'s tokens, loaded once per parsed document and carried by the
/// [`Ctx`] of every compile over it. `colors` map a name to any paint value
/// (referenced as `var(name)`); `fonts` map a name to an ordered set of font-ish
/// properties (a CSS-`font`-shorthand-like bundle) applied as an OVERRIDABLE base
/// on text carrying `x:font="name"` (the element's own `font-*` wins).
#[derive(Default)]
pub(super) struct Theme {
    pub(super) colors: std::collections::HashMap<String, String>,
    pub(super) fonts: std::collections::HashMap<String, Vec<(String, String)>>,
}

/// The font-ish properties an `<x:font>` token may carry.
pub(super) const FONT_PROPS: [&str; 7] = [
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
];

impl Theme {
    /// Collect `<x:theme>`'s `<x:color>` / `<x:font>` tokens.
    pub(super) fn load(doc: &roxmltree::Document) -> Theme {
        let mut theme = Theme::default();
        for th in doc
            .descendants()
            .filter(|n| n.tag_name().namespace() == Some(XSVG_NS) && n.tag_name().name() == "theme")
        {
            for c in th.children().filter(|c| c.is_element()) {
                match c.tag_name().name() {
                    "color" => {
                        if let (Some(name), Some(val)) = (c.attribute("name"), c.attribute("value"))
                        {
                            theme.colors.insert(name.to_string(), val.to_string());
                        }
                    }
                    "font" => {
                        if let Some(name) = c.attribute("name") {
                            let props = FONT_PROPS
                                .iter()
                                .filter_map(|&p| {
                                    c.attribute(p).map(|v| (p.to_string(), v.to_string()))
                                })
                                .collect();
                            theme.fonts.insert(name.to_string(), props);
                        }
                    }
                    _ => {}
                }
            }
        }
        theme
    }

    /// See [`Ctx::resolve_var`].
    pub(super) fn resolve_var<'v>(&self, v: &'v str) -> std::borrow::Cow<'v, str> {
        if !v.contains("var(") {
            return std::borrow::Cow::Borrowed(v);
        }
        let mut out = String::with_capacity(v.len());
        let mut rest = v;
        while let Some(i) = rest.find("var(") {
            out.push_str(&rest[..i]);
            let after = &rest[i + 4..];
            let Some(close) = after.find(')') else {
                out.push_str(&rest[i..]);
                rest = "";
                break;
            };
            let inner = &after[..close];
            let mut parts = inner.splitn(2, ',');
            let name = parts.next().unwrap_or("").trim().trim_start_matches("--");
            let fallback = parts.next().map(str::trim);
            match self.colors.get(name).map(String::as_str).or(fallback) {
                Some(val) => out.push_str(val),
                None => out.push_str(&rest[i..i + 4 + close + 1]), // leave var(...) verbatim
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        std::borrow::Cow::Owned(out)
    }
}
