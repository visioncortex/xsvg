// Element inspector for the interactive viewer. Hovering an element in the rendered
// SVG outlines it; clicking pins it, fills the info panel (tag / attributes / bbox),
// and projects it back onto the xsvg source in the read-only editor pane.
//
// The compiler tags emitted elements with `data-xsvg-pos` byte ranges; we resolve
// the hovered leaf to its nearest tagged ancestor (see core/sourcemap). Highlight
// rectangles live *inside* the SVG (user space) with a non-scaling stroke, so they
// track pan/zoom for free without recomputing on every transform change.
import { ByteIndex, nearestSource } from "../core/sourcemap";
import type { EditorHandle } from "../core/editor";

const SVG_NS = "http://www.w3.org/2000/svg";

export interface Inspector {
  clear(): void;
  destroy(): void;
}

export interface InspectorOptions {
  svgRoot: SVGSVGElement;
  panel: HTMLElement;
  source: string;
  editor: EditorHandle;
}

export function createInspector({ svgRoot, panel, source, editor }: InspectorOptions): Inspector {
  const bytes = new ByteIndex(source);

  const hoverRect = mkRect("xsvg-hl-hover");
  const pinRect = mkRect("xsvg-hl-pin");
  svgRoot.append(pinRect, hoverRect);

  const asGraphics = (el: Element | null): SVGGraphicsElement | null =>
    el && typeof (el as SVGGraphicsElement).getBBox === "function"
      ? (el as SVGGraphicsElement)
      : null;

  const place = (rect: SVGRectElement, el: SVGGraphicsElement) => {
    const box = bboxInRoot(el, svgRoot);
    if (!box) {
      rect.style.display = "none";
      return;
    }
    rect.setAttribute("x", String(box.x));
    rect.setAttribute("y", String(box.y));
    rect.setAttribute("width", String(box.w));
    rect.setAttribute("height", String(box.h));
    rect.style.display = "";
  };

  const onMove = (e: MouseEvent) => {
    const hit = nearestSource(e.target as Element, svgRoot);
    const el = asGraphics(hit?.el ?? null);
    if (hit && el && el !== svgRoot) place(hoverRect, el);
    else hoverRect.style.display = "none";
  };

  const onLeave = () => {
    hoverRect.style.display = "none";
  };

  const onClick = (e: MouseEvent) => {
    const hit = nearestSource(e.target as Element, svgRoot);
    const el = asGraphics(hit?.el ?? null);
    if (!hit || !el || el === svgRoot) return;
    place(pinRect, el);
    const from = bytes.toStr(hit.range.start);
    const to = bytes.toStr(hit.range.end);
    fillPanel(panel, el, source.slice(from, to));
    editor.highlight(from, to);
  };

  svgRoot.addEventListener("mousemove", onMove);
  svgRoot.addEventListener("mouseleave", onLeave);
  svgRoot.addEventListener("click", onClick);

  const clear = () => {
    hoverRect.style.display = "none";
    pinRect.style.display = "none";
    editor.highlight(-1, -1);
    panel.innerHTML = emptyPanel();
  };
  clear();

  return {
    clear,
    destroy: () => {
      svgRoot.removeEventListener("mousemove", onMove);
      svgRoot.removeEventListener("mouseleave", onLeave);
      svgRoot.removeEventListener("click", onClick);
      hoverRect.remove();
      pinRect.remove();
    },
  };
}

function mkRect(cls: string): SVGRectElement {
  const r = document.createElementNS(SVG_NS, "rect");
  r.setAttribute("class", cls);
  r.setAttribute("fill", "none");
  r.setAttribute("vector-effect", "non-scaling-stroke");
  r.setAttribute("pointer-events", "none");
  r.style.display = "none";
  return r;
}

/** Axis-aligned bbox of `el` expressed in `root`'s user-space coordinates, so a
 *  rect appended to `root` at these coords overlays `el` even through group/CSS
 *  transforms. */
function bboxInRoot(el: SVGGraphicsElement, root: SVGSVGElement): { x: number; y: number; w: number; h: number } | null {
  let bb: DOMRect;
  try {
    bb = el.getBBox();
  } catch {
    return null;
  }
  const rootCtm = root.getScreenCTM();
  const elCtm = el.getScreenCTM();
  if (!rootCtm || !elCtm || !(bb.width >= 0 && bb.height >= 0)) return null;
  const m = rootCtm.inverse().multiply(elCtm); // el user space → root user space
  const corners = [
    [bb.x, bb.y],
    [bb.x + bb.width, bb.y],
    [bb.x, bb.y + bb.height],
    [bb.x + bb.width, bb.y + bb.height],
  ].map(([x, y]) => {
    const p = root.createSVGPoint();
    p.x = x;
    p.y = y;
    return p.matrixTransform(m);
  });
  const xs = corners.map((p) => p.x);
  const ys = corners.map((p) => p.y);
  const minX = Math.min(...xs);
  const minY = Math.min(...ys);
  return { x: minX, y: minY, w: Math.max(...xs) - minX, h: Math.max(...ys) - minY };
}

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

function emptyPanel(): string {
  return `<p class="hint">Click an element in the canvas to inspect it and locate its xsvg source.</p>`;
}

function fillPanel(panel: HTMLElement, el: SVGGraphicsElement, snippet: string): void {
  const attrs = Array.from(el.attributes)
    .filter((a) => a.name !== "data-xsvg-pos")
    .map(
      (a) =>
        `<div class="attr"><span class="k">${escapeHtml(a.name)}</span><span class="v">${escapeHtml(
          a.value,
        )}</span></div>`,
    )
    .join("");

  let bboxLine = "";
  try {
    const b = el.getBBox();
    bboxLine = `<div class="row"><span class="label">bbox</span> ${fmt(b.x)}, ${fmt(b.y)} · ${fmt(
      b.width,
    )} × ${fmt(b.height)}</div>`;
  } catch {
    /* non-rendered element */
  }

  const trimmed = snippet.length > 400 ? snippet.slice(0, 400) + "…" : snippet;

  panel.innerHTML = `
    <div class="row"><span class="tag">&lt;${escapeHtml(el.tagName)}&gt;</span></div>
    ${bboxLine}
    <div class="attrs">${attrs || '<span class="hint">no attributes</span>'}</div>
    <div class="row"><span class="label">xsvg source</span></div>
    <pre class="snippet">${escapeHtml(trimmed)}</pre>`;
}

function fmt(n: number): string {
  return (Math.round(n * 100) / 100).toString();
}
