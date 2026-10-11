// Pan/zoom controller for the interactive viewer. Applies a CSS transform to a
// content wrapper (which holds the compiled <svg>): pointer-drag pans, wheel zooms
// toward the cursor. The compiled SVG is given an explicit pixel size matching its
// viewBox, so the content box has a definite size for fit/reset math.

const MIN_SCALE = 0.05;
const MAX_SCALE = 40;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** A pan/zoom transform, in the form the content element's CSS transform takes. */
export interface ViewState {
  scale: number;
  tx: number;
  ty: number;
}

/** Edges of the viewport covered by chrome (CSS px): fits and centred zooms
 *  use the uncovered rect. A host that floats panels over a fixed canvas —
 *  so the canvas never relayouts when a panel opens — sets these instead. */
export interface Insets {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface PanZoom {
  /** The covered edges (see `Insets`); partial updates keep the rest. */
  setInsets(insets: Partial<Insets>): void;
  insets(): Insets;
  /** The hand tool: every left press pans, painted content included, and
   *  no background press is a click. The viewport carries the `hand` class
   *  while on, for the cursor. */
  setHandMode(on: boolean): void;
  handMode(): boolean;
  /** Center the content at 1:1. */
  reset(): void;
  /** Put content point (cx, cy) at the viewport center, at 1:1 — 1:1 on a
   *  specific artboard rather than the whole drawing. */
  resetTo(cx: number, cy: number): void;
  /** Scale the content to fill the viewport (with margin) and center it. */
  fit(): void;
  /** Frame a specific rect (content/SVG-user units) in the viewport, centered
   *  with margin — used to zoom to an artboard. */
  fitTo(x: number, y: number, w: number, h: number): void;
  /** Zoom a step in / out, anchored on the viewport center (for the +/- buttons). */
  zoomIn(): void;
  zoomOut(): void;
  /** Current scale factor, for callers that need to convert screen↔content units. */
  scale(): number;
  /** The whole transform, so a host that rebuilds the content can put the user back
   *  where they were instead of snapping to a fresh fit. */
  view(): ViewState;
  /** Restore a transform captured by `view()`. */
  setView(v: ViewState): void;
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
  let bgClickCandidate = false; // this drag could still be a deselect click
  let spaceDown = false; // space held → left-drag pans from anywhere
  let lastX = 0;
  let lastY = 0;
  let startX = 0;
  let startY = 0;
  let pointerId = -1;
  const listeners = new Set<() => void>();
  const bgClickListeners = new Set<() => void>();
  const inset: Insets = { left: 0, top: 0, right: 0, bottom: 0 };
  let hand = false;
  // the uncovered part of the viewport: origin and size
  const visible = () => ({
    x: inset.left,
    y: inset.top,
    w: Math.max(0, viewport.clientWidth - inset.left - inset.right),
    h: Math.max(0, viewport.clientHeight - inset.top - inset.bottom),
  });

  const apply = () => {
    content.style.transformOrigin = "0 0";
    // The rendered translation snaps to whole device pixels: a fractional
    // offset makes the engine resample the composited layer and vector art
    // reads blurry (found live: a fit beside a sidebar). The camera state
    // stays exact, so zoom anchors and pans accumulate without drift.
    const dpr = window.devicePixelRatio || 1;
    const sx = Math.round(tx * dpr) / dpr;
    const sy = Math.round(ty * dpr) / dpr;
    content.style.transform = `translate(${sx}px, ${sy}px) scale(${scale})`;
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

  const zoomAtCenter = (factor: number) => {
    const v = visible();
    zoomBy(factor, v.x + v.w / 2, v.y + v.h / 2);
  };

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const rect = viewport.getBoundingClientRect();
    zoomBy(Math.exp(-e.deltaY * 0.0015), e.clientX - rect.left, e.clientY - rect.top);
  };

  const onPointerDown = (e: PointerEvent) => {
    const middle = e.button === 1;
    const spacePan = e.button === 0 && spaceDown;
    // A plain left press on the empty background pans (and may be a deselect
    // click); a press on a painted element is a click so the inspector can
    // select it — but middle-drag and space+drag pan from anywhere. "Empty"
    // is the stage, the content box, or the drawing's own <svg> root (a
    // painted child would be the target instead), so a drag on a transparent
    // artboard gutter pans. It must be the DRAWING's svg — not a control-button
    // icon <svg> — hence the parent check.
    const empty =
      e.target === viewport ||
      e.target === content ||
      (e.target instanceof SVGSVGElement && e.target.parentElement === content);
    const bgPan = e.button === 0 && empty;
    const handPan = e.button === 0 && hand;
    if (!(middle || spacePan || bgPan || handPan)) return;
    if (middle || spacePan || handPan) e.preventDefault(); // no text-selection / autoscroll
    dragging = true;
    bgClickCandidate = bgPan && !handPan; // the hand never deselects
    pointerId = e.pointerId;
    startX = lastX = e.clientX;
    startY = lastY = e.clientY;
    viewport.setPointerCapture(pointerId);
    viewport.classList.add("grabbing");
  };

