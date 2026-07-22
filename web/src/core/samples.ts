// The bundled dataset samples, enumerated from the directory at build time (raw
// strings), plus the curated CATALOG re-exported for convenience. In dev, Vite
// re-evaluates the glob when files are added/removed.
import { CATALOG, type Category, type Sample } from "./catalog";

export { CATALOG };
export type { Category, Sample };

const sampleModules = import.meta.glob("../../../dataset/*.xsvg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** name → source, e.g. "wrap-vs-overflow.xsvg" → "<xsvg …>…". */
export const SAMPLES: Record<string, string> = {};
for (const [path, content] of Object.entries(sampleModules)) {
  SAMPLES[path.split("/").pop()!] = content;
}

export const SAMPLE_NAMES = Object.keys(SAMPLES).sort();

// All dataset files (incl. link deps like logo.xsvg) as raw strings, so a sample's
// cross-file `<use href="logo.xsvg">` links against the *bundled* files in the browser
// — the CLI reads the same files from disk. Keyed by bare filename.
const depModules = import.meta.glob("../../../dataset/*.{svg,xsvg}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const DATASET_BY_NAME: Record<string, string> = {};
for (const [path, content] of Object.entries(depModules)) {
  DATASET_BY_NAME[path.split("/").pop()!] = content;
}

/** Resolve a bundled dataset dependency by filename. Pass as `compileXsvg`/`createPreview`'s
 *  `resolve` so a sample's `<use href="…">` links in-browser without a network fetch. */
export function datasetResolver(_base: string, href: string): [string, string] | null {
  const name = href.split("/").pop() ?? href;
  const src = DATASET_BY_NAME[name];
  return src ? [name, src] : null;
}

export const DEFAULT_SAMPLE = SAMPLE_NAMES.includes("wrap-vs-overflow.xsvg")
  ? "wrap-vs-overflow.xsvg"
  : SAMPLE_NAMES[0];

/** Normalize a requested name (append `.xsvg`), returning it if bundled, else null. */
export function resolveSample(name: string | null | undefined): string | null {
  if (!name) return null;
  const n = name.endsWith(".xsvg") ? name : `${name}.xsvg`;
  return SAMPLES[n] ? n : null;
}

/** The sample named by `?file=<name>` in the current URL, if bundled, else null. */
export function requestedSample(): string | null {
  return resolveSample(new URLSearchParams(location.search).get("file"));
}
