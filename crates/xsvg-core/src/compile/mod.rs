//! The xsvg compiler core: parse xsvg/SVG input, run the lowering passes, and emit a
//! plain-SVG-subset string. Platform-agnostic — no wasm/JS or native font code lives
//! here. Everything platform-specific is a trait the core calls (`Measurer`, `Shaper`,
//! `GlyphOutliner`, all defined in `xsvg-core`); the `xsvg-wasm` crate backs them with
//! browser callbacks and `xsvg-cli` backs them with native font libraries. Both are thin
//! adapters over the `compile_impl` / `compile_fragment_impl` entry points here.
pub(crate) use crate::{
    boolean_svg_paths, filter_primitives, layout_area_runs, layout_flow, layout_flow_runs,
    layout_region, layout_text_area_runs, line_advance, measure_runs, offset_svg_paths,
    parse_filter_functions, run_offset, split_flows, svg_path_bbox, warp_svg_path,
    warp_text_on_path, Align, Anchor, AreaLayout, AreaSpec, BendField, BoolOp, BoolOperand, Chain,
    DisplayAlign, EnvelopePreset, Field, Fit, FontMetrics, FreeDistort, GlyphOutliner, Homography,
    LineIncrement, Measurer, PathEffect, PathFrame, PlacedLine, QualityProfile, RegionSpec,
    RoughenField, Shaper, Taper, TextAlign, TextAreaSpec, TextOverflow, TextStyle, VAlign,
    WarpAxis,
};

const XSVG_NS: &str = "https://xsvg.visioncortex.org";
const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Maximum element nesting depth accepted. `roxmltree`'s parser recurses per level,
/// so pathologically deep input would overflow the stack (a hard abort, worse on
/// wasm's smaller stack). Real documents nest a few dozen deep; this is a generous
/// ceiling that still leaves a wide stack margin.
const MAX_NESTING_DEPTH: usize = 512;

/// Depth cap for **reference chains** (`ref_geometry` recursion). Much lower
/// than [`MAX_NESTING_DEPTH`]: each chain link recurses through a full emitter
/// (frames of kilobytes, not the parser's small ones), so 512 links would
/// overflow a 1–2 MB stack. Real derivation chains are a handful of links; 32
/// is far beyond that, and a deeper chain degrades with a marker (§4 totality)
/// instead of trapping.
const MAX_REF_DEPTH: usize = 32;

/// Total reference resolutions allowed per compile call. The memo makes legit
/// documents consume roughly one unit per distinct id'd target; only
/// cycle-poisoned fan-out (unmemoizable by design) burns fuel combinatorially,
/// and this is the bound that stops it.
const REF_FUEL: u32 = 65_536;

/// Depth cap for cross-file `<use>` links — how deep the dependency DAG may nest
/// before a link degrades with a marker (§4 totality; guards against a very deep
/// or resolver-misbehaving chain, independent of the same-document [`MAX_REF_DEPTH`]).
const MAX_LINK_DEPTH: usize = 16;

/// Platform seam for cross-file `<use href="file.svg">` linking (§4). Given the
/// referrer's canonical key (`base`) and the raw `href`, the host resolves the path
/// relative to `base`, enforces its own security model (same-origin `fetch` in the
/// browser, disk reads natively), and returns the dependency's **canonical key** plus
/// its **source text** — or `None` to degrade. Keys only need to be stable and
/// comparable (the cycle guard compares them); they are typically the resolved path/URL.
pub trait Resolver {
    fn resolve(&self, base: &str, href: &str) -> Option<(String, String)>;
}

impl<T: Resolver + ?Sized> Resolver for &T {
    fn resolve(&self, base: &str, href: &str) -> Option<(String, String)> {
        (**self).resolve(base, href)
    }
}

/// A resolver that links nothing — every external `<use>` degrades with a marker.
/// Backs the non-linking entry point ([`compile_impl`]) and callers without deps.
pub struct NoResolver;
impl Resolver for NoResolver {
    fn resolve(&self, _base: &str, _href: &str) -> Option<(String, String)> {
        None
    }
}

