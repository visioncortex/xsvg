// Shape rasterizer for `<x:textbox in="#shape">` region flow — the Node equivalent of the
// browser's getBBox + isPointInFill probe. `svgpath` parses `d` and converts arcs/shortcuts
// to plain curves; we flatten those to edges and sample nonzero-winding inside-ness on the
// SAME row/step grid the browser (and native CLI) use, so the produced spans line up.
//
// Returns the reference layout `[minX, minY, w, h, rowH, l0, r0, l1, r1, …]` — one inside
// [left, right] span per ~rowH-tall row (a NaN pair for an empty row).
import svgpath from "svgpath";

type Edge = [number, number, number, number];
const FLATTEN = 24; // segments per curve

function flattenCubic(
  x0: number, y0: number, x1: number, y1: number, x2: number, y2: number, x3: number, y3: number,
  out: Edge[],
): void {
  let px = x0, py = y0;
  for (let i = 1; i <= FLATTEN; i++) {
    const t = i / FLATTEN, u = 1 - t;
    const a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t;
    const x = a * x0 + b * x1 + c * x2 + d * x3;
    const y = a * y0 + b * y1 + c * y2 + d * y3;
    out.push([px, py, x, y]);
    px = x; py = y;
  }
}

function flattenQuad(
  x0: number, y0: number, x1: number, y1: number, x2: number, y2: number, out: Edge[],
): void {
  let px = x0, py = y0;
  for (let i = 1; i <= FLATTEN; i++) {
    const t = i / FLATTEN, u = 1 - t;
    const x = u * u * x0 + 2 * u * t * x1 + t * t * x2;
    const y = u * u * y0 + 2 * u * t * y1 + t * t * y2;
    out.push([px, py, x, y]);
    px = x; py = y;
  }
}

/** Flatten a path `d` to edges, implicitly closing each subpath (fill semantics). */
function toEdges(d: string): Edge[] {
  const edges: Edge[] = [];
  let sx = 0, sy = 0, cx = 0, cy = 0, open = false;
  const close = () => {
    if (open && (cx !== sx || cy !== sy)) edges.push([cx, cy, sx, sy]);
  };
  svgpath(d)
    .abs()
    .unarc()
    .unshort()
    .iterate((seg) => {
      const c = seg[0] as string;
      const a = seg as unknown as number[];
      switch (c) {
        case "M":
          close();
          sx = cx = a[1]; sy = cy = a[2]; open = true;
          break;
        case "L":
          edges.push([cx, cy, a[1], a[2]]); cx = a[1]; cy = a[2];
          break;
        case "H":
          edges.push([cx, cy, a[1], cy]); cx = a[1];
          break;
        case "V":
          edges.push([cx, cy, cx, a[1]]); cy = a[1];
          break;
        case "C":
          flattenCubic(cx, cy, a[1], a[2], a[3], a[4], a[5], a[6], edges); cx = a[5]; cy = a[6];
          break;
        case "Q":
          flattenQuad(cx, cy, a[1], a[2], a[3], a[4], edges); cx = a[3]; cy = a[4];
          break;
        case "Z":
        case "z":
          if (cx !== sx || cy !== sy) edges.push([cx, cy, sx, sy]);
          cx = sx; cy = sy;
          break;
      }
    });
  close();
  return edges;
}

// Winding number (Dan Sunday): nonzero → inside, matching the browser's isPointInFill.
function inside(edges: Edge[], px: number, py: number): boolean {
  let wn = 0;
  for (const [x1, y1, x2, y2] of edges) {
    const isLeft = (x2 - x1) * (py - y1) - (px - x1) * (y2 - y1);
    if (y1 <= py) {
      if (y2 > py && isLeft > 0) wn++;
    } else if (y2 <= py && isLeft < 0) {
      wn--;
    }
  }
  return wn !== 0;
}

/** The core's `Shaper` seam for Node: `[minX, minY, w, h, rowH, l0, r0, …]`. */
export function rasterize(d: string, rowH: number): number[] {
  let edges: Edge[];
  try {
    edges = toEdges(d);
  } catch {
    return [];
  }
  if (edges.length === 0) return [];
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
  for (const [x1, y1, x2, y2] of edges) {
    minX = Math.min(minX, x1, x2); maxX = Math.max(maxX, x1, x2);
    minY = Math.min(minY, y1, y2); maxY = Math.max(maxY, y1, y2);
  }
  const w = maxX - minX, h = maxY - minY;
  if (!(w > 0 && h > 0 && rowH > 0)) return [];

  const rows = Math.max(1, Math.ceil(h / rowH));
  const rh = h / rows;
  const xSteps = Math.max(24, Math.min(400, Math.ceil(w)));
  const dx = w / xSteps;
  const out: number[] = [minX, minY, w, h, rh];
  for (let r = 0; r < rows; r++) {
    const y = minY + (r + 0.5) * rh;
    let left = NaN, right = NaN;
    for (let i = 0; i <= xSteps; i++) {
      const x = minX + i * dx;
      if (inside(edges, x, y)) {
        if (Number.isNaN(left)) left = x;
        right = x;
      }
    }
    out.push(left, right);
  }
  return out;
}
