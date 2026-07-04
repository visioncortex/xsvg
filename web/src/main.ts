// Full-screen xsvg viewer + a dev-only sample index.
//
//   /                     → (dev only) a <ul> index of dataset/ samples
//   /view/<name>.xsvg     → that sample, rendered full-screen
//   /?file=<name>         → same, via query param (works on any static host)
//
// The compiled SVG fills the viewport; the chosen sample, source, compiled
// output, and sample list are logged to the console.
import { compileXsvg } from "./xsvg";
import { CATALOG } from "./samples";

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

/** One `<a>` card for a sample, given its display title and blurb. */
function sampleCard(file: string, title: string, blurb: string): string {
  return `<li><a href="/view/${encodeURIComponent(file)}">
    <span class="title">${escapeHtml(title)}</span>
    <span class="blurb">${escapeHtml(blurb)}</span>
  </a></li>`;
}

// Curated, categorized index (source of truth: samples.ts). Any *.xsvg on disk that
// the catalog doesn't mention is appended under "Uncategorized" so nothing hides.
function renderIndex() {
  console.log("[xsvg] index — samples in dataset/:", sampleNames);

  const catalogued = new Set<string>();
  const sections = CATALOG.map((cat) => {
    const cards = cat.samples
      .filter((s) => {
        if (!samples[s.file]) {
          console.warn(`[xsvg] catalog lists missing sample: ${s.file}`);
          return false;
        }
        catalogued.add(s.file);
        return true;
      })
      .map((s) => sampleCard(s.file, s.title, s.blurb))
      .join("");
    const note = cat.note ? `<p class="note">${escapeHtml(cat.note)}</p>` : "";
    return `<section><h2>${escapeHtml(cat.name)}</h2>${note}<ul>${cards}</ul></section>`;
  });

  const orphans = sampleNames.filter((n) => !catalogued.has(n));
  if (orphans.length) {
    const cards = orphans.map((n) => sampleCard(n, n, "(not yet in the curated index)")).join("");
    sections.push(`<section><h2>Uncategorized</h2><ul>${cards}</ul></section>`);
  }

  app.innerHTML = `<nav class="index">
    <h1>xsvg samples</h1>
    ${sections.join("")}
  </nav>`;
}

async function renderSample(name: string) {
  const source = samples[name] ?? "";
  console.log("%c[xsvg] sample: " + name, "font-weight:bold");
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
