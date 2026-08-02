// Browser host for xsvg geometry plugins (docs/Extensibility.md).
//
// The compiler is synchronous and cannot load anything; loading modules is this
// file's job and happens *before* a compile. The document says which modules it
// wants via `<x:import>`, the compiler's `imports()` pre-scan reports them, and
// the registry below fetches, verifies, instantiates, and then answers the two
// synchronous callbacks the compiler calls during lowering.

import type { ElementSpec, PluginSchemas } from "./params.js";

/** One `<x:import>` binding, as reported by the compiler's pre-scan. */
export interface ImportDecl {
  ns: string;
  src: string;
  integrity: string | null;
}

/**
 * How a host fetches plugin modules. The default resolves `src` against the
 * document base and fetches same-origin; an embedder with its own module store
 * (an extension host, a bundle) supplies its own.
 */
export interface PluginLoader {
  /** Canonical identity of `src` resolved against `base`, or null to decline. */
  key(base: string, src: string): string | null;
  /** Fetch module bytes for a canonical key. */
  fetch(key: string): Promise<ArrayBuffer>;
}

export function fetchPluginLoader(base: string): PluginLoader {
  return {
    key(b, src) {
      try {
        return new URL(src, b || base).href;
      } catch {
        return null;
      }
    },
    async fetch(key) {
      const res = await fetch(key);
      if (!res.ok) throw new Error(`${res.status} ${res.statusText}`);
      return res.arrayBuffer();
    },
  };
}

/** What a module says about itself. Only the fields the host acts on are typed. */
interface Manifest {
  abi?: number;
  ns?: string;
  elements?: Record<string, ElementSpec>;
}

/** The ABI this host speaks (docs/Extensibility.md §4). */
const ABI_VERSION = 1;

/** The core namespace, whose elements the compiler describes itself. */
const XSVG_NS = "https://xsvg.visioncortex.org";

interface Exports {
  memory: WebAssembly.Memory;
  xsvg_abi_version(): number;
  xsvg_alloc(len: number): number;
  xsvg_free(ptr: number, len: number): void;
  xsvg_describe(): number;
  xsvg_generate(ptr: number, len: number): number;
}

/** A validated module: the compiled code plus what its manifest declared. */
class PluginModule {
  private instance: Exports | null = null;

  private constructor(
    readonly module: WebAssembly.Module,
    readonly manifest: Manifest,
  ) {}

  /** Compile, instantiate once to read the manifest, and check it fits the ABI. */
  static async load(bytes: ArrayBuffer, ns: string): Promise<PluginModule> {
    const module = await WebAssembly.compile(bytes);
    const probe = instantiate(module);
    if (probe.xsvg_abi_version() !== ABI_VERSION)
      throw new Error(`module speaks ABI ${probe.xsvg_abi_version()}, host speaks ${ABI_VERSION}`);
    let manifest: Manifest;
    try {
      manifest = JSON.parse(readReply(probe, probe.xsvg_describe()));
    } catch {
      throw new Error("module manifest is not valid JSON");
    }
    // A module must claim the namespace it was bound to, or a document could be
    // pointed at a module that answers for someone else's elements.
    if (manifest.ns !== ns)
      throw new Error(`module declares namespace ${manifest.ns ?? "(none)"}, bound as ${ns}`);
    return new PluginModule(module, manifest);
  }

  /**
   * Drop the live instance. The registry does this between compiles so any state
   * a module retained cannot leak from one compile into the next — incremental
   * re-emission assumes a fragment's output depends only on its subtree, and a
   * stateful generator would quietly break that. Re-instantiating an
   * already-compiled module is cheap; recompiling it would not be.
   */
  reset(): void {
    this.instance = null;
  }

  private live(): Exports {
    if (!this.instance) this.instance = instantiate(this.module);
    return this.instance;
  }

  element(tag: string): { input: string; children: string } | null {
    const e = this.manifest.elements?.[tag];
    if (!e) return null;
    return { input: e.input ?? "none", children: e.children ?? "none" };
  }

  generate(requestJson: string): string {
    const ex = this.live();
    const buf = new TextEncoder().encode(requestJson);
    const ptr = ex.xsvg_alloc(buf.length);
    // read `buffer` after every call into the module: allocating may have grown it
    new Uint8Array(ex.memory.buffer).set(buf, ptr);
    return readReply(ex, ex.xsvg_generate(ptr, buf.length));
  }
}

function instantiate(module: WebAssembly.Module): Exports {
  // No imports by contract: a generator gets no WASI, no host functions, no way
  // to observe anything but its own request.
  return new WebAssembly.Instance(module, {}).exports as unknown as Exports;
}

