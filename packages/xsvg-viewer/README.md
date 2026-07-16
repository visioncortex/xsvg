# @visioncortex/xsvg-viewer

Render [xsvg](https://xsvg.visioncortex.org) in the browser. xsvg is an XML graphics
format that compiles to plain SVG — with the text layout, warps, and gradient meshes SVG
never got. This package bundles the WASM compiler and a drop-in preview surface.

```bash
npm install @visioncortex/xsvg-viewer
```

## Compile

```ts
import { compileXsvg } from "@visioncortex/xsvg-viewer";

const svg = await compileXsvg(source); // a plain-SVG string; runs entirely client-side
el.innerHTML = svg;
```

Fonts named `font-family="-x-google-<Name>"` are fetched from Google Fonts and their
glyphs outlined on demand (for `outline="true"` runs); a miss falls back to live `<text>`.

## Preview surface

`createPreview` mounts a self-contained, fit-to-contain view that, for multi-artboard
documents, adds a slide deck — a thumbnail rail, a ‹ n/N › nav, and a rail toggle:

```ts
import { createPreview } from "@visioncortex/xsvg-viewer";

const preview = createPreview(hostEl, { hashDeepLink: true, showErrors: true });
await preview.render(source); // re-render any time; keeps the last good frame on error
```

`hostEl` must be a positioned box (the surface fills it absolutely). Styles are injected
automatically — no stylesheet import needed.

## API

- `compileXsvg(source, { quality?, sourcemap? })` → `Promise<string>`
- `createPreview(host, { hashDeepLink?, showErrors? })` → `{ render, destroy }`
- `findArtboards(svg)`, `makeThumb(svg, board)` — slide-deck primitives
- `registerOutlineFont(family, url)` — register a font for `outline="true"` baking
- `downloadSvg(source, basename)` — compile and save as `<basename>.svg`

For a browser-free (Node) compile, see **@visioncortex/xsvg-compile**.