/// Everything a lowering pass needs from the platform: font metrics + shape raster +
/// glyph outliner, the quality profile (bake tolerances, §7.1), plus whether to emit
/// source-position attributes (`data-xsvg-pos`).
struct Ctx<'a> {
    m: &'a dyn Measurer,
    shaper: &'a dyn Shaper,
    outliner: &'a dyn GlyphOutliner,
    quality: QualityProfile,
    /// Whether to emit `data-xsvg-pos` source ranges. Toggled **off** while a linked
    /// dependency is compiled: its element ranges index the *dependency* file, not the
    /// entry document the viewer maps against, so baked content carries no range and a
    /// click resolves up to the wrapper — i.e. to the `<use>` in the entry source (§4.2).
    sourcemap: std::cell::Cell<bool>,
    /// ids currently being resolved as compiled-output references — the cycle
    /// guard for `ref_geometry` (a target that is already on this stack degrades
    /// with a marker instead of recursing forever). Its length is also the
    /// reference-chain depth, capped at [`MAX_REF_DEPTH`] (§4 totality: sibling
    /// chains must not overflow the stack either).
    resolving: std::cell::RefCell<Vec<String>>,
    /// per-compile memo of **context-free** `ref_geometry` results (id → joined
    /// `d`). Only resolutions untouched by a cycle/depth cut are cached — a cut
    /// result depends on what was on the stack, so it must not leak into other
    /// contexts. Collapses diamond-shaped reference fan-out from exponential to
    /// linear; dies with this compile call, so it can never go stale.
    resolved: std::cell::RefCell<std::collections::HashMap<String, Option<String>>>,
    /// counts cycle/depth cuts; `ref_geometry` snapshots it around a resolution
    /// to decide whether the result was context-free (memoizable).
    cuts: std::cell::Cell<u64>,
    /// remaining reference resolutions for this compile call ([`REF_FUEL`]).
    /// The depth cap bounds how deep a resolution tree goes; this bounds how
    /// WIDE — cycle-poisoned fan-out defeats the memo (cut results are never
    /// cached), so without fuel a document could branch 2^depth resolutions.
    fuel: std::cell::Cell<u32>,
    /// forces `outline="true"` semantics while scratch-serializing a reference
    /// target, so referenced text contributes glyph geometry instead of nothing.
    force_outline: std::cell::Cell<bool>,
    /// Cross-file `<use>` linking seam (§4).
    resolver: &'a dyn Resolver,
    /// canonical keys of the files currently being linked — the cross-file cycle
    /// guard (a link back to an ancestor degrades). Its length is the link depth,
    /// capped at [`MAX_LINK_DEPTH`].
    files: std::cell::RefCell<Vec<String>>,
    /// canonical key of the file being serialized, so relative hrefs resolve against it.
    base: std::cell::RefCell<String>,
    /// Stable node identity supplied by an editing host (§Incremental; consumer:
    /// xsvg-kit). With keys, emitted identity and generated filter ids become
    /// independent of byte offsets. [`NoKeys`] preserves byte-span behavior.
    keys: &'a dyn NodeKeys,
    /// Byte starts an **active** key provider answered `None` for — elements the
    /// host was required to name and did not. A set, not a count: the same node can
    /// be asked twice (identity stamping and def-id seeding, plus scratch passes).
    key_misses: std::cell::RefCell<std::collections::HashSet<usize>>,
    /// Loadable geometry generators (docs/Extensibility.md). [`NoGenerators`]
    /// degrades every plugin element, the default for hosts that load nothing.
    generators: &'a dyn GeneratorHost,
    /// `<x:import>` namespace bindings from the entry document — the set of
    /// namespaces that dispatch to `generators` rather than dropping as foreign.
    imports: &'a ImportTable,
    /// `<x:theme>` tokens of the entry document (§4.1), resolved during
    /// serialization. A field, not a thread-local: two documents compiled
    /// interleaved on one thread each see their own tokens.
    theme: &'a Theme,
    /// The entry document and its id table, so `#id` resolution inside it is a
    /// lookup rather than a scan. Nodes of a linked dependency belong to another
    /// document and fall back to scanning theirs.
    doc: &'a roxmltree::Document<'a>,
    ids: &'a std::collections::HashMap<String, roxmltree::NodeId>,
    /// Set while generator output is being sanitized, so a module emitting an
    /// element in its own namespace degrades instead of recursing forever.
    in_plugin: std::cell::Cell<bool>,
}

/// The borrowed view of a platform a compile runs against — what [`Ctx`] is built
/// from. [`Platform::backend`] produces one; nothing else does.
#[doc(hidden)]
pub struct Backend<'a> {
    pub m: &'a dyn Measurer,
    pub shaper: &'a dyn Shaper,
    pub outliner: &'a dyn GlyphOutliner,
    pub resolver: &'a dyn Resolver,
    pub keys: &'a dyn NodeKeys,
    pub generators: &'a dyn GeneratorHost,
}

/// Everything the compiler needs from the world it runs in, owned and long-lived:
/// font metrics ([`Measurer`]), shape rasterization ([`Shaper`]), glyph outlines
/// ([`GlyphOutliner`]), cross-file linking ([`Resolver`]), editor node identity
/// ([`NodeKeys`]) and loadable generators ([`GeneratorHost`]). An embedder builds
/// one when it starts and keeps it: the adapters behind it are where the work that
/// survives a compile lives — a font database, a loaded module, and the measurement
/// cache [`Platform::with_measure_cache`] puts in front of the measurer.
///
/// A [`CompileSession`] borrows a platform for one flush (`platform.session(text,
/// …)`), so nothing is passed per call. The adapters keep their concrete types
/// rather than being erased, so `Send` and `Sync` follow from them: a platform of
/// thread-safe adapters gives `Send` sessions with nothing asserted, and a browser
/// platform over JS handles simply is not, which on a single thread costs nothing.
pub struct Platform<M, S, O, R = NoResolver, K = NoKeys, G = NoGenerators> {
    m: M,
    shaper: S,
    outliner: O,
    resolver: R,
    keys: K,
    generators: G,
}

impl<M: Measurer, S: Shaper, O: GlyphOutliner> Platform<M, S, O> {
    /// The three font seams every compile needs; everything else inert (no linking,
    /// byte-span identity, no generators) until a `with_*` supplies it.
    pub fn new(m: M, shaper: S, outliner: O) -> Self {
        Self {
            m,
            shaper,
            outliner,
            resolver: NoResolver,
            keys: NoKeys,
            generators: NoGenerators,
        }
    }
}

impl<M, S, O, R, K, G> Platform<M, S, O, R, K, G> {
    pub fn with_resolver<R2: Resolver>(self, resolver: R2) -> Platform<M, S, O, R2, K, G> {
        Platform {
            m: self.m,
            shaper: self.shaper,
            outliner: self.outliner,
            resolver,
            keys: self.keys,
            generators: self.generators,
        }
    }

    pub fn with_keys<K2: NodeKeys>(self, keys: K2) -> Platform<M, S, O, R, K2, G> {
        Platform {
            m: self.m,
            shaper: self.shaper,
            outliner: self.outliner,
            resolver: self.resolver,
            keys,
            generators: self.generators,
        }
    }

    pub fn with_generators<G2: GeneratorHost>(self, generators: G2) -> Platform<M, S, O, R, K, G2> {
        Platform {
            m: self.m,
            shaper: self.shaper,
            outliner: self.outliner,
            resolver: self.resolver,
            keys: self.keys,
            generators,
        }
    }

