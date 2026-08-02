// opentype.js and svgpath ship no bundled types; we narrow the few APIs we use locally.
declare module "opentype.js";
declare module "svgpath";

// The package targets Node without the DOM lib, which is where TypeScript happens to
// declare `WebAssembly`. Node has had it since v8; these are the few members the
// plugin host uses (docs/Extensibility.md §5).
declare namespace WebAssembly {
  class Module {
    constructor(bytes: BufferSource);
  }
  class Instance {
    constructor(module: Module, imports?: Record<string, unknown>);
    readonly exports: Record<string, unknown>;
  }
  interface Memory {
    readonly buffer: ArrayBuffer;
  }
}
