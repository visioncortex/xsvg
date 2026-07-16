// Artboards (§5.2): named slide frames the compiler marks with
// `data-xsvg-artboard` (and optionally `data-xsvg-frame="x y w h"`). Both the
// preview (page through them) and the viewer (zoom to the first) read them
// from the rendered SVG here.

export interface Artboard {
  el: SVGGraphicsElement;
  label: string;
  /** frame in SVG user units: [x, y, w, h] */
  frame: [number, number, number, number];
}

/** A thumbnail for one artboard: a clone of the compiled SVG reframed to the
 *  artboard's frame (no re-render — the outer <svg>'s overflow-hidden clips it
 *  to the slide). Carries the .deck-thumb class; the caller wires the click. */
export function makeThumb(svg: SVGSVGElement, board: Artboard): HTMLDivElement {
  const thumb = document.createElement("div");
  thumb.className = "deck-thumb";
  thumb.style.aspectRatio = `${board.frame[2]} / ${board.frame[3]}`;
  const clone = svg.cloneNode(true) as SVGSVGElement;
  clone.setAttribute("viewBox", board.frame.join(" "));
  clone.removeAttribute("width");
  clone.removeAttribute("height");
  // The thumb box already has the frame's aspect ratio, so fill it exactly —
  // "meet" would letterbox with hairline bars (the thumb's white bg showing).
  clone.setAttribute("preserveAspectRatio", "none");
  thumb.appendChild(clone);
  return thumb;
}

/** All artboards in a compiled SVG, in document order. Falls back to the
 *  element's bounding box when no explicit `data-xsvg-frame` was given (the
 *  SVG must be in the rendered DOM for `getBBox`). */
export function findArtboards(svg: SVGSVGElement): Artboard[] {
  const els = svg.querySelectorAll<SVGGraphicsElement>("[data-xsvg-artboard]");
  return Array.from(els).map((el) => {
    const label = el.getAttribute("data-xsvg-artboard") ?? "";
    const f = el.getAttribute("data-xsvg-frame");
    let frame: [number, number, number, number];
    if (f) {
      const p = f.split(/[\s,]+/).map(Number);
      frame = [p[0], p[1], p[2], p[3]];
    } else {
      const b = el.getBBox();
      frame = [b.x, b.y, b.width, b.height];
    }
    return { el, label, frame };
  });
}
