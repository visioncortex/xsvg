// Checks the plugin sandbox's **acquisition** and **execution** layers
// (docs/Sandbox.md §2, §3), which decide whether hostile code runs at all.
//
// The output layer (§4) is pinned in Rust, and being host-independent, one suite
// covers every host. These two are not: they live in each host separately, and the
// browser's — the only thing standing between a document and a silently swapped
// remote module — had no tests whatsoever. Output hardening does nothing for a
// module that gets *loaded*; it only stops what a loaded module may draw.
//
//   npm run check:sandbox

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { PluginRegistry as BrowserRegistry } from "../packages/xsvg-viewer/src/plugins.ts";
import { PluginRegistry as NodeRegistry } from "../packages/xsvg-compile/src/plugins.ts";

const ROOT = resolve(import.meta.dirname, "..");
// A real module, so a passing load proves the whole path, not a stub of it.
const WASM = readFileSync(join(ROOT, "dataset/clipart.wasm"));
const HASH = `sha256-${createHash("sha256").update(WASM).digest("base64")}`;
const WRONG = `sha256-${createHash("sha256").update("not the module").digest("base64")}`;
const NS = "https://xsvg.visioncortex.org/ext/clipart";

// `isCrossOrigin` falls back to `location.href` for an empty base; every case below
// passes one explicitly, but a missing global should not be what makes a test pass.
globalThis.location ??= { href: "https://app.example/doc.xsvg" };

let passed = 0;
function check(name, fn) {
  try {
    const r = fn();
    if (r instanceof Promise) return r.then(() => void passed++, fail(name));
    passed++;
  } catch (e) {
    fail(name)(e);
  }
}
const fail = (name) => (e) => {
  console.error(`FAIL  ${name}\n      ${String(e.message ?? e).split("\n")[0]}`);
  process.exitCode = 1;
};

/** Whether the namespace's module actually loaded.
 *
 *  Not `generate`: a *loaded* module answers that with its own complaint about the
 *  request, which reads like a load failure and is not one. A namespace whose module
 *  failed claims every tag as ready (so `generate` can report the real reason); one
 *  that loaded consults its manifest and calls an unknown tag unknown. */
const loaded = (reg) => reg.element(NS, "no-such-element-exists")?.state === "unknown";

/** The load failure a document's marker would carry, or null if it loaded. */
function reason(reg) {
  if (loaded(reg)) return null;
  return JSON.parse(reg.generate(NS, "arrow", "{}")).error ?? null;
}

/** A host loader serving fixed bytes, as a sandboxed embedder does. */
const serving = (bytes) => ({
  key: (base, src) => `${base}#${src}`,
  fetch: async () => bytes,
});

// ---- acquisition: the browser ---------------------------------------------

const BASE = "https://app.example/doc.xsvg";
const decl = (extra = {}) => [{ ns: NS, src: "clipart.wasm", integrity: null, ...extra }];

await check("a module whose bytes match its hash loads", async () => {
  const reg = await BrowserRegistry.load(decl({ integrity: HASH }), serving(WASM), BASE);
  assert.ok(loaded(reg), `a correctly pinned module should load: ${reason(reg)}`);
  // and it answers from its own manifest, not the failed-namespace stub
  assert.equal(reg.element(NS, "callout")?.children, "container");
});

await check("a module whose bytes do not match its hash is refused", async () => {
  const reg = await BrowserRegistry.load(decl({ integrity: WRONG }), serving(WASM), BASE);
  assert.match(reason(reg) ?? "", /integrity mismatch/);
});

await check("a cross-origin module with no hash is refused", async () => {
  // Exactly the case where a swapped module would be undetectable: bytes from
  // somewhere else, and nothing in the document that would notice them changing.
  const remote = { key: () => "https://cdn.example/clipart.wasm", fetch: async () => WASM };
  const reg = await BrowserRegistry.load(decl(), remote, BASE);
  assert.match(reason(reg) ?? "", /cross-origin module needs an integrity hash/);

  // ...and is allowed once pinned, which is the whole point of the rule
  const pinned = await BrowserRegistry.load(decl({ integrity: HASH }), remote, BASE);
  assert.ok(loaded(pinned), `a pinned cross-origin module should load: ${reason(pinned)}`);
});

await check("a same-origin module loads without a hash, and a bad hash still wins", async () => {
  const same = { key: () => "https://app.example/clipart.wasm", fetch: async () => WASM };
  assert.ok(loaded(await BrowserRegistry.load(decl(), same, BASE)));
  const reg = await BrowserRegistry.load(decl({ integrity: WRONG }), same, BASE);
  assert.match(reason(reg) ?? "", /integrity mismatch/, "same-origin must not skip a declared hash");
});

await check("an unparseable location is treated as cross-origin", async () => {
  // Fail closed: a key the URL parser rejects must demand a hash rather than be
  // waved through as "not cross-origin".
  const opaque = { key: () => "not a url at all", fetch: async () => WASM };
  const reg = await BrowserRegistry.load(decl(), opaque, BASE);
  assert.match(reason(reg) ?? "", /cross-origin module needs an integrity hash/);
});

