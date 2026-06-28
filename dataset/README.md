# dataset — sample xsvg diagrams

Hand-authored `.xsvg` samples that exercise the v0 compiler (rect→path lowering, `inline-size`
wrapping, and `<x:textbox>` shrink-to-fit). Each is a complete `<svg xmlns:x="…">` document.

| File | Shows |
|---|---|
| [wrap-vs-overflow.xsvg](wrap-vs-overflow.xsvg) | The core win: plain `<text>` overflowing vs `inline-size` wrapping vs `<x:textbox fit="shrink">` |
| [pipeline.xsvg](pipeline.xsvg) | Boxes + arrows; each stage label wraps & shrinks to fit a uniform box |
| [flowchart.xsvg](flowchart.xsvg) | Start / process / decision (diamond) / end, with wrapping labels |
| [cards.xsvg](cards.xsvg) | Equal-size cards whose variable-length descriptions all shrink to fit |

## Viewing them

With the dev server running (`npm run dev`), open any sample by name:

```
http://localhost:5173/view/wrap-vs-overflow.xsvg
http://localhost:5173/?file=pipeline.xsvg
```

The SVG renders full-screen; the source and compiled output are logged to the browser console. The
`?file=` form works on any static host; the `/view/<name>` pretty-URL form relies on the dev
server's SPA fallback.
