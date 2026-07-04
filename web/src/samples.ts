// Curated index of the dataset samples, grouped by the feature they exercise.
// This is the source of truth for the dev index page (main.ts) — hand-maintained
// so the landing page reads as a guided tour rather than a raw directory listing.
// Kept in step with dataset/README.md. Any *.xsvg on disk that isn't listed here
// still shows up under "Uncategorized" so new samples are never silently hidden.

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
        blurb: "in=\"#rect\" binds a label to each rounded box — draw once, attach the text",
      },
      {
        file: "region-flow.xsvg",
        title: "Text in a shape",
        blurb: "Flowed inside a triangle, circle, diamond, and a concave hourglass; valign centers the block",
      },
      {
        file: "badges.xsvg",
        title: "Shaped badges",
        blurb: "Centered labels poured into a hexagon, circle seal, shield, and pentagon",
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
    ],
  },
  {
    name: "Edge cases & invariants",
    samples: [
      {
        file: "degenerate.xsvg",
        title: "Degenerate input",
        blurb: "empty text, inline-size=0, font-size=0, shrink floors, oversized words",
      },
      {
        file: "descenders.xsvg",
        title: "Baseline stability",
        blurb: "Descenders (Gg) do not shift the baseline vs Bb",
      },
    ],
  },
];
