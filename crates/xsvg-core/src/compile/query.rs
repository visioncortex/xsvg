//! Document queries (xsvg-kit docs/Queries.md): what the compiler does with a
//! node, asked of the parse it does it from. An editor's chrome needs the same
//! answers the emitters compute — the font a box resolves to, the style of the
//! run under the caret, which run attributes an element draws — and copying the
//! rules into the editor is how the two drift apart. So the rules live here
//! once, as data the emitters compute from and the queries report:
//!
//! - **The cascade** ([`Level`]): the ordered places a text property may come
//!   from — the element, its `<x:font>` token, a paragraph or list over its box,
//!   a cell over its row, column and table, the ancestors `font-family` alone
//!   inherits from. [`style_from`], the paragraph planner, the list and table
//!   emitters and [`resolved`] all walk the same levels, so the style a query
//!   reports is the style that was laid out.
//! - **Run support** ([`run_support`]): which run attributes an element's
//!   emitter draws.
//!
//! A value counts at a level only when it parses (CSS's rule: an invalid
//! declaration is ignored, and the cascade falls through).

use super::*;

/// The properties a text style is made of — the eight `<x:font>` token
/// properties, then the paint every text front-end takes.
pub(super) const STYLE_PROPS: [&str; 9] = [
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "text-transform",
    "fill",
];

/// What an element reads of its own: every style property, token first.
const ELEMENT: &[&str] = &STYLE_PROPS;
/// What a paragraph or list inside a box reads of its own over the box's: the
/// type, its spacing and its colour, including its font token.
const BLOCK: &[&str] = &[
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-transform",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "fill",
];
/// What a cell, its row and its column read over the table (§6.15): the block
/// properties and the cell's alignment — `<x:col align="center">` is a column's
/// alignment, and the table's is only the fallback.
const CELL: &[&str] = &[
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-transform",
    "line-height",
    "letter-spacing",
    "word-spacing",
    "fill",
    "align",
    "valign",
    "direction",
];
/// What an ancestor above the text element gives it: the family alone (§4.1).
const INHERITED: &[&str] = &["font-family"];

/// The compiler's value for a style property nothing set and no declaration
/// defaults.
fn initial(prop: &str) -> Option<&'static str> {
    Some(match prop {
        "font-family" => "sans-serif",
        "font-size" => "16",
        "font-weight" | "font-style" | "letter-spacing" | "word-spacing" => "normal",
        "line-height" => "1.2",
        "text-transform" => "none",
        "fill" => "#000",
        _ => return None,
    })
}

/// A `letter-spacing` / `word-spacing` value (§6.8): `normal` (zero) or a
/// number of user units, `px` being the same unit. Anything else — `em`, `%`,
/// a `var()` — is no value at all, never its leading number.
pub(super) fn spacing_value(value: &str) -> Option<f64> {
    let v = value.trim();
    if v == "normal" {
        return Some(0.0);
    }
    v.strip_suffix("px")
        .unwrap_or(v)
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

/// Whether `value` counts for `prop` at a level, or the cascade falls through.
fn valid(prop: &str, value: &str) -> bool {
    match prop {
        "font-size" | "line-height" => parse_num(value).is_some_and(|n| n > 0.0),
        "letter-spacing" | "word-spacing" => spacing_value(value).is_some(),
        "text-transform" => TextTransform::parse(value).is_some(),
        _ => true,
    }
}

/// One step of a text cascade.
#[derive(Clone, Copy)]
pub(super) enum Level<'a> {
    /// `node`'s own attributes for `props`, then its `<x:font>` token's.
    Attrs {
        node: roxmltree::Node<'a, 'a>,
        props: &'static [&'static str],
        token: Option<(&'a str, &'a [(String, String)])>,
    },
    /// A value the emitter supplies on `node`'s behalf (a header cell's bold).
    Fixed {
        node: roxmltree::Node<'a, 'a>,
        prop: &'static str,
        value: &'static str,
    },
}

/// Where a level's value came from.
#[derive(Clone, Copy)]
enum Origin<'a> {
    Attr,
    Token(&'a str),
    Fixed,
}

impl<'a> Level<'a> {
    fn node(&self) -> roxmltree::Node<'a, 'a> {
        match self {
            Level::Attrs { node, .. } | Level::Fixed { node, .. } => *node,
        }
    }

