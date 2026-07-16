// Standalone vanilla renderer: compiles a bundled sample and shows the plain SVG
// fit-to-contain within the viewport (scaled to fit, centered, never scrolls) so
// the page embeds cleanly in an iframe. The default target of .claude/screenshot.sh.
//
//   /preview/?file=<name>        render that dataset sample, fit to the viewport
//   /preview/?file=<name>#3      jump to the 3rd artboard of a multi-slide document
//
// The deck (rail + ‹ n/N › nav + #hash deep-linking) lives in the shared
// createPreview() component, which the playground reuses.
import { createPreview } from "@visioncortex/xsvg-viewer";
import { SAMPLES, DEFAULT_SAMPLE, requestedSample } from "../core/samples";

const name = requestedSample() ?? DEFAULT_SAMPLE;
document.title = `xsvg — ${name}`;
const app = document.getElementById("app")!;

const preview = createPreview(app, { hashDeepLink: true, showErrors: true });

void (async () => {
  const source = SAMPLES[name];
  if (source) await preview.render(source);
})();
