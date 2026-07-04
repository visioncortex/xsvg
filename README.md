# xsvg

**eXtensible SVG** — an XML interchange format that compiles to a subset of SVG. Pure-Rust core,
compiled to WASM, run entirely client-side in the browser.

- [Vision.md](docs/Vision.md) — north star
- [Specification.md](docs/Specification.md) — normative spec (language + lowering rules), evolving
- [Plan.md](docs/Plan.md) — architecture, roadmap, first-milestone spec
- [Syntax.md](docs/Syntax.md) — language design (graceful-degradation superset of SVG)
- [Typography.md](docs/Typography.md) — typesetting capabilities catalog
- [Research.md](docs/Research.md) — cited prior-art research

## v0 scaffolding

A static, fully client-side web page: the Rust compiler runs as WASM in the browser, and the
browser's own SVG engine renders the compiled output. No application backend.

### Prerequisites

- **Rust** (stable) + the wasm target: `rustup target add wasm32-unknown-unknown`
- **wasm-pack**: `cargo install wasm-pack` (or `brew install wasm-pack`)
- **Node** — `nvm use` (pinned to `22.21.1` via [.nvmrc](.nvmrc)), then `npm install`

### Commands

| Command | What it does |
|---|---|
| `npm run dev` | Build the wasm (debug) and start the Vite dev server with hot reload |
| `npm run build` | Build the wasm (release), bundle the tool pages into `dist/`, and emit the single-file embed `dist/xsvg.js` |
| `npm run build:embed` | (Re)build just the single-file embed `dist/xsvg.js` |
| `npm run start` | Serve the pre-built `dist/` app (`vite preview`) |
| `npm run typecheck` | `tsc --noEmit` over the TS sources |
| `cargo test` | Run the Rust unit tests for the compiler |

> Editing **TypeScript** hot-reloads instantly. Editing **Rust** requires re-running
> `npm run wasm:dev` (the `dev` script does this on start); Vite then reloads.

### Three use cases, one compile core

The web project separates into three deliverables that all share one WASM compile
core (`web/src/core/`):

1. **Vanilla embed** — a barebone, drop-in viewer. `npm run build` emits a single
   self-contained `dist/xsvg.js` (the WASM is inlined). A host page needs nothing
   else — it renders like an image:

   ```html
   <script src="xsvg.js"></script>
   <xsvg-view src="diagram.xsvg"></xsvg-view>
   ```

   Inline documents go in a `<script type="application/xsvg+xml">` data island so
   the HTML parser leaves the custom XML alone. See `web/embed-demo/`.

2. **Interactive viewer** (`/viewer/`) — pan, zoom, drop a `.xsvg` file to open,
   and an **element inspector**: click a rendered element to see its tag/attributes
   and jump to the originating xsvg source (the compiler emits a source map when
   asked; see below).

3. **Playground** (`/playground/`) — a CodeMirror editor on the left, a live
   compiled preview on the right; a sample picker seeds the editor and the document
   round-trips through the URL for shareable links.

### Launch

```bash
nvm use        # Node 22.21.1 (from .nvmrc)
npm run dev    # builds the wasm, starts Vite, opens http://localhost:5173
```

The landing page at **http://localhost:5173/** is a hub linking to the viewer and
playground, plus a categorized index of every `dataset/` sample. Each page is a
directory-style route (clean URLs, no `.html`, works on any static host). Open a
sample directly in either tool by name (`?file=`):

```
http://localhost:5173/viewer/?file=region-flow.xsvg
http://localhost:5173/playground/?file=pipeline.xsvg
```

Dev-only pages (served by the dev server, not part of the production build):
`/fixtures/` (font-metric fixture generator) and `/embed-demo/` (`<xsvg-view>` demo).

> **Blank page or 404 at `localhost:5173`?** A stale Vite from an earlier run may be holding the
> port with the wrong root. Run `pkill -f node_modules/.bin/vite`, then `npm run dev` again.

### Source maps

The compiler's `compile(input, quality, sourcemap, …)` entry takes a `sourcemap`
flag. When on, every emitted top-level element carries `data-xsvg-pos="START-END"`
— the byte range of the originating xsvg node in the input. The interactive viewer
turns it on to project a clicked element back to its authoring source; the vanilla
embed leaves it off so the emitted SVG stays clean.

### Layout

```
crates/
  xsvg-core/   pure-Rust shared types (QualityProfile, …) — no platform deps
  xsvg-wasm/   wasm-bindgen entry: compile(input, quality, sourcemap) -> svg
web/
  index.html             landing hub             (/)
  viewer/index.html      interactive viewer      (/viewer/)
  playground/index.html  playground              (/playground/)
  preview/index.html     bare vanilla renderer   (/preview/)
  fixtures/index.html    dev: fixture generator  (/fixtures/)
  embed-demo/index.html  dev: <xsvg-view> demo   (/embed-demo/)
  vite.embed.config.ts   single-file embed build (inlines the wasm as base64)
  src/
    core/       shared compile core: compiler.ts (wasm wrapper), samples.ts + catalog.ts,
                sourcemap.ts (byte↔string index), editor.ts (CodeMirror factory)
    embed/      xsvg-view.ts (<xsvg-view> web component) + index.ts (single-file entry)
    viewer/     main.ts, pan-zoom.ts, inspector.ts, viewer.css
    playground/ main.ts, playground.css
    dev/        index.ts (hub), fixtures.ts
  pkg/          generated by wasm-pack (git-ignored)
dataset/        sample .xsvg diagrams (open via ?file=<name>)
dist/           production build output, incl. the single-file xsvg.js (git-ignored)
```

### v0 compiler scope

Parses xsvg/SVG and runs lowering passes, emitting a plain-SVG string:

- sharp-cornered `<rect>` → `<path>`
- **`<text inline-size="W">` → wrapped `<tspan>` lines** (Syntax.md Rung 1)
- **`<textArea>` → SVG Tiny 1.2 flowed text** (Syntax.md Rung 2): `text-align`, `display-align`, `line-increment`, `auto` width/height, overflow clipping
- **`<x:textbox>` → wrapped, aligned, shrink-to-fit text** (Syntax.md Rung 3, `fit="shrink"`)
- other `x:` extensions are recognized and skipped with a marker (later phases — see
  [Plan.md](docs/Plan.md) §3)

Text layout uses real font metrics: the WASM compiler calls a browser `measure(text, fontCss)`
callback (canvas `measureText`) — the browser implementation of the core's pure `Measurer` seam, so
the wrap/fit logic stays platform-free and unit-tested in `xsvg-core`.
