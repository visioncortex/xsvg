//! `<xsvg-view-interactive>` — the full interactive viewer, as a custom element.
//!
//!   import "@visioncortex/xsvg-viewer/interactive";
//!   // <xsvg-view-interactive src="deck.xsvg"></xsvg-view-interactive>
//!   // <xsvg-view-interactive src="deck.xsvg" inspector></xsvg-view-interactive>
//!
//! This is the viewer that powers the project's own /viewer page — pan/zoom canvas on a
//! checkerboard stage, a slide-deck rail for multi-artboard documents, and floating
//! controls (zoom capsule, fit/actual-size). `inspector` (opt-in) adds the sidebar:
//! an Inspector pane (element tag · attributes · bbox) over a read-only source pane,
//! both collapsible, with a resizable gutter and a bidirectional element↔source link.
//!
//! The inspector is a lazy `import()` — its CodeMirror dependency (an optional peer)
//! loads only when `inspector` is set. Bundler-only: not in the CDN bundle.
import { compileXsvg } from "./compiler";
import { findArtboards, makeThumb, type Artboard } from "./artboards";
import { createPanZoom, type PanZoom } from "./pan-zoom";
import type { MountedInspector } from "./interactive-inspector";

const CODE_ICON = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="8 7 3 12 8 17"/><polyline points="16 7 21 12 16 17"/><line x1="13.5" y1="4" x2="10.5" y2="20"/></svg>`;
const ZOOM_IN_ICON = `<svg viewBox="-6 -2 24 24"><path d="M 6.76 5.09L 6.76 9.08L 10.69 9.08L 10.69 10.49L 6.76 10.49L 6.76 14.45L 5.19 14.45L 5.19 10.49L 1.31 10.49L 1.31 9.08L 5.19 9.08L 5.19 5.09Z"/></svg>`;
const ZOOM_OUT_ICON = `<svg viewBox="-6 -2 24 24"><path d="M 1.99 10.47L 1.99 9.01L 10.01 9.01L 10.01 10.47Z"/></svg>`;
const FIT_ICON = `<svg class="icon-fit" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9V4h5"/><path d="M20 9V4h-5"/><path d="M4 15v5h5"/><path d="M20 15v5h-5"/></svg><svg class="icon-oneone" viewBox="0 0 24 24" hidden><text x="12" y="16" text-anchor="middle" font-size="11" font-weight="700" fill="currentColor" font-family="ui-monospace, SFMono-Regular, Menlo, monospace">1:1</text></svg>`;

