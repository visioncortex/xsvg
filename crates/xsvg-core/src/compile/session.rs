//! One parse per flush (docs/Compile Session.md).
//!
//! An editor compiles a few fragments of one document per keystroke, and every
//! pure `compile_fragment_*_impl` call redoes the whole-document work first: the
//! nesting check, the XML parse, the theme and import pre-scans. [`CompileSession`]
//! does that work once, over text it borrows for the length of one flush, on a
//! [`Platform`] that outlives it. The pure entries are one-shot sessions. Nothing
//! here outlives its input and the core keeps no state between calls.

use super::plugin::{load_imports, ImportTable};
use super::{
    check_nesting_depth, serialize, top_level_at, Backend, Ctx, Host, TextLayout,
    Theme, MAX_NESTING_DEPTH,
};
use crate::QualityProfile;
use std::collections::HashMap;

#[cfg(test)]
thread_local! {
    /// How many documents `parse_document` has parsed on this thread — the
    /// one-parse property's witness (docs/Compile Session.md §6).
    pub(super) static PARSE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Every whole-document parse in the compiler goes through here, so a test can
/// count them.
pub(super) fn parse_document(input: &str) -> Result<roxmltree::Document<'_>, roxmltree::Error> {
    #[cfg(test)]
    PARSE_COUNT.with(|c| c.set(c.get() + 1));
    roxmltree::Document::parse(input)
}

/// The whole-document work, done once: the parse and the pre-scans every
/// compile over this text needs. Owned by a [`CompileSession`]; borrowed by
/// each per-call [`Ctx`]. The `&dyn` entry points use it directly with a
/// stack-local [`Backend`].
pub(super) struct Parsed<'a> {
    pub(super) doc: roxmltree::Document<'a>,
    pub(super) theme: Theme,
    pub(super) imports: ImportTable,
    /// `id` → node, first declaration wins (what a document-order scan finds).
    pub(super) ids: HashMap<String, roxmltree::NodeId>,
}

impl<'a> Parsed<'a> {
    /// Fails exactly where the pure entries fail: nesting depth, then the XML parse.
    pub(super) fn new(input: &'a str) -> Result<Self, String> {
        check_nesting_depth(input, MAX_NESTING_DEPTH)?;
        let doc = parse_document(input).map_err(|e| format!("xsvg parse error: {e}"))?;
        let theme = Theme::load(&doc);
        let imports = load_imports(&doc);
        let mut ids = HashMap::new();
        for n in doc.descendants() {
            if let Some(id) = n.attribute("id") {
                ids.entry(id.to_string()).or_insert_with(|| n.id());
            }
        }
        Ok(Self {
            doc,
            theme,
            imports,
            ids,
        })
    }

    /// The whole document: the header, the serialized root, and the unkeyed marker.
    pub(super) fn compile(
        &self,
        backend: &Backend,
        quality: QualityProfile,
        sourcemap: bool,
        base: &str,
    ) -> Result<String, String> {
        let mut out = String::new();
        out.push_str(&format!(
            "<!-- compiled by xsvg v0 (quality={}) -->\n",
            quality.as_str()
        ));
        let ctx = Ctx::new(backend, quality, sourcemap, base, self);
        serialize(self.doc.root_element(), &mut out, true, &ctx);
        // "Key every element" is a host contract this crate cannot enforce, so a
        // violation gets reported where the sloppy host will actually see it: in its own
        // output. Nothing for correct hosts (and the byte-span default is not a keyed
        // compile at all); trailing, so every fragment is still a verbatim slice.
        let missed = ctx.key_misses.borrow().len();
        if missed > 0 {
            out.push_str(&format!(
                "\n<!-- xsvg: {missed} element(s) unkeyed; identity fell back to byte spans (docs/Incremental.md §3.1) -->"
            ));
        }
        Ok(out)
    }

