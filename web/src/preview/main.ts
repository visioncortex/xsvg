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
import { findArtboards } from "../core/artboards";

const name = requestedSample() ?? DEFAULT_SAMPLE;
document.title = `xsvg — ${name}`;
const app = document.getElementById("app")!;

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

/** Slide nav for a multi-artboard document: < prev, "n / N", next >. */
function setupDeck(svg: SVGSVGElement): void {
  const boards = findArtboards(svg);
  if (boards.length < 2) return;

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

  let i = 0;
  const show = (n: number) => {
    i = Math.max(0, Math.min(boards.length - 1, n));
    svg.setAttribute("viewBox", boards[i].frame.join(" "));
    label.textContent = `${i + 1} / ${boards.length} · ${boards[i].label}`;
    prev.disabled = i === 0;
    next.disabled = i === boards.length - 1;
  };
  prev.addEventListener("click", () => show(i - 1));
  next.addEventListener("click", () => show(i + 1));
  window.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft" || e.key === "PageUp") show(i - 1);
    else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") show(i + 1);
  });
  show(0);
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
