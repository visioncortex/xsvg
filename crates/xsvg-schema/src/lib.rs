//! Machine-readable descriptions of xsvg elements (docs/Extensibility.md §4).
//!
//! An element's parameters — names, types, defaults, ranges, help text — are what
//! a property panel, a plugin manifest, and generated documentation all need, and
//! what a hand-maintained table gets wrong. Declaring them **as a Rust struct**
//! and deriving both the description and the parsing makes the two impossible to
//! separate: a parameter with no field cannot be read, and a field with no use is
//! a `dead_code` warning.
//!
//! ```ignore
//! #[derive(Params)]
//! #[element(name = "offset", doc = "Minkowski grow/shrink of a referenced region")]
//! struct Offset {
//!     #[param(default = 0.0, min = 0.0)] distance: f64,
//!     #[param(default = 4.0)] miter_limit: f64,   // → "miter-limit"
//!     join: Join,                                  // enum → its values, free
//! }
//! ```
//!
//! Both tiers use this: `xsvg-core` describes its built-in `x:` elements, and
//! `xsvg-plugin` generates a module's manifest from it — so a plugin author writes
//! one struct instead of keeping JSON and parsing code in agreement by hand.

#![forbid(unsafe_code)]

// The derive expands to `::xsvg_schema::…` paths, so this crate must be able to
// name itself for its own tests and doc examples.
extern crate self as xsvg_schema;

pub use xsvg_schema_derive::{ParamEnum, Params};

/// How a parameter's value should be read and edited.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamKind {
    Number,
    /// A paint value. Deliberately distinct from `Text`: an editor should offer a
    /// colour affordance but must still accept `var(accent)`, `currentColor`, or
    /// `none`, which no colour picker can hold.
    Color,
    /// One of [`ParamSpec::values`].
    Enum,
    Text,
    Bool,
}

impl ParamKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Number => "number",
            Self::Color => "color",
            Self::Enum => "enum",
            Self::Text => "string",
            Self::Bool => "boolean",
        }
    }
}

/// Whether an element consumes referenced geometry via `in="#id"`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InputMode {
    #[default]
    None,
    Optional,
    Required,
}

impl InputMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Optional => "optional",
            Self::Required => "required",
        }
    }
}

/// Whether an element is a leaf or a container whose children are compiled first.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChildMode {
    #[default]
    None,
    Container,
}

impl ChildMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Container => "container",
        }
    }
}

/// One declared parameter.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    /// The attribute name as authored, e.g. `head-width`.
    pub name: &'static str,
    pub kind: ParamKind,
    /// The value used when the attribute is absent, rendered as text. `None` means
    /// the element computes one (e.g. "defaults to 2.5x thickness") — say so in
    /// [`Self::doc`] rather than inventing a constant that isn't true.
    pub default: Option<&'static str>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// Allowed values when `kind` is [`ParamKind::Enum`], else empty.
    pub values: &'static [&'static str],
    pub doc: Option<&'static str>,
    /// Other parameters that, when present, make this one have no effect — e.g.
    /// `<clip:arrow length>` is read only in the polar form, so setting `x2`/`y2`
    /// supersedes it.
    ///
    /// Only the element knows this; without it an editing host offers a control that
    /// silently does nothing, which reads as a broken editor rather than as an
    /// inapplicable parameter. Declaring it is what lets the host say *why*.
    pub superseded_by: &'static [&'static str],
    /// Why a host must not offer this parameter for editing, or `None` when it may.
    ///
    /// Not about permission — the attribute is the author's to write, and the panel
    /// still shows its value. It is about effects a host cannot carry out from an
    /// attribute edit alone: `font-family` selects a face that has to be *loaded*
    /// before anything can be measured in it, asynchronously, changing layout and
    /// outline availability for the whole document. An editor that offered a field
    /// there would silently lay the document out in a fallback face.
    pub read_only: Option<&'static str>,
    /// Whether this parameter places or sizes the element — the position and extent
    /// a selection rectangle and its handles would express (`x`/`y`, `width`/`height`,
    /// `cx`/`cy`/`r`, endpoints, and polar substitutes for an endpoint).
    ///
    /// A **fact**, not a policy: the element states what the parameter *is*, and each
    /// host decides what to do with it. An editor with drag handles on canvas owns
    /// geometry there and would not want a duplicate numeric field; the viewer's
    /// inspector, which has no handles, leaves geometry to the source pane and offers
    /// only the styling half. Either way a host that ignores this keeps working.
    ///
    /// Not every dimension is geometry. `stroke-width`, `inner-radius`, and `bulge`
    /// shape the drawing without placing or sizing the element, and stay editable.
    pub geometry: bool,
    /// The XML namespace this attribute lives in, or `None` for a plain unprefixed
    /// name (nearly all of them).
    ///
    /// `x:border-width` is `border-width` in the xsvg namespace, and the compiler
    /// reads it by namespace — so the prefix a document happens to bind is the
    /// document's business. A host must resolve this namespace to *that document's*
    /// prefix before reading or writing the attribute: hard-coding `x:` would write
    /// an unbound prefix into a document that spells it otherwise, which is not
    /// merely wrong but malformed XML.
    pub ns: Option<&'static str>,
}