    fn value(&self, prop: &str) -> Option<(&'a str, Origin<'a>)> {
        match *self {
            Level::Attrs { node, props, token } => {
                if !props.contains(&prop) {
                    return None;
                }
                if let Some(v) = node.attribute(prop).filter(|v| valid(prop, v)) {
                    return Some((v, Origin::Attr));
                }
                let (name, tok) = token?;
                tok.iter()
                    .find(|(p, v)| p == prop && valid(prop, v))
                    .map(|(_, v)| (v.as_str(), Origin::Token(name)))
            }
            Level::Fixed { prop: p, value, .. } => (p == prop).then_some((value, Origin::Fixed)),
        }
    }
}

/// The first level that sets `prop`.
fn find<'a>(levels: &[Level<'a>], prop: &str) -> Option<(&'a str, Origin<'a>, Level<'a>)> {
    levels
        .iter()
        .find_map(|l| l.value(prop).map(|(v, o)| (v, o, *l)))
}

/// `node`'s `<x:font>` token, when it names one the theme has.
fn token_of<'a>(
    node: roxmltree::Node<'a, 'a>,
    theme: &'a Theme,
) -> Option<(&'a str, &'a [(String, String)])> {
    let name = node.attribute((XSVG_NS, "font"))?;
    theme.fonts.get(name).map(|props| (name, props.as_slice()))
}

/// A text element's own cascade: its attributes and token, then every
/// ancestor's family (attribute, then token) — what [`style_from`] reads.
pub(super) fn element_levels<'a>(
    node: roxmltree::Node<'a, 'a>,
    theme: &'a Theme,
) -> Vec<Level<'a>> {
    let mut levels = vec![Level::Attrs {
        node,
        props: ELEMENT,
        token: token_of(node, theme),
    }];
    // `ancestors()` ends at this document's root: a linked file never sees its
    // host's family
    for a in node.ancestors().skip(1).filter(|a| a.is_element()) {
        levels.push(Level::Attrs {
            node: a,
            props: INHERITED,
            token: token_of(a, theme),
        });
    }
    levels
}

fn is_x(node: roxmltree::Node, local: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(XSVG_NS)
        && node.tag_name().name() == local
}

/// The levels a node inside a text element reads before the element's own —
/// a paragraph or list over its box, an item over its list, a cell over its
/// row, column and table — and that element. `None` for a node that is its
/// own text element (or none).
pub(super) fn block_levels<'a>(
    node: roxmltree::Node<'a, 'a>,
    theme: Option<&'a Theme>,
) -> Option<(Vec<Level<'a>>, roxmltree::Node<'a, 'a>)> {
    let attrs = |node, props| Level::Attrs {
        node,
        props,
        token: theme.and_then(|t| token_of(node, t)),
    };
    let parent = node.parent_element()?;
    if (is_x(node, "p") || is_x(node, "list")) && is_x(parent, "textbox") {
        return Some((vec![attrs(node, BLOCK)], parent));
    }
    if is_x(node, "li") && is_x(parent, "list") {
        return Some(
            match parent.parent_element().filter(|b| is_x(*b, "textbox")) {
                Some(textbox) => (vec![attrs(node, BLOCK), attrs(parent, BLOCK)], textbox),
                None => (vec![attrs(node, BLOCK)], parent),
            },
        );
    }
    if (is_x(node, "td") || is_x(node, "th")) && is_x(parent, "tr") {
        let table = parent.parent_element().filter(|t| is_x(*t, "table"))?;
        return Some((cell_levels(node, parent, table, theme), table));
    }
    None
}

/// A cell's levels over its table (§6.15): cell › row › column › a header
/// cell's bold.
pub(super) fn cell_levels<'a>(
    cell: roxmltree::Node<'a, 'a>,
    row: roxmltree::Node<'a, 'a>,
    table: roxmltree::Node<'a, 'a>,
    theme: Option<&'a Theme>,
) -> Vec<Level<'a>> {
    let is_cell = |c: &roxmltree::Node| is_x(*c, "td") || is_x(*c, "th");
    let ci = row
        .children()
        .filter(is_cell)
        .position(|c| c == cell)
        .unwrap_or(0);
    let col = table.children().filter(|c| is_x(*c, "col")).nth(ci);
    let mut levels = vec![
        Level::Attrs {
            node: cell,
            props: CELL,
            token: theme.and_then(|t| token_of(cell, t)),
        },
        Level::Attrs {
            node: row,
            props: CELL,
            token: theme.and_then(|t| token_of(row, t)),
        },
    ];
    if let Some(col) = col {
        levels.push(Level::Attrs {
            node: col,
            props: CELL,
            token: theme.and_then(|t| token_of(col, t)),
        });
    }
    if is_x(cell, "th") {
        levels.push(Level::Fixed {
            node: cell,
            prop: "font-weight",
            value: "700",
        });
    }
    levels
}

