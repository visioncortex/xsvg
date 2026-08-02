// Checks that every host-facing hook reaches every host-facing surface.
//
// `compileXsvg` takes hooks that let an embedder supply bytes the library cannot
// fetch for itself — a cross-file dependency, a generator module. They exist for
// hosts that are sandboxed: a VS Code webview under a strict CSP can reach neither
// `file://` nor an arbitrary origin, so a hook it cannot reach is a document it
// cannot render at all.
//
// The failure mode is silent and total. `compileXsvg` supported a `plugins` loader
// from the day generators landed, and its docstring said it was for "an embedder
// with its own module store" — but no element or preview surface forwarded it, so
// `<x:import>` fell back to same-origin fetch and external-WASM plugins were
// impossible in any sandboxed host. Nothing failed; the hook was simply unreachable.
//
// So: each surface must forward each hook, checked against the real call sites. Run:
//
//   npm run check:hosts

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const SRC = resolve(import.meta.dirname, "../packages/xsvg-viewer/src");

/** Hooks an embedder supplies because the library cannot obtain the bytes itself. */
const HOOKS = ["resolve", "loader", "plugins"];

/** Every surface a host renders through, and where its options object lives. */
const SURFACES = [
  { file: "element.ts", what: "<xsvg-view>" },
  { file: "interactive.ts", what: "<xsvg-view-interactive>" },
  { file: "preview.ts", what: "createPreview" },
];

let passed = 0;
function check(name, fn) {
  try {
    fn();
    passed++;
  } catch (e) {
    console.error(`FAIL  ${name}\n      ${e.message.split("\n")[0]}`);
    process.exitCode = 1;
  }
}

/** The options object literal of a source's `compileXsvg(...)` call. */
function compileOptions(src) {
  const at = src.indexOf("compileXsvg(");
  assert.ok(at >= 0, "no compileXsvg call");
  const open = src.indexOf("{", at);
  // Brace-match so nested callbacks (onPlugins, onSession) don't end it early.
  let depth = 0;
  for (let i = open; i < src.length; i++) {
    if (src[i] === "{") depth++;
    else if (src[i] === "}" && --depth === 0) return src.slice(open, i + 1);
  }
  throw new Error("unterminated options object");
}

for (const { file, what } of SURFACES) {
  const src = readFileSync(resolve(SRC, file), "utf8");
  const opts = compileOptions(src);
  for (const hook of HOOKS) {
    check(`${what} forwards \`${hook}\``, () => {
      assert.match(
        opts,
        new RegExp(`\\b${hook}\\s*:`),
        `${file} never passes \`${hook}\` to compileXsvg, so a host that sets it is ignored`,
      );
    });
  }
  // A property nothing can set is the same dead end as one nothing forwards.
  if (file !== "preview.ts") {
    for (const hook of HOOKS) {
      check(`${what} exposes \`${hook}\``, () => {
        assert.match(src, new RegExp(`set ${hook}\\s*\\(`), `${file} has no \`${hook}\` setter`);
      });
    }
  }
}

check("PluginLoader is nameable by a host writing one", () => {
  // A hook whose type a TypeScript embedder cannot import is only half-exposed —
  // `DepLoader` is exported for exactly this reason, so its counterpart must be too.
  const index = readFileSync(resolve(SRC, "index.ts"), "utf8");
  for (const t of ["DepLoader", "PluginLoader"]) {
    assert.match(index, new RegExp(`\\b${t}\\b`), `${t} is not exported from the package root`);
  }
});

if (!process.exitCode) console.log(`host hooks: ${passed} checks passed`);