impl ParamSpec {
    /// A minimal spec, for `const` construction by the derive.
    pub const fn new(name: &'static str, kind: ParamKind) -> Self {
        Self {
            name,
            kind,
            default: None,
            min: None,
            max: None,
            step: None,
            values: &[],
            doc: None,
            superseded_by: &[],
            read_only: None,
            geometry: false,
            ns: None,
        }
    }
}

/// One element: what it is called, what it takes, and what it does with children.
#[derive(Clone, Copy, Debug)]
pub struct ElementSpec {
    pub name: &'static str,
    pub doc: Option<&'static str>,
    /// The element's own parameters.
    pub params: &'static [ParamSpec],
    /// Shared parameter groups this element also accepts — the font block every
    /// text element takes, for instance. Kept as a list of slices rather than one
    /// flat array because a `const` cannot concatenate slices, and flattening at
    /// read time costs nothing.
    pub groups: &'static [&'static [ParamSpec]],
    pub input: InputMode,
    pub children: ChildMode,
}

impl ElementSpec {
    /// Every parameter this element accepts, own and inherited.
    pub fn all_params(&self) -> impl Iterator<Item = &'static ParamSpec> {
        self.params.iter().chain(self.groups.iter().copied().flatten())
    }
}

/// Read attributes from wherever they live — an XML node in the compiler, a
/// decoded request in a plugin. The one seam the generated parsing needs.
pub trait AttrSource {
    fn attr(&self, name: &str) -> Option<&str>;
}

/// An element described by a struct. Both halves are derived together, which is
/// what keeps them in agreement.
pub trait Params: Sized {
    const SPEC: ElementSpec;
    fn from_attrs(src: &dyn AttrSource) -> Self;
}

/// A parameter type with a closed set of spellings.
pub trait ParamEnum: Sized {
    const VALUES: &'static [&'static str];
    fn parse(s: &str) -> Option<Self>;
    fn as_str(&self) -> &'static str;
}

// ---- JSON -----------------------------------------------------------------

/// Append `s` as a quoted JSON string.
pub fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                for shift in [12, 8, 4, 0] {
                    out.push(char::from_digit((c as u32 >> shift) & 0xF, 16).unwrap_or('0'));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_num(out: &mut String, v: f64) {
    if v.is_finite() {
        out.push_str(&format!("{v}"));
    } else {
        out.push('0');
    }
}

impl ParamSpec {
    fn write_json(&self, out: &mut String) {
        out.push_str("{\"type\":");
        push_json_str(out, self.kind.as_str());
        if let Some(d) = self.default {
            out.push_str(",\"default\":");
            // numbers and booleans stay unquoted so the JSON types match the kind
            match self.kind {
                ParamKind::Number | ParamKind::Bool => out.push_str(d),
                _ => push_json_str(out, d),
            }
        }
        for (key, v) in [("min", self.min), ("max", self.max), ("step", self.step)] {
            if let Some(v) = v {
                out.push_str(&format!(",\"{key}\":"));
                push_num(out, v);
            }
        }
        if !self.values.is_empty() {
            out.push_str(",\"values\":[");
            for (i, v) in self.values.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_str(out, v);
            }
            out.push(']');
        }
        if !self.superseded_by.is_empty() {
            out.push_str(",\"supersededBy\":[");
            for (i, v) in self.superseded_by.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json_str(out, v);
            }
            out.push(']');
        }
        if let Some(r) = self.read_only {
            out.push_str(",\"readOnly\":");
            push_json_str(out, r);
        }
        if self.geometry {
            out.push_str(",\"geometry\":true");
        }
        if let Some(ns) = self.ns {
            out.push_str(",\"ns\":");
            push_json_str(out, ns);
        }
        if let Some(d) = self.doc {
            out.push_str(",\"doc\":");
            push_json_str(out, d);
        }
        out.push('}');
    }
}

