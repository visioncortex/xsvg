// Standalone vanilla renderer: compiles a bundled sample and injects the plain SVG,
// fit-to-contain within the viewport (scaled to fit, centered, never scrolls) so the
// page embeds cleanly in an iframe. The default target of .claude/screenshot.sh.
//
//   /preview/?file=<name>       render that dataset sample, fit to the viewport
//
// When the document has multiple artboards (§5.2) they become slides: a < > nav
// pages through them by reframing the SVG's viewBox to each artboard's frame
// (the SVG's own preserveAspectRatio="meet" then letterboxes it to fit).
import { compileXsvg } from "../core/compiler";
import { SAMPLES, DEFAULT_SAMPLE, requestedSample } from "../core/samples";
import { findArtboards, makeThumb } from "../core/artboards";

const name = requestedSample() ?? DEFAULT_SAMPLE;
document.title = `xsvg — ${name}`;
const app = document.getElementById("app")!;

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

/** Slide deck for a multi-artboard document: a PowerPoint-style thumbnail rail
 *  on the left (toggled from the bottom-left button) plus a ‹ n/N › nav. */
function setupDeck(svg: SVGSVGElement): void {
  const boards = findArtboards(svg);
  if (boards.length < 2) return;

  // ---- thumbnail rail: each thumb is a clone of the SVG reframed to a slide
  const rail = document.createElement("div");
  rail.className = "deck-rail";
  const thumbs: HTMLElement[] = boards.map((b, idx) => {
    const thumb = makeThumb(svg, b);
    thumb.addEventListener("click", () => show(idx));
    rail.appendChild(thumb);
    return thumb;
  });
  document.body.appendChild(rail);

  // ---- bottom-center ‹ n/N › nav
  const nav = document.createElement("div");
  nav.className = "deck-nav";
  const prev = document.createElement("button");
  prev.textContent = "‹";
  const label = document.createElement("span");
  label.className = "deck-label";
  const next = document.createElement("button");
  next.textContent = "›";
  nav.append(prev, label, next);
  document.body.appendChild(nav);

  // ---- bottom-left rail toggle
  const toggle = document.createElement("button");
  toggle.className = "deck-toggle";
  toggle.title = "Toggle slides";
  toggle.textContent = "▤";
  toggle.addEventListener("click", () => document.body.classList.toggle("rail-open"));
  document.body.appendChild(toggle);
  document.body.classList.add("rail-open"); // decks open the rail by default

  let i = 0;
  const show = (n: number) => {
    i = Math.max(0, Math.min(boards.length - 1, n));
    svg.setAttribute("viewBox", boards[i].frame.join(" "));
    label.textContent = `${i + 1} / ${boards.length} · ${boards[i].label}`;
    prev.disabled = i === 0;
    next.disabled = i === boards.length - 1;
    thumbs.forEach((t, k) => t.classList.toggle("active", k === i));
  };
  prev.addEventListener("click", () => show(i - 1));
  next.addEventListener("click", () => show(i + 1));
  window.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft" || e.key === "PageUp") show(i - 1);
    else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") show(i + 1);
  });
  // Deep-link to a slide with a 1-based #hash (e.g. …#3), and follow hash changes.
  const fromHash = () => {
    const n = parseInt(location.hash.slice(1), 10);
    return Number.isFinite(n) ? n - 1 : 0;
  };
  window.addEventListener("hashchange", () => show(fromHash()));
  show(fromHash());
}

void (async () => {
  const source = SAMPLES[name];
  if (!source) return;
  try {
    app.innerHTML = await compileXsvg(source);
    const svg = app.querySelector("svg");
    if (svg) setupDeck(svg as SVGSVGElement);
  } catch (err) {
    app.innerHTML = `<pre class="error">${escapeHtml(String(err))}</pre>`;
  }
})();