/// The value the levels give `prop`, else nothing.
pub(super) fn level_value<'a>(levels: &[Level<'a>], prop: &str) -> Option<&'a str> {
    find(levels, prop).map(|(v, _, _)| v)
}

/// `fallback` with whatever the levels set laid over it — the one way every
/// text front-end computes a style. A paragraph overlays its levels on its
/// box's style; [`style_from`] overlays an element's on the initial style.
pub(super) fn overlay(levels: &[Level], fallback: &TextStyle) -> TextStyle {
    let mut st = fallback.clone();
    let get = |p: &str| level_value(levels, p);
    if let Some(v) = get("font-family") {
        st.family = v.to_string();
    }
    if let Some(v) = get("font-size").and_then(parse_num) {
        st.size = v;
    }
    if let Some(v) = get("font-weight") {
        st.weight = v.to_string();
    }
    if let Some(v) = get("font-style") {
        st.style = v.to_string();
    }
    if let Some(v) = get("line-height").and_then(parse_num) {
        st.line_height = v;
    }
    if let Some(v) = get("letter-spacing").and_then(spacing_value) {
        st.letter_spacing = v;
    }
    if let Some(v) = get("word-spacing").and_then(spacing_value) {
        st.word_spacing = v;
    }
    if let Some(v) = get("text-transform").and_then(TextTransform::parse) {
        st.text_transform = v;
    }
    st
}

/// The style a text element starts from before anything sets a property.
pub(super) fn initial_style() -> TextStyle {
    TextStyle {
        family: "sans-serif".to_string(),
        size: 16.0,
        weight: "normal".to_string(),
        style: "normal".to_string(),
        line_height: 1.2,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        text_transform: TextTransform::default(),
    }
}

/// Every level `node` reads, nearest first, and the text element it belongs to.
fn cascade<'a>(
    node: roxmltree::Node<'a, 'a>,
    theme: &'a Theme,
) -> (Vec<Level<'a>>, roxmltree::Node<'a, 'a>) {
    let (mut levels, base) = block_levels(node, Some(theme)).unwrap_or((Vec::new(), node));
    levels.extend(element_levels(base, theme));
    (levels, base)
}

/// The default a built-in element declares for one of its parameters.
fn declared_default(node: roxmltree::Node, name: &str) -> Option<&'static str> {
    if node.tag_name().namespace() != Some(XSVG_NS) {
        return None;
    }
    let local = node.tag_name().name();
    elements::built_in_elements()
        .iter()
        .find(|e| e.name == local)?
        .all_params()
        .find(|p| p.name == name)?
        .default
}

/// An attribute by its query name: `x:` names the xsvg namespace, whatever
/// prefix the document binds to it.
fn attr_by_name<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    match name.strip_prefix("x:") {
        Some(local) => node.attribute((XSVG_NS, local)),
        None => node.attribute(name),
    }
}

/// Where a resolved value came from.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// The node's own attribute.
    Own,
    /// The node's own `<x:font>` token of this name.
    Token(String),
    /// An enclosing node: the paragraph's box, the item's list, the cell's row,
    /// column or table, or — for `font-family` — any ancestor. `from` is its
    /// source byte offset; `token`, when its `<x:font>` token supplied it.
    Inherited { from: usize, token: Option<String> },
    /// The emitter supplies it: a header cell is bold.
    Implied,
    /// Nothing set it: the element's declared default, else the compiler's.
    Initial,
}

/// A property's effective value on a node.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    pub property: String,
    /// As authored, `var(…)` included. `None` when nothing sets it and neither
    /// the element nor the compiler has a default (the renderer's applies).
    pub value: Option<String>,
    /// [`Self::value`] with theme colours substituted — what renders.
    pub computed: Option<String>,
    pub source: Source,
}

