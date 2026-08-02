//! Declarations for the built-in `x:` elements (docs/Extensibility.md §11).
//!
//! Every element's parameters live here as a struct, and the derive generates
//! both the machine-readable description and the code that reads it — the same
//! mechanism a plugin uses for its manifest. That is what lets the compiler
//! answer `describe(ns, tag)` for its own elements, so an editor building
//! property controls never has to know which tier it is editing.
//!
//! A parameter with no field cannot be read, and a field nothing reads is a
//! `dead_code` warning, so the description cannot drift from the behaviour.

use xsvg_schema::{AttrSource, ElementSpec, ParamEnum, Params};

/// Read attributes straight off an XML node, so the derived parsing works
/// against the document with no intermediate copy.
pub(super) struct Attrs<'a, 'i>(pub roxmltree::Node<'a, 'i>);

impl AttrSource for Attrs<'_, '_> {
    fn attr(&self, name: &str) -> Option<&str> {
        self.0.attribute(name)
    }
}

/// Parse an element's declared parameters from its node.
pub(super) fn params_of<P: Params>(node: roxmltree::Node) -> P {
    P::from_attrs(&Attrs(node))
}

// ---- geometry -------------------------------------------------------------

#[derive(Params)]
#[element(
    name = "offset",
    doc = "Grow or shrink a referenced region by a distance (Minkowski offset).",
    input = "required"
)]
pub(super) struct Offset {
    #[param(default = 0.0, min = -1000.0, max = 1000.0, doc = "negative shrinks")]
    pub distance: f64,
    #[param(default = "round")]
    pub join: OffsetJoin,
    #[param(default = 4.0, min = 1.0, max = 100.0)]
    pub miter_limit: f64,
    #[param(default = "nonzero", doc = "how the referenced region reads its own winding")]
    pub fill_rule: FillRule,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum OffsetJoin {
    /// The true disc offset.
    #[default]
    Round,
    Miter,
    Bevel,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum FillRule {
    #[default]
    Nonzero,
    #[value("evenodd")]
    EvenOdd,
}

#[derive(Params)]
#[element(name = "boolean", doc = "Combine its operand shapes with a path-algebra operation.")]
pub(super) struct Boolean {
    /// Optional on purpose. A non-`Option` enum falls back to its default when a
    /// value is misspelled, which is right for a forgiving parameter like
    /// alignment — but `<x:boolean>` must degrade with a marker instead of
    /// silently unioning, so absence and "unknown spelling" have to stay
    /// distinguishable.
    #[param(default = "union")]
    pub op: Option<BooleanOp>,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum BooleanOp {
    #[default]
    Union,
    Intersect,
    Subtract,
    Exclude,
}

#[derive(Params)]
#[element(
    name = "warp",
    doc = "Distort its children through a field — a preset envelope, a bend along a spine, or free corner distortion."
)]
pub(super) struct Warp {
    #[param(doc = "envelope preset name; empty means none")]
    pub field: Option<String>,
    #[param(default = 0.0, min = -100.0, max = 100.0, doc = "percent")]
    pub bend: f64,
    #[param(default = "h")]
    pub axis: WarpAxisParam,
    #[param(default = 10.0, min = 0.0, max = 100.0, doc = "roughen detail")]
    pub detail: f64,
    #[param(default = 0.0, min = -100.0, max = 100.0, doc = "horizontal distortion, percent")]
    pub distort_h: f64,
    #[param(default = 0.0, min = -100.0, max = 100.0, doc = "vertical distortion, percent")]
    pub distort_v: f64,
    #[param(doc = "placement along the spine")]
    pub align: PathAlignParam,
    #[param(default = 0.0, doc = "offset along the spine")]
    pub start: f64,
    #[param(doc = "four x,y pairs for free distortion")]
    pub corners: Option<String>,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum WarpAxisParam {
    #[default]
    #[value("h")]
    Horizontal,
    #[value("v")]
    Vertical,
}

// ---- charts ---------------------------------------------------------------

#[derive(Params)]
#[element(name = "pie", doc = "A pie or donut chart built from <x:slice> children.")]
pub(super) struct Pie {
    #[param(default = 0.0, geometry)]
    pub cx: f64,
    #[param(default = 0.0, geometry)]
    pub cy: f64,
    #[param(default = 100.0, min = 0.0, max = 2000.0, geometry)]
    pub r: f64,
    #[param(default = 0.0, min = 0.0, max = 2000.0, doc = "non-zero makes a donut")]
    pub inner_radius: f64,
    #[param(default = -90.0, min = -360.0, max = 360.0, doc = "degrees; -90 starts at 12 o'clock")]
    pub start: f64,
    #[param(default = 0.0, min = 0.0, max = 45.0, doc = "degrees between slices")]
    pub gap: f64,
    #[param(color)]
    pub stroke: Option<String>,
    #[param(default = 0.0, min = 0.0, max = 40.0)]
    pub stroke_width: f64,
}

#[derive(Params)]
#[element(
    name = "plot",
    doc = "A cartesian plot frame for <x:bars> and <x:line> children.",
    input = "optional"
)]
pub(super) struct Plot {
    #[param(default = 0.0, geometry)]
    pub x: f64,
    #[param(default = 0.0, geometry)]
    pub y: f64,
    #[param(default = 400.0, min = 0.0, max = 4000.0, geometry)]
    pub width: f64,
    #[param(default = 200.0, min = 0.0, max = 4000.0, geometry)]
    pub height: f64,
    #[param(color, default = "#e2e8f0")]
    pub grid_color: String,
    #[param(default = 1.0, min = 0.0, max = 20.0)]
    pub grid_width: f64,
    #[param(color, default = "#64748b")]
    pub label_fill: String,
    #[param(default = 11.0, min = 0.0, max = 200.0)]
    pub label_size: f64,
    #[param(default = 0.0, min = 0.0, max = 20.0, doc = "0 draws no y axis labels")]
    pub y_ticks: f64,
}

