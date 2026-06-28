// Full-screen xsvg viewer + a dev-only sample index.
//
//   /                     → (dev only) a <ul> index of dataset/ samples
//   /view/<name>.xsvg     → that sample, rendered full-screen
//   /?file=<name>         → same, via query param (works on any static host)
//
// The compiled SVG fills the viewport; the chosen sample, source, compiled
// output, and sample list are logged to the console.
import { compileXsvg } from "./xsvg";

// All dataset samples, enumerated from the directory at build time (raw strings).
// In dev, Vite re-evaluates this glob when files are added/removed.
const sampleModules = import.meta.glob("../../dataset/*.xsvg", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const samples: Record<string, string> = {};
for (const [path, content] of Object.entries(sampleModules)) {
  samples[path.split("/").pop()!] = content; // "wrap-vs-overflow.xsvg"
}
const sampleNames = Object.keys(samples).sort();
const DEFAULT_SAMPLE = sampleNames.includes("wrap-vs-overflow.xsvg")
  ? "wrap-vs-overflow.xsvg"
  : sampleNames[0];

/** Sample explicitly requested by `/view/<name>` or `?file=<name>`, else null. */
function requestedSample(): string | null {
  const fromPath = location.pathname.match(/^\/view\/(.+)$/)?.[1];
  let name = fromPath
    ? decodeURIComponent(fromPath)
    : (new URLSearchParams(location.search).get("file") ?? "");
  if (!name) return null;
  if (!name.endsWith(".xsvg")) name += ".xsvg";
  return samples[name] ? name : null;
}

const app = document.getElementById("app")!;

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

function renderIndex() {
  console.log("[xsvg] index — samples in dataset/:", sampleNames);
  const items = sampleNames
    .map((n) => `<li><a href="/view/${encodeURIComponent(n)}">${escapeHtml(n)}</a></li>`)
    .join("");
  app.innerHTML = `<nav class="index"><h1>xsvg samples</h1><ul>${items}</ul></nav>`;
}

async function renderSample(name: string) {
  const source = samples[name] ?? "";
  console.log("%c[xsvg] sample: " + name, "font-weight:bold");
  console.log("[xsvg] source:\n" + source);
  try {
    const svg = await compileXsvg(source, "balanced");
    console.log("[xsvg] compiled SVG:\n" + svg);
    app.innerHTML = svg;
  } catch (err) {
    console.error("[xsvg] compile error:", err);
    app.innerHTML = `<pre class="error">${escapeHtml(String(err))}</pre>`;
  }
}

async function render() {
  const name = requestedSample();
  if (name) {
    await renderSample(name);
  } else if (import.meta.env.DEV) {
    renderIndex();
  } else {
    await renderSample(DEFAULT_SAMPLE);
  }
}

window.addEventListener("popstate", () => void render());
void render();
