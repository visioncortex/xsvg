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