    /// The top-level element containing byte `offset`.
    pub(super) fn compile_fragment(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
        sourcemap: bool,
        base: &str,
    ) -> Result<String, String> {
        let node = top_level_at(&self.doc, offset)
            .ok_or_else(|| format!("no top-level element at byte offset {offset}"))?;
        let mut out = String::new();
        serialize(node, &mut out, false, &Ctx::new(backend, quality, sourcemap, base, self));
        Ok(out)
    }

    /// Caret geometry for the `<x:textbox>` starting at byte `offset`. Layout does
    /// not read the source map or the base, so neither is a parameter.
    pub(super) fn text_layout(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<TextLayout> {
        let node = self
            .doc
            .descendants()
            .find(|n| n.is_element() && n.range().start == offset)?;
        if node.tag_name().namespace() != Some(super::XSVG_NS) || node.tag_name().name() != "textbox" {
            return None;
        }
        super::layout_textbox(node, &Ctx::new(backend, quality, false, "", self))
    }

    /// Source byte range of every element in document order, the root included.
    pub(super) fn node_spans(&self) -> Vec<(usize, usize)> {
        self.doc
            .root_element()
            .descendants()
            .filter(|n| n.is_element())
            .map(|n| {
                let r = n.range();
                (r.start, r.end)
            })
            .collect()
    }

    pub(super) fn fragment_range(&self, offset: usize) -> Option<(usize, usize)> {
        top_level_at(&self.doc, offset).map(|n| {
            let r = n.range();
            (r.start, r.end)
        })
    }

    /// Byte ranges of every *other* top-level element that must re-emit when the
    /// fragments at `seeds` change: the transitive closure over compile-time-baked
    /// references (`in="#id"`, connector endpoints, boolean operands, mesh fills),
    /// plus everything in a namespace whose `<x:import>` is among the seeds. Live
    /// references (`href`, `url(#…)` paints) re-resolve in the DOM and are not
    /// reported. Seeds are excluded; results are in document order.
    pub(super) fn dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)> {
        let doc = &self.doc;
        let targets: Vec<roxmltree::Node> = {
            let mut v: Vec<roxmltree::Node> = Vec::new();
            for &off in seeds {
                if let Some(t) = top_level_at(doc, off) {
                    if !v.iter().any(|u| u.range() == t.range()) {
                        v.push(t);
                    }
                }
            }
            v
        };
        if targets.is_empty() {
            return Vec::new();
        }
        // ids whose (compiled) geometry has changed; grows as dependents join
        let mut live: Vec<&str> = targets
            .iter()
            .flat_map(|t| t.descendants().filter_map(|n| n.attribute("id")))
            .collect();
        // Editing an <x:import> rebinds a whole namespace — every element in it lowers
        // through a different module, so all of them are dependents (§Extensibility §3).
        // This edge is by namespace rather than by id, so it can't live in `baked_refs`.
        let dirty_ns: Vec<&str> = targets
            .iter()
            .flat_map(|t| t.descendants())
            .filter(|n| {
                n.tag_name().namespace() == Some(super::XSVG_NS) && n.tag_name().name() == "import"
            })
            .filter_map(|n| n.attribute("ns"))
            .collect();
        if live.is_empty() && dirty_ns.is_empty() {
            return Vec::new();
        }
        let tops: Vec<roxmltree::Node> = doc
            .root_element()
            .children()
            .filter(|c| c.is_element() && !targets.iter().any(|t| t.range() == c.range()))
            .collect();
        let mut included = vec![false; tops.len()];
        loop {
            let mut changed = false;
            for (i, tl) in tops.iter().enumerate() {
                if included[i] {
                    continue;
                }
                let refs_live = tl.descendants().any(|n| {
                    super::baked_refs(n, &self.ids)
                        .iter()
                        .any(|r| live.contains(&r.strip_prefix('#').unwrap_or(r)))
                        || n.tag_name()
                            .namespace()
                            .is_some_and(|ns| dirty_ns.contains(&ns))
                });
                if refs_live {
                    included[i] = true;
                    changed = true;
                    // its output changed, so ids it defines go live too
                    for id in tl.descendants().filter_map(|n| n.attribute("id")) {
                        if !live.contains(&id) {
                            live.push(id);
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut out: Vec<(usize, usize)> = tops
            .iter()
            .zip(&included)
            .filter(|(_, inc)| **inc)
            .map(|(tl, _)| {
                let r = tl.range();
                (r.start, r.end)
            })
            .collect();
        out.sort_unstable();
        out
    }
}

/// One document, parsed and pre-scanned once, ready to compile any number of its
/// fragments on a [`Platform`] (docs/Compile Session.md).
///
/// Created by [`Platform::session`] at the top of a render flush over the text the
/// embedder just serialized, used for every compile that flush needs, and dropped
/// with it: the session borrows the input, so a stale parse cannot be held. It
/// retains exactly what the pure entries recompute per call — the parse, the
/// `<x:theme>` tokens, the `<x:import>` bindings, the id table and the quality
/// profile — and nothing per compile: each call builds a fresh [`Ctx`] with its own
/// memo, cycle guard and fuel, so degradation behaviour is unchanged.
///
/// `P` is the platform's concrete type, so a session is `Send` exactly when the
/// platform is `Sync` — true of any platform over thread-safe adapters, with
/// nothing asserted — and a document may then be parsed off the thread that
/// compiles it. A session is not `Sync`, and need not be.
///
/// Every byte a session emits is a byte the pure entry emits for the same input:
/// `compile_fragment(offset)` equals `compile_fragment_platform_impl(input, …,
/// offset, base, platform)`, and likewise for the other methods.
pub struct CompileSession<'a, P: Host> {
    parsed: Parsed<'a>,
    platform: &'a P,
    quality: QualityProfile,
    sourcemap: bool,
    base: String,
}

impl<'a, P: Host> CompileSession<'a, P> {
    /// Parse once, pre-scan once. Fails exactly where the pure entries fail
    /// (nesting depth, XML parse error), with the same message. Usually reached
    /// as [`Platform::session`].
    pub fn new(
        platform: &'a P,
        input: &'a str,
        quality: &str,
        sourcemap: bool,
        base: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            parsed: Parsed::new(input)?,
            platform,
            quality: QualityProfile::parse(quality),
            sourcemap,
            base: base.to_string(),
        })
    }

    /// The whole document — byte-identical to [`super::compile_platform_impl`].
    pub fn compile(&self) -> Result<String, String> {
        self.parsed
            .compile(&self.platform.backend(), self.quality, self.sourcemap, &self.base)
    }

    /// The top-level element containing byte `offset` — byte-identical to
    /// [`super::compile_fragment_platform_impl`], and so to that element's span in
    /// [`Self::compile`] (the fragment invariant, docs/Incremental.md).
    pub fn compile_fragment(&self, offset: usize) -> Result<String, String> {
        self.parsed.compile_fragment(
            offset,
            &self.platform.backend(),
            self.quality,
            self.sourcemap,
            &self.base,
        )
    }

    /// Baked-reference dependents of the fragments at `seeds` — the same answer as
    /// [`super::dependents_multi_impl`], from the retained parse.
    pub fn dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)> {
        self.parsed.dependents(seeds)
    }

    /// Caret geometry for the `<x:textbox>` starting at byte `offset` — the same
    /// answer as [`super::text_layout_impl`]. `None` when the offset is not a
    /// textbox or the box resolves to nothing renderable.
    pub fn text_layout(&self, offset: usize) -> Option<TextLayout> {
        self.parsed
            .text_layout(offset, &self.platform.backend(), self.quality)
    }

    /// Source byte range of every element, in document order — the same answer as
    /// [`super::node_spans_impl`].
    pub fn node_spans(&self) -> Vec<(usize, usize)> {
        self.parsed.node_spans()
    }

    /// Source byte range of the fragment unit containing `offset` — the same answer
    /// as [`super::fragment_range_impl`].
    pub fn fragment_range(&self, offset: usize) -> Option<(usize, usize)> {
        self.parsed.fragment_range(offset)
    }
}