// ---- mesh -----------------------------------------------------------------

#[derive(Params)]
#[element(name = "mesh", doc = "A gradient mesh, rasterized to a texel-aligned image.")]
pub(super) struct Mesh {
    #[param(default = 0.0, geometry)]
    pub x: f64,
    #[param(default = 0.0, geometry)]
    pub y: f64,
    #[param(default = 0.0, min = 0.0, max = 4000.0, geometry)]
    pub width: f64,
    #[param(default = 0.0, min = 0.0, max = 4000.0, geometry)]
    pub height: f64,
    #[param(default = 0.0, min = 0.0, max = 64.0, doc = "with rows, builds a uniform grid")]
    pub cols: f64,
    #[param(default = 0.0, min = 0.0, max = 64.0)]
    pub rows: f64,
}

// ---- connector ------------------------------------------------------------

#[derive(Params)]
#[element(name = "connector", doc = "A routed line between two elements, with arrowheads.")]
pub(super) struct Connector {
    #[param(doc = "#id of the element the line leaves")]
    pub from: Option<String>,
    #[param(doc = "#id of the element the line reaches")]
    pub to: Option<String>,
    #[param(doc = "which side to leave from: top, right, bottom, left")]
    pub from_point: Option<String>,
    #[param(doc = "which side to arrive at")]
    pub to_point: Option<String>,
    #[param(default = "straight")]
    pub route: ConnectorRoute,
    #[param(default = "end")]
    pub arrow: ConnectorArrow,
    #[param(min = 0.0, max = 100.0, doc = "defaults to 3.5x stroke-width, minimum 7")]
    pub arrow_size: Option<f64>,
    #[param(default = 44.0, min = 0.0, max = 500.0, doc = "curve bow, for route=curve")]
    pub bulge: f64,
    #[param(color, default = "#334155")]
    pub stroke: String,
    #[param(default = 1.0, min = 0.0, max = 40.0)]
    pub stroke_width: f64,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum ConnectorRoute {
    #[default]
    Straight,
    Curve,
    #[value("x-major")]
    XMajor,
    #[value("y-major")]
    YMajor,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum ConnectorArrow {
    #[default]
    End,
    Start,
    Both,
    None,
}

// ---- text -----------------------------------------------------------------

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum AlignParam {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum VAlignParam {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Placement along a path or spine. A **different vocabulary** from
/// [`AlignParam`]: `run_offset` centres on `middle`, where box alignment centres
/// on `center`. Declaring them separately is the point — one shared enum would
/// quietly accept the wrong spelling on half the elements.
#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum PathAlignParam {
    #[default]
    Start,
    Middle,
    End,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum OverflowParam {
    #[default]
    Clip,
    Ellipsis,
}

#[derive(ParamEnum, Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum FitParam {
    #[default]
    None,
    /// Shrink the font until the text fits its box.
    Shrink,
}

#[derive(Params)]
#[element(
    name = "textbox",
    doc = "Text laid out inside a box — wrapping, alignment, shrink-to-fit, and flow into a referenced shape.",
    input = "optional",
    group = TextFont,
    group = TextBorder
)]
pub(super) struct Textbox {
    // `in="#shape"` takes the box from the referenced shape instead (see
    // `textbox_area_spec`, which reads geometry from the target and only the layout
    // options from the textbox), so these four stop governing the moment it is set.
    #[param(default = 0.0, superseded_by = "in", geometry)]
    pub x: f64,
    #[param(default = 0.0, superseded_by = "in", geometry)]
    pub y: f64,
    #[param(default = 0.0, min = 0.0, max = 4000.0, superseded_by = "in", geometry)]
    pub width: f64,
    #[param(default = 0.0, min = 0.0, max = 4000.0, superseded_by = "in", geometry)]
    pub height: f64,
    #[param(default = 0.0, min = 0.0, max = 400.0)]
    pub padding: f64,
    pub align: AlignParam,
    pub valign: VAlignParam,
    pub fit: FitParam,
    #[param(default = 6.0, min = 1.0, max = 400.0, doc = "smallest font size shrink-to-fit will use")]
    pub fit_min: f64,
    #[param(name = "text-overflow")]
    pub text_overflow: OverflowParam,
    #[param(default = 0.0, min = -400.0, max = 400.0, doc = "first-line indent")]
    pub text_indent: f64,
    #[param(default = false, doc = "bake the glyphs to paths instead of live text")]
    pub outline: bool,
}

/// The shared font block. Described beside [`super::style_from`], which reads it —
/// see the comment there for why this one group is not derived.
pub(super) struct TextFont;

impl Params for TextFont {
    const SPEC: ElementSpec = ElementSpec {
        name: "font",
        doc: None,
        params: super::FONT_PARAMS,
        groups: &[],
        input: xsvg_schema::InputMode::None,
        children: xsvg_schema::ChildMode::None,
    };
    fn from_attrs(_: &dyn AttrSource) -> Self {
        // style_from() does the reading: it needs the node for theme-token fallback.
        TextFont
    }
}

/// The bordered-text block (§6.17), on the elements whose emitters read it —
/// `<x:textbox>` and `<x:textpath>`. Described beside [`super::text_border_attrs`].
///
/// Declaring it is what puts it in the property panel at all: the compiler reads
/// these straight off the node, so before this they worked but nothing could
/// *report* them, and an editor had no way to know they existed.
pub(super) struct TextBorder;

impl Params for TextBorder {
    const SPEC: ElementSpec = ElementSpec {
        name: "border",
        doc: None,
        params: super::TEXT_BORDER_PARAMS,
        groups: &[],
        input: xsvg_schema::InputMode::None,
        children: xsvg_schema::ChildMode::None,
    };
    fn from_attrs(_: &dyn AttrSource) -> Self {
        // text_border_attrs() does the reading: it needs the node to look the
        // attributes up by namespace rather than by written prefix.
        TextBorder
    }
}

#[derive(Params)]
#[element(
    name = "textpath",
    doc = "Text set along a referenced path.",
    input = "required",
    group = TextFont,
    group = TextBorder
)]
pub(super) struct Textpath {
    #[param(default = 0.0)]
    pub start: f64,
    pub align: PathAlignParam,
    #[param(default = 0.0, min = -400.0, max = 400.0)]
    pub baseline_shift: f64,
    #[param(default = "skew", doc = "skew or rainbow")]
    pub effect: String,
}

