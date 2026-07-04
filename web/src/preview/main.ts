// Standalone vanilla renderer: compiles a bundled sample and injects the plain SVG,
// fit-to-contain within the viewport (scaled to fit, centered, never scrolls) so the
// page embeds cleanly in an iframe. The default target of .claude/screenshot.sh.
//
//   /preview/?file=<name>       render that dataset sample, fit to the viewport
//
// The SVG fills the viewport box (width/height:100% in CSS) and its own viewBox +
// preserveAspectRatio="xMidYMid meet" (the SVG default) scales the drawing to fit —
// up *or* down — and centers it. (max-width/max-height would only ever shrink, so a
// small diagram would sit at native size using a fraction of the space.)
import { compileXsvg } from "../core/compiler";
import { SAMPLES, DEFAULT_SAMPLE, requestedSample } from "../core/samples";

const name = requestedSample() ?? DEFAULT_SAMPLE;
document.title = `xsvg — ${name}`;
const app = document.getElementById("app")!;

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

void (async () => {
  const source = SAMPLES[name];
  if (!source) return;
  try {
    app.innerHTML = await compileXsvg(source);
  } catch (err) {
    app.innerHTML = `<pre class="error">${escapeHtml(String(err))}</pre>`;
  }
})();
