//! `<XsvgView>` — the embeddable viewer as a React component.
//!
//!   import { XsvgView } from "@visioncortex/xsvg-viewer/react";
//!
//!   <XsvgView source={`<svg …>…</svg>`} />      // inline xsvg
//!   <XsvgView src="/diagrams/pie.xsvg" />        // fetched by URL
//!
//! It compiles the source with the WASM compiler (client-side) and renders the plain
//! SVG. `react` is a peer dependency — this module is only pulled in when you import
//! the `/react` entry, so non-React consumers never load it.
import { createElement, useEffect, useRef, useState } from "react";
import type { DragEvent as ReactDragEvent, HTMLAttributes } from "react";
import { compileXsvg, type DepLoader } from "./compiler";

/** Cross-file `<use href>` linking inputs, shared by both components (see `compileXsvg`). */
interface LinkProps {
  /** Base URL the source's relative `<use href="…">` links resolve against. Defaults
   *  to the `src` URL when fetching, else the page URL. */
  baseUrl?: string;
  /** Custom sync cross-file resolver — bundled/in-memory deps; single compile pass. */
  resolve?: (base: string, href: string) => [string, string] | null;
  /** Custom async dependency loader (`key`/`fetch`). Ignored when `resolve` is given. */
  loader?: DepLoader;
}

export interface XsvgViewProps extends Omit<HTMLAttributes<HTMLDivElement>, "onError">, LinkProps {
  /** Inline xsvg source. Takes precedence over `src`. */
  source?: string;
  /** URL to fetch xsvg source from (used when `source` is absent). */
  src?: string;
  /** Quality profile string (default "balanced"). */
  quality?: string;
  /** Allow dropping a local file onto the viewer to render it. Off by default —
   *  the viewer renders only `source`/`src` unless this is set. */
  droppable?: boolean;
  /** Called if compilation or fetching fails. */
  onError?: (err: unknown) => void;
}

// Compiled SVGs carry a viewBox but no width/height — make the root fill its box.
function fit(svg: string): string {
  return svg.replace(/^<svg\b/, '<svg style="display:block;width:100%;height:auto"');
}

export interface XsvgViewInteractiveProps extends HTMLAttributes<HTMLElement>, LinkProps {
  /** Inline xsvg source. Takes precedence over `src`. */
  source?: string;
  /** URL to fetch xsvg source from. */
  src?: string;
  /** Quality profile string (default "balanced"). */
  quality?: string;
  /** Add the source pane + element↔source inspector. Off by default. Lazy-loads
   *  CodeMirror (an optional peer dependency) only when enabled. */
  inspector?: boolean;
  /** Allow dropping a local file onto the viewer to render it. Off by default. */
  droppable?: boolean;
}

/** The full interactive viewer (pan/zoom, artboard deck, optional inspector) as a
 *  React component. Registers the `<xsvg-view-interactive>` element on mount via a
 *  dynamic import, so `XsvgView` above never pulls in the pan/zoom code. */
export function XsvgViewInteractive({ source, src, quality, inspector, droppable, baseUrl, resolve, loader, ...rest }: XsvgViewInteractiveProps) {
  const ref = useRef<
    HTMLElement & {
      source?: string | null;
      baseUrl?: string | null;
      resolve?: XsvgViewInteractiveProps["resolve"] | null;
      loader?: DepLoader | null;
    }
  >(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let live = true;
    void import("./interactive").then(() => { if (live) setReady(true); });
    return () => { live = false; };
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el || !ready) return;
    if (src != null) el.setAttribute("src", src); else el.removeAttribute("src");
    if (quality != null) el.setAttribute("quality", quality); else el.removeAttribute("quality");
    el.toggleAttribute("inspector", !!inspector);
    el.toggleAttribute("droppable", !!droppable);
    // Linking inputs and inline source flow through the element's properties. Every
    // assignment re-renders, so only touch what changed, and set source last so it
    // compiles with the linking inputs in place. (resolve/loader compare by reference
    // — memoize them in the consumer, per the usual React contract.)
    if (el.baseUrl !== (baseUrl ?? null)) el.baseUrl = baseUrl ?? null;
    if (el.resolve !== (resolve ?? null)) el.resolve = resolve ?? null;
    if (el.loader !== (loader ?? null)) el.loader = loader ?? null;
    if (source != null) el.source = source;
  }, [ready, source, src, quality, inspector, droppable, baseUrl, resolve, loader]);

  return createElement("xsvg-view-interactive", { ref, ...rest });
}

export function XsvgView({ source, src, quality, droppable, onError, baseUrl, resolve, loader, ...rest }: XsvgViewProps) {
  const [html, setHtml] = useState("");
  const [error, setError] = useState<string | null>(null);
  // A dropped file overrides source/src; cleared whenever those inputs change.
  const [dropped, setDropped] = useState<string | null>(null);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => { setDropped(null); }, [source, src]);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        let text = dropped ?? source ?? "";
        if (!text && src) {
          const res = await fetch(src);
          if (!res.ok) throw new Error(`failed to fetch ${src}: ${res.status}`);
          text = await res.text();
        }
        text = text.trim();
        if (!text) {
          if (!cancelled) { setHtml(""); setError(null); }
          return;
        }
        // Base for relative <use href> deps: an explicit baseUrl wins; else a fetched
        // `src` file is its own base; inline source otherwise resolves against the page.
        const base =
          baseUrl ?? (!(dropped ?? source) && src ? new URL(src, location.href).href : undefined);
        const svg = await compileXsvg(text, { quality, baseUrl: base, resolve, loader });
        if (!cancelled) { setHtml(fit(svg)); setError(null); }
      } catch (err) {
        if (cancelled) return;
        setHtml("");
        setError(String(err));
        onErrorRef.current?.(err);
      }
    })();
    return () => { cancelled = true; };
  }, [dropped, source, src, quality, baseUrl, resolve, loader]);

  // Drop-to-load is opt-in; without `droppable` the viewer renders only source/src.
  const dropHandlers = droppable
    ? {
        onDragOver: (e: ReactDragEvent) => e.preventDefault(),
        onDrop: (e: ReactDragEvent) => {
          e.preventDefault();
          const file = e.dataTransfer.files?.[0];
          if (file) void file.text().then(setDropped);
        },
      }
    : {};

  if (error !== null) {
    return (
      <pre style={{ color: "#b00020", whiteSpace: "pre-wrap", font: "12px/1.4 ui-monospace, monospace", margin: 0 }}>
        {error}
      </pre>
    );
  }
  return <div {...rest} {...dropHandlers} dangerouslySetInnerHTML={{ __html: html }} />;
}

export default XsvgView;
