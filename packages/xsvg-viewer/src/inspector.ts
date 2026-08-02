// Element inspector for the interactive viewer — bi-directional link between the
// rendered SVG and the xsvg source:
//   • canvas → source: hover outlines an element, click pins it, fills the info
//     panel (tag / attributes / bbox), and highlights its originating source.
//   • source → canvas: clicking in the source pane pins the element whose
//     `data-xsvg-pos` range is under the cursor (see main.ts for the wiring).
//
// The compiler tags emitted elements with `data-xsvg-pos` byte ranges; we resolve
// the hovered leaf to its nearest tagged ancestor (see core/sourcemap). Highlight
// rectangles live *inside* the SVG (user space) with a non-scaling stroke, so they
// track pan/zoom for free without recomputing on every transform change.
import { ByteIndex, nearestSource, posAttr, type SourceRange } from "./sourcemap";
import type { EditorHandle } from "./editor";
import {
  namespaceOf,
  parseStartTag,
  renderParams,
  setAttribute,
  type PluginSchemas,
} from "./params";

const SVG_NS = "http://www.w3.org/2000/svg";

export interface Inspector {
  clear(): void;
  /** Source → canvas: pin the innermost element whose source range contains the
   *  given editor offset (UTF-16). No-op if the offset is outside every element. */
  selectAtSourceOffset(offset: number): void;
  /**
   * Adopt a document that was patched in place (docs/Incremental.md §4): re-index
   * the spans against the new DOM, take `source` as the new text, and follow the
   * pinned element through the edit — `mapOffset` carries an old source byte offset
   * to where it now sits.
   *
   * The panel's parameter controls are deliberately **left standing**. A patch is
   * normally the result of the user moving one of them, and rebuilding the panel
   * mid-gesture would destroy the control under the pointer — the very thing the
   * incremental path exists to avoid.
   */
  refresh(source: string, mapOffset: (byte: number) => number): void;
  destroy(): void;
}

export interface InspectorOptions {
  svgRoot: SVGSVGElement;
  panel: HTMLElement;
  source: string;
  editor: EditorHandle;
  /** Parameter schemas of the document's generator modules. With these, a pinned
   *  plugin element gets editable controls instead of a read-only attribute table. */
  schemas?: PluginSchemas | null;
  /** Apply an edit made in the panel. `pinAt` is the UTF-16 offset of the edited
   *  element, so the host can re-pin it after the document recompiles. */
  onEdit?: (source: string, pinAt: number) => void;
}