#[derive(Params)]
#[element(
    name = "list",
    doc = "A bulleted or numbered list of <x:li> items, with hanging indents that cycle by depth.",
    input = "optional",
    group = TextFont
)]
pub(super) struct List {
    #[param(default = 0.0, geometry)]
    pub x: f64,
    #[param(default = 0.0, geometry)]
    pub y: f64,
    #[param(default = 320.0, min = 0.0, max = 4000.0, geometry)]
    pub width: f64,
    #[param(default = "bullet", doc = "bullet or number; marker glyphs cycle by depth")]
    pub list: String,
    #[param(doc = "literal marker text for every item, overriding the list style")]
    pub marker: Option<String>,
    #[param(color, doc = "defaults to the list's own fill")]
    pub marker_fill: Option<String>,
    #[param(default = 1.0, min = 0.0, max = 10.0, step = 0.1, doc = "marker size as a multiple of the item size")]
    pub marker_size: f64,
    #[param(min = 0.0, max = 200.0, doc = "gap between marker and text; defaults to 0.5em")]
    pub marker_gap: Option<f64>,
    #[param(min = 0.0, max = 400.0, doc = "indent step per level; defaults to 1.5em")]
    pub indent: Option<f64>,
    #[param(min = 0.0, max = 200.0, doc = "space between items; defaults to 0.35em")]
    pub item_spacing: Option<f64>,
    pub valign: VAlignParam,
    #[param(color, default = "#111827")]
    pub fill: String,
}

