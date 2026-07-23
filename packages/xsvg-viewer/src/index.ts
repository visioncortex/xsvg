//! @visioncortex/xsvg-viewer — render xsvg in the browser.
//!
//! xsvg is an XML graphics format that compiles to plain SVG (the text layout, warps, and
//! gradient meshes SVG never got). This package bundles the WASM compiler and a drop-in
//! preview surface:
//!
//!   import { compileXsvg, createPreview } from "@visioncortex/xsvg-viewer";
//!
//!   // one-shot compile to a plain-SVG string
//!   const svg = await compileXsvg(source);
//!
//!   // or a self-contained preview (fit-to-contain + slide deck for multi-artboard docs)
//!   const preview = createPreview(hostEl, { hashDeepLink: true, showErrors: true });
//!   await preview.render(source);

export { compileXsvg, initFromBytes, registerOutlineFont, rasterize } from "./compiler";
export type { CompileOptions, DepLoader } from "./compiler";

export { createPreview } from "./preview";
export type { Preview, PreviewOptions, RenderResult } from "./preview";

export { findArtboards, makeThumb } from "./artboards";
export type { Artboard } from "./artboards";

export { downloadSvg } from "./download";