    /// Put a [`MeasureCache`] in front of the measurer, so a word measured once in a
    /// font is never measured again while this platform lives (docs/Compile
    /// Session.md §5). Output is unchanged; only the calls that reach the adapter are.
    pub fn with_measure_cache(self) -> Platform<MeasureCache<M>, S, O, R, K, G>
    where
        M: Measurer,
    {
        Platform {
            m: MeasureCache::new(self.m),
            shaper: self.shaper,
            outliner: self.outliner,
            resolver: self.resolver,
            keys: self.keys,
            generators: self.generators,
        }
    }
}

impl<M: Measurer, S: Shaper, O: GlyphOutliner, R: Resolver, K: NodeKeys, G: GeneratorHost>
    Platform<M, S, O, R, K, G>
{
    /// One document, parsed once, ready to compile any number of its fragments on
    /// this platform (docs/Compile Session.md).
    pub fn session<'s>(
        &'s self,
        input: &'s str,
        quality: &str,
        sourcemap: bool,
        base: &str,
    ) -> Result<CompileSession<'s, Self>, String> {
        CompileSession::new(self, input, quality, sourcemap, base)
    }

    pub(super) fn backend(&self) -> Backend<'_> {
        Backend {
            m: &self.m,
            shaper: &self.shaper,
            outliner: &self.outliner,
            resolver: &self.resolver,
            keys: &self.keys,
            generators: &self.generators,
        }
    }
}

/// What a [`CompileSession`] needs of its platform: the borrowed view a [`Ctx`] is
/// built from. Implemented by every [`Platform`]; sealed, since [`Backend`] is the
/// crate's own.
pub trait Host: sealed::Sealed {
    #[doc(hidden)]
    fn backend(&self) -> Backend<'_>;
}

mod sealed {
    pub trait Sealed {}
}

impl<M: Measurer, S: Shaper, O: GlyphOutliner, R: Resolver, K: NodeKeys, G: GeneratorHost>
    sealed::Sealed for Platform<M, S, O, R, K, G>
{
}

impl<M: Measurer, S: Shaper, O: GlyphOutliner, R: Resolver, K: NodeKeys, G: GeneratorHost> Host
    for Platform<M, S, O, R, K, G>
{
    fn backend(&self) -> Backend<'_> {
        Platform::backend(self)
    }
}

impl<'a> Ctx<'a> {
    /// Build the per-compile context from the platform's seams plus the parsed document.
    fn new(
        seams: &Backend<'a>,
        quality: QualityProfile,
        sourcemap: bool,
        base: &str,
        parsed: &'a session::Parsed<'a>,
    ) -> Self {
        Self {
            m: seams.m,
            shaper: seams.shaper,
            outliner: seams.outliner,
            quality,
            sourcemap: std::cell::Cell::new(sourcemap),
            resolving: std::cell::RefCell::new(Vec::new()),
            resolved: std::cell::RefCell::new(std::collections::HashMap::new()),
            cuts: std::cell::Cell::new(0),
            fuel: std::cell::Cell::new(REF_FUEL),
            force_outline: std::cell::Cell::new(false),
            resolver: seams.resolver,
            files: std::cell::RefCell::new(vec![base.to_string()]),
            base: std::cell::RefCell::new(base.to_string()),
            keys: seams.keys,
            key_misses: std::cell::RefCell::new(std::collections::HashSet::new()),
            generators: seams.generators,
            imports: &parsed.imports,
            theme: &parsed.theme,
            doc: &parsed.doc,
            ids: &parsed.ids,
            in_plugin: std::cell::Cell::new(false),
        }
    }

    /// The element `r` (`#id` or bare id) names, in `node`'s document. A table
    /// lookup for the entry document; a scan for a linked dependency's.
    fn resolve_ref<'n>(
        &self,
        node: roxmltree::Node<'n, 'n>,
        r: &str,
    ) -> Option<roxmltree::Node<'n, 'n>> {
        let doc = node.document();
        if std::ptr::eq(
            doc as *const _ as *const (),
            self.doc as *const _ as *const (),
        ) {
            let id = r.strip_prefix('#').unwrap_or(r);
            return self.ids.get(id).and_then(|&nid| doc.get_node(nid));
        }
        resolve_ref(node, r)
    }

    /// Resolve every `var(name)` / `var(name, fallback)` in an attribute value
    /// against the theme's color tokens. An unknown token with no fallback is left
    /// verbatim (renders as the property's initial value, like a dangling CSS
    /// variable).
    fn resolve_var<'v>(&self, v: &'v str) -> std::borrow::Cow<'v, str> {
        self.theme.resolve_var(v)
    }

    /// A paint value for output: theme tokens resolved, *then* escaped — a token's
    /// value is authored too, so resolving after escaping would let it carry the
    /// quote.
    fn paint(&self, v: &str) -> String {
        esc(&self.resolve_var(v))
    }

    /// The `<x:font>` token's properties named by an element's `x:font`, if any.
    fn font_props(&self, node: roxmltree::Node) -> Option<Vec<(String, String)>> {
        let name = node.attribute((XSVG_NS, "font"))?;
        self.theme.fonts.get(name).cloned()
    }

    /// The host's key for the node starting at `start`, recording a miss when an
    /// **active** provider fails to name it. Every key lookup goes through here, so
    /// the unkeyed marker in [`CompileSession::compile`] sees every violation.
    fn key_for(&self, start: usize) -> Option<String> {
        let k = self.keys.key(start);
        if k.is_none() && self.keys.active() {
            self.key_misses.borrow_mut().insert(start);
        }
        k
    }
}

/// Stable node identity for keyed compiles: `key(byte_start)` returns the host's
/// stable key for the element whose source range starts there, or `None` to fall
/// back to byte spans. With a provider, `data-xsvg-node="<key>"` replaces
/// `data-xsvg-pos="start-end"` and filter ids become `x-flt-<key>` — an unchanged
/// fragment's output is then **byte-stable under text shifts elsewhere** in the
/// document (docs/Incremental.md), which is what makes surgical patching and
/// exact-bytes verification possible for an editor. Keys are consulted only while
/// the sourcemap is on (never inside linked dependencies, whose offsets index the
/// dependency file).
///
/// Hosts MUST key **every** element, not just top-level fragments: emitters tag
/// nested elements too (warp children, paragraphs), and an unkeyed element falls
/// back to byte spans — silently reintroducing offset-dependence.
pub trait NodeKeys {
    fn key(&self, byte_start: usize) -> Option<String>;