export function createInspector({
  svgRoot,
  panel,
  source,
  editor,
  schemas,
  onEdit,
}: InspectorOptions): Inspector {
  // Mutable across an incremental patch: the document's text changes under us, and
  // every offset the panel hands back has to be read against the *current* text.
  let bytes = new ByteIndex(source);

  // Two regions, so element info can be restated on every live edit without
  // touching the parameter controls sitting below it.
  panel.innerHTML = "";
  const info = document.createElement("div");
  const controls = document.createElement("div");
  panel.append(info, controls);

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
    // A straight/axis-aligned connector (or any 1-D shape) has a zero-area bbox, and an
    // SVG rect with width or height 0 isn't rendered — so inflate a degenerate axis to a
    // minimum band, keeping the highlight visible over lines.
    const MIN = 3;
    let { x, y, w, h } = box;
    if (w < MIN) { x -= (MIN - w) / 2; w = MIN; }
    if (h < MIN) { y -= (MIN - h) / 2; h = MIN; }
    rect.setAttribute("x", String(x));
    rect.setAttribute("y", String(y));
    rect.setAttribute("width", String(w));
    rect.setAttribute("height", String(h));
    rect.style.display = "";
  };

  // The element the panel is currently describing. Tracked by source range rather
  // than by node, because a patch replaces the node itself.
  let pinned: { el: SVGGraphicsElement; range: SourceRange } | null = null;

  // Pin an element (both directions funnel through here): outline it on the canvas,
  // fill the panel, and highlight its source range in the editor.
  const selectElement = (el: SVGGraphicsElement, range: SourceRange) => {
    pinned = { el, range };
    place(pinRect, el);
    fillPanel(info, el);
    addParams(range);
    editor.highlight(bytes.toStr(range.start), bytes.toStr(range.end));
  };

  // Offer the pinned element's declared parameters as controls. The schema comes
  // from the same derived declarations for every tier — the compiler's own `x:`
  // elements and a third-party module's alike — so this never branches on who
  // provides the element, and reading it runs no generator.
  const addParams = (range: SourceRange) => {
    controls.innerHTML = "";
    if (!schemas || !onEdit) return;
    const tag = parseStartTag(source.slice(bytes.toStr(range.start)));
    if (!tag) return;
    const ns = namespaceOf(source, tag.prefix);
    const spec = ns ? schemas.spec(ns, tag.local) : null;
    if (!spec) return;

    const head = document.createElement("div");
    head.className = "row";
    const label = document.createElement("span");
    label.className = "label";
    label.textContent = "parameters";
    head.append(label);
    controls.append(head);
    if (spec.doc) {
      const doc = document.createElement("p");
      doc.className = "hint";
      doc.textContent = spec.doc;
      controls.append(doc);
    }
    controls.append(
      renderParams(spec, tag.attrs, (name, value) => {
        // Resolved at edit time, not capture time: a live control outlives several
        // patches, and both the text and the element's offset move under it.
        if (!pinned) return;
        const at = bytes.toStr(pinned.range.start);
        const next = setAttribute(source, at, name, value);
        if (next != null && next !== source) onEdit(next, at);
      }),
    );
  };

  // Tagged elements with their source ranges as UTF-16 char offsets, for the
  // source → canvas lookup (smallest range containing the cursor wins).
  let items: { el: SVGGraphicsElement; range: SourceRange; from: number; to: number }[] = [];
  const indexSpans = () => {
    items = [];
    for (const el of Array.from(svgRoot.querySelectorAll<SVGGraphicsElement>("[data-xsvg-pos]"))) {
      const range = posAttr(el);
      if (range) items.push({ el, range, from: bytes.toStr(range.start), to: bytes.toStr(range.end) });
    }
  };
  indexSpans();

  // Adopt a patched document. Everything derived from the text is rebuilt; the
  // parameter controls are not (see `Inspector.refresh`).
  const refresh = (nextSource: string, mapOffset: (byte: number) => number) => {
    source = nextSource;
    bytes = new ByteIndex(source);
    indexSpans();
    if (!pinned) return;
    const start = mapOffset(pinned.range.start);
    const hit = items.find((it) => it.range.start === start);
    if (!hit) {
      // The pinned element no longer has a span of its own — it degraded to a
      // marker, or the edit dissolved it. Drop the pin rather than point at a stale
      // node, but leave the controls so the user can undo what they just did.
      pinned = null;
      pinRect.style.display = "none";
      return;
    }
    pinned = { el: hit.el, range: hit.range };
    place(pinRect, hit.el);
    fillPanel(info, hit.el);
    editor.highlight(hit.from, hit.to, false);
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
    selectElement(el, hit.range);
  };

  const selectAtSourceOffset = (offset: number) => {
    let best: (typeof items)[number] | null = null;
    for (const it of items) {
      if (offset >= it.from && offset <= it.to && (!best || it.to - it.from < best.to - best.from)) {
        best = it;
      }
    }
    if (best) selectElement(best.el, best.range);
  };

  svgRoot.addEventListener("mousemove", onMove);
  svgRoot.addEventListener("mouseleave", onLeave);
  svgRoot.addEventListener("click", onClick);

  const clear = () => {
    pinned = null;
    hoverRect.style.display = "none";
    pinRect.style.display = "none";
    editor.highlight(-1, -1);
    info.innerHTML = emptyPanel();
    controls.innerHTML = "";
  };
  clear();

  return {
    clear,
    selectAtSourceOffset,
    refresh,
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
  return `<p class="hint">Click an element on the canvas — or click in the source — to link the two.</p>`;
}

function fillPanel(info: HTMLElement, el: SVGGraphicsElement): void {
  const rows = Array.from(el.attributes)
    .filter((a) => a.name !== "data-xsvg-pos")
    .map(
      (a) =>
        `<tr><th scope="row">${escapeHtml(a.name)}</th><td>${escapeHtml(a.value)}</td></tr>`,
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

  info.innerHTML = `
    <div class="row"><span class="tag">&lt;${escapeHtml(el.tagName)}&gt;</span></div>
    ${bboxLine}
    ${rows ? `<table class="attrs">${rows}</table>` : '<span class="hint">no attributes</span>'}`;
}

function fmt(n: number): string {
  return (Math.round(n * 100) / 100).toString();
}