impl ElementSpec {
    /// This element as a manifest entry (the value side of `elements`).
    pub fn write_json(&self, out: &mut String) {
        out.push('{');
        if let Some(d) = self.doc {
            out.push_str("\"doc\":");
            push_json_str(out, d);
            out.push(',');
        }
        out.push_str("\"params\":{");
        for (i, p) in self.all_params().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_json_str(out, p.name);
            out.push(':');
            p.write_json(out);
        }
        out.push_str("},\"input\":");
        push_json_str(out, self.input.as_str());
        out.push_str(",\"children\":");
        push_json_str(out, self.children.as_str());
        out.push('}');
    }
}

/// Build a plugin manifest (docs/Extensibility.md §4) from declared elements.
/// Generated rather than hand-written, so a module's description cannot drift
/// from the code that reads its parameters.
pub fn manifest_json(
    abi: i32,
    ns: &str,
    name: &str,
    version: &str,
    elements: &[ElementSpec],
) -> String {
    let mut out = format!("{{\"abi\":{abi},\"ns\":");
    push_json_str(&mut out, ns);
    out.push_str(",\"name\":");
    push_json_str(&mut out, name);
    out.push_str(",\"version\":");
    push_json_str(&mut out, version);
    out.push_str(",\"elements\":{");
    for (i, e) in elements.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_str(&mut out, e.name);
        out.push(':');
        e.write_json(&mut out);
    }
    out.push_str("}}");
    out
}

// ---- parsing helpers the derive calls -------------------------------------

