// <xsvg-view> — the embeddable v0 viewer (see SYNTAX.md / PLAN.md §1).
//
// Usage:
//   <xsvg-view src="diagram.xsvg"></xsvg-view>
// or with an inline data island (keeps the custom XML opaque to the HTML parser):
//   <xsvg-view>
//     <script type="application/xsvg+xml"><svg …>…</svg></script>
//   </xsvg-view>
//
// It compiles its source via WASM and renders the resulting SVG into its shadow
// root (style-isolated). The browser's own SVG engine does the actual drawing.
// This is the "barebone viewer" use case — no pan/zoom/inspector, renders like an
// image. Source maps are intentionally off, so the emitted SVG stays clean.
import { compileXsvg } from "../core/compiler";

export class XsvgView extends HTMLElement {
  static observedAttributes = ["src", "quality"];
  private shadow: ShadowRoot;

  constructor() {
    super();
    this.shadow = this.attachShadow({ mode: "open" });
  }

  connectedCallback() {
    void this.render();
  }

  attributeChangedCallback() {
    void this.render();
  }

  private async readSource(): Promise<string> {
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
    try {
      const source = await this.readSource();
      if (!source) return;
      const quality = this.getAttribute("quality") ?? "balanced";
      const svg = await compileXsvg(source, { quality });
      // compiled SVGs have a viewBox but no width/height — make it fill the host
      this.shadow.innerHTML = `<style>svg{display:block;width:100%;height:auto}</style>${svg}`;
    } catch (err) {
      this.shadow.innerHTML = `<pre style="color:#b00020;white-space:pre-wrap;font:12px/1.4 ui-monospace,monospace;margin:0">${escapeHtml(
        String(err),
      )}</pre>`;
    }
  }
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);
}

if (!customElements.get("xsvg-view")) {
  customElements.define("xsvg-view", XsvgView);
}
