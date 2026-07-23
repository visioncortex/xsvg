//! `<xsvg-view>` — the embeddable viewer, as a framework-free custom element.
//!
//! Importing this module registers the element, so a page can drop xsvg in anywhere:
//!
//!   import "@visioncortex/xsvg-viewer/element";
//!   // <xsvg-view src="diagram.xsvg"></xsvg-view>
//!
//! or with an inline data island (keeps the custom XML opaque to the HTML parser):
//!
//!   <xsvg-view>
//!     <script type="application/xsvg+xml"><svg …>…</svg></script>
//!   </xsvg-view>
//!
//! It compiles its source via WASM and renders the resulting plain SVG into its shadow
//! root (style-isolated); the browser's own SVG engine does the drawing. This is the
//! "render like an image" surface — no pan/zoom/inspector, ideal for docs & iframes.
//! For a bundler-free `<script>` include, use the self-contained `dist/xsvg.js` build.
import { compileXsvg, type DepLoader } from "./compiler";

export class XsvgView extends HTMLElement {
  static observedAttributes = ["src", "quality", "base-url"];
  private shadow: ShadowRoot;
  private token = 0;
  // A file dropped onto the viewer, rendered in place of `src`. Only ever set when the
  // opt-in `droppable` attribute is present; otherwise the viewer stays locked to `src`.
  private dropped: string | null = null;
  private baseUrlProp: string | null = null;
  private resolveFn: ((base: string, href: string) => [string, string] | null) | null = null;
  private loaderObj: DepLoader | null = null;

  /** Base the source's relative `<use href>` links resolve against — for inline/data-island
   *  sources, which otherwise resolve against the page. Also settable as the `base-url`
   *  attribute; a `src` attribute carries its own base. */
  set baseUrl(v: string | null) {
    this.baseUrlProp = v;
    void this.render();
  }
  get baseUrl() {
    return this.baseUrlProp;
  }

  /** Custom sync cross-file resolver (see `compileXsvg`) — bundled/in-memory deps. */
  set resolve(fn: ((base: string, href: string) => [string, string] | null) | null) {
    this.resolveFn = fn;
    void this.render();
  }
  get resolve() {
    return this.resolveFn;
  }

  /** Custom async dependency loader (see `DepLoader`). Ignored when `resolve` is set. */
  set loader(l: DepLoader | null) {
    this.loaderObj = l;
    void this.render();
  }
  get loader() {
    return this.loaderObj;
  }

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "open" });
  }

  connectedCallback() {
    // Handlers are always attached but no-op unless `droppable` is set, so toggling the
    // attribute takes effect without re-wiring. Drop is off by default: a bare
    // <xsvg-view> renders only its `src`/inline source.
    this.addEventListener("dragover", this.onDragOver);
    this.addEventListener("drop", this.onDrop);
    void this.render();
  }

  disconnectedCallback() {
    this.removeEventListener("dragover", this.onDragOver);
    this.removeEventListener("drop", this.onDrop);
  }

  attributeChangedCallback(name: string) {
    if (name === "src") this.dropped = null; // a new src supersedes a dropped file
    void this.render();
  }

  private onDragOver = (e: DragEvent) => {
    if (this.hasAttribute("droppable")) e.preventDefault(); // become a drop target only when opted in
  };
  private onDrop = (e: DragEvent) => {
    if (!this.hasAttribute("droppable")) return;
    e.preventDefault();
    const file = e.dataTransfer?.files?.[0];
    if (file) void file.text().then((text) => { this.dropped = text; void this.render(); });
  };

  private async readSource(): Promise<string> {
    if (this.dropped != null) return this.dropped;
    const src = this.getAttribute("src");
    if (src) {
      const res = await fetch(src);
      if (!res.ok) throw new Error(`failed to fetch ${src}: ${res.status}`);
      return res.text();
    }
    const island = this.querySelector('script[type="application/xsvg+xml"]');
    return (island?.textContent ?? this.textContent ?? "").trim();
  }

  private async render() {
    const mine = ++this.token; // ignore a render superseded by a newer attribute change
    try {
      const source = await this.readSource();
      if (mine !== this.token) return;
      if (!source) return;
      const quality = this.getAttribute("quality") ?? "balanced";
      // Base for relative <use href> deps: an explicit baseUrl property / base-url
      // attribute wins; else a `src` file is its own base; inline islands otherwise
      // resolve against the page.
      const src = this.getAttribute("src");
      const explicit = this.baseUrlProp ?? this.getAttribute("base-url");
      const baseUrl =
        (explicit ? new URL(explicit, location.href).href : undefined) ??
        (src ? new URL(src, location.href).href : undefined);
      const svg = await compileXsvg(source, {
        quality,
        baseUrl,
        resolve: this.resolveFn ?? undefined,
        loader: this.loaderObj ?? undefined,
      });
      if (mine !== this.token) return;
      // compiled SVGs carry a viewBox but no width/height — make it fill the host.
      this.shadow.innerHTML = `<style>:host{display:block}svg{display:block;width:100%;height:auto}</style>${svg}`;
    } catch (err) {
      if (mine !== this.token) return;
      this.shadow.innerHTML = `<pre style="color:#b00020;white-space:pre-wrap;font:12px/1.4 ui-monospace,monospace;margin:0">${escapeHtml(
        String(err),
      )}</pre>`;
    }
  }
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);
}

if (typeof customElements !== "undefined" && !customElements.get("xsvg-view")) {
  customElements.define("xsvg-view", XsvgView);
}
