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
import "./preview.css";

export interface PreviewOptions {
  /** Page through artboards via a 1-based location.hash (e.g. …#3), following
   *  hashchange. Off by default so hosts that use the hash for other purposes
   *  (the playground's #src= share links) aren't hijacked. */
  hashDeepLink?: boolean;
  /** Render compile errors into the stage instead of throwing. The standalone
   *  page wants this; the playground keeps the last good preview and shows its
   *  own error box, so it leaves this off and catches the throw. */
  showErrors?: boolean;
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
    root.classList.add("rail-open"); // decks open the rail by default

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
