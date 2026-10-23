//! The compiler's entry points.
//!
//! One compile is a document, a [`Platform`] and an [`Options`]; [`compile_with`]
//! is the entry, and every other `*_impl` here is it with some of the platform
//! inert or the options fixed — kept because the CLI, the wasm adapter and the
//! tests call them by name, and because a `&dyn` signature is what a host outside
//! Rust binds to. They stay thin: each one builds a platform and calls through.
//! The parse-only entries (`node_spans_impl`, `fragment_range_impl`, the
//! dependents) and the text-layout seam are here too, as one-shot sessions.

use super::session::Parsed;
use super::ToPathError;
use super::{
    CaretStops, CompileSession, Host, NoKeys, NoResolver, Platform, TextLayout, TextboxSize,
    UnitSpan,
};
use super::{NodeKeys, Resolver};
use crate::{GlyphOutliner, Measurer, Shaper};

/// What a compile is asked for, beyond the document and the platform.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// `fast` | `balanced` | `highest` | `raster` (§7.1); anything else reads as
    /// `balanced`.
    pub quality: &'a str,
    /// Emit source positions — `data-xsvg-pos="start-end"`, or `data-xsvg-node`
    /// under a keyed platform — on every top-level element, so a viewer can
    /// project rendered elements back to their source.
    pub sourcemap: bool,
    /// The entry document's canonical key: relative cross-file `<use>` hrefs
    /// resolve against it, and a dependency linking back to it is a cycle (§4).
    pub base: &'a str,
    /// `Some(offset)`: re-emit only the top-level element containing that byte —
    /// byte-identical to its span in the whole compile (docs/Incremental.md).
    /// `None`: the whole document.
    pub fragment: Option<usize>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Self {
            quality: "balanced",
            sourcemap: false,
            base: "",
            fragment: None,
        }
    }
}

/// Compile `input` on `platform` as `options` asks. The one entry; a one-shot
/// [`CompileSession`].
pub fn compile_with(
    input: &str,
    options: &Options,
    platform: &impl Host,
) -> Result<String, String> {
    let session = CompileSession::new(
        platform,
        input,
        options.quality,
        options.sourcemap,
        options.base,
    )?;
    match options.fragment {
        Some(offset) => session.compile_fragment(offset),
        None => session.compile(),
    }
}

/// The platform the `&dyn` entries compile on: the three font seams, plus a
/// resolver and keys where the entry takes them. `&dyn Trait` implements `Trait`,
/// so these are ordinary platforms over borrowed adapters.
#[allow(clippy::type_complexity)]
fn borrowed<'a>(
    m: &'a dyn Measurer,
    shaper: &'a dyn Shaper,
    outliner: &'a dyn GlyphOutliner,
    resolver: &'a dyn Resolver,
    keys: &'a dyn NodeKeys,
) -> Platform<
    &'a dyn Measurer,
    &'a dyn Shaper,
    &'a dyn GlyphOutliner,
    &'a dyn Resolver,
    &'a dyn NodeKeys,
> {
    Platform::new(m, shaper, outliner)
        .with_resolver(resolver)
        .with_keys(keys)
}

/// Pure compile entry: no wasm/JS types, so it is unit-testable on native targets.
/// Cross-file `<use>` links degrade (no resolver) — use [`compile_linked_impl`] to link.
pub fn compile_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_linked_impl(
        input,
        quality,
        sourcemap,
        m,
        shaper,
        outliner,
        &NoResolver,
        "",
    )
}

/// Compile entry with cross-file `<use>` linking: `resolver` fetches dependencies and
/// `base` is the entry document's canonical key (relative hrefs resolve against it, and
/// a dependency that links back to it is a cycle). See [`Resolver`].
#[allow(clippy::too_many_arguments)]
pub fn compile_linked_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
) -> Result<String, String> {
    compile_linked_keyed_impl(
        input, quality, sourcemap, m, shaper, outliner, resolver, base, &NoKeys,
    )
}

/// Keyed compile without cross-file linking — the editor-host entry ([`NodeKeys`]).
pub fn compile_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    keys: &dyn NodeKeys,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_linked_keyed_impl(
        input,
        quality,
        sourcemap,
        m,
        shaper,
        outliner,
        &NoResolver,
        "",
        keys,
    )
}

/// [`compile_linked_impl`] with host-supplied node keys ([`NodeKeys`]).
#[allow(clippy::too_many_arguments)]
pub fn compile_linked_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
    keys: &dyn NodeKeys,
) -> Result<String, String> {
    compile_with(
        input,
        &Options {
            quality,
            sourcemap,
            base,
            fragment: None,
        },
        &borrowed(m, shaper, outliner, resolver, keys),
    )
}

/// The full compile entry: a whole [`Platform`], generators included
/// (docs/Extensibility.md). The older entry points are this one with inert
/// defaults.
pub fn compile_platform_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    base: &str,
    platform: &impl Host,
) -> Result<String, String> {
    compile_with(
        input,
        &Options {
            quality,
            sourcemap,
            base,
            fragment: None,
        },
        platform,
    )
}