  // Chrome/Edge start middle-button autoscroll on the compatibility mousedown,
  // which preventDefault on pointerdown does not suppress — kill it here.
  const onMouseDown = (e: MouseEvent) => {
    if (e.button === 1) e.preventDefault();
  };

  const editable = (el: Element | null): boolean =>
    !!el &&
    (el.tagName === "INPUT" ||
      el.tagName === "TEXTAREA" ||
      (el as HTMLElement).isContentEditable);
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.code === "Space" && !spaceDown && !editable(document.activeElement)) {
      spaceDown = true;
      viewport.classList.add("space-pan");
      e.preventDefault(); // don't scroll / activate a focused button
    }
  };
  const onKeyUp = (e: KeyboardEvent) => {
    if (e.code === "Space") {
      spaceDown = false;
      viewport.classList.remove("space-pan");
    }
  };
  // If focus leaves the window while space is held, the keyup is missed — clear
  // it so clicks don't stay stuck in pan mode.
  const onBlur = () => {
    spaceDown = false;
    viewport.classList.remove("space-pan");
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
    // A background press that barely moved is a click, not a pan → notify
    // (deselect). Middle/space pans never deselect, even if they don't move.
    if (bgClickCandidate && Math.hypot(e.clientX - startX, e.clientY - startY) < 4) {
      bgClickListeners.forEach((cb) => cb());
    }
  };

  const reset = () => {
    const [w, h] = contentSize();
    const v = visible();
    scale = 1;
    tx = v.x + (v.w - w) / 2;
    ty = v.y + (v.h - h) / 2;
    apply();
  };

  const fit = () => {
    const [w, h] = contentSize();
    const v = visible();
    if (!(w > 0 && h > 0 && v.w > 0 && v.h > 0)) {
      reset();
      return;
    }
    scale = clamp(Math.min(v.w / w, v.h / h) * 0.94, MIN_SCALE, MAX_SCALE);
    tx = v.x + (v.w - scale * w) / 2;
    ty = v.y + (v.h - scale * h) / 2;
    apply();
  };

  const resetTo = (cx: number, cy: number) => {
    const v = visible();
    scale = 1;
    tx = v.x + v.w / 2 - cx;
    ty = v.y + v.h / 2 - cy;
    apply();
  };

  const fitTo = (x: number, y: number, w: number, h: number) => {
    const v = visible();
    if (!(w > 0 && h > 0 && v.w > 0 && v.h > 0)) {
      fit();
      return;
    }
    scale = clamp(Math.min(v.w / w, v.h / h) * 0.94, MIN_SCALE, MAX_SCALE);
    tx = v.x + (v.w - scale * w) / 2 - scale * x;
    ty = v.y + (v.h - scale * h) / 2 - scale * y;
    apply();
  };

  viewport.addEventListener("wheel", onWheel, { passive: false });
  viewport.addEventListener("pointerdown", onPointerDown);
  viewport.addEventListener("mousedown", onMouseDown);
  viewport.addEventListener("pointermove", onPointerMove);
  viewport.addEventListener("pointerup", onPointerUp);
  viewport.addEventListener("pointercancel", onPointerUp);
  window.addEventListener("keydown", onKeyDown);
  window.addEventListener("keyup", onKeyUp);
  window.addEventListener("blur", onBlur);

  return {
    setInsets: (i) => Object.assign(inset, i),
    insets: () => ({ ...inset }),
    setHandMode: (on) => {
      hand = on;
      viewport.classList.toggle("hand", on);
    },
    handMode: () => hand,
    reset,
    resetTo,
    fit,
    fitTo,
    zoomIn: () => zoomAtCenter(1.25),
    zoomOut: () => zoomAtCenter(1 / 1.25),
    scale: () => scale,
    view: () => ({ scale, tx, ty }),
    setView: (v) => {
      scale = v.scale;
      tx = v.tx;
      ty = v.ty;
      apply();
    },
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
      viewport.removeEventListener("mousedown", onMouseDown);
      viewport.removeEventListener("pointermove", onPointerMove);
      viewport.removeEventListener("pointerup", onPointerUp);
      viewport.removeEventListener("pointercancel", onPointerUp);
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", onBlur);
      viewport.classList.remove("space-pan");
      listeners.clear();
      bgClickListeners.clear();
    },
  };
}
