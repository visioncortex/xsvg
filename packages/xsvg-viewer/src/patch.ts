// The DOM half of the incremental patch protocol (docs/Incremental.md §4): given
// the text a drawing was compiled from and an edited version of it, re-emit only
// the elements the edit can have changed and swap them into the rendered SVG.
//
// Deliberately importing types only: everything wasm-backed comes in through
// `PatchContext` (the session, the element spans), and the offset primitives come
// in as arguments the caller has already built for its own bookkeeping. With no
// runtime imports at all, this file is the protocol alone — and runs anywhere,
// which is how its sibling-run walking gets exercised against a real DOM headlessly
// (scripts/dom-patch-check.mjs; the string-level twin is incremental-check.mjs).

import type { CompileSession } from "./compiler";
import type { NodeKeyStore } from "./keys";
import type { ByteIndex, SingleEdit } from "./sourcemap";

/** What a patch needs beyond the two texts: the incremental handle on the last
 *  compile, the store naming every element, and the compiler's element-span
 *  report (`nodeSpans`, injected so this module never touches wasm itself). */
export interface PatchContext {
  session: CompileSession;
  keys: NodeKeyStore;
  spans(source: string): [number, number][];
}

/** One edit, described the way the caller already holds it: the texts on both
 *  sides, the single contiguous change between them (`singleEdit`), and each
 *  text's byte index. */
export interface DocumentEdit {
  compiled: string;
  next: string;
  edit: SingleEdit;
  oldBytes: ByteIndex;
  newBytes: ByteIndex;
}

/**
 * Patch `svg` in place for a single contiguous source edit.
 *
 * Returns false — having changed nothing — whenever the edit's effect cannot be
 * confined to a known set of top-level elements, leaving the caller to
 * recompile. Everything is resolved, compiled and parsed before anything is
 * mutated: a patch that failed halfway would leave the drawing disagreeing with
 * its own source.
 *
 * `keys` is advanced across the edit as a side effect — even when the patch
 * declines. That is safe for the fall-back path (the recompile syncs the store
 * against the same `next` text) and required for the success path (the names
 * the compiler stamps must be the names the store now holds).
 */
export function patchDocument(ctx: PatchContext, svg: SVGSVGElement, doc: DocumentEdit): boolean {
  const { session, keys } = ctx;
  const { compiled, next, edit } = doc;

  // The common prefix ends at the same byte offset in both versions, so that one
  // offset is the pivot: everything before it is unmoved, everything after shifts
  // by the length difference.
  const pivot = doc.newBytes.toByte(edit.from);
  const delta = doc.newBytes.byteLength - doc.oldBytes.byteLength;

  // Which elements have to be re-emitted. Parse-only, so it needs no keys and can
  // run while the store still describes the document as rendered.
  const unit = session.range(next, pivot);
  if (!unit) return false; // the edit is in the root tag or between elements
  const targets = [unit, ...session.dependents(next, pivot)];

  // Reproduce each target as it currently stands, to learn how many sibling nodes
  // it occupies. This must happen while the store still holds the *old* naming,
  // hence before the shift below.
  const current = targets.map(([start]) =>
    session.fragment(compiled, keptStart(start, pivot, delta)),
  );

  // Carry every element's name across the edit. From here the arithmetic is done:
  // a name is what the DOM is anchored on *and* what the compiler stamps, and
  // neither moves when the text does. That is the whole point of keys — the
  // shifting is confined to these two lines instead of being chased through span
  // attributes and generated def ids, one of which the host cannot chase at all.
  keys.shift(pivot, delta);
  keys.sync(ctx.spans(next));

  const swaps: { run: ChildNode[]; nodes: ChildNode[] }[] = [];
  for (const [i, [start]] of targets.entries()) {
    // Unchanged by construction: the element kept its name across the shift, so
    // the name the store answers with now is the one already in the DOM.
    const name = keys.key(start);
    if (!name) return false;
    // The store mints names as `n<digits>`, so they need no selector escaping.
    const anchor = svg.querySelector(`[data-xsvg-node="${name}"]`);
    // No anchor: this fragment emitted no element last time (a degradation marker
    // is a comment, which carries no attributes), so there is nothing to replace.
    if (!anchor) return false;
    const run = fragmentRun(anchor, name, current[i]);
    if (!run) return false;
    const markup = session.fragment(next, start);
    if (markup == null) return false;
    swaps.push({ run, nodes: parseSvgFragment(markup) });
  }

  for (const { run, nodes } of swaps) {
    run[0].replaceWith(...nodes);
    for (let i = 1; i < run.length; i++) run[i].remove();
  }
  return true;
}

const SVG_NS = "http://www.w3.org/2000/svg";

/** Where an offset in the edited source sat in the previous one — for asking the
 *  session to reproduce a fragment as it currently stands in the DOM. */
function keptStart(start: number, pivot: number, delta: number): number {
  return start <= pivot ? start : start - delta;
}

/**
 * The consecutive sibling nodes in the rendered document that together are one
 * fragment's output.
 *
 * A fragment is not always a single element. It may emit a `<filter>` definition
 * ahead of the shape that references it, or a degradation comment ahead of both,
 * and replacing only the element would strand the rest — leaving a second `<filter>`
 * with a duplicate id, which the browser resolves to the *stale* one. So the run is
 * measured against a re-compile of the fragment as it currently stands (byte-identical
 * to what is in the DOM, by the incremental invariant): its node count gives the run's
 * length, and the position of the keyed node within it gives where the run starts
 * relative to the anchor.
 *
 * Null when the current markup cannot be reproduced, or when the DOM does not hold as
 * many siblings as it claims — either way the caller falls back to a full compile.
 */
function fragmentRun(anchor: Element, name: string, current: string | null): ChildNode[] | null {
  if (current == null) return null;
  const nodes = parseSvgFragment(current);
  const at = nodes.findIndex(
    (n) => n.nodeType === 1 && (n as Element).getAttribute("data-xsvg-node") === name,
  );
  if (at < 0) return null;
  const run: ChildNode[] = [anchor];
  let before: ChildNode = anchor;
  for (let i = 0; i < at; i++) {
    if (!before.previousSibling) return null;
    run.unshift((before = before.previousSibling));
  }
  let after: ChildNode = anchor;
  for (let i = at + 1; i < nodes.length; i++) {
    if (!after.nextSibling) return null;
    run.push((after = after.nextSibling));
  }
  return run;
}

/**
 * Parse re-emitted fragment markup into nodes ready to splice into the drawing.
 *
 * Parsed against an `<svg>` context element, so `<path>` and friends land in the
 * SVG namespace rather than becoming inert unknown HTML elements. All child nodes
 * are returned, not just the first element: a fragment may be preceded by
 * degradation marker comments, and dropping those would lose the compiler's
 * explanation of what it declined to draw.
 */
function parseSvgFragment(markup: string): ChildNode[] {
  const holder = document.createElementNS(SVG_NS, "svg");
  holder.innerHTML = markup;
  return Array.from(holder.childNodes);
}
