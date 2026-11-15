//! One parse per flush (docs/Compile Session.md).
//!
//! An editor compiles a few fragments of one document per keystroke, and every
//! pure `compile_fragment_*_impl` call redoes the whole-document work first: the
//! nesting check, the XML parse, the theme and import pre-scans. [`CompileSession`]
//! does that work once, over text it borrows for the length of one flush, on a
//! [`Platform`](super::Platform) that outlives it. The pure entries are one-shot sessions. Nothing
//! here outlives its input and the core keeps no state between calls.

use super::plugin::{load_imports, ImportTable};
use super::{
    check_nesting_depth, serialize, top_level_at, Backend, Ctx, Host, TextLayout, Theme,
    MAX_NESTING_DEPTH,
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

/// Where the caret-stop scratch compile of `node` starts: the outermost ancestor
/// whose lowering can change or claim its geometry — any element outside plain
/// SVG (an `<x:warp>` maps the stops through its field, an `<x:boolean>` or a
/// generator harvests them away, a foreign element drops them) or a `<use>` —
/// else `node` itself. Above that, a plain SVG ancestor is passthrough: it
/// neither moves the paths the element emits nor records anything, so the
/// stops are exactly the ones its whole fragment would record. `None` under
/// `x:hidden`, which compiles to nothing.
fn stops_scope<'a, 'i>(node: roxmltree::Node<'a, 'i>) -> Option<roxmltree::Node<'a, 'i>> {
    let mut scope = node;
    for a in node.ancestors().filter(|a| a.is_element()) {
        if a.parent_element().is_none() {
            break; // the root element is never part of a fragment
        }
        if super::is_hidden(a) {
            return None;
        }
        if !super::is_passthrough_container(a) {
            scope = a;
        }
    }
    Some(scope)
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
        // an `<x:doc>` island's ids do not exist to the compiler (§4.4): they can be
        // neither referenced nor shadow a real element's
        for n in super::visible(doc.root()) {
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
        serialize(
            node,
            &mut out,
            false,
            &Ctx::new(backend, quality, sourcemap, base, self),
        );
        Ok(out)
    }

    /// The deepest independently compilable unit at `offset`: descend through
    /// plain containers, stopping at lowering/filter boundaries or an own tag.
    pub(super) fn unit_at<'s>(&'s self, offset: usize) -> Option<roxmltree::Node<'s, 's>> {
        let mut unit = top_level_at(&self.doc, offset)?;
        while super::is_unit_container(unit) {
            let Some(child) = unit.children().filter(|c| c.is_element()).find(|c| {
                let r = c.range();
                r.start <= offset && offset < r.end
            }) else { break };
            unit = child;
        }
        Some(unit)
    }

    pub(super) fn unit_range(&self, offset: usize) -> Option<(usize, usize)> {
        self.unit_at(offset).map(|n| {
            let r = n.range();
            (r.start, r.end)
        })
    }

    /// Compile one unit on its own: byte-identical to its span in its fragment's
    /// output (the §9 invariant), as a fragment is to its span in the full one.
    pub(super) fn compile_unit(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
        sourcemap: bool,
        base: &str,
    ) -> Result<String, String> {
        let unit = self
            .unit_at(offset)
            .ok_or_else(|| format!("no top-level element at byte offset {offset}"))?;
        let mut out = String::new();
        serialize(
            unit,
            &mut out,
            false,
            &Ctx::new(backend, quality, sourcemap, base, self),
        );
        Ok(out)
    }

    /// The fragment at `offset` plus where each of its units landed in it, in
    /// output order (restacked when the container restacks). Empty for a
    /// top-level element that is not a passthrough container: it is its own unit.
    pub(super) fn compile_fragment_spans(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
        sourcemap: bool,
        base: &str,
    ) -> Result<(String, Vec<super::UnitSpan>), String> {
        let node = top_level_at(&self.doc, offset)
            .ok_or_else(|| format!("no top-level element at byte offset {offset}"))?;
        let ctx = Ctx::new(backend, quality, sourcemap, base, self);
        *ctx.unit_spans.borrow_mut() = Some(Vec::new());
        ctx.unit_container.set(Some(node.id()));
        let mut out = String::new();
        serialize(node, &mut out, false, &ctx);
        let spans = ctx.unit_spans.take().unwrap_or_default();
        Ok((out, spans))
    }

    /// Caret geometry for the `<x:textbox>` starting at byte `offset`. Layout does
    /// not read the source map or the base, so neither is a parameter.
    pub(super) fn text_layout(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<TextLayout> {
        let node = super::visible(self.doc.root())
            .find(|n| n.is_element() && n.range().start == offset)?;
        if node.tag_name().namespace() != Some(super::XSVG_NS)
            || node.tag_name().name() != "textbox"
        {
            return None;
        }
        super::layout_textbox(node, &Ctx::new(backend, quality, false, "", self))
    }

    /// The size the `<x:textbox>` starting at byte `offset` needs for its content
    /// (`measure_textbox`). `None` when the offset is not a rectangular textbox.
    pub(super) fn measure_textbox(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<super::TextboxSize> {
        let node = super::visible(self.doc.root())
            .find(|n| n.is_element() && n.range().start == offset)?;
        if node.tag_name().namespace() != Some(super::XSVG_NS)
            || node.tag_name().name() != "textbox"
        {
            return None;
        }
        super::measure_textbox(node, &Ctx::new(backend, quality, false, "", self))
    }

    /// Caret stops of the outlined text element starting at byte `offset`
    /// (`stops.rs`). The element compiles once more into a scratch buffer with
    /// the recorder on, so the stops come out of the same outline, placement and
    /// fields the rendered paths did. `None` when nothing of that element was
    /// outlined — it renders live text, which the host measures itself.
    ///
    /// The scratch compile starts at the element's [`stops_scope`], not its
    /// top-level fragment: an editor asks on every keystroke, and the fragment
    /// is often a whole artboard whose other content (an offset, a warp, a
    /// table) has nothing to say about where this element's glyphs sit.
    pub(super) fn caret_stops(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<super::CaretStops> {
        let node = super::visible(self.doc.root())
            .find(|n| n.is_element() && n.range().start == offset)?;
        top_level_at(&self.doc, offset)?;
        self.caret_stops_from(node, stops_scope(node)?, backend, quality)
    }

    /// The stops `node` records while `scope` (itself or an ancestor) compiles.
    pub(super) fn caret_stops_from(
        &self,
        node: roxmltree::Node,
        scope: roxmltree::Node,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<super::CaretStops> {
        let ctx = Ctx::new(backend, quality, false, "", self);
        *ctx.stops.borrow_mut() = Some(super::stops::StopRecorder::default());
        let mut scratch = String::new();
        serialize(scope, &mut scratch, false, &ctx);
        let span = super::stops::span_of(node);
        let runs: Vec<super::StopRun> = ctx
            .stops
            .take()?
            .runs
            .into_iter()
            .filter(|(s, _)| *s == span)
            .map(|(_, run)| run)
            .collect();
        (!runs.is_empty()).then_some(super::CaretStops { runs })
    }

    /// The element starting at byte `offset`, outside any `<x:doc>` island.
    fn element_at<'s>(&'s self, offset: usize) -> Option<roxmltree::Node<'s, 's>> {
        super::visible(self.doc.root()).find(|n| n.is_element() && n.range().start == offset)
    }

    /// Document queries (query.rs): the element at `offset`, asked of this parse.
    pub(super) fn resolved(
        &self,
        offset: usize,
        properties: &[&str],
    ) -> Option<Vec<super::Resolved>> {
        let node = self.element_at(offset)?;
        Some(
            properties
                .iter()
                .map(|p| super::query::resolved(node, p, &self.theme))
                .collect(),
        )
    }

    pub(super) fn text_pieces(&self, offset: usize) -> Option<Vec<super::TextPiece>> {
        super::query::text_pieces(self.element_at(offset)?, &self.theme)
    }

    pub(super) fn blocks(&self, offset: usize) -> Option<Vec<super::Block>> {
        super::query::blocks(self.element_at(offset)?)
    }

    pub(super) fn run_support(&self, offset: usize) -> Option<super::RunSupport> {
        super::run_support(self.element_at(offset)?)
    }

    /// Each corner the element at `offset` draws (§5.4): its anchor, the radius
    /// asked and drawn, where its curve leaves and joins the segments, its
    /// centre. Empty for an element with corners asked but none drawn; `None`
    /// for one asking none or without geometry.
    pub(super) fn corners(&self, offset: usize) -> Option<Vec<super::CornerInfo>> {
        let node = self.element_at(offset)?;
        let spec = super::corners::corner_spec(node, &self.theme)?;
        let base = if node.tag_name().name() == "polyline" {
            node.attribute("points")
                .and_then(super::points_to_path_d)
                .map(|d| d.trim_end_matches('Z').trim_end().to_string())?
        } else {
            super::shape_base_d(node)?
        };
        super::corners::apply_corners(&base, &spec).map(|(_, infos)| infos)
    }

    /// The operands of the `<x:boolean>` at `offset` for the curve-keeping
    /// algebra (docs/Incremental.md §11): lowered as convert-to-path lowers them
    /// (text outlined), arcs as cubics, with the op and the flattened result.
    pub(super) fn boolean_operands(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
    ) -> Option<super::BooleanOperands> {
        let node = self.element_at(offset)?;
        if node.tag_name().namespace() != Some(super::XSVG_NS)
            || node.tag_name().name() != "boolean"
        {
            return None;
        }
        let ctx = Ctx::new(backend, quality, false, "", self);
        ctx.force_outline.set(true);
        ctx.to_path.set(true);
        super::boolean_operands_at(node, &ctx)
    }

    /// Convert to path (trace.rs; docs/Incremental.md §11): the element at
    /// `offset` compiled on its own with outlining forced — its glyphs as paths,
    /// its backdrops as rects, a shape as its `<path>` — every attribute the
    /// conversion does not spend kept as authored, paints with their tokens, no
    /// source map. `Err` names what could not be traced.
    pub(super) fn to_path(
        &self,
        offset: usize,
        backend: &Backend,
        quality: QualityProfile,
        bake_corners: bool,
    ) -> Result<String, super::ToPathError> {
        let node = self
            .element_at(offset)
            .ok_or_else(|| super::ToPathError::limit("no element starts at this offset"))?;
        let (ns, name) = (node.tag_name().namespace(), node.tag_name().name());
        let text = (ns == Some(super::XSVG_NS)
            && matches!(name, "textbox" | "textpath" | "list" | "table"))
            || (ns == Some(super::SVG_NS) && matches!(name, "text" | "textArea"));
        let shape = ns == Some(super::SVG_NS)
            && matches!(
                name,
                "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "path"
            );
        let boolean = ns == Some(super::XSVG_NS) && name == "boolean";
        // a list inside a text box is one of the box's paragraphs (§6.16):
        // the box lays it out, so the box is what converts
        if ns == Some(super::XSVG_NS)
            && name == "list"
            && node.parent().is_some_and(|p| {
                p.tag_name().namespace() == Some(super::XSVG_NS) && p.tag_name().name() == "textbox"
            })
        {
            return Err(super::ToPathError::limit(
                "an <x:list> inside an <x:textbox> converts with its box",
            ));
        }
        if !text && !shape && !boolean {
            return Err(super::ToPathError::limit(format!(
                "<{name}> is not convertible"
            )));
        }
        let ctx = Ctx::new(backend, quality, false, "", self);
        ctx.force_outline.set(true);
        ctx.to_path.set(true);
        ctx.live_corners.set(!bake_corners);
        *ctx.to_path_carry.borrow_mut() = Some((offset, super::carried_attrs(node, false)));
        let mut out = String::new();
        if text {
            serialize(node, &mut out, false, &ctx);
        } else if boolean {
            if !super::emit_boolean_as_path(node, &mut out, &ctx) {
                return Err(super::ToPathError::limit(
                    "<x:boolean> has no geometry to combine",
                ));
            }
        } else if !super::emit_shape_as_path(node, &mut out, &ctx) {
            return Err(super::ToPathError::limit(format!(
                "<{name}> has no geometry"
            )));
        }
        if let Some(miss) = ctx.outline_miss.take() {
            return Err(miss);
        }
        if text && !out.contains("<path") {
            return Err(super::ToPathError::limit("nothing to trace"));
        }
        Ok(out)
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
        let tops: Vec<roxmltree::Node> = doc
            .root_element()
            .children()
            .filter(|c| c.is_element())
            .collect();
        self.dependents_among(seeds, |off| top_level_at(doc, off), tops)
    }

    /// [`Parsed::dependents`] through nested containers (docs/Incremental.md §9): seeds and
    /// results are **units**, so a sibling unit inside the same container that
    /// bakes a seed's geometry is reported — `<x:offset in="#w">` beside the
    /// `<x:warp id="w">` it traces, which the top-level scan never sees.
    pub(super) fn unit_dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)> {
        fn collect<'a>(node: roxmltree::Node<'a, 'a>, out: &mut Vec<roxmltree::Node<'a, 'a>>) {
            let children: Vec<_> = node.children().filter(|c| c.is_element()).collect();
            if super::is_unit_container(node) && !children.is_empty() {
                for child in children { collect(child, out); }
            } else {
                out.push(node);
            }
        }
        let mut units = Vec::new();
        for top in self.doc.root_element().children().filter(|c| c.is_element()) {
            collect(top, &mut units);
        }
        self.dependents_among(seeds, |off| self.unit_at(off), units)
    }

    /// The transitive closure over compile-time-baked references, among
    /// `candidates`: the seeds' own candidates (`unit_of`) are excluded, ids they
    /// define go live, and any candidate baking a live id — or lowering through a
    /// namespace whose `<x:import>` is a seed — joins and makes its own ids live.
    fn dependents_among<'s>(
        &'s self,
        seeds: &[usize],
        unit_of: impl Fn(usize) -> Option<roxmltree::Node<'s, 's>>,
        candidates: Vec<roxmltree::Node<'s, 's>>,
    ) -> Vec<(usize, usize)> {
        let targets: Vec<roxmltree::Node> = {
            let mut v: Vec<roxmltree::Node> = Vec::new();
            for &off in seeds {
                if let Some(t) = unit_of(off) {
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
            .flat_map(|t| super::visible(*t).chain(t.ancestors().skip(1)).filter_map(|n| n.attribute("id")))
            .collect();
        // Editing an <x:import> rebinds a whole namespace — every element in it lowers
        // through a different module, so all of them are dependents (§Extensibility §3).
        // This edge is by namespace rather than by id, so it can't live in `baked_refs`.
        let dirty_ns: Vec<&str> = targets
            .iter()
            .flat_map(|t| super::visible(*t))
            .filter(|n| {
                n.tag_name().namespace() == Some(super::XSVG_NS) && n.tag_name().name() == "import"
            })
            .filter_map(|n| n.attribute("ns"))
            .collect();
        if live.is_empty() && dirty_ns.is_empty() {
            return Vec::new();
        }
        let tops: Vec<roxmltree::Node> = candidates
            .into_iter()
            .filter(|c| !targets.iter().any(|t| t.range() == c.range()))
            .collect();
        let mut included = vec![false; tops.len()];
        loop {
            let mut changed = false;
            for (i, tl) in tops.iter().enumerate() {
                if included[i] {
                    continue;
                }
                let refs_live = super::visible(*tl).any(|n| {
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
                    for id in super::visible(*tl).chain(tl.ancestors().skip(1)).filter_map(|n| n.attribute("id")) {
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

/// A document parsed and pre-scanned once — the XML tree, the `<x:theme>`
/// tokens, the `<x:import>` bindings, the id table — **borrowing** its text.
/// It answers the document queries (xsvg-kit docs/Queries.md; Incremental.md
/// §10) and compiles through [`CompileSession::from_parsed`].
///
/// The core owns no text and keeps no state between calls: a host that wants a
/// parse to outlive one call owns the text and pairs it with this (a
/// self-referential cell, since the tree borrows the text). `ParsedDocument` is
/// covariant in `'a`, which such a cell requires.
pub struct ParsedDocument<'a> {
    pub(super) parsed: Parsed<'a>,
}

impl<'a> ParsedDocument<'a> {
    /// Parse once, pre-scan once. Fails exactly where a compile fails (nesting
    /// depth, XML parse error), with the same message.
    pub fn new(input: &'a str) -> Result<Self, String> {
        Ok(Self {
            parsed: Parsed::new(input)?,
        })
    }

    /// The effective value of each of `properties` on the element starting at
    /// byte `offset`, by the rules its emitter applies (xsvg-kit
    /// docs/Queries.md): a style property ([`super::query::STYLE_PROPS`]) walks
    /// the element's cascade — its own attribute and `<x:font>` token, the box
    /// over a paragraph, the list over an item, the row, column and table over a
    /// cell, the ancestors for `font-family` — and reports where the value came
    /// from; any other attribute (`x:` naming the xsvg namespace) is the
    /// element's own, else the default its element declares. `None` when no
    /// element starts there.
    pub fn resolved(&self, offset: usize, properties: &[&str]) -> Option<Vec<super::Resolved>> {
        self.parsed.resolved(offset, properties)
    }

    /// The document's `<x:color>` and `<x:font>` tokens, as the compiler loaded
    /// them: in document order, a later token of a name replacing an earlier
    /// one, nothing inside `<x:doc>`.
    pub fn theme(&self) -> super::ThemeTokens {
        super::query::theme_tokens(&self.parsed.theme)
    }

    /// The styled pieces of the text of the element starting at `offset`, each
    /// in the style it renders in, offsets in UTF-16 code units of the element's
    /// text content. `None` when the element holds no text element.
    pub fn text_style(&self, offset: usize) -> Option<Vec<super::TextPiece>> {
        self.parsed.text_pieces(offset)
    }

    /// The block leaves — paragraphs and list items — of the box or list
    /// starting at `offset`, in reading order. `None` for anything else.
    pub fn blocks(&self, offset: usize) -> Option<Vec<super::Block>> {
        self.parsed.blocks(offset)
    }

    /// What a run inside the text of the element starting at `offset` may carry
    /// and see drawn. `None` when the element holds no text.
    pub fn run_support(&self, offset: usize) -> Option<super::RunSupport> {
        self.parsed.run_support(offset)
    }

    /// The corners the shape at `offset` draws (§5.4), for an editor's corner
    /// widget: where each lands and whether its radius was limited.
    pub fn corners(&self, offset: usize) -> Option<Vec<super::CornerInfo>> {
        self.parsed.corners(offset)
    }
}

/// Compile-time proof that a host may shorten the lifetime of a parse — what a
/// self-referential cell pairing it with its text needs. Fails to build if a
/// field ever makes `ParsedDocument` invariant.
#[allow(dead_code)]
fn parsed_document_is_covariant<'long: 'short, 'short>(
    d: ParsedDocument<'long>,
) -> ParsedDocument<'short> {
    d
}

/// The parse a session compiles from: its own, or the host's.
enum Held<'a> {
    Owned(Box<ParsedDocument<'a>>),
    Borrowed(&'a ParsedDocument<'a>),
}

/// One document, parsed and pre-scanned once, ready to compile any number of its
/// fragments on a [`Platform`](super::Platform) (docs/Compile Session.md).
///
/// Created by [`Platform::session`](super::Platform::session) at the top of a render flush over the text the
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
    doc: Held<'a>,
    platform: &'a P,
    quality: QualityProfile,
    sourcemap: bool,
    base: String,
}

impl<'a, P: Host> CompileSession<'a, P> {
    /// Parse once, pre-scan once. Fails exactly where the pure entries fail
    /// (nesting depth, XML parse error), with the same message. Usually reached
    /// as [`Platform::session`](super::Platform::session).
    pub fn new(
        platform: &'a P,
        input: &'a str,
        quality: &str,
        sourcemap: bool,
        base: &str,
    ) -> Result<Self, String> {
        Ok(Self {
            doc: Held::Owned(Box::new(ParsedDocument::new(input)?)),
            platform,
            quality: QualityProfile::parse(quality),
            sourcemap,
            base: base.to_string(),
        })
    }

    /// A session over a document the host already parsed — no parse of its own.
    /// A host keeps the [`ParsedDocument`] of its current text between flushes
    /// (paired with the text it borrows, which the host owns), answers queries
    /// from it, and compiles from it here while the text is unchanged: one
    /// parse per change, shared by the flush and every query. Every byte is the
    /// byte [`Self::new`] over the same text would emit.
    pub fn from_parsed(
        platform: &'a P,
        doc: &'a ParsedDocument<'a>,
        quality: &str,
        sourcemap: bool,
        base: &str,
    ) -> Self {
        Self {
            doc: Held::Borrowed(doc),
            platform,
            quality: QualityProfile::parse(quality),
            sourcemap,
            base: base.to_string(),
        }
    }

    /// The parsed document this session compiles: its queries
    /// ([`ParsedDocument::resolved`] and the rest) answer from the same parse.
    pub fn document(&self) -> &ParsedDocument<'a> {
        match &self.doc {
            Held::Owned(d) => d,
            Held::Borrowed(d) => d,
        }
    }

    fn parsed(&self) -> &Parsed<'a> {
        &self.document().parsed
    }

    /// The whole document — byte-identical to [`super::compile_platform_impl`].
    pub fn compile(&self) -> Result<String, String> {
        self.parsed().compile(
            &self.platform.backend(),
            self.quality,
            self.sourcemap,
            &self.base,
        )
    }

    /// The top-level element containing byte `offset` — byte-identical to
    /// [`super::compile_fragment_platform_impl`], and so to that element's span in
    /// [`Self::compile`] (the fragment invariant, docs/Incremental.md).
    pub fn compile_fragment(&self, offset: usize) -> Result<String, String> {
        self.parsed().compile_fragment(
            offset,
            &self.platform.backend(),
            self.quality,
            self.sourcemap,
            &self.base,
        )
    }

    /// Source byte range of the incremental unit containing `offset`
    /// (docs/Incremental.md §9): a leaf below nested passthrough
    /// container, else the top-level element itself.
    pub fn unit_range(&self, offset: usize) -> Option<(usize, usize)> {
        self.parsed().unit_range(offset)
    }

    /// Compile the unit containing `offset` on its own — byte-identical to its
    /// span in its fragment's output.
    pub fn compile_unit(&self, offset: usize) -> Result<String, String> {
        self.parsed().compile_unit(
            offset,
            &self.platform.backend(),
            self.quality,
            self.sourcemap,
            &self.base,
        )
    }

    /// The fragment containing `offset` plus where each of its units landed in
    /// it, in output order; the span list is empty when the top-level element is
    /// its own unit.
    pub fn compile_fragment_spans(
        &self,
        offset: usize,
    ) -> Result<(String, Vec<super::UnitSpan>), String> {
        self.parsed().compile_fragment_spans(
            offset,
            &self.platform.backend(),
            self.quality,
            self.sourcemap,
            &self.base,
        )
    }

    /// [`CompileSession::dependents`] at unit granularity: seeds and results are
    /// unit source ranges, so dependents inside the same container are found.
    pub fn unit_dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)> {
        self.parsed().unit_dependents(seeds)
    }

    /// Baked-reference dependents of the fragments at `seeds` — the same answer as
    /// [`super::dependents_multi_impl`], from the retained parse.
    pub fn dependents(&self, seeds: &[usize]) -> Vec<(usize, usize)> {
        self.parsed().dependents(seeds)
    }

    /// Caret geometry for the `<x:textbox>` starting at byte `offset` — the same
    /// answer as [`super::text_layout_impl`]. `None` when the offset is not a
    /// textbox or the box resolves to nothing renderable.
    pub fn text_layout(&self, offset: usize) -> Option<TextLayout> {
        self.parsed()
            .text_layout(offset, &self.platform.backend(), self.quality)
    }

    /// How big the `<x:textbox>` at byte `offset` must be to hold its content —
    /// the same answer as [`super::measure_textbox_impl`]. `None` when the offset
    /// is not a rectangular textbox.
    pub fn measure_textbox(&self, offset: usize) -> Option<super::TextboxSize> {
        self.parsed()
            .measure_textbox(offset, &self.platform.backend(), self.quality)
    }

    /// Caret stops on the outlined text element at byte `offset` — the same
    /// answer as [`super::caret_stops_impl`]. `None` when nothing there outlines.
    pub fn caret_stops(&self, offset: usize) -> Option<super::CaretStops> {
        self.parsed()
            .caret_stops(offset, &self.platform.backend(), self.quality)
    }

    /// Convert to path (docs/Incremental.md §11): the element at `offset` as the
    /// markup that replaces it in the source — its text as glyph paths, a shape as
    /// a `<path>`, every unspent attribute as authored. `Err` says why not: the
    /// font without a file, or a named limit. The same answer as
    /// [`super::to_path_impl`].
    pub fn to_path(&self, offset: usize) -> Result<String, super::ToPathError> {
        self.parsed()
            .to_path(offset, &self.platform.backend(), self.quality, false)
    }

    /// The operands of the `<x:boolean>` at `offset`, for `xsvg-path-ops`
    /// (docs/Incremental.md §11): what a host hands the curve-keeping algebra,
    /// whose answer replaces the `d` of [`Self::to_path`]'s markup. The same
    /// answer as [`super::boolean_operands_impl`].
    pub fn boolean_operands(&self, offset: usize) -> Option<super::BooleanOperands> {
        self.parsed()
            .boolean_operands(offset, &self.platform.backend(), self.quality)
    }

    /// Convert to spline (docs/Incremental.md §11): [`Self::to_path`] with the
    /// corners baked and every arc expanded to cubic Béziers — a quarter turn
    /// per cubic at most, so a circle is four anchors — for an editor whose
    /// paths are Béziers only. The same answer as [`super::to_spline_impl`].
    pub fn to_spline(&self, offset: usize) -> Result<String, super::ToPathError> {
        self.parsed()
            .to_path(offset, &self.platform.backend(), self.quality, true)
            .map(|m| super::spline::markup_to_spline(&m))
    }

    /// Source byte range of every element, in document order — the same answer as
    /// [`super::node_spans_impl`].
    pub fn node_spans(&self) -> Vec<(usize, usize)> {
        self.parsed().node_spans()
    }

    /// Source byte range of the fragment unit containing `offset` — the same answer
    /// as [`super::fragment_range_impl`].
    pub fn fragment_range(&self, offset: usize) -> Option<(usize, usize)> {
        self.parsed().fragment_range(offset)
    }
}
