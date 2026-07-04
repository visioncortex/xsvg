// Pan/zoom controller for the interactive viewer. Applies a CSS transform to a
// content wrapper (which holds the compiled <svg>): pointer-drag pans, wheel zooms
// toward the cursor. The compiled SVG is given an explicit pixel size matching its
// viewBox, so the content box has a definite size for fit/reset math.

const MIN_SCALE = 0.05;
const MAX_SCALE = 40;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

export interface PanZoom {
  /** Center the content at 1:1. */
  reset(): void;
  /** Scale the content to fill the viewport (with margin) and center it. */
  fit(): void;
  /** Zoom a step in / out, anchored on the viewport center (for the +/- buttons). */
  zoomIn(): void;
  zoomOut(): void;
  /** Current scale factor, for callers that need to convert screen↔content units. */
  scale(): number;
  /** Register a callback fired whenever the transform changes. Returns an unsubscribe. */
  onChange(cb: () => void): () => void;
  /** Register a callback fired on a genuine click (press without drag) on the empty
   *  background. Used to de-select the inspected element. Returns an unsubscribe. */
  onBackgroundClick(cb: () => void): () => void;
  destroy(): void;
}

export function createPanZoom(viewport: HTMLElement, content: HTMLElement): PanZoom {
  let scale = 1;
  let tx = 0;
  let ty = 0;
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  let startX = 0;
  let startY = 0;
  let pointerId = -1;
  const listeners = new Set<() => void>();
  const bgClickListeners = new Set<() => void>();

  const apply = () => {
    content.style.transformOrigin = "0 0";
    content.style.transform = `translate(${tx}px, ${ty}px) scale(${scale})`;
    listeners.forEach((cb) => cb());
  };

  const contentSize = (): [number, number] => {
    const svg = content.querySelector("svg");
    return [svg?.clientWidth || content.clientWidth, svg?.clientHeight || content.clientHeight];
  };

  // Zoom by `factor`, keeping the point (cx, cy) — viewport-local px — fixed.
  const zoomBy = (factor: number, cx: number, cy: number) => {
    const next = clamp(scale * factor, MIN_SCALE, MAX_SCALE);
    const k = next / scale;
    tx = cx - k * (cx - tx);
    ty = cy - k * (cy - ty);
    scale = next;
    apply();
  };

  const zoomAtCenter = (factor: number) =>
    zoomBy(factor, viewport.clientWidth / 2, viewport.clientHeight / 2);

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const rect = viewport.getBoundingClientRect();
    zoomBy(Math.exp(-e.deltaY * 0.0015), e.clientX - rect.left, e.clientY - rect.top);
  };

  const onPointerDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    // Only the empty background pans; a press on the content (the svg) is a click,
    // so the inspector can select the element under the pointer.
    if (e.target !== viewport) return;
    dragging = true;
    pointerId = e.pointerId;
    startX = lastX = e.clientX;
    startY = lastY = e.clientY;
    viewport.setPointerCapture(pointerId);
    viewport.classList.add("grabbing");
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!dragging) return;
    tx += e.clientX - lastX;
    ty += e.clientY - lastY;
    lastX = e.clientX;
    lastY = e.clientY;
    apply();
  };

  const onPointerUp = (e: PointerEvent) => {
    if (!dragging) return;
    dragging = false;
    if (pointerId >= 0 && viewport.hasPointerCapture(pointerId)) {
      viewport.releasePointerCapture(pointerId);
    }
    viewport.classList.remove("grabbing");
    // A background press that barely moved is a click, not a pan → notify (deselect).
    if (Math.hypot(e.clientX - startX, e.clientY - startY) < 4) {
      bgClickListeners.forEach((cb) => cb());
    }
  };

  const reset = () => {
    const [w, h] = contentSize();
    scale = 1;
    tx = (viewport.clientWidth - w) / 2;
    ty = (viewport.clientHeight - h) / 2;
    apply();
  };

  const fit = () => {
    const [w, h] = contentSize();
    const vw = viewport.clientWidth;
    const vh = viewport.clientHeight;
    if (!(w > 0 && h > 0 && vw > 0 && vh > 0)) {
      reset();
      return;
    }
    scale = clamp(Math.min(vw / w, vh / h) * 0.94, MIN_SCALE, MAX_SCALE);
    tx = (vw - scale * w) / 2;
    ty = (vh - scale * h) / 2;
    apply();
  };

  viewport.addEventListener("wheel", onWheel, { passive: false });
  viewport.addEventListener("pointerdown", onPointerDown);
  viewport.addEventListener("pointermove", onPointerMove);
  viewport.addEventListener("pointerup", onPointerUp);
  viewport.addEventListener("pointercancel", onPointerUp);

  return {
    reset,
    fit,
    zoomIn: () => zoomAtCenter(1.25),
    zoomOut: () => zoomAtCenter(1 / 1.25),
    scale: () => scale,
    onChange: (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    onBackgroundClick: (cb) => {
      bgClickListeners.add(cb);
      return () => bgClickListeners.delete(cb);
    },
    destroy: () => {
      viewport.removeEventListener("wheel", onWheel);
      viewport.removeEventListener("pointerdown", onPointerDown);
      viewport.removeEventListener("pointermove", onPointerMove);
      viewport.removeEventListener("pointerup", onPointerUp);
      viewport.removeEventListener("pointercancel", onPointerUp);
      listeners.clear();
      bgClickListeners.clear();
    },
  };
}
