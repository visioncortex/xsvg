// Node host for xsvg geometry plugins (docs/Extensibility.md).
//
// The browser host has to fetch modules asynchronously before compiling; on Node
// the whole pipeline is synchronous, so loading is a `readFileSync` at the point
// the document's imports are known. Node supplies the WASM runtime itself, so
// this needs no dependency the package didn't already have.

import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, resolve as resolvePath } from "node:path";

/** One `<x:import>` binding, as reported by the compiler's pre-scan. */
export interface ImportDecl {
  ns: string;
  src: string;
  integrity: string | null;
}

/** The ABI this host speaks (docs/Extensibility.md §4). */
const ABI_VERSION = 1;

interface Exports {
  memory: WebAssembly.Memory;
  xsvg_abi_version(): number;
  xsvg_alloc(len: number): number;
  xsvg_free(ptr: number, len: number): void;
  xsvg_describe(): number;
  xsvg_generate(ptr: number, len: number): number;
}

interface Manifest {
  ns?: string;
  elements?: Record<string, { input?: string; children?: string }>;
}

/** Read a `u32`-LE length-prefixed UTF-8 reply and hand the buffer back. */
function readReply(ex: Exports, ptr: number): string {
  const len = new DataView(ex.memory.buffer).getUint32(ptr, true);
  const bytes = new Uint8Array(ex.memory.buffer, ptr + 4, len).slice();
  ex.xsvg_free(ptr, 4 + len);
  return new TextDecoder().decode(bytes);
}

/** Instantiate with no imports: a generator gets no host functions at all. */
function instantiate(module: WebAssembly.Module): Exports {
  return new WebAssembly.Instance(module, {}).exports as unknown as Exports;
}

class PluginModule {
  private instance: Exports | null = null;

  private constructor(
    private readonly module: WebAssembly.Module,
    readonly manifest: Manifest,
  ) {}

  static load(bytes: Buffer, ns: string): PluginModule {
    const module = new WebAssembly.Module(bytes);
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
    // pointed at one that answers for someone else's elements.
    if (manifest.ns !== ns)
      throw new Error(`module declares namespace ${manifest.ns ?? "(none)"}, bound as ${ns}`);
    return new PluginModule(module, manifest);
  }

  /** Drop the live instance so no state survives into the next compile. */
  reset(): void {
    this.instance = null;
  }

  element(tag: string): { input: string; children: string } | null {
    const e = this.manifest.elements?.[tag];
    return e ? { input: e.input ?? "none", children: e.children ?? "none" } : null;
  }

  generate(requestJson: string): string {
    const ex = (this.instance ??= instantiate(this.module));
    const buf = new TextEncoder().encode(requestJson);
    const ptr = ex.xsvg_alloc(buf.length);
    // re-read the buffer after allocating: the module may have grown memory
    new Uint8Array(ex.memory.buffer).set(buf, ptr);
    return readReply(ex, ex.xsvg_generate(ptr, buf.length));
  }
}

/**
 * The modules a document's imports resolve to, plus the two synchronous callbacks
 * the compiler drives during lowering.
 *
 * A namespace that failed to load is remembered as failed: each of its elements
 * degrades with a marker naming the reason, and the compile still succeeds.
 */
export class PluginRegistry {
  private loaded = new Map<string, PluginModule>();
  private failed = new Map<string, string>();

  /** Resolve every declared import against the document's path and load it. */
  static load(decls: ImportDecl[], basePath: string): PluginRegistry {
    const reg = new PluginRegistry();
    const dir = dirname(basePath);
    for (const decl of decls) {
      try {
        // A module is a local file like any other dependency; a URL is the
        // browser's business, and Node silently reaching the network would be a
        // surprise in a synchronous API.
        if (decl.src.includes("://"))
          throw new Error(`cannot fetch ${decl.src} — give a local path`);
        const path = resolvePath(dir, decl.src);
        const bytes = readFileSync(path);
        if (decl.integrity) verifyIntegrity(bytes, decl.integrity);
        reg.loaded.set(decl.ns, PluginModule.load(bytes, decl.ns));
      } catch (e) {
        reg.failed.set(decl.ns, e instanceof Error ? e.message : String(e));
      }
    }
    return reg;
  }

  get empty(): boolean {
    return this.loaded.size === 0 && this.failed.size === 0;
  }

  /** Start a fresh compile: no module carries state across this boundary. */
  beginCompile(): void {
    for (const m of this.loaded.values()) m.reset();
  }

  element = (ns: string, tag: string): { state: string; input?: string; children?: string } | null => {
    const mod = this.loaded.get(ns);
    if (!mod) {
      // A namespace whose module failed claims its elements anyway, so `generate`
      // can decline each with the real reason instead of a generic marker.
      return this.failed.has(ns) ? { state: "ready", input: "none", children: "none" } : null;
    }
    const e = mod.element(tag);
    return e ? { state: "ready", ...e } : { state: "unknown" };
  };

  generate = (ns: string, _tag: string, requestJson: string): string => {
    const mod = this.loaded.get(ns);
    if (!mod) return JSON.stringify({ error: this.failed.get(ns) ?? "module not loaded" });
    try {
      return mod.generate(requestJson);
    } catch (e) {
      // A trap kills the instance, not the compile.
      mod.reset();
      return JSON.stringify({ error: `generator trapped: ${e instanceof Error ? e.message : e}` });
    }
  };
}

/** Check an SRI-style `sha256-<base64>` digest. */
function verifyIntegrity(bytes: Buffer, want: string): void {
  const b64 = want.startsWith("sha256-") ? want.slice(7) : null;
  if (b64 == null) throw new Error(`unsupported integrity algorithm in "${want}"`);
  if (createHash("sha256").update(bytes).digest("base64") !== b64)
    throw new Error("integrity mismatch");
}

/** The registry a document with no imports (or plugins disabled) gets. */
export const noPlugins = new PluginRegistry();
