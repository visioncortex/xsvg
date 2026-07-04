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
