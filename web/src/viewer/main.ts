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
import { compileXsvg } from "@visioncortex/xsvg-viewer";
import { SAMPLES, requestedSample } from "../core/samples";
import { createEditor } from "../core/editor";
import { createPanZoom, type PanZoom } from "./pan-zoom";
import { createInspector, type Inspector } from "./inspector";
import { findArtboards, makeThumb } from "@visioncortex/xsvg-viewer";
import { downloadSvg } from "@visioncortex/xsvg-viewer";

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
const downloadLink = byId("download-svg") as HTMLAnchorElement;

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

// The currently-open document, so "Download SVG" can recompile it cleanly.
let currentSource: string | null = null;
let currentName = "drawing";

async function open(name: string, source: string): Promise<void> {
  docName.textContent = name;
  currentSource = source;
  currentName = name;
  downloadLink.hidden = false;
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
      inspector = createInspector({
        svgRoot: svgEl as SVGSVGElement,
        panel,
        source,
        editor,
      });
      panzoom.onBackgroundClick(() => inspector?.clear()); // click empty area to deselect
      // Artboards (§5.2): a left slide rail + zoom-to-first when the document
      // has any; else fit the whole drawing.
      setupDeck(svgEl as SVGSVGElement);
    }
  } catch (err) {
    content.innerHTML = "";
    errorBox.hidden = false;
    errorBox.textContent = String(err);
  }
}

// Slide deck: a left thumbnail rail for multi-artboard documents; clicking a
// thumb (or arrow keys) zooms the pan/zoom to that artboard.
const deckRail = byId("deck-rail");
const deckToggle = byId("deck-toggle");
let deckSelect: ((i: number) => void) | null = null;
let deckActive = 0;
let deckFrame: [number, number, number, number] | null = null;

// Re-frame after a stage resize (code panel, sidebar drag): in a deck, keep the
// ACTIVE artboard framed rather than fitting the whole tiled drawing.
function refit(): void {
  if (!panzoom || !fitted) return;
  if (deckFrame) panzoom.fitTo(...deckFrame);
  else panzoom.fit();
}

function setupDeck(svg: SVGSVGElement): void {
  deckRail.innerHTML = "";
  deckSelect = null;
  deckFrame = null;
  const boards = findArtboards(svg);
  deckToggle.hidden = boards.length < 2;
  if (boards.length < 2) {
    viewerEl.classList.remove("deck-open");
    if (boards.length === 1) {
      deckFrame = boards[0].frame; // single artboard: sizer still centers on it
      panzoom!.fitTo(...deckFrame);
    } else {
      panzoom!.fit();
    }
    fitted = true;
    updateFitIcon();
    return;
  }
  const thumbs = boards.map((b, idx) => {
    const t = makeThumb(svg, b);
    t.addEventListener("click", () => deckSelect?.(idx));
    deckRail.appendChild(t);
    return t;
  });
  deckSelect = (i: number) => {
    deckActive = Math.max(0, Math.min(boards.length - 1, i));
    deckFrame = boards[deckActive].frame;
    panzoom!.fitTo(...deckFrame);
    thumbs.forEach((t, k) => t.classList.toggle("active", k === deckActive));
    fitted = true;
    updateFitIcon();
  };
  viewerEl.classList.add("deck-open");
  deckSelect(0);
}

deckToggle.addEventListener("click", () => {
  viewerEl.classList.toggle("deck-open");
  deckSelect?.(deckActive); // refit the current slide as the stage resizes
});
window.addEventListener("keydown", (e) => {
  if (!deckSelect) return;
  const el = document.activeElement;
  if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || (el as HTMLElement).isContentEditable))
    return;
  if (e.key === "ArrowLeft" || e.key === "PageUp") deckSelect(deckActive - 1);
  else if (e.key === "ArrowRight" || e.key === "PageDown") deckSelect(deckActive + 1);
});

// Download the compiled plain SVG (compiled clean, without the source map).
downloadLink.addEventListener("click", (e) => {
  e.preventDefault();
  if (currentSource) void downloadSvg(currentSource, currentName);
});

// Floating controls: code toggle, zoom, fit/1:1 toggle.
byId("code-btn").addEventListener("click", () => {
  const open = viewerEl.classList.toggle("inspector-open");
  if (open) editor.view.requestMeasure(); // CodeMirror measures once it's visible
  refit(); // keep the active artboard framed as the stage resizes
});
byId("zoom-btn-plus").addEventListener("click", () => panzoom?.zoomIn());
byId("zoom-btn-minus").addEventListener("click", () => panzoom?.zoomOut());

// Collapsible panes: clicking a section header toggles its body.
document.querySelectorAll<HTMLElement>(".pane > h2").forEach((h2) => {
  h2.addEventListener("click", () => {
    const pane = h2.parentElement!;
    const collapsing = pane.classList.toggle("collapsed");
    // CodeMirror must re-measure when the source pane becomes visible again
    if (!collapsing && pane.classList.contains("source-pane")) editor.view.requestMeasure();
  });
});

// Draggable sidebar width: a guide line tracks the pointer while dragging and
// the width is committed only on release, so the reflow happens once.
// Double-click the handle to snap back to the CSS default width.
const sidebar = document.querySelector(".sidebar") as HTMLElement;
byId("sidebar-resize").addEventListener("dblclick", () => {
  sidebar.style.removeProperty("flex-basis");
  sidebar.style.removeProperty("width");
  refit();
  editor.view.requestMeasure();
});
byId("sidebar-resize").addEventListener("pointerdown", (e: PointerEvent) => {
  e.preventDefault();
  const startX = e.clientX;
  let moved = false;
  let guide: HTMLElement | null = null;
  const move = (ev: PointerEvent) => {
    // ignore a stationary click (its pointerup must NOT resize — that would
    // creep the width by the gutter and eat the double-click-to-reset)
    if (!moved && Math.abs(ev.clientX - startX) < 3) return;
    moved = true;
    if (!guide) {
      guide = document.createElement("div");
      guide.className = "resize-guide";
      document.body.appendChild(guide);
    }
    guide.style.left = `${ev.clientX}px`;
  };
  const up = (ev: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    guide?.remove();
    if (!moved) return; // a click, not a drag — leave the width alone
    // sidebar hugs the right edge, so its width is the distance from the pointer
    const w = Math.max(240, Math.min(window.innerWidth - 320, window.innerWidth - ev.clientX));
    sidebar.style.flexBasis = `${w}px`;
    sidebar.style.width = `${w}px`;
    refit(); // keep the active artboard framed as the stage resizes
    editor.view.requestMeasure();
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
});
byId("fit-btn").addEventListener("click", () => {
  if (!panzoom) return;
  // In a deck, the sizer toggles around the ACTIVE slide: frame it, or 1:1
  // centered on it — not the whole tiled drawing.
  if (deckFrame) {
    const [x, y, w, h] = deckFrame;
    if (fitted) panzoom.resetTo(x + w / 2, y + h / 2);
    else panzoom.fitTo(x, y, w, h);
  } else if (fitted) {
    panzoom.reset();
  } else {
    panzoom.fit();
  }
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