/// `property` on `node`, by the rules its emitter applies. Style properties
/// ([`STYLE_PROPS`]) walk the node's cascade; any other attribute is the node's
/// own, else the default its element declares.
pub(super) fn resolved(node: roxmltree::Node, property: &str, theme: &Theme) -> Resolved {
    let done = |value: Option<&str>, source| Resolved {
        property: property.to_string(),
        computed: value.map(|v| theme.resolve_var(v).into_owned()),
        value: value.map(str::to_string),
        source,
    };
    if !STYLE_PROPS.contains(&property) {
        // a cell's alignment is the table's rule (§6.15): cell › row › column
        // › table, not the paragraph planner's own-else-box
        if (is_x(node, "td") || is_x(node, "th"))
            && matches!(property, "align" | "valign" | "direction")
        {
            if let Some((levels, table)) = block_levels(node, Some(theme)) {
                return match find(&levels, property) {
                    Some((v, _, level)) if level.node() == node => done(Some(v), Source::Own),
                    Some((v, _, level)) => done(
                        Some(v),
                        Source::Inherited {
                            from: level.node().range().start,
                            token: None,
                        },
                    ),
                    None => match attr_by_name(table, property) {
                        Some(v) => done(
                            Some(v),
                            Source::Inherited {
                                from: table.range().start,
                                token: None,
                            },
                        ),
                        None => done(declared_default(table, property), Source::Initial),
                    },
                };
            }
        }
        // a block's layout properties cascade from its box by the planner's rules
        if let Some((_, container)) = block_levels(node, Some(theme)) {
            if let Some((v, source)) = block_layout(node, container, is_x(node, "li"), property) {
                return done(v, source);
            }
        }
        return match attr_by_name(node, property) {
            Some(v) => done(Some(v), Source::Own),
            None => done(declared_default(node, property), Source::Initial),
        };
    }
    let (levels, base) = cascade(node, theme);
    match find(&levels, property) {
        Some((v, origin, level)) => {
            let token = match origin {
                Origin::Token(name) => Some(name.to_string()),
                _ => None,
            };
            let source = match origin {
                Origin::Fixed => Source::Implied,
                _ if level.node() != node => Source::Inherited {
                    from: level.node().range().start,
                    token,
                },
                Origin::Token(name) => Source::Token(name.to_string()),
                Origin::Attr => Source::Own,
            };
            done(Some(v), source)
        }
        None => done(
            declared_default(base, property).or_else(|| initial(property)),
            Source::Initial,
        ),
    }
}

/// The layout properties the paragraph planner reads for a block of a box
/// (§6.16), each with where it came from: `align` and `direction` the
/// paragraph's own, else its box's; `text-indent` the paragraph's own, else
/// none — a box's indent is for its own single flow. An item in a list is
/// always start-aligned and unindented (its gutter is the inset), its direction
/// its own, else its list's, else the box's — standalone or boxed alike.
pub(super) fn block_layout<'a>(
    block: roxmltree::Node<'a, 'a>,
    container: roxmltree::Node<'a, 'a>,
    in_list: bool,
    prop: &str,
) -> Option<(Option<&'a str>, Source)> {
    let inherited = |v: &'a str| {
        (
            Some(v),
            Source::Inherited {
                from: container.range().start,
                token: None,
            },
        )
    };
    Some(match prop {
        "align" if in_list => (Some("start"), Source::Implied),
        "align" => match (block.attribute("align"), container.attribute("align")) {
            (Some(v), _) => (Some(v), Source::Own),
            (None, Some(v)) => inherited(v),
            (None, None) => (Some("start"), Source::Initial),
        },
        "text-indent" if in_list => (Some("0"), Source::Implied),
        "text-indent" => match block.attribute("text-indent") {
            Some(v) => (Some(v), Source::Own),
            None => (Some("0"), Source::Initial),
        },
        "direction" => {
            // an item's own, else its list's, else the box's; a paragraph's
            // own, else the box's
            let list = block
                .parent()
                .filter(|p| in_list && is_x(*p, "list") && *p != container);
            let from_list = list.and_then(|l| Some((l, l.attribute("direction")?)));
            match (
                block.attribute("direction"),
                from_list,
                container.attribute("direction"),
            ) {
                (Some(v), _, _) => (Some(v), Source::Own),
                (None, Some((l, v)), _) => (
                    Some(v),
                    Source::Inherited {
                        from: l.range().start,
                        token: None,
                    },
                ),
                (None, None, Some(v)) => inherited(v),
                (None, None, None) => (Some("ltr"), Source::Initial),
            }
        }
        _ => return None,
    })
}