const STYLE = `
:host { display: block; height: 100%; min-height: 240px; color: #0f172a; }
.root { display: flex; flex-direction: column; height: 100%; }
.body { flex: 1; display: flex; min-height: 0; }

/* Pan/zoom stage with a design-tool checkerboard */
.stage { position: relative; flex: 1; overflow: hidden; background: #f1f5f9;
  background-image: linear-gradient(45deg,#e6ebf2 25%,transparent 25%), linear-gradient(-45deg,#e6ebf2 25%,transparent 25%), linear-gradient(45deg,transparent 75%,#e6ebf2 75%), linear-gradient(-45deg,transparent 75%,#e6ebf2 75%);
  background-size: 20px 20px; background-position: 0 0, 0 10px, 10px -10px, -10px 0; cursor: grab; touch-action: none; }
.stage.grabbing { cursor: grabbing; }
.stage.space-pan, .stage.space-pan .content { cursor: grab; }
.stage.space-pan.grabbing, .stage.space-pan.grabbing .content { cursor: grabbing; }
.content { position: absolute; top: 0; left: 0; cursor: default; }
.stage.grabbing .content { cursor: grabbing; }
.content > svg { display: block; }
.err { padding: 10px 14px; border-top: 1px solid #fecaca; background: #fef2f2; color: #b00020; font: 12.5px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; }
[hidden] { display: none !important; }

/* Slide rail */
.deck-rail { flex: 0 0 172px; box-sizing: border-box; padding: 12px 12px 16px; background: #f1f5f9; border-right: 1px solid #e2e8f0; overflow-y: auto; display: none; }
.root.deck-open .deck-rail { display: block; }
.deck-thumb { display: block; width: 100%; margin-bottom: 10px; border: 2px solid #e2e8f0; border-radius: 6px; overflow: hidden; background: #fff; cursor: pointer; box-shadow: 0 1px 3px rgba(15,23,42,.12); }
.deck-thumb:hover { border-color: #cbd5e1; }
.deck-thumb.active { border-color: #2563eb; }
.deck-thumb > svg { display: block; width: 100%; height: 100%; }
.deck-toggle { position: absolute; left: 12px; bottom: 12px; width: 34px; height: 34px; display: grid; place-items: center; border: 1px solid #d5dbe3; border-radius: 8px; background: #fff; box-shadow: 0 1px 4px rgba(15,23,42,.12); color: #475569; font-size: 16px; cursor: pointer; user-select: none; }
.deck-toggle[hidden] { display: none; }
.deck-toggle:hover { color: #2563eb; border-color: #93c5fd; background: #f0f7ff; }
.root.deck-open .deck-toggle { color: #2563eb; border-color: #93c5fd; background: #eff6ff; }

/* Inspector highlight rectangles (SVG user space, non-scaling stroke) */
.xsvg-hl-hover { stroke: #6366f1; stroke-width: 1.5; stroke-dasharray: 4 3; }
.xsvg-hl-pin { stroke: #e11d48; stroke-width: 2; }

/* Floating controls: code toggle, zoom capsule, fit/1:1 */
.controls { position: absolute; right: 12px; bottom: 12px; display: flex; flex-direction: column; gap: 8px; align-items: center; }
.code-btn, .zoom-capsule, .fit-btn { background: #fff; border: 1px solid #d5dbe3; border-radius: 8px; box-shadow: 0 1px 4px rgba(15,23,42,.12); color: #475569; cursor: pointer; user-select: none; }
.code-btn, .fit-btn, .zoom-plus, .zoom-minus { width: 34px; height: 34px; display: grid; place-items: center; }
.code-btn:hover, .fit-btn:hover, .zoom-plus:hover, .zoom-minus:hover { color: #2563eb; background: #f0f7ff; }
.code-btn:hover, .fit-btn:hover { border-color: #93c5fd; }
.root.inspector-open .code-btn { color: #2563eb; border-color: #93c5fd; background: #eff6ff; }
.zoom-capsule { display: flex; flex-direction: column; overflow: hidden; }
.zoom-plus { border-bottom: 1px solid #e2e8f0; }
.code-btn svg, .fit-btn svg { width: 20px; height: 20px; display: block; }
.fit-btn svg[hidden] { display: none; }
.zoom-plus svg, .zoom-minus svg { width: 22px; height: 22px; display: block; fill: currentColor; }

/* Resize gutter */
.resize-handle { display: none; flex: 0 0 6px; cursor: col-resize; background: transparent; }
.root.inspector-open .resize-handle { display: block; }
.resize-handle:hover { background: #e2e8f0; }
.resize-guide { position: absolute; top: 0; bottom: 0; width: 2px; background: #2563eb; z-index: 50; pointer-events: none; }

/* Sidebar: inspector (top) + source (bottom), collapsible; hidden until toggled */
.sidebar { width: 380px; flex: 0 0 380px; display: none; flex-direction: column; border-left: 1px solid #e2e8f0; min-height: 0; }
.root.inspector-open .sidebar { display: flex; }
.pane { display: flex; flex-direction: column; min-height: 0; }
.pane h2 { margin: 0; padding: 8px 12px; font: 700 11px/1.4 system-ui, sans-serif; text-transform: uppercase; letter-spacing: .06em; color: #64748b; background: #f8fafc; border-bottom: 1px solid #e2e8f0; cursor: pointer; user-select: none; display: flex; align-items: center; gap: 6px; }
.pane h2::before { content: ""; width: 0; height: 0; border-left: 4px solid currentColor; border-top: 3px solid transparent; border-bottom: 3px solid transparent; transform: rotate(90deg); transition: transform .12s ease; }
.pane.collapsed h2::before { transform: rotate(0deg); }
.pane.collapsed { flex: 0 0 auto; }
.pane.collapsed > .panel, .pane.collapsed > .source { display: none; }
.inspector-pane { flex: 0 0 34%; }
.sidebar:has(.source-pane.collapsed) .inspector-pane:not(.collapsed) { flex: 1 1 0; }
.panel { flex: 1; min-height: 0; overflow: auto; padding: 10px 12px; font-size: 13px; }
.panel .hint { color: #94a3b8; margin: 4px 0; }
.panel .tag { font: 600 13px/1.4 ui-monospace, SFMono-Regular, Menlo, monospace; color: #2563eb; }
.panel .bbox, .panel .row { margin: 4px 0; color: #475569; }
.panel table.attrs { margin: 6px 0; width: 100%; table-layout: fixed; border-collapse: collapse; font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; }
.panel table.attrs th, .panel table.attrs td { text-align: left; vertical-align: top; padding: 1px 0; word-break: break-word; }
.panel table.attrs th { width: 92px; padding-right: 8px; font-weight: normal; color: #7c3aed; }
.panel table.attrs td { color: #0f172a; }
.source-pane { flex: 1; border-top: 1px solid #e2e8f0; }
.source { flex: 1; min-height: 0; overflow: hidden; }
.cm-editor .xsvg-src-hl, .xsvg-src-hl { background: rgba(225,29,72,.16); outline: 1px solid rgba(225,29,72,.45); }

/* Dark mode — follows the viewer's prefers-color-scheme. The source pane keeps the
   editor's own theme. */
@media (prefers-color-scheme: dark) {
  :host { color: #e2e8f0; background: #0f172a; }
  /* only color + image — NOT the \`background\` shorthand, which would reset the
     base rule's background-size/position and stop the checkerboard from tiling */
  .stage { background-color: #1e293b;
    background-image: linear-gradient(45deg,#263143 25%,transparent 25%), linear-gradient(-45deg,#263143 25%,transparent 25%), linear-gradient(45deg,transparent 75%,#263143 75%), linear-gradient(-45deg,transparent 75%,#263143 75%); }
  .err { background: #3b0d10; border-top-color: #7f1d1d; color: #fca5a5; }
  .deck-rail { background: #1e293b; border-right-color: #334155; }
  .deck-thumb { background: #0f172a; border-color: #334155; }
  .deck-thumb:hover { border-color: #475569; }
  .deck-thumb.active { border-color: #3b82f6; }
  .deck-toggle, .code-btn, .zoom-capsule, .fit-btn { background: #1e293b; border-color: #334155; color: #94a3b8; box-shadow: 0 1px 4px rgba(0,0,0,.4); }
  .deck-toggle:hover, .code-btn:hover, .fit-btn:hover, .zoom-plus:hover, .zoom-minus:hover { color: #60a5fa; background: #334155; }
  .deck-toggle:hover, .code-btn:hover, .fit-btn:hover { border-color: #3b82f6; }
  .root.deck-open .deck-toggle, .root.inspector-open .code-btn { color: #60a5fa; border-color: #3b82f6; background: #1e3a5f; }
  .zoom-plus { border-bottom-color: #334155; }
  .xsvg-hl-hover { stroke: #818cf8; }
  .xsvg-hl-pin { stroke: #fb7185; }
  .resize-handle:hover { background: #334155; }
  .sidebar { background: #0f172a; border-left-color: #334155; }
  .pane h2 { color: #94a3b8; background: #1e293b; border-bottom-color: #334155; }
  .panel .tag { color: #60a5fa; }
  .panel .bbox, .panel .row { color: #94a3b8; }
  .panel .hint { color: #64748b; }
  .panel table.attrs th { color: #c4b5fd; }
  .panel table.attrs td { color: #e2e8f0; }
  .cm-editor .xsvg-src-hl, .xsvg-src-hl { background: rgba(251,113,133,.22); outline-color: rgba(251,113,133,.5); }
  /* CodeMirror source pane: dark gutter (line numbers), surface, and native scrollbar.
     Prefixed with .source to outweigh CodeMirror's own injected single-class rules. */
  .source { color-scheme: dark; }
  .source .cm-editor { background: #0f172a !important; color: #e2e8f0; }
  .source .cm-gutters { background: #0f172a !important; color: #64748b !important; border-right-color: #1e293b !important; }
  .source .cm-activeLineGutter { background: #1e293b !important; color: #cbd5e1 !important; }
  .source .cm-lineNumbers .cm-gutterElement { color: #64748b; }
  .source .cm-activeLine { background: rgba(148,163,184,.06); }
  .source .cm-cursor { border-left-color: #e2e8f0; }
  .source .cm-selectionBackground, .source .cm-focused .cm-selectionBackground { background: #334155 !important; }
}
`;