#[derive(Params)]
#[element(
    name = "table",
    doc = "A grid of <x:tr>/<x:td> cells with shared padding, rules, and header styling.",
    input = "optional",
    group = TextFont
)]
pub(super) struct Table {
    #[param(default = 0.0, geometry)]
    pub x: f64,
    #[param(default = 0.0, geometry)]
    pub y: f64,
    #[param(default = 400.0, min = 0.0, max = 4000.0, geometry)]
    pub width: f64,
    #[param(doc = "explicit column widths, space or comma separated")]
    pub cols: Option<String>,
    #[param(default = 8.0, min = 0.0, max = 200.0)]
    pub cell_padding: f64,
    #[param(default = 0.0, min = 0.0, max = 400.0)]
    pub row_min_height: f64,
    pub align: AlignParam,
    /// Cells centre vertically by default, unlike a textbox.
    #[param(default = "middle")]
    pub valign: VAlignParam,
    #[param(color, default = "#0f172a")]
    pub fill: String,
    #[param(color, default = "#f1f5f9")]
    pub header_fill: String,
    #[param(color, default = "#cbd5e1", doc = "rule colour")]
    pub stroke: String,
    #[param(default = 1.0, min = 0.0, max = 40.0)]
    pub stroke_width: f64,
}

/// Every built-in element, for the compiler's own `describe` surface — the same
/// shape a plugin's manifest takes, so a host merges the two without branching.
pub fn built_in_elements() -> &'static [ElementSpec] {
    ELEMENTS
}

/// The built-in elements as a manifest, in the shape a plugin module answers
/// with. A property panel therefore reads both tiers through one path and never
/// has to know which it is editing.
pub fn describe_impl() -> String {
    xsvg_schema::manifest_json(1, super::XSVG_NS, "xsvg", env!("CARGO_PKG_VERSION"), ELEMENTS)
}

static ELEMENTS: &[ElementSpec] = &[
    Offset::SPEC,
    Boolean::SPEC,
    Warp::SPEC,
    Pie::SPEC,
    Plot::SPEC,
    Mesh::SPEC,
    Connector::SPEC,
    Textbox::SPEC,
    Textpath::SPEC,
    List::SPEC,
    Table::SPEC,
];