    /// Whether a host actually supplied this provider. `None` from [`key`](Self::key)
    /// then means *this element was missed* — a contract violation worth reporting —
    /// rather than "no keys in play". Defaulted so existing implementors (xsvg-kit)
    /// need not change; only [`NoKeys`] answers `false`.
    fn active(&self) -> bool {
        true
    }
}

impl<T: NodeKeys + ?Sized> NodeKeys for &T {
    fn key(&self, byte_start: usize) -> Option<String> {
        (**self).key(byte_start)
    }
    fn active(&self) -> bool {
        (**self).active()
    }
}

/// No host keys: byte-span behavior, the default for every pre-existing entry point.
pub struct NoKeys;
impl NodeKeys for NoKeys {
    fn key(&self, _: usize) -> Option<String> {
        None
    }
    fn active(&self) -> bool {
        false
    }
}

/// `" data-xsvg-pos=\"START-END\""` (byte offsets of `node` in the original xsvg
/// source) when the source map is enabled, else empty. Attached to each emitted
/// top-level element so a viewer can project a rendered element back to its source.
fn pos_attr(node: roxmltree::Node, ctx: &Ctx) -> String {
    if !ctx.sourcemap.get() {
        return String::new();
    }
    let r = node.range();
    if let Some(k) = ctx.key_for(r.start) {
        return format!(" data-xsvg-node=\"{k}\"");
    }
    format!(" data-xsvg-pos=\"{}-{}\"", r.start, r.end)
}
/// The authored frame of a framed element, for keyed (editor) output only:
/// ` data-xsvg-frame="x y w h"` in the element's own user space. A textbox
/// compiles to the glyphs it shows, so its rendered bbox is the ink — but
/// what an editor selects, hits and resizes is the frame the text flows in
/// (Slides: a click anywhere in the box). Omitted for `in="#rect"` geometry
/// (the target rect is the frame there) and outside keyed mode, so viewer
/// output is untouched.
fn frame_attr(node: roxmltree::Node, ctx: &Ctx) -> String {
    if !ctx.sourcemap.get() || ctx.key_for(node.range().start).is_none() || node.has_attribute("in")
    {
        return String::new();
    }
    let (Some(w), Some(h)) = (
        node.attribute("width").and_then(parse_num),
        node.attribute("height").and_then(parse_num),
    ) else {
        return String::new();
    };
    format!(
        " data-xsvg-frame=\"{} {} {} {}\"",
        fmt(attr_num(node, "x", 0.0)),
        fmt(attr_num(node, "y", 0.0)),
        fmt(w),
        fmt(h)
    )
}

/// The direct element child of the document root whose source byte range contains
/// `offset` — the **fragment unit** of incremental compilation (docs/Incremental.md).
fn top_level_at<'a>(
    doc: &'a roxmltree::Document<'a>,
    offset: usize,
) -> Option<roxmltree::Node<'a, 'a>> {
    doc.root_element()
        .children()
        .filter(|c| c.is_element())
        .find(|c| {
            let r = c.range();
            r.start <= offset && offset < r.end
        })
}

/// Source byte ranges of every *other* top-level element that must be re-emitted
/// when the fragment at `offset` changes — the **transitive closure** over
/// compile-time-baked `in="#id"` references (textbox / textpath / warp bend).
/// Chains are real since `x:` elements can be `in`-targets themselves (their
/// compiled output is the referenced geometry): editing `#a` re-emits
/// `<x:warp id="w" in="#a">`, whose changed output re-emits `<x:textpath
/// in="#w">`, and so on to a fixpoint. Live references (`href`, `url(#…)`
/// paints) re-resolve in the DOM and are deliberately NOT reported. Results are
/// in document order.
/// The references this node **bakes** at compile time: an `in="#id"`
/// attribute (textbox / textpath / warp bend), or the `href` of a `<use>` that
/// is a direct child of `<x:boolean>` (an operand by reference — §7.4). A
/// passthrough `<use>` anywhere else is a live reference the browser resolves,
/// so it is deliberately not reported.
fn baked_refs<'a>(
    n: roxmltree::Node<'a, 'a>,
    ids: &std::collections::HashMap<String, roxmltree::NodeId>,
) -> Vec<&'a str> {
    let mut out = Vec::new();
    if let Some(r) = n.attribute("in") {
        out.push(r);
    }
    // <x:connector from to> bakes both endpoint boxes into the routed path (§7.6)
    if n.tag_name().namespace() == Some(XSVG_NS) && n.tag_name().name() == "connector" {
        for r in [n.attribute("from"), n.attribute("to")]
            .into_iter()
            .flatten()
        {
            out.push(r);
            // An endpoint may carry a `:anchor` suffix (`#box:right`), and the
            // dependency is on the id in front of it. Report both spellings rather
            // than re-deciding here whether the suffix is an anchor keyword: naming
            // an id that does not exist costs a comparison, while missing one leaves
            // a connector routed to where its box used to be.
            if let Some((base, _)) = r.rsplit_once(':') {
                out.push(base);
            }
        }
    }
    if n.tag_name().name() == "use"
        && n.tag_name().namespace() != Some(XSVG_NS)
        && n.parent().is_some_and(|p| {
            p.tag_name().namespace() == Some(XSVG_NS) && p.tag_name().name() == "boolean"
        })
    {
        out.extend(
            n.attribute("href")
                .or_else(|| n.attribute((XLINK_NS, "href"))),
        );
    }
    // a fill referencing a <meshgradient> is compiled, not live (§8.2)
    if let Some(id) = n
        .attribute("fill")
        .and_then(|f| f.strip_prefix("url(#"))
        .and_then(|f| f.strip_suffix(')'))
    {
        let target = ids.get(id).and_then(|&nid| n.document().get_node(nid));
        if target.is_some_and(|t| t.tag_name().name() == "meshgradient") {
            out.push(id);
        }
    }
    out
}

