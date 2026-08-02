// End-to-end check of the plugin seam (docs/Extensibility.md), headless.
//
// Drives the *real* pieces: the compiler's `imports()` pre-scan, a real
// `.wasm` generator module loaded off disk, and `compile_ext` with the two host
// callbacks. Nothing here is a stand-in except the font seams, which have no
// bearing on whether a generator was reached.
//
//   node scripts/plugin-e2e.mjs [document.xsvg]
//
// Run `npm run wasm:node` and `npm run ext:build` first.

import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const wasm = require("../packages/xsvg-compile/pkg/xsvg_wasm.js");

const ABI_VERSION = 1;

// ---- the module host: memory plumbing, nothing more -----------------------

function loadModule(bytes, ns) {
  const ex = new WebAssembly.Instance(new WebAssembly.Module(bytes), {}).exports;
  if (ex.xsvg_abi_version() !== ABI_VERSION)
    throw new Error(`module speaks ABI ${ex.xsvg_abi_version()}, host speaks ${ABI_VERSION}`);

  const readReply = (ptr) => {
    const len = new DataView(ex.memory.buffer).getUint32(ptr, true);
    const bytes = new Uint8Array(ex.memory.buffer, ptr + 4, len).slice();
    ex.xsvg_free(ptr, 4 + len);
    return new TextDecoder().decode(bytes);
  };

  const manifest = JSON.parse(readReply(ex.xsvg_describe()));
  if (manifest.ns !== ns)
    throw new Error(`module declares ${manifest.ns}, bound as ${ns}`);

  return {
    manifest,
    generate(requestJson) {
      const buf = new TextEncoder().encode(requestJson);
      const ptr = ex.xsvg_alloc(buf.length);
      // re-read the buffer after allocating: the module may have grown memory
      new Uint8Array(ex.memory.buffer).set(buf, ptr);
      return readReply(ex.xsvg_generate(ptr, buf.length));
    },
  };
}

// ---- run ------------------------------------------------------------------

const docPath = resolve(process.argv[2] ?? "dataset/clipart.xsvg");
const source = readFileSync(docPath, "utf8");
const baseDir = dirname(docPath);

const decls = JSON.parse(wasm.imports(source));
console.log(`imports declared: ${decls.length}`);
const modules = new Map();
for (const d of decls) {
  const path = resolve(baseDir, d.src);
  modules.set(d.ns, loadModule(readFileSync(path), d.ns));
  console.log(`  ${d.ns}\n    ← ${d.src} (${Object.keys(modules.get(d.ns).manifest.elements).join(", ")})`);
}

const element = (ns, tag) => {
  const mod = modules.get(ns);
  if (!mod) return null;
  const e = mod.manifest.elements?.[tag];
  return e
    ? { state: "ready", input: e.input ?? "none", children: e.children ?? "none" }
    : { state: "unknown" };
};
const generate = (ns, _tag, requestJson) => modules.get(ns).generate(requestJson);

// Font seams: a generator never sees them, but text children still need measuring.
const seams = [
  (text, css) => text.length * (parseFloat(css) || 16) * 0.5,
  () => [12.8, 3.2, 11.2, 8],
  () => [],
  () => "",
  () => NaN,
  () => null,
];

const svg = wasm.compile_ext(source, "balanced", false, ...seams, docPath, element, generate);

const markers = [...svg.matchAll(/<!-- xsvg: ([^>]*?) -->/g)].map((m) => m[1]);
console.log(`\npaths emitted: ${(svg.match(/<path/g) ?? []).length}`);
console.log(`markers: ${markers.length ? markers.join(" | ") : "none"}`);
console.log(`\n${svg}`);

if (markers.length) {
  console.error("\nFAIL: the document degraded somewhere");
  process.exit(1);
}