export class XsvgViewInteractive extends HTMLElement {
  static observedAttributes = ["src", "quality", "inspector"];
  private shadow: ShadowRoot;
  private root!: HTMLElement;
  private stage!: HTMLElement;
  private content!: HTMLElement;
  private sidebar!: HTMLElement;
  private sourcePane!: HTMLElement;
  private panel!: HTMLElement;
  private rail!: HTMLElement;
  private deckToggle!: HTMLElement;
  private errBox!: HTMLElement;
  private codeBtn!: HTMLElement;

  private token = 0;
  private panzoom: PanZoom | null = null;
  private inspector: MountedInspector | null = null;
  private fitted = true;
  private deckFrame: [number, number, number, number] | null = null;
  private deckSelectFn: ((i: number) => void) | null = null;
  private deckActive = 0;
  private dropped: string | null = null;
  private srcProp: string | null = null;
  private resizeObs: ResizeObserver | null = null;

  /** Inline xsvg source as a JS property (used by the React wrapper / thin hosts). */
  set source(v: string | null) { this.srcProp = v; this.dropped = null; void this.render(); }
  get source(): string | null { return this.srcProp; }

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "open" });
    this.shadow.innerHTML = `
      <style>${STYLE}</style>
      <div class="root">
        <div class="body">
          <aside class="deck-rail"></aside>
          <div class="stage" part="stage">
            <div class="content"></div>
            <button class="deck-toggle" title="Slides" hidden>&#9636;</button>
            <div class="controls">
              <div class="code-btn" title="Toggle source" hidden>${CODE_ICON}</div>
              <div class="zoom-capsule">
                <div class="zoom-plus" title="Zoom in">${ZOOM_IN_ICON}</div>
                <div class="zoom-minus" title="Zoom out">${ZOOM_OUT_ICON}</div>
              </div>
              <div class="fit-btn" title="Fit / actual size">${FIT_ICON}</div>
            </div>
          </div>
          <div class="resize-handle" title="Drag to resize"></div>
          <aside class="sidebar" part="sidebar">
            <section class="pane inspector-pane"><h2>Inspector</h2><div class="panel"></div></section>
            <section class="pane source-pane"><h2>xsvg source</h2><div class="source"></div></section>
          </aside>
        </div>
        <pre class="err" hidden></pre>
      </div>`;
    const $ = <T extends Element>(s: string) => this.shadow.querySelector(s) as T;
    this.root = $(".root");
    this.stage = $(".stage");
    this.content = $(".content");
    this.sidebar = $(".sidebar");
    this.sourcePane = $(".source");
    this.panel = $(".panel");
    this.rail = $(".deck-rail");
    this.deckToggle = $(".deck-toggle");
    this.errBox = $(".err");
    this.codeBtn = $(".code-btn");

    $(".zoom-plus").addEventListener("click", () => this.panzoom?.zoomIn());
    $(".zoom-minus").addEventListener("click", () => this.panzoom?.zoomOut());
    $(".fit-btn").addEventListener("click", () => this.toggleFit());
    this.codeBtn.addEventListener("click", () => {
      const open = this.root.classList.toggle("inspector-open");
      if (open) this.inspector?.remeasure();
      this.refit();
    });
    this.deckToggle.addEventListener("click", () => {
      this.root.classList.toggle("deck-open");
      this.deckSelectFn?.(this.deckActive);
    });
    // Collapsible panes.
    this.shadow.querySelectorAll<HTMLElement>(".pane > h2").forEach((h2) => {
      h2.addEventListener("click", () => {
        const pane = h2.parentElement!;
        const collapsing = pane.classList.toggle("collapsed");
        if (!collapsing && pane.classList.contains("source-pane")) this.inspector?.remeasure();
      });
    });
    this.setupResize($(".resize-handle"));
  }

  connectedCallback() {
    this.addEventListener("dragover", this.onDragOver);
    this.addEventListener("drop", this.onDrop);
    window.addEventListener("keydown", this.onKeyDown);
    this.resizeObs = new ResizeObserver(() => this.refit());
    this.resizeObs.observe(this.stage);
    void this.render();
  }

  disconnectedCallback() {
    this.removeEventListener("dragover", this.onDragOver);
    this.removeEventListener("drop", this.onDrop);
    window.removeEventListener("keydown", this.onKeyDown);
    this.resizeObs?.disconnect();
    this.teardown();
  }

  attributeChangedCallback(name: string) {
    if (name === "src") this.dropped = null;
    void this.render();
  }

  private onDragOver = (e: DragEvent) => { if (this.hasAttribute("droppable")) e.preventDefault(); };
  private onDrop = (e: DragEvent) => {
    if (!this.hasAttribute("droppable")) return;
    e.preventDefault();
    const file = e.dataTransfer?.files?.[0];
    if (file) void file.text().then((text) => { this.dropped = text; void this.render(); });
  };

  private onKeyDown = (e: KeyboardEvent) => {
    if (!this.deckSelectFn) return;
    const el = (this.getRootNode() as Document | ShadowRoot).activeElement;
    if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || (el as HTMLElement).isContentEditable)) return;
    if (e.key === "ArrowLeft" || e.key === "PageUp") this.deckSelectFn(this.deckActive - 1);
    else if (e.key === "ArrowRight" || e.key === "PageDown") this.deckSelectFn(this.deckActive + 1);
  };

  private async readSource(): Promise<string> {
    if (this.dropped != null) return this.dropped;
    if (this.srcProp != null) return this.srcProp;
    const src = this.getAttribute("src");
    if (src) {
      const res = await fetch(src);
      if (!res.ok) throw new Error(`failed to fetch ${src}: ${res.status}`);
      return res.text();
    }
    const island = this.querySelector('script[type="application/xsvg+xml"]');
    return (island?.textContent ?? this.textContent ?? "").trim();
  }

  private teardown() {
    this.inspector?.destroy();
    this.panzoom?.destroy();
    this.inspector = null;
    this.panzoom = null;
    this.deckSelectFn = null;
  }

  private async render() {
    const mine = ++this.token;
    const withInspector = this.hasAttribute("inspector");
    this.codeBtn.hidden = !withInspector;
    if (!withInspector) this.root.classList.remove("inspector-open");
    try {
      const source = await this.readSource();
      if (mine !== this.token) return;
      if (!source) return;
      const quality = this.getAttribute("quality") ?? "balanced";
      const svg = await compileXsvg(source, { quality, sourcemap: withInspector });
      if (mine !== this.token) return;

      this.teardown();
      this.errBox.hidden = true;
      this.content.innerHTML = svg;
      const svgEl = this.content.querySelector("svg") as SVGSVGElement | null;
      if (!svgEl) return;
      sizeToViewBox(svgEl);

      this.panzoom = createPanZoom(this.stage, this.content);
      this.setupDeck(svgEl);

      if (withInspector) {
        const { mountInspector } = await import("./interactive-inspector");
        if (mine !== this.token) return;
        this.inspector = mountInspector({ svgRoot: svgEl, panel: this.panel, sourcePane: this.sourcePane, source });
        this.panzoom.onBackgroundClick(() => this.inspector?.clear());
        if (this.root.classList.contains("inspector-open")) this.inspector.remeasure();
      }
    } catch (err) {
      if (mine !== this.token) return;
      this.teardown();
      this.content.innerHTML = "";
      this.errBox.hidden = false;
      this.errBox.textContent = String(err);
    }
  }

  private setupDeck(svg: SVGSVGElement) {
    this.rail.innerHTML = "";
    this.deckFrame = null;
    this.deckSelectFn = null;
    const boards = findArtboards(svg);
    this.deckToggle.hidden = boards.length < 2;
    if (boards.length < 2) {
      this.root.classList.remove("deck-open");
      if (boards.length === 1) { this.deckFrame = boards[0].frame; this.panzoom!.fitTo(...this.deckFrame); }
      else this.panzoom!.fit();
      this.fitted = true;
      this.updateFitIcon();
      return;
    }
    const thumbs = boards.map((b: Artboard, i: number) => {
      const t = makeThumb(svg, b);
      t.addEventListener("click", () => this.deckSelectFn?.(i));
      this.rail.appendChild(t);
      return t;
    });
    this.deckSelectFn = (i: number) => {
      this.deckActive = Math.max(0, Math.min(boards.length - 1, i));
      this.deckFrame = boards[this.deckActive].frame;
      this.panzoom!.fitTo(...this.deckFrame);
      thumbs.forEach((t, k) => t.classList.toggle("active", k === this.deckActive));
      this.fitted = true;
      this.updateFitIcon();
    };
    this.root.classList.add("deck-open");
    this.deckSelectFn(0);
  }

  private toggleFit() {
    if (!this.panzoom) return;
    if (this.deckFrame) {
      const [x, y, w, h] = this.deckFrame;
      if (this.fitted) this.panzoom.resetTo(x + w / 2, y + h / 2);
      else this.panzoom.fitTo(x, y, w, h);
    } else if (this.fitted) this.panzoom.reset();
    else this.panzoom.fit();
    this.fitted = !this.fitted;
    this.updateFitIcon();
  }

  private updateFitIcon() {
    const fit = this.shadow.querySelector<SVGElement>(".icon-fit")!;
    const one = this.shadow.querySelector<SVGElement>(".icon-oneone")!;
    fit.toggleAttribute("hidden", !this.fitted);
    one.toggleAttribute("hidden", this.fitted);
  }

  private refit() {
    if (!this.panzoom || !this.fitted) return;
    if (this.deckFrame) this.panzoom.fitTo(...this.deckFrame);
    else this.panzoom.fit();
  }

  private setupResize(handle: HTMLElement) {
    const body = this.shadow.querySelector(".body") as HTMLElement;
    handle.addEventListener("dblclick", () => {
      this.sidebar.style.removeProperty("flex-basis");
      this.sidebar.style.removeProperty("width");
      this.refit();
      this.inspector?.remeasure();
    });
    handle.addEventListener("pointerdown", (e: PointerEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      let moved = false;
      let guide: HTMLElement | null = null;
      const move = (ev: PointerEvent) => {
        if (!moved && Math.abs(ev.clientX - startX) < 3) return;
        moved = true;
        if (!guide) { guide = document.createElement("div"); guide.className = "resize-guide"; body.appendChild(guide); }
        const rect = body.getBoundingClientRect();
        guide.style.left = `${ev.clientX - rect.left}px`;
      };
      const up = (ev: PointerEvent) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        guide?.remove();
        if (!moved) return;
        const rect = this.getBoundingClientRect();
        const w = Math.max(240, Math.min(rect.width - 200, rect.right - ev.clientX));
        this.sidebar.style.flexBasis = `${w}px`;
        this.sidebar.style.width = `${w}px`;
        this.refit();
        this.inspector?.remeasure();
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
    });
  }
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

if (typeof customElements !== "undefined" && !customElements.get("xsvg-view-interactive")) {
  customElements.define("xsvg-view-interactive", XsvgViewInteractive);
}