/// Parse a number the way every length attribute in xsvg is read: the numeric
/// prefix, so `13px` is 13, and never a non-finite value — totality is the
/// compiler's contract and a parameter must not be the thing that breaks it.
pub fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(s.len());
    s[..end].parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn parse_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Attrs(HashMap<&'static str, &'static str>);
    impl AttrSource for Attrs {
        fn attr(&self, name: &str) -> Option<&str> {
            self.0.get(name).copied()
        }
    }
    fn attrs(pairs: &[(&'static str, &'static str)]) -> Attrs {
        Attrs(pairs.iter().copied().collect())
    }

    #[derive(ParamEnum, PartialEq, Debug, Default)]
    enum Heads {
        #[default]
        End,
        Start,
        Both,
        None,
    }

    #[derive(ParamEnum, PartialEq, Debug, Default)]
    enum Marker {
        #[default]
        Bullet,
        /// A spelling that is not just the lower-cased variant name.
        LowerAlpha,
        #[value("1.")]
        Decimal,
    }

    #[derive(Params, Debug)]
    #[element(name = "arrow", doc = "a thick arrow", children = "container")]
    struct Arrow {
        #[param(default = 16.0, min = 0.0, max = 200.0)]
        thickness: f64,
        /// No default: the element computes one, and the schema says so instead
        /// of inventing a constant that isn't true.
        #[param(doc = "defaults to 2.5x thickness")]
        head_width: Option<f64>,
        heads: Heads,
        #[param(color, default = "currentColor")]
        fill: String,
        stroke: Option<String>,
        #[param(default = false)]
        outlined: bool,
        /// Read only when the endpoint form is absent — a fact only the element
        /// knows, so it has to be declared for a host to act on.
        #[param(default = 100.0, superseded_by = "x2, y2")]
        length: f64,
        /// Places the element. Declared so a host can decide where geometry is
        /// edited — on canvas with handles, or not at all.
        #[param(default = 0.0, geometry)]
        x: f64,
    }

    #[test]
    fn a_field_name_becomes_its_attribute_name() {
        let p = Arrow::SPEC.params;
        assert_eq!(p[1].name, "head-width", "underscores become dashes");
        assert_eq!(Arrow::SPEC.name, "arrow");
        assert_eq!(Arrow::SPEC.children, ChildMode::Container);
        assert_eq!(Arrow::SPEC.input, InputMode::None, "defaults when unstated");
    }

    #[test]
    fn declared_metadata_reaches_the_spec() {
        let by = |n: &str| Arrow::SPEC.params.iter().find(|p| p.name == n).unwrap();
        assert_eq!(by("thickness").kind, ParamKind::Number);
        assert_eq!(by("thickness").default, Some("16.0"));
        assert_eq!(by("thickness").min, Some(0.0));
        assert_eq!(by("thickness").max, Some(200.0));
        assert_eq!(by("fill").kind, ParamKind::Color, "#[param(color)]");
        assert_eq!(by("stroke").kind, ParamKind::Text);
        assert_eq!(by("outlined").kind, ParamKind::Bool);
        // an element that computes its default declares none, and says why
        assert_eq!(by("head-width").default, None);
        assert_eq!(by("head-width").doc, Some("defaults to 2.5x thickness"));
        // a parameter another one can render inert names what supersedes it, so an
        // editing host can disable the control instead of offering one that silently
        // does nothing
        assert_eq!(by("length").superseded_by, ["x2", "y2"]);
        assert!(by("thickness").superseded_by.is_empty());
        // geometry is a declared fact, and dimensions that merely shape the drawing
        // are not it — `thickness` stays editable wherever `x` is not offered
        assert!(by("x").geometry);
        assert!(!by("thickness").geometry);
        // a plain attribute names no namespace; only hand-written specs do
        assert_eq!(by("x").ns, None);
    }

    #[test]
    fn an_enum_parameter_carries_its_own_spellings() {
        let heads = Arrow::SPEC.params.iter().find(|p| p.name == "heads").unwrap();
        assert_eq!(heads.kind, ParamKind::Enum);
        // the values come from the enum's own derive — they cannot be restated wrongly
        assert_eq!(heads.values, &["end", "start", "both", "none"]);
        assert_eq!(Marker::VALUES, &["bullet", "lower-alpha", "1."]);
        assert_eq!(Marker::parse("lower-alpha"), Some(Marker::LowerAlpha));
        assert_eq!(Marker::parse("1."), Some(Marker::Decimal));
        assert_eq!(Marker::parse("nope"), None);
    }

    #[test]
    fn reading_uses_the_same_declarations() {
        let a = Arrow::from_attrs(&attrs(&[
            ("thickness", "24"),
            ("heads", "both"),
            ("stroke", "#111"),
            ("outlined", "true"),
        ]));
        assert_eq!(a.thickness, 24.0);
        assert_eq!(a.heads, Heads::Both);
        assert_eq!(a.head_width, None);
        assert_eq!(a.fill, "currentColor", "declared default applies");
        assert_eq!(a.stroke.as_deref(), Some("#111"));
        assert!(a.outlined);
    }

    #[test]
    fn a_unit_suffix_parses_its_numeric_prefix() {
        // every length attribute in xsvg reads this way, so a declared parameter
        // must too or `13px` would silently become the default
        assert_eq!(parse_num("13px"), Some(13.0));
        assert_eq!(parse_num(" 40 "), Some(40.0));
        assert_eq!(parse_num("-1e999"), None, "non-finite is rejected");
        assert_eq!(parse_num("px"), None);
        // `e` is exponent syntax, so an em suffix is NOT a numeric prefix — the
        // same quirk the compiler's own parse_num has, matched deliberately so a
        // declared parameter reads exactly like every other length attribute
        assert_eq!(parse_num("2.5em"), None);
    }

    #[test]
    fn nonsense_values_fall_back_rather_than_poisoning_the_output() {
        // totality: a parameter must never be the thing that emits NaN
        let a = Arrow::from_attrs(&attrs(&[
            ("thickness", "banana"),
            ("head-width", "NaN"),
            ("heads", "sideways"),
            ("outlined", "maybe"),
        ]));
        assert_eq!(a.thickness, 16.0);
        assert_eq!(a.head_width, None);
        assert_eq!(a.heads, Heads::End, "unknown spelling → the default variant");
        assert!(!a.outlined);
    }

    #[test]
    fn a_manifest_is_generated_from_the_declarations() {
        let json = manifest_json(1, "https://x.example/c", "clip", "0.1.0", &[Arrow::SPEC]);
        // numbers stay unquoted so a consumer's types match the declared kind
        assert!(json.contains(r#""thickness":{"type":"number","default":16.0,"min":0,"max":200}"#), "{json}");
        assert!(json.contains(r#""values":["end","start","both","none"]"#), "{json}");
        assert!(json.contains(r#""children":"container""#), "{json}");
        assert!(json.contains(r#""ns":"https://x.example/c""#), "{json}");
        assert!(json.contains(r#""supersededBy":["x2","y2"]"#), "{json}");
        assert!(json.contains(r#""geometry":true"#), "{json}");
        // absent rather than false, so a manifest costs nothing for the common case
        assert!(!json.contains(r#""geometry":false"#), "{json}");
    }
}