/// Recursively emit a node as SVG.
/// Lower a `filter` attribute holding CSS filter functions (§8) to a real
/// `<filter>` definition: the def is emitted immediately before the element
/// (self-contained per fragment — §Incremental), and the returned markup is
/// the substitute ` filter="url(#…)"` attribute. `None` leaves the attribute
/// as authored: absent, `none`, a `url(#…)` reference, or a list the parser
/// declines (unknown function, invalid argument) — browsers still honor those
/// live, mirroring CSS's whole-declaration-invalid rule. sRGB interpolation is
/// pinned so lowered output matches live browser rendering; the region gets
/// the ±10% margin so strokes outside the fill bbox survive.
/// Seed for generated def ids (`x-flt-…`, `x-mesh-…`, `x-mgc-…`): the host key
/// when one is provided (offset-independent, see [`NodeKeys`]), else the node's
/// byte start — the legacy form, byte-for-byte.
fn gen_id_seed(node: roxmltree::Node, ctx: &Ctx) -> String {
    match ctx
        .sourcemap
        .get()
        .then(|| ctx.key_for(node.range().start))
        .flatten()
    {
        Some(k) => k,
        None => node.range().start.to_string(),
    }
}

fn lower_filter_attr(node: roxmltree::Node, out: &mut String, ctx: &Ctx) -> Option<String> {
    let fns = parse_filter_functions(node.attribute("filter")?)?;
    let id = format!("x-flt-{}", gen_id_seed(node, ctx));
    // pointwise functions need only the stroke margin. Blur/drop-shadow spill:
    // on a plain shape the spill is computed EXACTLY (3σ per blur + the shadow
    // offset + half the stroke width) as a userSpaceOnUse region; on
    // unmeasurable content (groups) the region falls back to ±50% of the bbox
    // — the one bound a bbox-relative region can express.
    let bleeds = fns.iter().any(|f| f.bleeds());
    let region = if !bleeds {
        " x=\"-10%\" y=\"-10%\" width=\"120%\" height=\"120%\"".to_string()
    } else if let Some(bb) = shape_to_path_d(node).and_then(|d| svg_path_bbox(&d)) {
        let mut m = attr_num(node, "stroke-width", 1.0) / 2.0;
        for f in &fns {
            m += match f {
                crate::AdjustFn::Blur(r) => 3.0 * r,
                crate::AdjustFn::DropShadow { dx, dy, r, .. } => 3.0 * r + dx.abs().max(dy.abs()),
                _ => 0.0,
            };
        }
        format!(
            " filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            fmt(bb.x0 - m),
            fmt(bb.y0 - m),
            fmt(bb.width() + 2.0 * m),
            fmt(bb.height() + 2.0 * m)
        )
    } else {
        " x=\"-50%\" y=\"-50%\" width=\"200%\" height=\"200%\"".to_string()
    };
    out.push_str(&format!(
        "<filter id=\"{id}\" color-interpolation-filters=\"sRGB\"{region}>{}</filter>",
        filter_primitives(&fns)
    ));
    Some(format!(" filter=\"url(#{id})\""))
}

/// The compile-time z-band of a layer (§5.1): `x:layer="background"` sinks
/// behind everything (−1), `"foreground"` floats in front (+1), any other
/// value is the content band (0). `None` when the element carries no
/// `x:layer` attribute at all.
fn layer_band(node: roxmltree::Node) -> Option<i32> {
    node.attribute((XSVG_NS, "layer")).map(|v| match v {
        "background" => -1,
        "foreground" => 1,
        _ => 0,
    })
}

/// The layer visibility toggle: `x:hidden` (any value but `false`) compiles
/// the element and its subtree to nothing — hide a layer without deleting it.
fn is_hidden(node: roxmltree::Node) -> bool {
    matches!(node.attribute((XSVG_NS, "hidden")), Some(v) if v != "false")
}

/// Artboard metadata (§5.2): a `<g x:artboard="Label">` is a named frame — a
/// slide — that the viewer/preview can zoom to and page through. It compiles to
/// a plain `<g>` (renders normally in any viewer) carrying
/// `data-xsvg-artboard="Label"`, plus `data-xsvg-frame="x y w h"` when an
/// explicit `x:frame` is given (else the tools use the group's bbox). Returns
/// the attribute string to append, or empty when the element isn't an artboard.
fn artboard_attr(node: roxmltree::Node) -> String {
    let Some(label) = node.attribute((XSVG_NS, "artboard")) else {
        return String::new();
    };
    let mut s = String::from(" data-xsvg-artboard=\"");
    push_escaped(&mut s, label, true);
    s.push('"');
    if let Some(frame) = node.attribute((XSVG_NS, "frame")) {
        let nums: Vec<f64> = frame
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .filter_map(parse_num)
            .collect();
        if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
            s.push_str(&format!(
                " data-xsvg-frame=\"{} {} {} {}\"",
                fmt(nums[0]),
                fmt(nums[1]),
                fmt(nums[2]),
                fmt(nums[3])
            ));
        }
    }
    s
}

