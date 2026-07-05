// Interactive viewer bootstrap — pan/zoom canvas + source-mapped element inspector.
//
//   /viewer/?file=<name>       opens a bundled dataset sample
//   drop a .xsvg file           opens it
//
// Compiles with the source map on, renders into a pan/zoom stage, mirrors the xsvg
// into a read-only editor pane, and wires the inspector so clicking a rendered
// element highlights its originating source.
import "../base.css";
import "./viewer.css";
import { compileXsvg } from "../core/compiler";
import { SAMPLES, requestedSample } from "../core/samples";
import { createEditor } from "../core/editor";
import { createPanZoom, type PanZoom } from "./pan-zoom";
import { createInspector, type Inspector } from "./inspector";

function byId(id: string): HTMLElement {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
}

const viewerEl = document.querySelector(".viewer") as HTMLElement;
const stage = byId("stage");
const content = byId("content");
const panel = byId("panel");
const errorBox = byId("error");
const docName = byId("doc-name");
const dropHint = byId("drop-hint");
const playgroundLink = byId("open-playground") as HTMLAnchorElement;

const editor = createEditor({ parent: byId("source"), readOnly: true });
let panzoom: PanZoom | null = null;
let inspector: Inspector | null = null;
let fitted = true; // whether the view is currently fit-to-screen (vs 1:1)

// Source → canvas: a click in the read-only source pane pins the element whose
// xsvg range is under the pointer. posAtCoords maps the click to a doc offset.
editor.view.dom.addEventListener("mousedown", (e) => {
  const pos = editor.view.posAtCoords({ x: e.clientX, y: e.clientY });
  if (pos != null) inspector?.selectAtSourceOffset(pos);
});

const iconFit = byId("fit-btn").querySelector<SVGElement>(".icon-fit")!;
const iconOneOne = byId("fit-btn").querySelector<SVGElement>(".icon-oneone")!;

/** Show the icon for the current zoom mode: four-corners when fit, "1:1" at actual size. */
function updateFitIcon(): void {
  iconFit.toggleAttribute("hidden", !fitted);
  iconOneOne.toggleAttribute("hidden", fitted);
}

/** Give the compiled SVG an explicit pixel size from its viewBox, so the pan/zoom
 *  content box has a definite size to fit and center. */
function sizeToViewBox(svg: SVGSVGElement): void {
  const vb = svg.getAttribute("viewBox");
  const p = vb ? vb.split(/[\s,]+/).map(Number) : [];
  const [w, h] = p.length === 4 && p[2] > 0 && p[3] > 0 ? [p[2], p[3]] : [400, 300];
  svg.setAttribute("width", String(w));
  svg.setAttribute("height", String(h));
}

async function open(name: string, source: string): Promise<void> {
  docName.textContent = name;
  dropHint.hidden = true; // a document is loaded now — drop the hint
  // The filename links to its bare preview (only bundled samples have a /preview/
  // route; a dropped file stays plain text).
  if (SAMPLES[name]) docName.setAttribute("href", `/preview/?file=${encodeURIComponent(name)}`);
  else docName.removeAttribute("href");
  playgroundLink.href = `/playground/${SAMPLES[name] ? `?file=${encodeURIComponent(name)}` : ""}`;
  editor.setDoc(source);

  inspector?.destroy();
  panzoom?.destroy();
  inspector = null;
  panzoom = null;

  try {
    const svg = await compileXsvg(source, { sourcemap: true });
    content.innerHTML = svg;
    errorBox.hidden = true;
    const svgEl = content.querySelector("svg");
    if (svgEl) {
      sizeToViewBox(svgEl as SVGSVGElement);
      panzoom = createPanZoom(stage, content);
      panzoom.fit();
      fitted = true;
      updateFitIcon();
      inspector = createInspector({
        svgRoot: svgEl as SVGSVGElement,
        panel,
        source,
        editor,
      });
      panzoom.onBackgroundClick(() => inspector?.clear()); // click empty area to deselect
    }
  } catch (err) {
    content.innerHTML = "";
    errorBox.hidden = false;
    errorBox.textContent = String(err);
  }
}

// Floating controls: code toggle, zoom, fit/1:1 toggle.
byId("code-btn").addEventListener("click", () => {
  const open = viewerEl.classList.toggle("inspector-open");
  if (open) editor.view.requestMeasure(); // CodeMirror measures once it's visible
  if (fitted) panzoom?.fit(); // keep the diagram framed as the stage resizes
});
byId("zoom-btn-plus").addEventListener("click", () => panzoom?.zoomIn());
byId("zoom-btn-minus").addEventListener("click", () => panzoom?.zoomOut());
byId("fit-btn").addEventListener("click", () => {
  if (!panzoom) return;
  if (fitted) panzoom.reset();
  else panzoom.fit();
  fitted = !fitted;
  updateFitIcon();
});

// File drop
["dragenter", "dragover"].forEach((t) =>
  stage.addEventListener(t, (e) => {
    e.preventDefault();
    stage.classList.add("drag");
  }),
);
stage.addEventListener("dragleave", (e) => {
  if (e.target === stage) stage.classList.remove("drag");
});
stage.addEventListener("drop", (e) => {
  e.preventDefault();
  stage.classList.remove("drag");
  const file = (e as DragEvent).dataTransfer?.files?.[0];
  if (file) void file.text().then((text) => open(file.name, text));
});

// No document by default — the canvas starts empty (showing the drop hint) until a
// sample is requested via ?file=<name> or a file is dropped.
const initial = requestedSample();
if (initial) void open(initial, SAMPLES[initial]!);
