//! SDK for writing xsvg geometry plugins (docs/Extensibility.md).
//!
//! A plugin is a WASM module exporting five functions. This crate implements the
//! whole ABI — memory handoff, JSON decode, length-prefixed replies — so a plugin
//! is just a [`Plugin`] impl plus [`export_plugin!`]:
//!
//! ```ignore
//! #[derive(Params)]
//! #[element(name = "arrow", doc = "a thick arrow")]
//! struct Arrow {
//!     #[param(default = 16.0, min = 0.0)] thickness: f64,
//!     heads: Heads,                                   // enum → its own spellings
//! }
//!
//! struct MyClipart;
//! impl xsvg_plugin::Plugin for MyClipart {
//!     const NS: &'static str = "https://example.com/clipart";
//!     const NAME: &'static str = "clipart";
//!     const VERSION: &'static str = "0.1.0";
//!     fn elements() -> &'static [ElementSpec] { &[Arrow::SPEC] }
//!     fn generate(tag: &str, req: &Request) -> Result<String, String> { … }
//! }
//! xsvg_plugin::export_plugin!(MyClipart);
//! ```
//!
//! The manifest is **generated from those declarations**, so a module's published
//! description cannot disagree with the code that reads its parameters — the two
//! are the same struct.
//!
//! The module gets no imports, no WASI, and no host callbacks: a generator is a
//! pure function from request to SVG fragment. Whatever it returns is re-parsed
//! and re-emitted by the compiler, so it cannot escape the static SVG subset —
//! and it need not be defensive about that either.
//!
//! ## ABI (version 1)
//!
//! | Export | Meaning |
//! |---|---|
//! | `xsvg_abi_version() -> i32` | `1` |
//! | `xsvg_alloc(len: i32) -> i32` | allocate `len` bytes, return the pointer |
//! | `xsvg_free(ptr: i32, len: i32)` | release a buffer the host owns |
//! | `xsvg_describe() -> i32` | pointer to the length-prefixed manifest |
//! | `xsvg_generate(ptr, len) -> i32` | pointer to the length-prefixed reply |
//!
//! Both returned buffers are `u32` little-endian length followed by UTF-8 bytes;
//! the host reads them and then calls `xsvg_free(ptr, 4 + len)`. `xsvg_generate`
//! takes ownership of its input buffer, so the host must not free that.

pub mod json;

pub use json::Json;
// Declare parameters once; the description and the parsing are both derived.
pub use xsvg_schema::{
    AttrSource, ChildMode, ElementSpec, InputMode, ParamEnum, ParamKind, ParamSpec, Params,
};

/// The ABI version this SDK implements.
pub const ABI_VERSION: i32 = 1;

/// One child of a container element, lowered by the compiler before the call.
pub struct Child {
    /// Compiled SVG. Splice it verbatim to keep the child's source stamps, and
    /// with them the viewer's click-to-source mapping.
    pub svg: String,
    /// `[x, y, width, height]` of what the child actually paints, or `None` when
    /// the compiler found nothing measurable.
    pub bbox: Option<[f64; 4]>,
}

/// Everything a generator is given.
pub struct Request {
    /// The element's attributes, minus the ones the compiler reserves
    /// (`id`, `transform`, `in`), with theme `var()` tokens already resolved.
    pub attrs: Vec<(String, String)>,
    /// Geometry resolved from `in="#id"`, as an SVG path `d`.
    pub input_d: Option<String>,
    /// Compiled children, in document order. Empty for a leaf element.
    pub children: Vec<Child>,
    /// The active quality profile's flattening tolerance, for anything this
    /// generator approximates with line or curve segments.
    pub tolerance: f64,
}

/// Lets the derived parsing read straight from a request.
impl xsvg_schema::AttrSource for Request {
    fn attr(&self, name: &str) -> Option<&str> {
        Request::attr(self, name)
    }
}