fn serialize(node: roxmltree::Node, out: &mut String, is_root: bool, ctx: &Ctx) {
    if !node.is_element() {
        if node.is_text() {
            if let Some(t) = node.text() {
                push_escaped(out, t, false);
            }
        }
        return; // comments, PIs, etc. are dropped
    }

    // x:hidden (§5.1) — the layer eyeball toggle; the subtree compiles to
    // nothing (never the root).
    if !is_root && is_hidden(node) {
        return;
    }

    // xsvg extension elements.
    if node.tag_name().namespace() == Some(XSVG_NS) {
        match node.tag_name().name() {
            "textbox" => emit_textbox(node, out, ctx),
            "textpath" => emit_textpath(node, out, ctx),
            "warp" => emit_warp(node, out, ctx),
            "boolean" => emit_boolean(node, out, ctx),
            "mesh" => emit_mesh(node, out, ctx),
            "connector" => emit_connector(node, out, ctx),
            "offset" => emit_offset(node, out, ctx),
            "list" => emit_list(node, out, ctx),
            "table" => emit_table(node, out, ctx),
            "pie" => emit_pie(node, out, ctx),
            "plot" => emit_plot(node, out, ctx),
            "theme" => {} // §4.1 definitions — loaded up front, emit nothing
            // §Extensibility definitions — collected up front like <x:theme>, so
            // only a binding we could not honor has anything to say here.
            "import" => {
                if let Some(why) = check_import(node, ctx.imports) {
                    out.push_str(&format!("<!-- xsvg: <x:import> {why} -->"));
                }
            }
            other => out.push_str(&format!("<!-- xsvg: <x:{other}> not yet lowered -->")),
        }
        return;
    }

    if let Some(ns) = node.tag_name().namespace() {
        // A namespace bound by <x:import> is a loadable generator (§Extensibility).
        if ns != SVG_NS && ctx.imports.has(ns) {
            emit_plugin(node, ns, out, ctx);
            return;
        }
        // Other foreign-namespace elements (editor metadata like sodipodi:/inkscape:)
        // can't be re-emitted faithfully — we declare no xmlns for them — so they drop
        // with a marker rather than being silently reparented into the SVG namespace.
        if ns != SVG_NS {
            out.push_str(&format!(
                "<!-- xsvg: foreign-namespace <{}> dropped -->",
                node.tag_name().name()
            ));
            return;
        }
    }

    // Cross-file link (§4): a <use> whose href points at another file (not a same-
    // document #fragment) is baked here — the dependency is compiled and stamped in.
    // A same-document <use href="#id"> stays a live reference (passthrough below).
    if node.tag_name().name() == "use" {
        if let Some(href) = node
            .attribute("href")
            .or_else(|| node.attribute((XLINK_NS, "href")))
        {
            // Not from inside generator output: a link makes the HOST fetch and bake
            // another file, and a module gets no say in what the host reads. The <use>
            // falls through and `confine` drops its non-fragment href.
            if is_external_href(href) && !ctx.in_plugin.get() {
                emit_link(node, href, out, ctx);
                return;
            }
        }
    }

    // <xsvg> root is just an alias for <svg>.
    let name = match node.tag_name().name() {
        "xsvg" => "svg",
        other => other,
    };

    // The static-subset deny list (§9, Plan R6): script and animation cannot
    // exist in the output contract, so they drop with a marker instead of
    // passing through.
    if matches!(
        name,
        "script" | "animate" | "animateMotion" | "animateTransform" | "set" | "discard"
    ) {
        out.push_str(&format!(
            "<!-- xsvg: <{name}> outside the static subset — dropped -->"
        ));
        return;
    }

    if name == "text" && node.attribute("inline-size").is_some() {
        emit_inline_size_text(node, out, ctx);
        return;
    }
    if name == "textArea" {
        emit_text_area(node, out, ctx);
        return;
    }

    // A fill referencing an SVG 2 <meshgradient> (the Inkscape dialect no
    // browser renders) compiles to the mesh pipeline, clipped by the shape.
    if let Some(mg) = mesh_fill_target(node, ctx) {
        if emit_meshgradient_fill(node, mg, out, ctx) {
            return;
        }
    }

    // Sharp-cornered <rect> → <path>. Rounded rects pass through unchanged.
    if name == "rect" && node.attribute("rx").is_none() && node.attribute("ry").is_none() {
        emit_rect_as_path(node, out, ctx);
        return;
    }

    let filter_sub = if is_root {
        None
    } else {
        lower_filter_attr(node, out, ctx)
    };
    out.push('<');
    out.push_str(name);
    if is_root {
        out.push_str(&format!(" xmlns=\"{SVG_NS}\""));
    }
    match &filter_sub {
        Some(sub) => {
            copy_attrs(node, out, &["filter"], ctx);
            out.push_str(sub);
        }
        None => copy_attrs(node, out, &[], ctx),
    }
    out.push_str(&pos_attr(node, ctx));
    if !is_root {
        out.push_str(&artboard_attr(node));
    }

    if node.has_children() {
        out.push('>');
        let elems: Vec<roxmltree::Node> = node.children().filter(|c| c.is_element()).collect();
        let restack = elems.iter().any(|c| {
            c.attribute((XSVG_NS, "layer")).is_some() || c.attribute((XSVG_NS, "order")).is_some()
        });
        if restack {
            // Layer restacking (§5.1): stable-sort direct children by
            // (band, order, document index). Loose content is band 0 / order
            // 0, so it keeps its place and only bucketed or explicitly-ordered
            // layers move. Non-element nodes (whitespace) drop in a restacked
            // container — insignificant for a group.
            let mut idx: Vec<usize> = (0..elems.len()).collect();
            idx.sort_by(|&a, &b| {
                let ka = (
                    layer_band(elems[a]).unwrap_or(0),
                    attr_num_ns(elems[a], "order", 0.0),
                );
                let kb = (
                    layer_band(elems[b]).unwrap_or(0),
                    attr_num_ns(elems[b], "order", 0.0),
                );
                ka.0.cmp(&kb.0)
                    .then(ka.1.partial_cmp(&kb.1).unwrap_or(std::cmp::Ordering::Equal))
                    .then(a.cmp(&b))
            });
            for &i in &idx {
                serialize(elems[i], out, false, ctx);
            }
        } else {
            for child in node.children() {
                serialize(child, out, false, ctx);
            }
        }
        out.push_str(&format!("</{name}>"));
    } else {
        out.push_str("/>");
    }
}

