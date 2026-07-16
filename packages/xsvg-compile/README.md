# @visioncortex/xsvg-compile

Compile [xsvg](https://xsvg.visioncortex.org) to plain SVG in **Node.js** — no browser. xsvg
is an XML graphics format that compiles to plain SVG (the text layout, warps, and gradient
meshes SVG never got). This package runs the same WASM compiler the browser uses, backed by
native Node modules: [opentype.js](https://github.com/opentypejs/opentype.js) for font
metrics + glyph outlining, and a JS shape rasterizer for `in="#shape"` text flow. One
portable wasm + pure-JS deps — no native addon, no cross-compilation.

```bash
npm install @visioncortex/xsvg-compile
```

```ts
import { compile } from "@visioncortex/xsvg-compile";
import { writeFileSync } from "node:fs";

const svg = compile(source, { fontDir: "./fonts" }); // synchronous → plain-SVG string
writeFileSync("out.svg", svg);
```

## Fonts

Text measurement (line wrapping) and `outline="true"` glyph baking need real fonts. Point
`fontDir` at a directory of `.ttf`/`.otf` files; each is classified by family name + italic
bit (Anton is matched by name for the display slot, everything else falls back to a sans).
Without `fontDir`, the compile still succeeds — text just uses default metrics and stays
live `<text>` (no outline baking).

## API

```ts
compile(source: string, opts?: {
  quality?: "fast" | "balanced" | "highest"; // default "balanced"
  sourcemap?: boolean;                        // data-xsvg-pos byte ranges
  fontDir?: string;                           // fonts for measurement + outlining
}): string
```

## Notes

- **Synchronous** — the nodejs-target wasm loads the module synchronously, so `compile`
  returns the string directly (unlike the browser package's async `compileXsvg`).
- Fonts are cached per `fontDir` across calls.
- opentype.js drives one instance per font file; for a variable font, bold text measures at
  the default weight. Supply static weight files if exact bold metrics matter.

For an in-browser renderer with a preview/slide-deck surface, see **@visioncortex/xsvg-viewer**.
