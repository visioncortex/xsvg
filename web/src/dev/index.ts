// Landing hub: the two links into the tools, plus a guided, categorized index of
// the bundled dataset samples. Each sample opens in the interactive viewer, with a
// secondary link into the playground. Any *.xsvg on disk the catalog doesn't
// mention is appended under "Uncategorized" so nothing is silently hidden.
import "../base.css";
import "./index.css";
import { CATALOG, SAMPLES, SAMPLE_NAMES } from "../core/samples";

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

function sampleCard(file: string, title: string, blurb: string): string {
  const f = encodeURIComponent(file);
  return `<li>
    <a class="open" href="/viewer/?file=${f}">
      <span class="title">${escapeHtml(title)}</span>
      <span class="blurb">${escapeHtml(blurb)}</span>
    </a>
  </li>`;
}

const catalogued = new Set<string>();
const sections = CATALOG.map((cat) => {
  const cards = cat.samples
    .filter((s) => {
      if (!SAMPLES[s.file]) {
        console.warn(`[xsvg] catalog lists missing sample: ${s.file}`);
        return false;
      }
      catalogued.add(s.file);
      return true;
    })
    .map((s) => sampleCard(s.file, s.title, s.blurb))
    .join("");
  const note = cat.note ? `<p class="note">${escapeHtml(cat.note)}</p>` : "";
  return `<section><h2>${escapeHtml(cat.name)}</h2>${note}<ul class="samples">${cards}</ul></section>`;
});

const orphans = SAMPLE_NAMES.filter((n) => !catalogued.has(n));
if (orphans.length) {
  const cards = orphans.map((n) => sampleCard(n, n, "(not yet in the curated index)")).join("");
  sections.push(`<section><h2>Uncategorized</h2><ul class="samples">${cards}</ul></section>`);
}

document.getElementById("app")!.innerHTML = `
  <div class="hub">
    <div class="hero">
      <h1>xsvg</h1>
      <p>eXtensible SVG — an XML graphics format that compiles to plain SVG, with the text layout, warps, and gradient meshes SVG never got.</p>
    </div>
    <div class="tools">
      <a class="tool" href="/viewer/">
        <span class="name">Interactive viewer →</span>
        <span class="desc">Pan, zoom, drop a file, and click any element to locate its xsvg source.</span>
      </a>
      <a class="tool" href="/playground/">
        <span class="name">Playground →</span>
        <span class="desc">Edit xsvg on the left, see the compiled SVG live on the right.</span>
      </a>
    </div>
    ${sections.join("")}
  </div>`;