await check("a host loader's own bytes are still verified", async () => {
  // The loader hook exists so a sandboxed embedder can supply bytes over its own
  // channel — it is not a way around the hash the document asked for.
  const reg = await BrowserRegistry.load(
    decl({ integrity: HASH }),
    serving(Buffer.from("definitely not wasm")),
    BASE,
  );
  assert.match(reason(reg) ?? "", /integrity mismatch/);
});

await check("a module that cannot be fetched degrades rather than throwing", async () => {
  const broken = { key: () => "https://app.example/x.wasm", fetch: async () => { throw new Error("404"); } };
  const reg = await BrowserRegistry.load(decl(), broken, BASE);
  assert.match(reason(reg) ?? "", /404/);
});

// ---- acquisition: Node ----------------------------------------------------

const DOC = join(ROOT, "dataset/doc.xsvg"); // resolves siblings in dataset/

check("Node verifies a declared hash", () => {
  assert.ok(loaded(NodeRegistry.load(decl({ integrity: HASH }), DOC)));
  assert.match(
    reason(NodeRegistry.load(decl({ integrity: WRONG }), DOC)) ?? "",
    /integrity mismatch/,
  );
});

check("the native hosts refuse to fetch a module over the network", () => {
  // A command-line or server-side compile silently reaching the network would be a
  // surprise; vendor the module beside the document instead.
  for (const src of [
    "https://cdn.example/clipart.wasm",
    "file:///tmp/clipart.wasm",
    // no authority, so a `://` test misses it and it becomes a file read
    "data:application/wasm;base64,AGFzbQ==",
  ]) {
    const reg = NodeRegistry.load(decl({ src }), DOC);
    assert.match(reason(reg) ?? "", /give a local path/, `\`${src}\` was not refused`);
  }
  // a relative path is still a path — including one that looks drive-lettered
  assert.doesNotMatch(
    reason(NodeRegistry.load(decl({ src: "c:missing.wasm" }), DOC)) ?? "",
    /give a local path/,
  );
});

// ---- execution: the module gets nothing to call ---------------------------

check("no host grants a generator any import", () => {
  // "Empty import object" is one token per host, and a regression that added a
  // capability would be silent — the module would simply gain it. Checked at the
  // source, because the guarantee is the absence of something.
  const sites = [
    ["packages/xsvg-viewer/src/plugins.ts", /new WebAssembly\.Instance\([^,]+,\s*\{\s*\}\s*\)/],
    ["packages/xsvg-compile/src/plugins.ts", /new WebAssembly\.Instance\([^,]+,\s*\{\s*\}\s*\)/],
  ];
  for (const [file, ok] of sites) {
    const src = readFileSync(join(ROOT, file), "utf8");
    const calls = src.match(/new WebAssembly\.Instance\([\s\S]*?\)/g) ?? [];
    assert.ok(calls.length > 0, `${file}: no instantiation found — did it move?`);
    for (const call of calls) {
      assert.match(call, ok, `${file} instantiates a generator with imports: ${call}`);
    }
  }
  // wasmi takes imports from a Linker; an empty one can satisfy no import at all,
  // so the guarantee there is that nothing is ever defined on it.
  const cli = readFileSync(join(ROOT, "crates/xsvg-cli/src/plugins.rs"), "utf8");
  assert.match(cli, /Linker::<\(\)>::new/, "the CLI no longer uses a bare Linker");
  for (const banned of [".define(", ".func_wrap(", ".func_new("]) {
    assert.ok(!cli.includes(banned), `the CLI defines a host function for generators: ${banned}`);
  }
});

// ---- the output rules are load-bearing ------------------------------------

check("every sandbox rule has a test that fails without it", () => {
  // The output rules (§4) were verified by hand: revert one, watch a test fail.
  // That is not evidence anyone else can re-run, so the pairing is recorded here —
  // a rule added without a test to notice its removal is a rule that will rot.
  const tests = readFileSync(join(ROOT, "crates/xsvg-core/src/compile/tests.rs"), "utf8");
  for (const [rule, test] of [
    ["plugin_url_ok", "plugin_output_cannot_reach_the_network"],
    ["foreignObject> from a generator", "plugin_output_cannot_reach_the_network"],
    ["x-plg-", "plugin_ids_cannot_shadow_the_document"],
    ["outside the static subset", "generator_output_is_confined_to_the_static_subset"],
    ["generators do not nest", "generators_do_not_nest_in_generator_output"],
  ]) {
    assert.ok(tests.includes(test), `no test named ${test} covers \`${rule}\``);
  }
  // and the asymmetry the model rests on is itself pinned
  assert.ok(tests.includes("an_author_may_still_write_what_a_generator_may_not"));
});

if (!process.exitCode) console.log(`sandbox: ${passed} checks passed`);