/// `<rect x y width height …>` → equivalent `<path d=… …>`.
fn emit_rect_as_path(node: roxmltree::Node, out: &mut String, ctx: &Ctx) {
    let x = attr_num(node, "x", 0.0);
    let y = attr_num(node, "y", 0.0);
    let w = attr_num(node, "width", 0.0);
    let h = attr_num(node, "height", 0.0);

    let filter_sub = lower_filter_attr(node, out, ctx);
    out.push_str("<path");
    match &filter_sub {
        Some(sub) => {
            copy_attrs(node, out, &["x", "y", "width", "height", "filter"], ctx);
            out.push_str(sub);
        }
        None => copy_attrs(node, out, &["x", "y", "width", "height"], ctx),
    }
    out.push_str(&pos_attr(node, ctx));
    out.push_str(&format!(" d=\"M{x},{y} h{w} v{h} h{} Z\"/>", -w));
}

/// Byte ranges of every ` d="…"` attribute value in `s`. Sound on compiler output:
/// attribute values are quote-escaped, and text-bearing children are rejected before
/// this scan, so a raw `"` cannot appear inside a match.
fn find_path_d_ranges(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(pos) = s[i..].find(" d=\"") {
        let start = i + pos + 4;
        let Some(len) = s[start..].find('"') else {
            break;
        };
        out.push((start, start + len));
        i = start + len + 1;
    }
    out
}

/// The bordered-text block every text element accepts (§6.17), declared beside
/// [`text_border_attrs`], which reads it.
///
/// Declared by hand for two reasons the derive cannot express: these live in the
/// **xsvg namespace** (`x:border-width`, not `border-width` — the plain spellings
/// are reserved for raw SVG stroke passthrough, which this element also honors),
/// and each one's default is contingent — naming either of the first two makes the
/// other take a default rather than stay absent.
pub(super) const TEXT_BORDER_PARAMS: &[xsvg_schema::ParamSpec] = &[
    ParamSpec {
        ns: Some(XSVG_NS),
        default: Some("3"),
        min: Some(0.0),
        max: Some(100.0),
        step: Some(0.5),
        doc: Some("width visible OUTSIDE each glyph; the emitted stroke is doubled, since the fill covers the inner half"),
        ..ParamSpec::new("border-width", xsvg_schema::ParamKind::Number)
    },
    ParamSpec {
        ns: Some(XSVG_NS),
        default: Some("#000000"),
        doc: Some("border paint; accepts a var(token)"),
        ..ParamSpec::new("border-color", xsvg_schema::ParamKind::Color)
    },
    ParamSpec {
        ns: Some(XSVG_NS),
        min: Some(0.0),
        max: Some(1.0),
        step: Some(0.05),
        doc: Some("optional border opacity"),
        ..ParamSpec::new("border-opacity", xsvg_schema::ParamKind::Number)
    },
];

// ---- helpers ---------------------------------------------------------------

/// The font block every text element accepts, declared beside the code that
/// reads it.
///
/// This is the one group that is **described here rather than derived**, because
/// its reading is not a plain attribute lookup: each property falls back to an
/// `<x:font>` theme token (§4.1), sizes are only honoured when positive, and
/// `letter-spacing`/`word-spacing` treat `normal` as zero. Keeping the
/// declaration touching [`style_from`] is the next best thing to deriving it —
/// the two are a screen apart, not a crate apart.
pub(super) const FONT_PARAMS: &[xsvg_schema::ParamSpec] = &[
    ParamSpec {
        default: Some("sans-serif"),
        // Naming a face is not an attribute edit like the others: the face has to be
        // *loaded* before a single glyph can be measured in it, and that is
        // asynchronous and document-wide. A host that offered a field here would lay
        // the whole document out in a fallback face and show the result as if it were
        // the real one — and outline baking would silently stop working besides. Until
        // a host can await font loading as part of applying an edit, this is
        // read-only.
        read_only: Some("changing the face needs it loaded first, which is asynchronous and affects the whole document"),
        ..ParamSpec::new("font-family", xsvg_schema::ParamKind::Text)
    },
    ParamSpec {
        default: Some("16"),
        min: Some(0.0),
        max: Some(400.0),
        ..ParamSpec::new("font-size", xsvg_schema::ParamKind::Number)
    },
    ParamSpec {
        default: Some("normal"),
        ..ParamSpec::new("font-weight", xsvg_schema::ParamKind::Text)
    },
    ParamSpec {
        default: Some("normal"),
        ..ParamSpec::new("font-style", xsvg_schema::ParamKind::Text)
    },
    ParamSpec {
        default: Some("1.2"),
        min: Some(0.0),
        max: Some(10.0),
        step: Some(0.05),
        ..ParamSpec::new("line-height", xsvg_schema::ParamKind::Number)
    },
    ParamSpec {
        doc: Some("extra tracking in user units; `normal` means none"),
        ..ParamSpec::new("letter-spacing", xsvg_schema::ParamKind::Text)
    },
    ParamSpec {
        doc: Some("extra space at each word gap; `normal` means none"),
        ..ParamSpec::new("word-spacing", xsvg_schema::ParamKind::Text)
    },
];

const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// An authored string made safe to place between the quotes of an output attribute.
///
/// Every value an author (or a theme token, or a generator) supplies is untrusted *as
/// markup*: formatted in raw, a value containing `"` closes the attribute and can open
/// a new one — `fill='#000" onload="…'` came out as a live handler, which is exactly
/// what the static subset exists to rule out. `tests/attribute_injection.rs` probes
/// every attribute of every dataset sample for this.
/// `none`, or an alignment keyword optionally followed by `meet`/`slice` — the
/// whole grammar of `preserveAspectRatio` (SVG 2 §8.2).
fn valid_preserve_aspect_ratio(v: &str) -> bool {
    const ALIGN: [&str; 10] = [
        "none", "xMinYMin", "xMidYMin", "xMaxYMin", "xMinYMid", "xMidYMid", "xMaxYMid", "xMinYMax",
        "xMidYMax", "xMaxYMax",
    ];
    let mut it = v.split_ascii_whitespace();
    match (it.next(), it.next(), it.next()) {
        (Some(a), None, None) => ALIGN.contains(&a),
        (Some(a), Some(m), None) => ALIGN.contains(&a) && matches!(m, "meet" | "slice"),
        _ => false,
    }
}

