// Interactive viewer page — a thin wrapper over the packaged <xsvg-view-interactive>
// element, which IS the viewer (pan/zoom + artboard deck + collapsible inspector with
// the source pane). This file only wires the app chrome the component doesn't own:
// opening bundled samples (?file=<name>) or a dropped file, the source→preview /
// playground links, and downloading the compiled SVG. The component is the shippable
// deliverable; this page dogfoods it.
import "../base.css";
import "@visioncortex/xsvg-viewer/interactive";
import { downloadSvg } from "@visioncortex/xsvg-viewer";
import { SAMPLES, requestedSample, datasetResolver } from "../core/samples";

type LinkResolver = (base: string, href: string) => [string, string] | null;
const view = document.getElementById("view") as HTMLElement & {
  source?: string | null;
  resolve?: LinkResolver | null;
};
// Samples are bundled strings, so their cross-file <use href> deps link against the
// bundled dataset (same as the preview) rather than a network fetch.
view.resolve = datasetResolver;
const docName = document.getElementById("doc-name")!;
const dropHint = document.getElementById("drop-hint")!;
const downloadLink = document.getElementById("download-svg") as HTMLAnchorElement;
const playgroundLink = document.getElementById("open-playground") as HTMLAnchorElement;

let currentSource: string | null = null;
let currentName = "drawing";

function open(name: string, source: string): void {
  currentSource = source;
  currentName = name;
  view.source = source; // property → the element recompiles + reframes
  docName.textContent = name;
  if (SAMPLES[name]) docName.setAttribute("href", `/preview/?file=${encodeURIComponent(name)}`);
  else docName.removeAttribute("href");
  playgroundLink.href = `/playground/${SAMPLES[name] ? `?file=${encodeURIComponent(name)}` : ""}`;
  downloadLink.hidden = false;
  dropHint.hidden = true;
}

// Drop-to-load, owned here (not the element's `droppable`) so we keep the source for
// the Download button and the sample-aware links.
["dragenter", "dragover"].forEach((t) => view.addEventListener(t, (e) => e.preventDefault()));
view.addEventListener("drop", (e) => {
  e.preventDefault();
  const file = (e as DragEvent).dataTransfer?.files?.[0];
  if (file) void file.text().then((text) => open(file.name, text));
});

downloadLink.addEventListener("click", (e) => {
  e.preventDefault();
  if (currentSource) void downloadSvg(currentSource, currentName);
});

const initial = requestedSample();
if (initial) open(initial, SAMPLES[initial]!);
