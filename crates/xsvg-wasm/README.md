<div align="center">

  <img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/visioncortex-banner.png">
  <h1>xsvg</h1>

  <p>
    <strong>eXtensible SVG — an XML graphics format that compiles to plain SVG</strong>
  </p>

  <p>The text layout, live diagrams &amp; mesh gradients SVG never shipped.</p>

  <h3>
    <a href="https://xsvg.visioncortex.org/docs">Documentation</a>
    <span> | </span>
    <a href="https://xsvg.visioncortex.org/examples">Examples</a>
    <span> | </span>
    <a href="https://github.com/visioncortex/xsvg/releases">Release</a>
  </h3>

[![xsvg-viewer on npm](https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/badge-viewer.svg)](https://www.npmjs.com/package/@visioncortex/xsvg-viewer)
[![xsvg-compile on npm](https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/badge-compile.svg)](https://www.npmjs.com/package/@visioncortex/xsvg-compile)
[![MIT license](https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/badge-license.svg)](LICENSE)

</div>

## What is xsvg?

You author a **superset of SVG** — plain XML with a handful of `x:`-prefixed extensions — and
xsvg **compiles it to a plain, static SVG file**. No runtime, no script, no editor: the output
opens anywhere an SVG does.

The extensions add the things SVG never got — real text layout (wrapping, justification,
shrink-to-fit, flow into any shape), envelope warps, boolean path algebra, offsets, re-routing
connectors, gradient meshes, and a data-coordinate frame for charts. Every one of them **lowers
to primitives SVG already renders**: `<path>`, `<text>`, `<image>`, gradients.

It's **degradation-safe by construction.** The `x:` tags live in their own XML namespace, so a
viewer that doesn't understand them simply skips them — the file never breaks. You compile when
you want the effects baked in.

```xml
<!-- author this… -->                     <!-- …get plain SVG -->
<x:pie cx="60" cy="60" r="50">      →     <path d="M60,10 A50,50 0 0 1 …Z" fill="#6366f1"/>
  <x:slice value="40"/>                   <path d="M60,60 L60,10 A50,50 …Z" fill="#0ea5e9"/>
  <x:slice value="60" fill="#0ea5e9"/>    …
</x:pie>
```

## Install

Two npm packages and a CLI, all over one compiler.

**Browser** — [`@visioncortex/xsvg-viewer`](packages/xsvg-viewer): the WASM compiler plus a
drop-in preview surface.

```bash
npm install @visioncortex/xsvg-viewer
```
```ts
import { compileXsvg, createPreview } from "@visioncortex/xsvg-viewer";

el.innerHTML = await compileXsvg(source);            // → a plain-SVG string

// …or a self-contained preview (fit-to-contain, + a slide deck for multi-artboard docs):
createPreview(host, { hashDeepLink: true }).render(source);
```

The same package also ships an embeddable viewer: an `<xsvg-view>` custom element
(`/element`), a React component (`/react`), and a single-file `<script>` bundle
(`dist/xsvg.js`, WASM inlined) for drop-in use with no build step.

**Node** — [`@visioncortex/xsvg-compile`](packages/xsvg-compile): the same compiler, no browser.

```bash
npm install @visioncortex/xsvg-compile
```
```ts
import { compile } from "@visioncortex/xsvg-compile";
import { writeFileSync } from "node:fs";

writeFileSync("out.svg", compile(source, { fontDir: "./fonts" })); // synchronous
```

**CLI** — a pure-Rust binary, no browser:

```bash
cargo install --path crates/xsvg-cli
xsvg deck.xsvg --font-directory ./fonts -o deck.svg
```

## Features

A tour of the `x:` extensions. Every example is real xsvg that compiles to the image beside it.

### Text layout — wrap · fit · justify · flow

SVG has no line breaking. xsvg lays text out from real font metrics: greedy wrapping, full
justification, shrink-to-fit, and — the party trick — **flowing text inside any shape's outline**,
not just a rectangle.

```xml
<circle id="cir" cx="120" cy="120" r="90"/>
<x:textbox in="#cir" align="center" valign="middle">
  Text flows inside any shape's outline, wrapping to the curve line by line, out to the rim.
</x:textbox>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/text-flow.png" alt="text flowed inside shapes">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/text-justify.png" alt="justified text with first-line indent">

### Type on a path

Ride text along any curve. `effect="follow"` is a live, selectable native `<textPath>`; `skew`
and `ribbon` deform the glyphs along the curve (verticals upright, or tilting with the path).

```xml
<path id="w" d="M40,120 C160,40 320,200 460,120"/>
<x:textpath in="#w" effect="ribbon" font-family="-x-google-Anton" font-size="34">RIBBON</x:textpath>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/textpath.png" alt="type on a path — skew, ribbon, follow">

### Warp the geometry

Illustrator-style envelope distortion — bend shapes and outlined text through arch, flag, rise,
fisheye, perspective, and free-distort fields.

```xml
<x:warp field="arch" bend="60">
  <rect x="0" y="0" width="200" height="70" rx="8"/>
</x:warp>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/warp.png" width="560" alt="envelope warp presets">

### Mesh gradients

Corner colours on a quad/tri mesh — the smooth multi-focal blends no linear or radial gradient can
express. SVG 2 specified `<meshgradient>`, but no browser draws it; xsvg bakes it to a tiny
texel-aligned PNG that the renderer's own bilinear filter reconstructs. Inkscape/SVG-2
`<meshgradient>` input is accepted too.

```xml
<x:mesh points="0,0 200,0 200,120 0,120">
  <x:face v="0 1 2 3" fill="#f59e0b #ec4899 #6366f1 #10b981"/>
</x:mesh>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/mesh.png" alt="mesh gradients">

### Diagrams — connectors & path algebra

`<x:connector>` routes a line between two elements' edges and **re-derives when a box moves**
(straight, orthogonal rails, or a curve; arrowheads baked). `<x:boolean>` is Pathfinder-style path
algebra (union · subtract · intersect · exclude); `<x:offset>` grows or shrinks a region by a
Minkowski distance (inset & outset).

```xml
<rect id="a" .../> <rect id="b" .../>
<x:connector from="#a" to="#b" route="curve" arrow="end"/>

<x:boolean op="subtract" fill="#1d4ed8">          <x:offset in="#blob" distance="14"
  <rect .../>                                                fill="none" stroke="#6366f1"/>
  <x:textbox outline="true">PUNCH</x:textbox>
</x:boolean>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/connectors.png" alt="self-routing connectors">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/boolean.png" alt="boolean path algebra">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/offset.png" alt="inset and outset offsets">

### Paragraphs, lists & tables

The document primitives SVG lacks. `<x:p>` stacks paragraphs with spacing, indents and
per-paragraph style; `<x:list>` gives hanging-indent bullets/numbers with markers that cycle by
depth; `<x:table>` has author-set columns and **row heights that grow to fit** wrapped content.

```xml
<x:table cols="150 * *" stripe="#f8fafc">
  <x:tr><x:th>Feature</x:th><x:th>What it does</x:th><x:th>Degrades to</x:th></x:tr>
  <x:tr><x:td>Mesh gradients</x:td><x:td>Corner colours on curved Coons patches…</x:td><x:td>PNG</x:td></x:tr>
</x:table>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/table.png" alt="content-driven tables">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/paragraphs.png" alt="paragraph typography">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/lists.png" alt="lists with depth-cycling markers">

### Data visualization

A data-coordinate frame: `<x:plot>` maps a domain onto a pixel box (y inverted) and draws
bottom-aligned `<x:bars>` and mapped `<x:line>` graphs; `<x:pie>` turns a value into an angle, with
per-slice radius, explode, donut, and polar-area — all baked to plain shapes, no chart runtime.

```xml
<x:plot y-domain="0 100" y-ticks="5">           <x:pie cx="80" cy="80" r="70" inner-radius="36">
  <x:bars fill="#6366f1">                          <x:slice value="40" fill="#6366f1"/>
    <x:bar value="45" label="Q1"/>                 <x:slice value="25" fill="#0ea5e9"/>
    <x:bar value="72" label="Q2"/>                 <x:slice value="35" fill="#f59e0b"/>
  </x:bars>                                       </x:pie>
</x:plot>
```

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/plot.png" alt="bar and line plots">
<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/pie.png" alt="pie, donut, polar-area">

### Theming

`<x:theme>` declares design tokens — a brand palette and a type scale — resolved at compile time.
Fills reference `var(name)`; text pulls a named `x:font`. Change a token and the whole document
restyles; an unknown viewer never sees the `var()` (it's already resolved).

<img src="https://raw.githubusercontent.com/visioncortex/xsvg/main/assets/readme/theme.png" alt="theming with design tokens">

## Packages & crates

| Name | Kind | What it is |
|---|---|---|
| [`@visioncortex/xsvg-viewer`](packages/xsvg-viewer) | npm · browser | WASM compiler + `createPreview` slide-deck surface |
| [`@visioncortex/xsvg-compile`](packages/xsvg-compile) | npm · Node | Browser-free `compile()`; opentype.js fonts, synchronous |
| `xsvg-cli` | crate · binary | The `xsvg` CLI — pure-Rust AOT compile, no browser |
| `xsvg-core` | crate · lib | The compiler + geometry/text/gradient primitives (`compile` feature) |

## How it compiles

One Rust compiler, two platform backings. `xsvg-core` holds the whole lowering pipeline behind
three platform seams — font metrics, glyph outlining, and shape rasterization (for text flow).
`xsvg-wasm` backs them with browser callbacks (canvas, opentype.js) for the viewer; `xsvg-cli` and
the Node package back them with native libraries (`ttf-parser`, `kurbo`, opentype.js). Because the
browser and native paths run the *same* compiler, their output is verified pixel-identical for
geometry and within a hair for text — see [docs/Headless.md](docs/Headless.md).

## Development

```bash
nvm use          # Node (pinned via .nvmrc)
npm install
npm run dev      # build the wasm (debug) + start Vite → http://localhost:5173
```

The dev app is a hub linking a live **playground**, an interactive **viewer** (pan/zoom + a
source-mapped element inspector), and a bare **preview**. Open any sample by name, e.g.
`/playground/?file=pie.xsvg`.

| Command | What it does |
|---|---|
| `npm run dev` | wasm (debug) + Vite dev server, hot reload |
| `npm run build` | wasm (release) + bundle the tool pages + single-file embed |
| `npm run build:packages` | build both npm packages (release wasm + dist) |
| `npm run compare` | browser-vs-native parity suite over every sample |
| `cargo test` | the compiler's Rust test suite |

## Documentation

- [Specification.md](docs/Specification.md) — the normative spec: language + lowering rules
- [Syntax.md](docs/Syntax.md) — language design (a graceful-degradation superset of SVG)
- [Typography.md](docs/Typography.md) — the typesetting capabilities catalog
- [Headless.md](docs/Headless.md) — the native compiler + browser-parity test suite
- [Vision.md](docs/Vision.md) · [Plan.md](docs/Plan.md) · [Research.md](docs/Research.md) — north star, architecture, prior art

## License

[MIT](LICENSE) © 2026 Seafire Software Limited.
