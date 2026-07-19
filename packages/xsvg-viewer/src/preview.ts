// Reusable preview surface: compile an xsvg document and show the plain SVG
// fit-to-contain, and — when the document has multiple artboards (§5.2) — build a
// PowerPoint-style slide deck around it (thumbnail rail, ‹ n/N › nav, rail toggle).
//
// Extracted so the standalone /preview page and the playground pane share one
// implementation: the playground gets the artboard/deck feature for free, and the
// two never drift. Mount it into any positioned host box:
//
//   const preview = createPreview(hostEl, { hashDeepLink: true, showErrors: true });
//   await preview.render(source);
import { compileXsvg } from "./compiler";
import { findArtboards, makeThumb } from "./artboards";

// The component's styles, injected once into <head> on first use. Inlined (rather than a
// side-effect `import "./preview.css"`) so the package works in any consumer with no
// css-aware bundler or extra stylesheet import. Everything is scoped under .xsvg-preview.
const PREVIEW_CSS = `
.xsvg-preview { position: absolute; inset: 0; overflow: hidden; }
.xsvg-stage { position: absolute; inset: 0; }
.xsvg-stage > svg { display: block; width: 100%; height: 100%; }
.xsvg-preview.rail-open .xsvg-stage { left: 172px; }
.xsvg-stage > .error { margin: 16px; font: 13px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; color: #b00020; white-space: pre-wrap; }
.xsvg-preview .deck-nav { position: absolute; left: 50%; bottom: 16px; transform: translateX(-50%); display: flex; align-items: center; gap: 10px; padding: 5px 8px; border-radius: 999px; background: rgba(15,23,42,.82); box-shadow: 0 2px 10px rgba(15,23,42,.25); color: #f8fafc; font: 13px/1 "Helvetica Neue", Arial, sans-serif; }
.xsvg-preview .deck-nav button { width: 30px; height: 30px; border: 0; border-radius: 999px; background: transparent; color: #f8fafc; font-size: 20px; line-height: 1; cursor: pointer; }
.xsvg-preview .deck-nav button:hover:not(:disabled) { background: rgba(255,255,255,.14); }
.xsvg-preview .deck-nav button:disabled { opacity: .35; cursor: default; }
.xsvg-preview .deck-label { min-width: 96px; text-align: center; letter-spacing: .02em; }
.xsvg-preview .deck-rail { position: absolute; left: 0; top: 0; bottom: 0; width: 172px; box-sizing: border-box; padding: 12px 12px 60px; background: #f1f5f9; border-right: 1px solid #e2e8f0; overflow-y: auto; display: none; }
.xsvg-preview.rail-open .deck-rail { display: block; }
.xsvg-preview .deck-thumb { display: block; width: 100%; margin-bottom: 10px; border: 2px solid #e2e8f0; border-radius: 6px; overflow: hidden; background: #fff; cursor: pointer; box-shadow: 0 1px 3px rgba(15,23,42,.12); }
.xsvg-preview .deck-thumb:hover { border-color: #cbd5e1; }
.xsvg-preview .deck-thumb.active { border-color: #2563eb; }
.xsvg-preview .deck-thumb > svg { display: block; width: 100%; height: 100%; }
.xsvg-preview .deck-toggle { position: absolute; left: 12px; bottom: 14px; z-index: 5; width: 34px; height: 34px; border: 0; border-radius: 8px; background: rgba(15,23,42,.82); color: #f8fafc; font-size: 16px; line-height: 1; cursor: pointer; box-shadow: 0 2px 8px rgba(15,23,42,.28); }
.xsvg-preview .deck-toggle:hover { background: rgba(30,41,59,.92); }
`;

let stylesInjected = false;
function ensureStyles(): void {
  if (stylesInjected || typeof document === "undefined") return;
  const style = document.createElement("style");
  style.setAttribute("data-xsvg-preview", "");
  style.textContent = PREVIEW_CSS;
  document.head.appendChild(style);
  stylesInjected = true;
}

export interface PreviewOptions {
  /** Page through artboards via a 1-based location.hash (e.g. …#3), following
   *  hashchange. Off by default so hosts that use the hash for other purposes
   *  (the playground's #src= share links) aren't hijacked. */
  hashDeepLink?: boolean;
  /** Render compile errors into the stage instead of throwing. The standalone
   *  page wants this; the playground keeps the last good preview and shows its
   *  own error box, so it leaves this off and catches the throw. */
  showErrors?: boolean;
  /** Whether a multi-artboard document opens with its slide rail expanded. On by
   *  default; set `false` to start with the rail collapsed — the toggle button is
   *  still shown, so the viewer can open it. */
  slides?: boolean;
}

/** "superseded" means a newer render() started before this one finished — the
 *  caller should treat it as a no-op (the newer render owns the final state). */