/// Incremental compilation (docs/Incremental.md): re-emit only the top-level
/// element containing byte `offset`. Emission is a pure function of the subtree
/// plus anything it references, so the result is **byte-identical to the span the
/// full compile would produce** for that element (enforced by test) — the caller
/// can splice it over the previous output surgically. Errors when `offset` doesn't
/// fall inside a top-level element.
pub fn compile_fragment_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_fragment_linked_impl(
        input,
        quality,
        sourcemap,
        offset,
        m,
        shaper,
        outliner,
        &NoResolver,
        "",
    )
}

/// [`compile_fragment_impl`] with cross-file linking, so an incremental re-emit of a
/// `<use href="file.svg">` stays byte-identical to its span in the full linked compile
/// (the fragment invariant, docs/Incremental.md). Same `resolver`/`base` as
/// [`compile_linked_impl`].
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_linked_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
) -> Result<String, String> {
    compile_fragment_linked_keyed_impl(
        input, quality, sourcemap, offset, m, shaper, outliner, resolver, base, &NoKeys,
    )
}

/// Keyed fragment compile without cross-file linking — the editor-host entry.
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    keys: &dyn NodeKeys,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, String> {
    compile_fragment_linked_keyed_impl(
        input,
        quality,
        sourcemap,
        offset,
        m,
        shaper,
        outliner,
        &NoResolver,
        "",
        keys,
    )
}

/// [`compile_fragment_linked_impl`] with host-supplied node keys ([`NodeKeys`]).
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_linked_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
    keys: &dyn NodeKeys,
) -> Result<String, String> {
    compile_with(
        input,
        &Options {
            quality,
            sourcemap,
            base,
            fragment: Some(offset),
        },
        &borrowed(m, shaper, outliner, resolver, keys),
    )
}

/// The full incremental entry — [`compile_platform_impl`]'s counterpart. A fragment
/// containing a plugin element re-emits byte-identically to its span in the full
/// compile, which is what makes generator purity a hard requirement (§6).
pub fn compile_fragment_platform_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    base: &str,
    platform: &impl Host,
) -> Result<String, String> {
    compile_with(
        input,
        &Options {
            quality,
            sourcemap,
            base,
            fragment: Some(offset),
        },
        platform,
    )
}

/// Source byte range of **every** element in the document, in document order
/// (the root included). Parse-only — nothing is lowered.
///
/// The companion to [`NodeKeys`]: that seam requires a host to key every element,
/// not just top-level fragments, and a host outside Rust has no other way to learn
/// what the elements *are*. It also gives such a host the key→range table it needs
/// to keep projecting rendered elements back to source once identity stops being a
/// byte range.
pub fn node_spans_impl(input: &str) -> Vec<(usize, usize)> {
    Parsed::new(input)
        .map(|p| p.node_spans())
        .unwrap_or_default()
}

/// Compile one incremental **unit** (docs/Incremental.md §9): the direct child of a
/// top-level passthrough container containing `offset`, else the top-level element
/// itself. Byte-identical to the unit's span in its fragment's output.
pub fn compile_unit_platform_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    base: &str,
    platform: &impl Host,
) -> Result<String, String> {
    CompileSession::new(platform, input, quality, sourcemap, base)?.compile_unit(offset)
}

/// [`compile_unit_platform_impl`] over the `&dyn` seams a wasm host binds.
#[allow(clippy::too_many_arguments)]
pub fn compile_unit_linked_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
    keys: &dyn NodeKeys,
) -> Result<String, String> {
    compile_unit_platform_impl(
        input,
        quality,
        sourcemap,
        offset,
        base,
        &borrowed(m, shaper, outliner, resolver, keys),
    )
}

/// The fragment containing `offset` plus where each of its units landed in it
/// (docs/Incremental.md §9), in output order. The span list is empty when the
/// top-level element is its own unit.
pub fn compile_fragment_spans_platform_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    base: &str,
    platform: &impl Host,
) -> Result<(String, Vec<UnitSpan>), String> {
    CompileSession::new(platform, input, quality, sourcemap, base)?.compile_fragment_spans(offset)
}

/// [`compile_fragment_spans_platform_impl`] over the `&dyn` seams a wasm host binds.
#[allow(clippy::too_many_arguments)]
pub fn compile_fragment_spans_linked_keyed_impl(
    input: &str,
    quality: &str,
    sourcemap: bool,
    offset: usize,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
    resolver: &dyn Resolver,
    base: &str,
    keys: &dyn NodeKeys,
) -> Result<(String, Vec<UnitSpan>), String> {
    compile_fragment_spans_platform_impl(
        input,
        quality,
        sourcemap,
        offset,
        base,
        &borrowed(m, shaper, outliner, resolver, keys),
    )
}

/// Source byte range of the unit containing `offset` (docs/Incremental.md §9).
pub fn unit_range_impl(input: &str, offset: usize) -> Option<(usize, usize)> {
    Parsed::new(input).ok()?.unit_range(offset)
}

