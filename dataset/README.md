# dataset — sample xsvg diagrams

Hand-authored `.xsvg` samples that exercise the v0 compiler (rect→path lowering, `inline-size`
wrapping, and `<x:textbox>` shrink-to-fit). Each is a complete `<svg xmlns:x="…">` document.

| File | Shows |
|---|---|
| [wrap-vs-overflow.xsvg](wrap-vs-overflow.xsvg) | The core win: plain `<text>` overflowing vs `inline-size` wrapping vs `<x:textbox fit="shrink">` |
| [pipeline.xsvg](pipeline.xsvg) | Boxes + arrows; each stage label wraps & shrinks to fit a uniform box |
| [flowchart.xsvg](flowchart.xsvg) | Start / process / decision (diamond) / end, with wrapping labels |
| [cards.xsvg](cards.xsvg) | Equal-size cards whose variable-length descriptions all shrink to fit |
| [textarea.xsvg](textarea.xsvg) | `<textArea>` (Rung 2, SVG Tiny 1.2): `text-align`, `display-align`, `line-increment`, auto width/height |
| [textarea-align.xsvg](textarea-align.xsvg) | `<textArea>` `text-align` × `display-align` matrix (all nine) |
| [textarea-sizing.xsvg](textarea-sizing.xsvg) | `<textArea>` `width=auto` (no wrap), wrapping, height clipping, `line-increment` auto/loose/tight |
| [textarea-ellipsis.xsvg](textarea-ellipsis.xsvg) | `text-overflow`: clip vs ellipsis (block overflow) and inline overflow truncation |
| [alignment.xsvg](alignment.xsvg) | `<x:textbox>` align × valign matrix (all nine placements) |
| [degenerate.xsvg](degenerate.xsvg) | Edge cases: empty text, `inline-size=0`, `font-size=0`, shrink, `fit-min>size`, oversized word |
| [descenders.xsvg](descenders.xsvg) | Proof that descenders (`Gg`) do not shift the baseline vs `Bb` (shared-baseline guide) |

## Viewing them

With the dev server running (`npm run dev`), open any sample by name:

```
http://localhost:5173/view/wrap-vs-overflow.xsvg
http://localhost:5173/?file=pipeline.xsvg
```

The SVG renders full-screen; the source and compiled output are logged to the browser console. The
`?file=` form works on any static host; the `/view/<name>` pretty-URL form relies on the dev
server's SPA fallback.
