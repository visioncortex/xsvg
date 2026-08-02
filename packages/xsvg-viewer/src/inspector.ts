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
import type { NodeIndex } from "./keys";
import type { EditorHandle } from "./editor";
import {
  namespaceOf,
  parseStartTag,
  prefixOf,
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
   * the spans against the new DOM and take `source` as the new text.
   *
   * The pinned element is followed by its **identity attribute**, not by a mapped
   * offset — with host keys that value is stable across the edit, so following it is
   * a lookup rather than arithmetic that has to agree with the compiler's.
   *
   * The panel's parameter controls are deliberately **left standing**. A patch is
   * normally the result of the user moving one of them, and rebuilding the panel
   * mid-gesture would destroy the control under the pointer — the very thing the
   * incremental path exists to avoid.
   */
  refresh(source: string): void;
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
  /** Resolves `data-xsvg-node` names back to source ranges. Required whenever the
   *  document was compiled with host keys, since identity is then a name rather
   *  than the range itself. */
  index?: NodeIndex | null;
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
  index,
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

  // What the panel is describing. A **source range** first and a rendered element
  // second: the element is what gets outlined on the canvas and listed in the
  // attribute table, but the range is what every edit is written against. They come
  // apart whenever an element compiles to nothing — a `<clip:arrow thickness="0">`
  // has no area, so it degrades to a marker comment — and when they do, the source is
  // still there and still editable. Losing the pin then would strand the author in
  // the one state they most need the panel for: the edit that made it vanish is the
  // edit they want to undo, and the canvas no longer offers anything to click.
  //
  // The element is tracked by its identity attribute, stable across a patch when the
  // host supplies keys, which is how the pin survives without re-deriving offsets.
  let pinned: { el: SVGGraphicsElement | null; range: SourceRange; id: string | null } | null =
    null;
  const identity = (el: Element) =>
    el.getAttribute("data-xsvg-node") ?? el.getAttribute("data-xsvg-pos");

  // Pin a source range (every direction funnels through here): outline its element on
  // the canvas if it has one, fill the panel, and highlight the range in the editor.
  const selectElement = (
    el: SVGGraphicsElement | null,
    range: SourceRange,
    id = el ? identity(el) : null,
  ) => {
    pinned = { el, range, id };
    if (el) {
      place(pinRect, el);
      fillPanel(info, el);
    } else {
      pinRect.style.display = "none";
      info.innerHTML = nothingRendered();
    }
    addParams(range);
    editor.highlight(bytes.toStr(range.start), bytes.toStr(range.end));
  };

  // Offer the pinned element's declared attributes as controls. The schema comes
  // from the same derived declarations for every tier — the compiler's own `x:`
  // elements and a third-party module's alike — so this never branches on who
  // provides the element, and reading it runs no generator.
  //
  // Geometry is not offered (see `renderParams`); the table above already shows it.
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
    label.textContent = "attributes";
    head.append(label);
    controls.append(head);
    if (spec.doc) {
      const doc = document.createElement("p");
      doc.className = "hint";
      doc.textContent = spec.doc;
      controls.append(doc);
    }
    controls.append(
      renderParams(
        spec,
        tag.attrs,
        (name, value) => {
          // Resolved at edit time, not capture time: a live control outlives several
          // patches, and both the text and the element's offset move under it. Only
          // the range is consulted — a control must keep working when the element it
          // edits currently renders nothing, since that is exactly when it is needed.
          if (!pinned) return;
          const at = bytes.toStr(pinned.range.start);
          const next = setAttribute(source, at, name, value);
          if (next != null && next !== source) onEdit(next, at);
        },
        // Namespaced attributes (`x:border-width`) take this document's own prefix.
        (attrNs) => prefixOf(source, attrNs),
      ),
    );
  };

  // Tagged elements with their source ranges as UTF-16 char offsets, for the
  // source → canvas lookup (smallest range containing the cursor wins).
  let items: { el: SVGGraphicsElement; range: SourceRange; from: number; to: number }[] = [];
  const indexSpans = () => {
    items = [];
    // Both identity spellings: a byte range, or a host key resolved through `index`.
    const tagged = svgRoot.querySelectorAll<SVGGraphicsElement>("[data-xsvg-pos],[data-xsvg-node]");
    for (const el of Array.from(tagged)) {
      const range = posAttr(el, index);
      if (range) items.push({ el, range, from: bytes.toStr(range.start), to: bytes.toStr(range.end) });
    }
  };
  indexSpans();

  // Adopt a patched document. Everything derived from the text is rebuilt; the
  // parameter controls are not (see `Inspector.refresh`).
  const refresh = (nextSource: string) => {
    source = nextSource;
    bytes = new ByteIndex(source);
    indexSpans();
    if (!pinned) return;
    const id = pinned.id;
    const hit = id ? items.find((it) => identity(it.el) === id) : undefined;
    if (!hit) {
      // The element stopped rendering — it degraded to a marker comment, or the edit
      // dissolved it. The *source* is still there, so keep the pin and keep editing:
      // dropping it here would leave the controls standing but inert, and the edit
      // that emptied the element is the one the author needs to reverse.
      //
      // `index` resolves from source spans rather than the DOM, so it still answers
      // for an element that drew nothing. Failing that the old range still serves —
      // an edit inside an element cannot move that element's start.
      const range = (id && index?.rangeOf(id)) || pinned.range;
      pinned = { el: null, range, id };
      pinRect.style.display = "none";
      info.innerHTML = nothingRendered();
      editor.highlight(bytes.toStr(range.start), bytes.toStr(range.end), false);
      return;
    }
    pinned = { el: hit.el, range: hit.range, id };
    place(pinRect, hit.el);
    fillPanel(info, hit.el);
    editor.highlight(hit.from, hit.to, false);
  };

  const onMove = (e: MouseEvent) => {
    const hit = nearestSource(e.target as Element, svgRoot, index);
    const el = asGraphics(hit?.el ?? null);
    if (hit && el && el !== svgRoot) place(hoverRect, el);
    else hoverRect.style.display = "none";
  };

  const onLeave = () => {
    hoverRect.style.display = "none";
  };

  const onClick = (e: MouseEvent) => {
    const hit = nearestSource(e.target as Element, svgRoot, index);
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
    if (best) {
      selectElement(best.el, best.range);
      return;
    }
    // Nothing rendered covers this offset. The source element is still real, though —
    // `index` is built from source spans — so pin it anyway, elementless. This is the
    // path a re-pin takes after an edit made the element vanish and forced a full
    // recompile, and without it the panel would come back dead.
    const key = index?.keyAt(bytes.toByte(offset));
    const range = key ? index?.rangeOf(key) : null;
    if (range) selectElement(null, range, key);
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

/** Shown in place of the attribute table when the pinned element draws nothing —
 *  a degenerate `<clip:arrow thickness="0">`, a declaration, a hidden layer. Says so
 *  rather than blanking, because the controls below it are still live and are how the
 *  author gets the element back. */
function nothingRendered(): string {
  return `<p class="hint">This element currently renders nothing — check the source pane for a
    marker explaining why. Its attributes are still editable below.</p>`;
}

/** Longest attribute value shown whole. A compiled `d` is routinely thousands of
 *  characters — baked glyph outlines, a warped path, a mesh silhouette — and one of
 *  them turns the attribute table into a wall of digits that buries every other row. */
const VALUE_LIMIT = 96;

/** An attribute value cut to a readable length, plus its true size when cut. The head
 *  is what identifies a value (`M0,0 C…`, `translate(…`), so the tail is what goes. */
function elide(value: string): { text: string; full: string | null } {
  if (value.length <= VALUE_LIMIT) return { text: value, full: null };
  return {
    text: `${value.slice(0, VALUE_LIMIT).trimEnd()}…`,
    full: `${value.length.toLocaleString()} chars`,
  };
}

function fillPanel(info: HTMLElement, el: SVGGraphicsElement): void {
  const rows = Array.from(el.attributes)
    // Neither identity spelling is an authored attribute — the byte span nor the
    // host-assigned key — so neither belongs in a table of the element's own attrs.
    .filter((a) => a.name !== "data-xsvg-pos" && a.name !== "data-xsvg-node")
    .map((a) => {
      const { text, full } = elide(a.value);
      const title = full ? ` title="${escapeHtml(a.value.slice(0, 4000))}"` : "";
      return `<tr><th scope="row">${escapeHtml(a.name)}</th><td${title}>${escapeHtml(text)}${
        full ? ` <span class="hint">(${full})</span>` : ""
      }</td></tr>`;
    })
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