/** Read a `u32`-LE length-prefixed UTF-8 reply and hand the buffer back. */
function readReply(ex: Exports, ptr: number): string {
  const len = new DataView(ex.memory.buffer).getUint32(ptr, true);
  const bytes = new Uint8Array(ex.memory.buffer, ptr + 4, len).slice();
  ex.xsvg_free(ptr, 4 + len);
  return new TextDecoder().decode(bytes);
}

/** `sha256-<base64>`, the same spelling as an HTML `integrity` attribute. */
async function digestMatches(bytes: ArrayBuffer, integrity: string): Promise<boolean> {
  const [algo, want] = integrity.split("-", 2);
  if (algo !== "sha256" || !want) return false;
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  let bin = "";
  for (const b of new Uint8Array(hash)) bin += String.fromCharCode(b);
  return btoa(bin) === want;
}

/**
 * The set of modules a document's imports resolve to, plus the two synchronous
 * callbacks the compiler drives during lowering.
 *
 * A namespace that failed to load is remembered as failed: the compiler degrades
 * each of its elements with a marker naming the reason, and nothing is retried
 * mid-compile.
 */
export class PluginRegistry {
  private loaded = new Map<string, PluginModule>();
  private failed = new Map<string, string>();

  /** Fetch, verify, and instantiate every declared import. Failures are recorded,
   *  not thrown — a document with a broken import still compiles (§3 totality). */
  static async load(
    decls: ImportDecl[],
    loader: PluginLoader,
    base: string,
  ): Promise<PluginRegistry> {
    const reg = new PluginRegistry();
    await Promise.all(
      decls.map(async (decl) => {
        try {
          const key = loader.key(base, decl.src);
          if (key == null) throw new Error(`cannot resolve ${decl.src}`);
          const bytes = await loader.fetch(key);
          // Cross-origin without a pinned hash is exactly the case where a
          // silently swapped module would be undetectable, so refuse it.
          if (decl.integrity) {
            if (!(await digestMatches(bytes, decl.integrity)))
              throw new Error("integrity mismatch");
          } else if (isCrossOrigin(key, base)) {
            throw new Error("cross-origin module needs an integrity hash");
          }
          reg.loaded.set(decl.ns, await PluginModule.load(bytes, decl.ns));
        } catch (e) {
          reg.failed.set(decl.ns, e instanceof Error ? e.message : String(e));
        }
      }),
    );
    return reg;
  }

  get empty(): boolean {
    return this.loaded.size === 0 && this.failed.size === 0;
  }

  /** Start a fresh compile: no module carries state across this boundary. */
  beginCompile(): void {
    for (const m of this.loaded.values()) m.reset();
  }

  /**
   * Parameter schemas for **every** element the document can use — the
   * compiler's own `x:` elements alongside whatever the loaded modules declare
   * (docs/Extensibility.md §11). Both come from the same derived declarations,
   * so a property panel reads one lookup and never branches on tier.
   *
   * Reading this runs no generator.
   */
  schemas(builtIns?: Record<string, ElementSpec>): PluginSchemas {
    const core = builtIns ?? {};
    return {
      spec: (ns, tag) =>
        (ns === XSVG_NS ? core[tag] : this.loaded.get(ns)?.manifest.elements?.[tag]) ?? null,
    };
  }

  /** The compiler's `element(ns, tag)` callback. */
  element = (ns: string, tag: string): { state: string; input?: string; children?: string } | null => {
    const mod = this.loaded.get(ns);
    if (!mod) {
      // A namespace whose module failed to load claims its elements anyway, so
      // that `generate` can decline each one with the real reason ("integrity
      // mismatch", "404"). Reporting it unbound instead would compile to a
      // generic marker and hide the thing the author needs to fix.
      return this.failed.has(ns) ? { state: "ready", input: "none", children: "none" } : null;
    }
    const e = mod.element(tag);
    return e ? { state: "ready", ...e } : { state: "unknown" };
  };

  /** The compiler's `generate(ns, tag, requestJson)` callback. */
  generate = (ns: string, _tag: string, requestJson: string): string => {
    const mod = this.loaded.get(ns);
    if (!mod) return JSON.stringify({ error: this.failed.get(ns) ?? "module not loaded" });
    try {
      return mod.generate(requestJson);
    } catch (e) {
      // A trap (out of memory, unreachable) kills the instance, not the compile.
      mod.reset();
      return JSON.stringify({ error: `generator trapped: ${e instanceof Error ? e.message : e}` });
    }
  };
}

function isCrossOrigin(url: string, base: string): boolean {
  try {
    return new URL(url).origin !== new URL(base || location.href).origin;
  } catch {
    return true;
  }
}

/** The registry every document gets when a host declares no plugin loader. */
export const noPlugins = new PluginRegistry();