/// The run attributes a text element's emitter draws (§6.11, §6.13, §6.17), so
/// an editor offers exactly the controls that change the picture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RunSupport {
    /// `font-family` / `-size` / `-weight` / `-style` on a run.
    pub typography: bool,
    pub fill: bool,
    /// `x:border-width` / `-color` / `-opacity`.
    pub border: bool,
    /// `x:highlight`.
    pub highlight: bool,
    /// `text-decoration`.
    pub decoration: bool,
    /// `baseline-shift`.
    pub shift: bool,
    /// A link: `<tspan href>` or an `<a>`.
    pub href: bool,
}

impl RunSupport {
    const ALL: Self = Self {
        typography: true,
        fill: true,
        border: true,
        highlight: true,
        decoration: true,
        shift: true,
        href: true,
    };
    /// Text on a path: paint runs only, its typography is the element's.
    const PAINT: Self = Self {
        fill: true,
        border: true,
        ..Self::NONE
    };
    /// A plain `<text>`, passed through: the browser composes a `<tspan>`'s
    /// typography, fill, decoration, baseline shift and an `<a>`'s link itself,
    /// and the compiler lowers a run's border (§6.17). A highlight is the one
    /// thing nobody draws — it is baked only by the laid-out front-ends.
    const PLAIN: Self = Self {
        highlight: false,
        ..Self::ALL
    };
    const NONE: Self = Self {
        typography: false,
        fill: false,
        border: false,
        highlight: false,
        decoration: false,
        shift: false,
        href: false,
    };
}

/// The text element whose emitter lays out `node`'s runs: `node` itself, or the
/// box, list or table its paragraph, item or cell belongs to.
pub(super) fn text_element<'a>(node: roxmltree::Node<'a, 'a>) -> roxmltree::Node<'a, 'a> {
    block_levels(node, None).map_or(node, |(_, base)| base)
}

/// What a run inside `node`'s text may carry and see drawn — by the compiler or,
/// for a plain `<text>`, by the renderer. Every laid-out front-end —
/// `<textArea>`, `<x:textbox>` (paragraphs, lists, `in=` regions), `<x:list>`,
/// `<x:table>`, `<text inline-size>` — takes runs through one walker
/// ([`collect_runs`]) and draws all of it; text on a path draws paint runs; a
/// plain `<text>` passes through, where SVG draws everything but a highlight
/// natively and the compiler lowers the border. `None` for a node that holds
/// no text.
pub fn run_support(node: roxmltree::Node) -> Option<RunSupport> {
    let el = text_element(node);
    let svg = el.tag_name().namespace() == Some(SVG_NS);
    let xsvg = el.tag_name().namespace() == Some(XSVG_NS);
    Some(match el.tag_name().name() {
        "textbox" | "list" | "table" if xsvg => RunSupport::ALL,
        "textArea" if svg => RunSupport::ALL,
        "text" if svg && el.attribute("inline-size").is_some() => RunSupport::ALL,
        "text" if svg => RunSupport::PLAIN,
        "textpath" if xsvg => RunSupport::PAINT,
        _ => return None,
    })
}

/// A run's border, as the emitter composes it (§6.17): each attribute the run
/// leaves out from the enclosing run, else the nearest bordered container.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PieceBorder {
    pub width: Option<String>,
    pub color: Option<String>,
    pub opacity: Option<String>,
}

/// One stretch of an element's text in one style.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPiece {
    /// UTF-16 code units into the element's text content (every descendant text
    /// node in document order, each `<tbreak/>` counting one).
    pub start: usize,
    pub end: usize,
    pub family: String,
    pub size: f64,
    pub weight: String,
    pub style: String,
    /// As authored, `var(…)` included.
    pub fill: String,
    pub decoration: Option<String>,
    pub highlight: Option<String>,
    pub shift: Option<String>,
    pub href: Option<String>,
    /// `None`: the element's own border, if any.
    pub border: Option<PieceBorder>,
}

