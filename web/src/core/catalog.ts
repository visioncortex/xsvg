// Curated index of the dataset samples, grouped by the feature they exercise.
// This is the source of truth for the landing page (dev/index.ts) — hand-maintained
// so it reads as a guided tour rather than a raw directory listing. Kept in step
// with dataset/README.md. Any *.xsvg on disk that isn't listed here still shows up
// under "Uncategorized" so new samples are never silently hidden.

export interface Sample {
  file: string;
  title: string;
  blurb: string;
}

export interface Category {
  name: string;
  note?: string;
  samples: Sample[];
}

export const CATALOG: Category[] = [
  {
    name: "A brief intro to xsvg",
    note: "Start here — one file, three x:artboard slides touring what the format is. <g x:artboard=\"…\"> names a slide frame; the preview pages them with < > nav (§5.2).",
    samples: [
      {
        file: "artboards.xsvg",
        title: "Artboards (slides)",
        blurb: "Three x:artboard slides in one file — a title, a bar chart, and a closing card; the preview pages them with a < > nav, the viewer zooms to the first, and it stays plain SVG in any viewer",
      },
    ],
  },
  {
    name: "Showcases",
    note: "Realistic composites that combine several features.",
    samples: [
      {
        file: "architecture.xsvg",
        title: "System architecture",
        blurb: "Shrink-to-fit service boxes, <tbreak/> data nodes, a glyph-x-scale banner, arrow markers",
      },
      {
        file: "kanban.xsvg",
        title: "Sprint board",
        blurb: "Cards that wrap and ellipsis-truncate, <tbreak/> title/body splits, right-aligned counts",
      },
      {
        file: "pipeline.xsvg",
        title: "Compile pipeline",
        blurb: "Stretched heading, five shrink-to-fit stages with wrapping captions, arrow markers",
      },
      {
        file: "flowchart.xsvg",
        title: "Request flow",
        blurb: "Branching yes/no decision, shrink-to-fit nodes, terminal states, arrow markers",
      },
    ],
  },
  {
    name: "Shape binding & region flow",
    note: "<x:textbox in=\"#id\"> — bind to a shape or flow inside its outline.",
    samples: [
      {
        file: "chat.xsvg",
        title: "Chat bubbles",
        blurb: "Two <x:textbox in=\"#rect\"> reuse each box — message + a bottom-right timestamp",
      },
      {
        file: "region-flow.xsvg",
        title: "Text in a shape",
        blurb: "Flowed inside a triangle, circle, diamond, and a concave hourglass; valign centers the block",
      },
      {
        file: "badges.xsvg",
        title: "Badges & text effects",
        blurb: "A certified seal with text curved around the rim (textpath rainbow), a shield with a warp-arched title, and a glossy meshgradient medal with a drop-shadowed label",
      },
    ],
  },
  {
    name: "Box models & alignment",
    samples: [
      {
        file: "textarea.xsvg",
        title: "textArea",
        blurb: "SVG Tiny 1.2 flow: text-align, display-align, line-increment, auto sizing",
      },
      {
        file: "textarea-align.xsvg",
        title: "textArea alignment",
        blurb: "text-align × display-align matrix (all nine)",
      },
      {
        file: "alignment.xsvg",
        title: "textbox alignment",
        blurb: "<x:textbox> align × valign matrix (cap-height centering)",
      },
    ],
  },
  {
    name: "Wrapping, fitting & overflow",
    samples: [
      {
        file: "wrap-vs-overflow.xsvg",
        title: "Wrap vs overflow",
        blurb: "The core win: overflowing <text> vs inline-size wrapping vs fit=\"shrink\"",
      },
      {
        file: "cards.xsvg",
        title: "Shrink-to-fit cards",
        blurb: "Equal-size cards whose variable-length text all shrinks to fit",
      },
      {
        file: "textarea-sizing.xsvg",
        title: "Auto sizing & clipping",
        blurb: "width=auto, wrapping, height clipping, line-increment auto/loose/tight",
      },
      {
        file: "textarea-ellipsis.xsvg",
        title: "text-overflow",
        blurb: "clip vs ellipsis, for both block and inline overflow",
      },
    ],
  },
  {
    name: "Paragraph & character typography",
    samples: [
      {
        file: "justify.xsvg",
        title: "Justification",
        blurb: "text-align=\"justify\": full lines flush, last line ragged, resets per <tbreak/>",
      },
      {
        file: "letter-spacing.xsvg",
        title: "letter-spacing",
        blurb: "Tracking scale, kerning-preserved pairs, layout-aware wrapping",
      },
      {
        file: "word-spacing.xsvg",
        title: "word-spacing",
        blurb: "Inter-word tracking; wider gaps wrap the same box sooner",
      },
      {
        file: "tbreak-and-glyph-scale.xsvg",
        title: "tbreak & glyph-x-scale",
        blurb: "Forced breaks plus condensed / regular / extended glyph widths",
      },
      {
        file: "styled-runs.xsvg",
        title: "Styled runs",
        blurb: "<tspan> runs: per-run fill / weight / style flowing and wrapping inline",
      },
      {
        file: "paragraphs.xsvg",
        title: "Paragraphs",
        blurb: "<x:p> children flow a <x:textbox> as stacked paragraphs — space-before/after (paragraph-spacing), per-paragraph align (justify), first-line indent, and per-paragraph font/fill — plus a vertically-centered pull-quote built from three styled paragraphs (§6.16)",
      },
      {
        file: "lists.xsvg",
        title: "Bullet & numbered lists",
        blurb: "<x:list> / <x:li indent='N'>: hanging-indent items whose drawn bullet shapes (disc / ring / square, optically balanced) and decimal→alpha→roman numbers cycle by depth, with the outer counter resuming after each sublist; a third list flows inside a referenced card (§6.14)",
      },
    ],
  },
  {
    name: "Vector output — create outlines",
    note: 'outline="true" — lower glyphs to <path> geometry (§6.12).',
    samples: [
      {
        file: "outline.xsvg",
        title: "Create outlines",
        blurb: 'font-family="-x-google-Anton" provisions a Google font by name; outline="true" also bakes its glyphs into <path>',
      },
    ],
  },
  {
    name: "Geometry transforms — text on a path",
    note: '<x:textpath in="#path"> — outline text and warp it onto a curve (§6.13).',
    samples: [
      {
        file: "textpath.xsvg",
        title: "Text on a path (skew & rainbow)",
        blurb: "skew shears upright glyphs onto a wave; rainbow rotates and bends them along an arc, with baseline-shift floating runs above and below the same path",
      },
      {
        file: "textpath-align.xsvg",
        title: "Placement & stair-step",
        blurb: 'align start/middle/end and a start offset place the run along the path; effect="stair" steps live selectable glyphs (also skew\'s no-font fallback)',
      },
      {
        file: "textpath-effects.xsvg",
        title: "Ribbon & native follow",
        blurb: 'effect="ribbon" tilts verticals with the curve (skew\'s complement); effect="follow" lowers to SVG\'s own <textPath> — live and selectable',
      },
    ],
  },
  {
    name: "Geometry transforms — warp",
    note: '<x:warp field="…"> — bake an envelope-preset field into plain path geometry (§7.3).',
    samples: [
      {
        file: "warp-presets.xsvg",
        title: "Envelope presets",
        blurb: "eight presets — arch/flag/rise/wave + fisheye/inflate/squeeze/twist — over a rect + outlined text; the quality profile grades the bake tolerance",
      },
      {
        file: "warp-presets-arc.xsvg",
        title: "Arc & shell families",
        blurb: "arc (annular sector, both bends) + arc-lower/upper, bulge, fish, shell-lower/upper — Make-with-Warp parity complete, 15/15",
      },
      {
        file: "warp-perspective.xsvg",
        title: "Perspective & free distort",
        blurb: "corners-solved homography (straight lines stay straight), bilinear free distort, and the distort-h/v sliders composing after a preset",
      },
      {
        file: "warp-bend.xsvg",
        title: "Bend & roughen",
        blurb: 'field="bend" in="#spine" flows a whole group along a path (align/start place it); field="roughen" jitters outlines with deterministic seeded noise',
      },
    ],
  },
  {
    name: "Connectors — diagramming",
    note: "<x:connector from to route> — lines routed between two boxes' edges (§7.6).",
    samples: [
      {
        file: "connectors.xsvg",
        title: "Connectors",
        blurb: "lines bound to two boxes, routed four ways — straight (edge-clipped), x-major and y-major orthogonal rails, and a smooth curve; arrowheads tint to the stroke, and the route re-derives when an endpoint moves",
      },
      {
        file: "connector-anchors.xsvg",
        title: "Endpoints — anchors & points",
        blurb: "beyond a bare #id: force which connection point a connector meets with #id:anchor — an edge, a corner (left-top…), or center — so same-side pairs curve into a leaf; or aim at a raw coordinate with to-point=\"x,y\", needing no target element",
      },
    ],
  },
  {
    name: "Tables — content-driven rows",
    note: "<x:table>/<x:tr>/<x:td>/<x:th> — author-set columns, row heights that grow to fit wrapped content (the Slides/Canva model), baked to rects + text (§6.15).",
    samples: [
      {
        file: "table.xsvg",
        title: "Tables",
        blurb: "a fixed label column plus two flexible columns (cols=\"150 * *\"), bold header cells with a header-fill, and body cells that wrap to their column and grow the row to fit the tallest — content-driven row heights, not HTML auto-layout; per-cell bg/align/weight overrides, all baked to plain rects + text",
      },
    ],
  },
  {
    name: "Theming — design tokens",
    note: "<x:theme> compile-time color tokens (var(name) in any paint) + font tokens (x:font, an overridable base) — §4.1.",
    samples: [
      {
        file: "theme.xsvg",
        title: "Design tokens",
        blurb: "a palette of <x:color> tokens and an <x:font> scale referenced by fill=\"var(name)\" (accent bar, swatches, button, list markers) and x:font=\"name\" (kicker/title/heading/body); font tokens are an overridable base, and everything resolves at compile time",
      },
    ],
  },
  {
    name: "Charts — plots & pie",
    note: "<x:plot> a linear data frame (bars & lines) and <x:pie> an angular one — coordinate systems, not a charting library (§7.8–7.9).",
    samples: [
      {
        file: "plot.xsvg",
        title: "Bar & line plots",
        blurb: "<x:plot> maps a data domain to a pixel box (y inverted, sizes stay in px): a bar chart with y-ticks, gridlines, labels and bottom-aligned <x:bar>s, and a line chart whose y-domain \"20 25\" puts 22.5 mid-height, with dot markers and an area fill",
      },
      {
        file: "pie.xsvg",
        title: "Pie, donut & polar-area",
        blurb: "value sets each slice's angle, with one slice grown and exploded for emphasis; a donut via inner-radius + a constant-width parallel gap; and a Nightingale polar-area rose — equal angles with per-slice radius encoding the datum — all from one primitive, baked to sector paths",
      },
    ],
  },
  {
    name: "Inset & outset — path offsetting",
    note: "<x:offset in distance join> — grow/shrink a region by a Minkowski offset, baked to one <path> (§7.7).",
    samples: [
      {
        file: "offset.xsvg",
        title: "Inset & outset",
        blurb: "concentric ripples from one blob, a self-intersecting pentagram outset (evenodd, overlaps cleaned by the boolean pass), a spiky star inset and outset, and stacked outsets behind outlined text as a sticker halo — round/miter/bevel joins, all baked references",
      },
    ],
  },
  {
    name: "Path algebra — booleans",
    note: '<x:boolean op="union|intersect|subtract|exclude"> — Pathfinder ops baked to one <path> (§7.4).',
    samples: [
      {
        file: "boolean.xsvg",
        title: "Boolean operations",
        blurb: "union merges a circle cloud under one stroke; subtract punches text from a plate; intersect/exclude; and a boolean warped by flag — composability both ways",
      },
      {
        file: "boolean-refs.xsvg",
        title: "Operands by reference",
        blurb: "<use href> children: a venn lens derived from circles that keep rendering, motifs stamped by offset and by rotation, a union punching a plate, and live text whose glyphs punch by reference",
      },
    ],
  },
  {
    name: "Mesh gradients",
    note: "<x:mesh> — corner colors on a quad/tri mesh with cracks, fitted and lowered as texel-aligned tiny PNGs (§8.2).",
    samples: [
      {
        file: "aqua.xsvg",
        title: "Aqua buttons",
        blurb: "the classic use case: gel pill buttons as two patches — an opaque body mesh under a FEATHERED capsule gloss (its mesh IS the inset lens shape; alpha dissolves the shine), grounded by a compiled drop-shadow()",
      },
      {
        file: "mesh.xsvg",
        title: "Mesh gradients",
        blurb: "a seamless two-quad sky, the bilinear twist, a hard crack, a barycentric fan, the grid-sugar glow, and an SVG2/Inkscape meshgradient fill — curved Coons patches no browser can draw, compiled to tiny PNGs",
      },
    ],
  },
  {
    name: "Pixel adjustments",
    note: 'filter="brightness(1.2) …" — CSS filter functions lowered to portable <filter> primitives (§8).',
    samples: [
      {
        file: "adjust.xsvg",
        title: "Filters & tone curves",
        blurb: "brightness/contrast/saturate/sepia/invert/hue-rotate compiled to sRGB filter graphs, plus -x-curve() Photoshop-style tone curves sampled into lookup tables",
      },
    ],
  },
  {
    name: "Composition by reference",
    note: 'in="#id" on an x: element resolves its compiled output — features chain.',
    samples: [
      {
        file: "compose.xsvg",
        title: "Reference the compiled output",
        blurb: "a textbox flowed inside a boolean union; type riding a warp's arched spine; a path → warp → textpath chain with one edit point",
      },
    ],
  },
  {
    name: "Layers — compile-time z-order",
    note: "x:layer=\"background|foreground\" on a <g> restacks at compile; still plain SVG uncompiled (§5.1).",
    samples: [
      {
        file: "layers.xsvg",
        title: "Compile-time layers",
        blurb: "x:layer background/foreground buckets restack a watermark, card, grid, and badge; x:order refines within a band, x:hidden drops a debug overlay — all ignorable x: attributes on plain groups",
      },
    ],
  },
  {
    name: "Edge cases & invariants",
    samples: [
      {
        file: "degenerate.xsvg",
        title: "Degenerate input",
        blurb: "empty text, inline-size=0, font-size=0, shrink floors, oversized words, degenerate textpath targets, reference cycles",
      },
      {
        file: "descenders.xsvg",
        title: "Baseline stability",
        blurb: "Descenders (Gg) do not shift the baseline vs Bb",
      },
    ],
  },
];