pub(super) fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    push_escaped(&mut out, v, true);
    out
}

/// Copy a node's attributes, normalizing namespaces: `x:` attrs are consumed (not
/// copied), `xlink:*` modernizes to the unprefixed SVG 2 form (`xlink:href` →
/// `href` — supported by every current renderer, and we declare no xlink xmlns),
/// `xml:*` keeps its reserved prefix (`xml:space`, `xml:lang`), and other foreign
/// namespaces drop (editor metadata we can't re-emit faithfully). `skip` filters
/// by local name.
fn copy_attrs(node: roxmltree::Node, out: &mut String, skip: &[&str], ctx: &Ctx) {
    for attr in node.attributes() {
        if skip.contains(&attr.name()) {
            continue;
        }
        // event handlers are outside the static subset (§9, Plan R6)
        if attr.name().starts_with("on") && attr.namespace().is_none() {
            continue;
        }
        let prefix = match attr.namespace() {
            None => "",
            Some(XLINK_NS) => "",
            Some(XML_NS) => "xml:",
            Some(_) => continue, // x: (consumed) and foreign metadata
        };
        // Generator output is confined after lowering (plugin.rs `confine`), so this
        // stays the plain rule set every document gets.
        let value = ctx.resolve_var(attr.value());
        out.push(' ');
        out.push_str(prefix);
        out.push_str(attr.name());
        out.push_str("=\"");
        push_escaped(out, &value, true);
        out.push('"');
    }
    // Font token (§4.1): apply the named `<x:font>`'s props the element did not
    // set itself — an overridable base. (Only fires when `x:font` is present;
    // harmless on non-text elements, which ignore font-*.)
    if let Some(props) = ctx.font_props(node) {
        for (k, v) in props {
            if node.attribute(k.as_str()).is_none() && !skip.contains(&k.as_str()) {
                out.push(' ');
                out.push_str(&k);
                out.push_str("=\"");
                push_escaped(out, &ctx.resolve_var(&v), true);
                out.push('"');
            }
        }
    }
}

fn attr_num(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    node.attribute(name).and_then(parse_num).unwrap_or(default)
}

/// Read an `x:`-namespaced numeric attribute (e.g. `x:glyph-x-scale` on a plain
/// SVG element), falling back to `default`.
fn attr_num_ns(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    node.attribute((XSVG_NS, name))
        .and_then(parse_num)
        .unwrap_or(default)
}

/// `attr_num` but falls back to `default` for non-positive values (e.g. a 0 or
/// negative `font-size`), keeping degenerate input out of the layout math.
fn attr_pos(node: roxmltree::Node, name: &str, default: f64) -> f64 {
    let v = attr_num(node, name, default);
    if v > 0.0 {
        v
    } else {
        default
    }
}

/// Reject pathologically nested input *before* the recursive XML parser sees it, so
/// deep nesting returns a clean error instead of overflowing the stack. A single
/// left-to-right scan tracking element depth; comments / CDATA / PIs / declarations
/// and self-closing tags do not add depth. This is a safety bound, not a validator —
/// `roxmltree` still does the real parsing (and rejects malformed input) afterward.
fn check_nesting_depth(input: &str, max: usize) -> Result<(), String> {
    let mut rest = input;
    let mut depth = 0usize;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map(|j| &after[j + 3..]).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("![CDATA[") {
            rest = after.find("]]>").map(|j| &after[j + 3..]).unwrap_or("");
        } else if rest.starts_with('!') || rest.starts_with('?') {
            rest = rest.find('>').map(|j| &rest[j + 1..]).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix('/') {
            depth = depth.saturating_sub(1);
            rest = after.find('>').map(|j| &after[j + 1..]).unwrap_or("");
        } else {
            let end = rest.find('>').unwrap_or(rest.len());
            if !rest[..end].ends_with('/') {
                depth += 1;
                if depth > max {
                    return Err(format!("xsvg: element nesting depth exceeds {max}"));
                }
            }
            rest = rest.get(end + 1..).unwrap_or("");
        }
    }
    Ok(())
}

/// Parse a leading numeric value, tolerating a trailing unit (e.g. `"13px"` → 13.0).
/// Rejects non-finite results (`inf`/`NaN`).
fn parse_num(s: &str) -> Option<f64> {
    let s = s.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(s.len());
    s[..end].parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Format a float without a trailing `.0` for whole numbers (tidier path/coords).
fn fmt(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.3}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Minimal XML escaping. `in_attr` also escapes double quotes.
fn push_escaped(out: &mut String, s: &str, in_attr: bool) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if in_attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests;

/// One parse per flush: the editor's compile session (docs/Compile Session.md).
mod session;
use crate::MeasureCache;
pub use session::CompileSession;

// ---- feature modules (split out of this file) ----
mod entries;
pub use entries::*;
mod theme;
use theme::*;
mod refs;
use refs::*;
mod link;
use link::*;
mod runs;
use runs::*;
mod textbox;
use textbox::*;
pub use textbox::{TextLayout, TextLayoutLine, TextLayoutPara};
mod textpath;
use textpath::*;
mod connector;
use connector::*;
mod table;
use table::*;
mod charts;
use charts::*;
mod list;
use list::*;
mod mesh;
use mesh::*;
mod pathops;
use pathops::*;
pub mod elements;
use elements::params_of;
// so submodules reach `.as_str()` on a declared enum parameter through `super::*`
pub(crate) use xsvg_schema::{ParamEnum, ParamSpec};
pub mod plugin;
use plugin::{check_import, emit_plugin, GeneratorHost, ImportTable, NoGenerators};