export type RenderResult = "ok" | "superseded";

export interface Preview {
  render(source: string): Promise<RenderResult>;
  destroy(): void;
}

const escapeHtml = (s: string) =>
  s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]!);

/** True when a keydown originates from a text-editing context (the playground's
 *  CodeMirror, an input, etc.) — the deck must not steal arrows/space there. */
function isEditingTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  return (
    el.isContentEditable ||
    /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) ||
    !!el.closest?.(".cm-editor")
  );
}

export function createPreview(host: HTMLElement, opts: PreviewOptions = {}): Preview {
  ensureStyles();
  const root = document.createElement("div");
  root.className = "xsvg-preview";
  const stage = document.createElement("div");
  stage.className = "xsvg-stage";
  root.appendChild(stage);
  host.appendChild(root);

  // Teardown for the current deck's listeners + chrome, if any.
  let teardownDeck: (() => void) | null = null;
  const clearDeck = () => {
    teardownDeck?.();
    teardownDeck = null;
  };

  function setupDeck(svg: SVGSVGElement): void {
    const boards = findArtboards(svg);
    if (boards.length < 2) return;

    // ---- thumbnail rail: each thumb is the SVG reframed to one slide
    const rail = document.createElement("div");
    rail.className = "deck-rail";
    const thumbs: HTMLElement[] = boards.map((b, idx) => {
      const thumb = makeThumb(svg, b);
      thumb.addEventListener("click", () => show(idx));
      rail.appendChild(thumb);
      return thumb;
    });

    // ---- bottom-center ‹ n/N › nav
    const nav = document.createElement("div");
    nav.className = "deck-nav";
    const prev = document.createElement("button");
    prev.textContent = "‹";
    const label = document.createElement("span");
    label.className = "deck-label";
    const next = document.createElement("button");
    next.textContent = "›";
    nav.append(prev, label, next);

    // ---- bottom-left rail toggle
    const toggle = document.createElement("button");
    toggle.className = "deck-toggle";
    toggle.title = "Toggle slides";
    toggle.textContent = "▤";
    toggle.addEventListener("click", () => root.classList.toggle("rail-open"));

    root.append(rail, nav, toggle);
    if (opts.slides !== false) root.classList.add("rail-open"); // open by default; slides:false starts collapsed

    let i = 0;
    const show = (n: number) => {
      i = Math.max(0, Math.min(boards.length - 1, n));
      svg.setAttribute("viewBox", boards[i].frame.join(" "));
      label.textContent = `${i + 1} / ${boards.length} · ${boards[i].label}`;
      prev.disabled = i === 0;
      next.disabled = i === boards.length - 1;
      thumbs.forEach((t, k) => t.classList.toggle("active", k === i));
    };
    prev.addEventListener("click", () => show(i - 1));
    next.addEventListener("click", () => show(i + 1));

    const onKey = (e: KeyboardEvent) => {
      if (isEditingTarget(e.target)) return;
      if (e.key === "ArrowLeft" || e.key === "PageUp") show(i - 1);
      else if (e.key === "ArrowRight" || e.key === "PageDown" || e.key === " ") show(i + 1);
    };
    window.addEventListener("keydown", onKey);

    // Deep-link to a slide with a 1-based #hash (e.g. …#3), following changes.
    let onHash: (() => void) | null = null;
    if (opts.hashDeepLink) {
      const fromHash = () => {
        const n = parseInt(location.hash.slice(1), 10);
        return Number.isFinite(n) ? n - 1 : 0;
      };
      onHash = () => show(fromHash());
      window.addEventListener("hashchange", onHash);
      show(fromHash());
    } else {
      show(0);
    }

    teardownDeck = () => {
      window.removeEventListener("keydown", onKey);
      if (onHash) window.removeEventListener("hashchange", onHash);
      rail.remove();
      nav.remove();
      toggle.remove();
      root.classList.remove("rail-open");
    };
  }

  let seq = 0;
  async function render(source: string): Promise<RenderResult> {
    const mine = ++seq;
    let svgHtml: string;
    try {
      svgHtml = await compileXsvg(source);
    } catch (err) {
      if (mine !== seq) return "superseded"; // a newer edit already superseded us
      if (opts.showErrors) {
        clearDeck();
        stage.innerHTML = `<pre class="error">${escapeHtml(String(err))}</pre>`;
        return "ok";
      }
      throw err; // host keeps the last good preview and shows its own error UI
    }
    if (mine !== seq) return "superseded";
    clearDeck();
    stage.innerHTML = svgHtml;
    const svg = stage.querySelector("svg");
    if (svg) setupDeck(svg as SVGSVGElement);
    return "ok";
  }

  return {
    render,
    destroy() {
      clearDeck();
      root.remove();
    },
  };
}