/// The runs of `node`'s text, each in the style it renders in: the node's
/// cascade style overridden by its `<tspan>`s as [`collect_runs`] composes
/// them, keeping only what the element draws ([`run_support`]). Adjacent text
/// in one style is one piece. For a box holding paragraphs, ask each paragraph:
/// the box's own pieces are in the box's style. `None` for a node holding no
/// text element.
pub(super) fn text_pieces(node: roxmltree::Node, theme: &Theme) -> Option<Vec<TextPiece>> {
    let support = run_support(node)?;
    let (levels, base) = cascade(node, theme);
    let style = overlay(&levels, &initial_style());
    let fill = find(&levels, "fill")
        .map(|(v, _, _)| v)
        .or_else(|| declared_default(base, "fill"))
        .unwrap_or("#000");
    let (segments, styles, emits) = collect_runs(node, &style);
    let mut pieces: Vec<(usize, TextPiece)> = Vec::new();
    let mut at = 0;
    for (text, sid) in &segments {
        let len = text.encode_utf16().count();
        if len == 0 {
            continue;
        }
        if let Some((last, p)) = pieces.last_mut() {
            if *last == *sid && p.end == at {
                p.end += len;
                at += len;
                continue;
            }
        }
        let (st, a) = (&styles[*sid], &emits[*sid]);
        let typo = if support.typography { st } else { &style };
        let keep = |on: bool, v: &Option<String>| v.clone().filter(|_| on);
        pieces.push((
            *sid,
            TextPiece {
                start: at,
                end: at + len,
                family: typo.family.clone(),
                size: typo.size,
                weight: typo.weight.clone(),
                style: typo.style.clone(),
                fill: a
                    .fill
                    .clone()
                    .filter(|_| support.fill)
                    .unwrap_or_else(|| fill.to_string()),
                decoration: keep(support.decoration, &a.decoration),
                highlight: keep(support.highlight, &a.highlight),
                shift: keep(support.shift, &a.shift),
                href: keep(support.href, &a.href),
                border: a
                    .border
                    .as_ref()
                    .filter(|_| support.border)
                    .map(|b| PieceBorder {
                        width: b.width.clone(),
                        color: b.color.clone(),
                        opacity: b.opacity.clone(),
                    }),
            },
        ));
        at += len;
    }
    Some(pieces.into_iter().map(|(_, p)| p).collect())
}

/// What a block of a box or list is.
#[derive(Clone, Debug, PartialEq)]
pub enum BlockKind {
    Paragraph,
    /// A list item: its nesting level (`indent`), the list's source offset, and
    /// the marker kind it numbers or bullets with (`bullet` / `number` / `none`).
    Item {
        level: usize,
        list: usize,
        marker: String,
    },
}

/// One block leaf of a box or list, in reading order.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Source byte range.
    pub start: usize,
    pub end: usize,
    pub kind: BlockKind,
}

/// The block leaves of a box in paragraph mode (§6.16) or of a list: its
/// paragraphs and each list's items, as the paragraph planner reads them.
/// `None` for anything else; empty for a box of one flow.
pub(super) fn blocks(node: roxmltree::Node) -> Option<Vec<Block>> {
    let item = |li: roxmltree::Node, list: roxmltree::Node| {
        let lp: elements::List = params_of(list);
        Block {
            start: li.range().start,
            end: li.range().end,
            kind: BlockKind::Item {
                level: attr_num(li, "indent", 0.0).max(0.0) as usize,
                list: list.range().start,
                marker: li.attribute("list").unwrap_or(lp.list.as_str()).to_string(),
            },
        }
    };
    let items = |list: roxmltree::Node| -> Vec<Block> {
        list.children()
            .filter(|c| is_x(*c, "li"))
            .map(|li| item(li, list))
            .collect()
    };
    if is_x(node, "list") {
        return Some(items(node));
    }
    if !is_x(node, "textbox") {
        return None;
    }
    let mut out = Vec::new();
    for b in blocks_of(node) {
        if b.tag_name().name() == "list" {
            out.extend(items(b));
        } else {
            out.push(Block {
                start: b.range().start,
                end: b.range().end,
                kind: BlockKind::Paragraph,
            });
        }
    }
    Some(out)
}

/// The document's theme tokens (§4.1): every `<x:theme>` outside `<x:doc>`,
/// a later token of a name replacing an earlier one and taking its place in
/// the order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeTokens {
    /// Name → paint value.
    pub colors: Vec<(String, String)>,
    /// Name → its properties, in [`STYLE_PROPS`] order.
    pub fonts: Vec<(String, Vec<(String, String)>)>,
}

pub(super) fn theme_tokens(theme: &Theme) -> ThemeTokens {
    ThemeTokens {
        colors: theme
            .color_order
            .iter()
            .map(|n| (n.clone(), theme.colors[n].clone()))
            .collect(),
        fonts: theme
            .font_order
            .iter()
            .map(|n| (n.clone(), theme.fonts[n].clone()))
            .collect(),
    }
}