/// [`dependents_multi_impl`] at unit granularity: seeds and results are unit
/// source ranges, so a dependent inside the same container is reported.
pub fn unit_dependents_impl(input: &str, seeds: &[usize]) -> Vec<(usize, usize)> {
    Parsed::new(input)
        .map(|p| p.unit_dependents(seeds))
        .unwrap_or_default()
}

/// Source byte range of the fragment unit containing `offset` (for the caller's
/// identity bookkeeping — ranges shift by the edit delta between compiles).
pub fn fragment_range_impl(input: &str, offset: usize) -> Option<(usize, usize)> {
    Parsed::new(input).ok()?.fragment_range(offset)
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
pub fn dependents_impl(input: &str, offset: usize) -> Vec<(usize, usize)> {
    dependents_multi_impl(input, &[offset])
}

/// [`dependents_impl`] seeded from several fragments at once (one parse, one
/// union fixpoint) — a committed editor transaction can dirty k fragments, and
/// their dependents overlap. Seed ranges themselves are excluded from the
/// result. Results are in document order.
pub fn dependents_multi_impl(input: &str, seeds: &[usize]) -> Vec<(usize, usize)> {
    Parsed::new(input)
        .map(|p| p.dependents(seeds))
        .unwrap_or_default()
}

/// Text-layout geometry for the `<x:textbox>` whose source range starts at
/// byte `offset` (the host's span table knows it — the same convention as
/// [`compile_fragment_impl`], but any element depth). Returns `None` when the
/// offset is not a textbox, the document doesn't parse, or the box resolves to
/// nothing renderable (broken `in=` reference, un-rasterizable region).
///
/// The geometry is byte-for-byte the emitter's: rectangular and paragraph
/// flows share their layout code with emission, and the dispatch mirrors
/// `emit_textbox` — pinned by the `text_layout_matches_emitted_tspans` canary.
pub fn text_layout_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Option<TextLayout> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .ok()?
        .text_layout(offset)
}

/// Caret stops for the outlined text element whose source range starts at byte
/// `offset` (docs/Incremental.md §6; consumer: xsvg-kit): an `<x:textbox
/// outline="true">` — inside any number of `<x:warp>`s or none — or an
/// `<x:textpath>` with a geometric effect. The stops are recorded by the compile
/// itself, so they sit between the glyphs the same fields drew. `None` when the
/// element renders live text instead (no outline, or the outliner declined —
/// the host's `charGeometry` case), is not text, or the document doesn't parse.
pub fn caret_stops_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Option<CaretStops> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .ok()?
        .caret_stops(offset)
}

/// How big the `<x:textbox>` whose source range starts at byte `offset` must be
/// to hold its content (docs/Incremental.md §8; consumer: xsvg-kit): laid out at
/// its authored width with height unbounded, `valign` top, no shrink and no
/// ellipsis, the outer height is the cap band of the stacked blocks plus padding
/// and the outer width the widest line plus padding (at most the authored width).
/// An editor pastes a block with `width` set and a placeholder `height`, asks,
/// and writes the answer back in the same transaction. `None` when the offset is
/// not a rectangular textbox (curved region flow has no height to grow into).
pub fn measure_textbox_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Option<TextboxSize> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .ok()?
        .measure_textbox(offset)
}

/// Convert to path (docs/Incremental.md §11; consumer: xsvg-kit): the element
/// whose source range starts at byte `offset`, compiled on its own with
/// outlining forced, as the markup an editor writes back in its place — glyph
/// paths and baked backdrops under one group that keeps every attribute the
/// conversion did not spend (`id`, `x:label`, `transform`, `opacity`, a
/// `filter` list, `var()` paints), or a basic shape as its `<path>`. `Err`
/// names the font the outliner had no bytes for, or the limit that stopped it.
pub fn to_path_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, ToPathError> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .map_err(ToPathError::limit)?
        .to_path(offset)
}

/// The operands of the `<x:boolean>` at `offset` for `xsvg-path-ops`
/// (docs/Incremental.md §11; consumer: xsvg-kit): see
/// [`crate::compile::CompileSession::boolean_operands`]. `None` when the offset
/// is not an `<x:boolean>` with a known op and usable geometry.
pub fn boolean_operands_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Option<super::BooleanOperands> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .ok()?
        .boolean_operands(offset)
}

/// Convert to spline (docs/Incremental.md §11): [`to_path_impl`] with every arc
/// of the result expanded to cubic Béziers, for an editor whose path model has
/// no arcs — a circle as four cubics at its extremes, a rounded corner as one.
pub fn to_spline_impl(
    input: &str,
    offset: usize,
    quality: &str,
    m: &dyn Measurer,
    shaper: &dyn Shaper,
    outliner: &dyn GlyphOutliner,
) -> Result<String, ToPathError> {
    Platform::new(m, shaper, outliner)
        .session(input, quality, false, "")
        .map_err(ToPathError::limit)?
        .to_spline(offset)
}