impl Request {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// A numeric attribute, falling back to `default` when absent or unparseable.
    /// Non-finite input takes the default too — totality is the compiler's
    /// contract and a generator must not be the thing that breaks it.
    pub fn num(&self, name: &str, default: f64) -> f64 {
        match self.attr(name).and_then(|v| v.trim().parse::<f64>().ok()) {
            Some(n) if n.is_finite() => n,
            _ => default,
        }
    }

    /// The union of every child's box, or `None` when no child had one.
    pub fn children_bbox(&self) -> Option<[f64; 4]> {
        self.children.iter().filter_map(|c| c.bbox).reduce(|a, b| {
            let x0 = a[0].min(b[0]);
            let y0 = a[1].min(b[1]);
            let x1 = (a[0] + a[2]).max(b[0] + b[2]);
            let y1 = (a[1] + a[3]).max(b[1] + b[3]);
            [x0, y0, x1 - x0, y1 - y0]
        })
    }

    fn decode(v: &Json) -> Option<Request> {
        let attrs = v
            .get("attrs")
            .and_then(Json::as_arr)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|p| {
                        let pair = p.as_arr()?;
                        Some((
                            pair.first()?.as_str()?.to_string(),
                            pair.get(1)?.as_str()?.to_string(),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let children = v
            .get("children")
            .and_then(Json::as_arr)
            .map(|items| {
                items
                    .iter()
                    .map(|c| Child {
                        svg: c
                            .get("svg")
                            .and_then(Json::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        bbox: c.get("bbox").and_then(Json::as_arr).and_then(|b| {
                            let mut out = [0.0; 4];
                            if b.len() != 4 {
                                return None;
                            }
                            for (slot, v) in out.iter_mut().zip(b) {
                                *slot = v.as_num()?;
                            }
                            Some(out)
                        }),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Request {
            attrs,
            input_d: v
                .get("input_d")
                .and_then(Json::as_str)
                .map(ToString::to_string),
            children,
            tolerance: v.get("tolerance").and_then(Json::as_num).unwrap_or(0.1),
        })
    }
}

/// What a plugin must provide.
pub trait Plugin {
    /// The namespace this module claims. A host checks it against the
    /// `<x:import>` that bound the module, so one cannot answer for another's
    /// elements.
    const NS: &'static str;
    const NAME: &'static str;
    const VERSION: &'static str;

    /// The elements this module provides, as derived descriptions. The manifest
    /// (docs/Extensibility.md §4) is generated from these.
    fn elements() -> &'static [ElementSpec];

    /// Lower one element. `Err` degrades that element with the given reason
    /// rather than failing the compile.
    fn generate(tag: &str, req: &Request) -> Result<String, String>;
}

/// The manifest JSON for a plugin, built from its declarations.
pub fn manifest<P: Plugin>() -> String {
    xsvg_schema::manifest_json(ABI_VERSION, P::NS, P::NAME, P::VERSION, P::elements())
}

/// Format a number for SVG output: short, locale-free, and never `NaN`/`inf`.
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    // three decimals is finer than any sane tolerance and keeps output compact
    let r = (v * 1000.0).round() / 1000.0;
    let mut s = format!("{r}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

/// Escape a string for use in an XML attribute value.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

// ---- ABI plumbing --------------------------------------------------------

/// Allocate `len` bytes for the host to write into.
///
/// # Safety
/// The returned pointer is owned by the caller until handed to `xsvg_generate`
/// or `xsvg_free`.
pub fn alloc_bytes(len: usize) -> *mut u8 {
    let mut v = vec![0u8; len];
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// Release a buffer previously handed to the host.
///
/// # Safety
/// `ptr`/`len` must come from [`alloc_bytes`] or a length-prefixed reply.
pub unsafe fn free_bytes(ptr: *mut u8, len: usize) {
    if !ptr.is_null() && len > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, len, len) });
    }
}

/// Pack a string as a `u32`-LE length followed by its UTF-8 bytes, and hand
/// ownership to the host.
pub fn reply(s: &str) -> *mut u8 {
    let bytes = s.as_bytes();
    let mut v = Vec::with_capacity(4 + bytes.len());
    v.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    v.extend_from_slice(bytes);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

/// Decode a request buffer (taking ownership) and run `P`, returning the reply
/// JSON. Exposed so a host-side test can exercise a plugin without WASM.
///
/// # Safety
/// `ptr`/`len` must describe a buffer from [`alloc_bytes`].
pub unsafe fn dispatch<P: Plugin>(ptr: *mut u8, len: usize) -> String {
    let input = unsafe { Vec::from_raw_parts(ptr, len, len) };
    let text = match core::str::from_utf8(&input) {
        Ok(t) => t,
        Err(_) => return error_json("request is not valid UTF-8"),
    };
    let Some(v) = json::parse(text) else {
        return error_json("request is not valid JSON");
    };
    let Some(tag) = v.get("tag").and_then(Json::as_str) else {
        return error_json("request names no element");
    };
    let Some(req) = Request::decode(&v) else {
        return error_json("request is malformed");
    };
    match P::generate(tag, &req) {
        Ok(svg) => {
            let mut out = String::from("{\"svg\":");
            json::push_str(&mut out, &svg);
            out.push('}');
            out
        }
        Err(why) => error_json(&why),
    }
}

fn error_json(why: &str) -> String {
    let mut out = String::from("{\"error\":");
    json::push_str(&mut out, why);
    out.push('}');
    out
}

/// Emit the five ABI exports for a [`Plugin`] impl.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ty) => {
        #[no_mangle]
        pub extern "C" fn xsvg_abi_version() -> i32 {
            $crate::ABI_VERSION
        }

        #[no_mangle]
        pub extern "C" fn xsvg_alloc(len: i32) -> *mut u8 {
            $crate::alloc_bytes(len.max(0) as usize)
        }

        /// # Safety
        /// `ptr`/`len` must come from `xsvg_alloc` or a reply buffer.
        #[no_mangle]
        pub unsafe extern "C" fn xsvg_free(ptr: *mut u8, len: i32) {
            unsafe { $crate::free_bytes(ptr, len.max(0) as usize) }
        }

        #[no_mangle]
        pub extern "C" fn xsvg_describe() -> *mut u8 {
            $crate::reply(&$crate::manifest::<$ty>())
        }

        /// # Safety
        /// `ptr`/`len` must come from `xsvg_alloc`; ownership passes to the callee.
        #[no_mangle]
        pub unsafe extern "C" fn xsvg_generate(ptr: *mut u8, len: i32) -> *mut u8 {
            let out = unsafe { $crate::dispatch::<$ty>(ptr, len.max(0) as usize) };
            $crate::reply(&out)
        }
    };
}

/// Encoding the *host* side of the ABI needs.
///
/// It lives here, next to the decoder it must agree with, so the two cannot drift
/// apart: a host that encodes with [`encode_request`] and a plugin that decodes
/// with this crate are compatible by construction.
pub mod host {
    use crate::json;

    /// One child of a container element: its compiled SVG and measured box.
    pub type ChildIn<'a> = (&'a str, Option<[f64; 4]>);

    /// Build the JSON for one `xsvg_generate` call.
    pub fn encode_request(
        tag: &str,
        attrs: &[(String, String)],
        input_d: Option<&str>,
        children: &[ChildIn],
        tolerance: f64,
    ) -> String {
        let mut out = String::from("{\"tag\":");
        json::push_str(&mut out, tag);
        out.push_str(",\"attrs\":[");
        for (i, (k, v)) in attrs.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('[');
            json::push_str(&mut out, k);
            out.push(',');
            json::push_str(&mut out, v);
            out.push(']');
        }
        out.push_str("],\"input_d\":");
        match input_d {
            Some(d) => json::push_str(&mut out, d),
            None => out.push_str("null"),
        }
        out.push_str(",\"children\":[");
        for (i, (svg, bbox)) in children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("{\"svg\":");
            json::push_str(&mut out, svg);
            out.push_str(",\"bbox\":");
            match bbox {
                Some(b) => {
                    out.push('[');
                    for (j, v) in b.iter().enumerate() {
                        if j > 0 {
                            out.push(',');
                        }
                        out.push_str(&num_json(*v));
                    }
                    out.push(']');
                }
                None => out.push_str("null"),
            }
            out.push('}');
        }
        out.push_str("],\"tolerance\":");
        out.push_str(&num_json(tolerance));
        out.push('}');
        out
    }

    /// JSON has no way to spell a non-finite number, and a plugin should never be
    /// handed one anyway — they collapse to zero, as they do everywhere else in
    /// the compiler.
    fn num_json(v: f64) -> String {
        if v.is_finite() {
            format!("{v}")
        } else {
            "0".to_string()
        }
    }

    /// Read a `{"svg":…}` / `{"error":…}` reply.
    pub fn decode_reply(src: &str) -> Result<String, String> {
        let Some(v) = json::parse(src) else {
            return Err("generator reply is not valid JSON".to_string());
        };
        if let Some(svg) = v.get("svg").and_then(crate::Json::as_str) {
            return Ok(svg.to_string());
        }
        Err(v
            .get("error")
            .and_then(crate::Json::as_str)
            .unwrap_or("generator reply has neither svg nor error")
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two halves of the ABI must agree; this is the round trip that says so.
    #[test]
    fn a_host_encoded_request_decodes_to_what_was_sent() {
        let attrs = vec![
            ("length".to_string(), "10".to_string()),
            ("fill".to_string(), "a\"b\\c".to_string()),
        ];
        let kids: Vec<host::ChildIn> = vec![
            ("<path d=\"M0 0\"/>", Some([1.0, 2.0, 3.0, 4.5])),
            ("<g/>", None),
        ];
        let src = host::encode_request("arrow", &attrs, Some("M0 0 L1 1"), &kids, 0.25);
        let v = json::parse(&src).expect("encodes valid JSON");
        assert_eq!(v.get("tag").and_then(Json::as_str), Some("arrow"));
        let req = Request::decode(&v).expect("decodes");
        assert_eq!(req.attr("fill"), Some("a\"b\\c"));
        assert_eq!(req.num("length", 0.0), 10.0);
        assert_eq!(req.input_d.as_deref(), Some("M0 0 L1 1"));
        assert_eq!(req.tolerance, 0.25);
        assert_eq!(req.children.len(), 2);
        assert_eq!(req.children[0].bbox, Some([1.0, 2.0, 3.0, 4.5]));
        assert_eq!(req.children[1].bbox, None);
        assert_eq!(req.children_bbox(), Some([1.0, 2.0, 3.0, 4.5]));
    }

    #[test]
    fn non_finite_numbers_never_reach_a_plugin() {
        let kids: Vec<host::ChildIn> = vec![("<g/>", Some([f64::NAN, 0.0, f64::INFINITY, 1.0]))];
        let src = host::encode_request("x", &[], None, &kids, f64::NAN);
        assert!(json::parse(&src).is_some(), "still valid JSON: {src}");
        assert!(!src.contains("NaN") && !src.contains("inf"), "{src}");
    }

    #[test]
    fn replies_carry_either_markup_or_a_reason() {
        assert_eq!(
            host::decode_reply(r#"{"svg":"<path/>"}"#).as_deref(),
            Ok("<path/>")
        );
        assert_eq!(
            host::decode_reply(r#"{"error":"nope"}"#).unwrap_err(),
            "nope"
        );
        assert!(host::decode_reply("not json").is_err());
        assert!(host::decode_reply("{}").is_err());
    }
}
