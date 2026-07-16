// woff2 → sfnt(ttf) decompressor, built from google/woff2 + brotli via emscripten.
// See ../../../../woff2/ for the source + `npm run woff2:build`.
//
// The .mjs is an ESM factory (emcc -s MODULARIZE -s EXPORT_ES6); the .wasm is a
// standalone asset. We import the wasm through Vite's `?url` so Vite hashes it and
// hands back the final URL, which we feed to emscripten's `locateFile` — no
// reliance on the module guessing its own path (the part that breaks under Vite).

// The .mjs is generated emscripten ESM (allowJs resolves its default export).
import createWoff2Decoder from "./decompress.mjs";
import wasmUrl from "./decompress.wasm?url";

interface Woff2Module {
  // embind: std::string arg accepts a Uint8Array (raw bytes); returns a
  // Uint8Array view into wasm memory, or `false` on failure.
  decompress(input: Uint8Array): Uint8Array | false;
}

let modP: Promise<Woff2Module> | undefined;

function load(): Promise<Woff2Module> {
  return (modP ??= createWoff2Decoder({
    locateFile: (path: string) => (path.endsWith(".wasm") ? wasmUrl : path),
  }) as Promise<Woff2Module>);
}

/** Decompress a woff2 buffer to sfnt (ttf) bytes. Throws if the input isn't valid woff2. */
export async function decompress(input: Uint8Array): Promise<Uint8Array> {
  const mod = await load();
  const view = mod.decompress(input);
  if (view === false) throw new Error("ConvertWOFF2ToTTF failed");
  // Copy out of the wasm heap before it can be reused — the embind result is a
  // view over a std::string that no longer exists once decompress() returns.
  return new Uint8Array(view);
}
