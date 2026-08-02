// Host-supplied node keys (docs/Incremental.md §3.1).
//
// By default the compiler stamps identity from a node's byte start —
// `data-xsvg-pos="993-1088"`, and generated def ids like `x-flt-993`. That makes an
// element's *output* depend on how much text precedes it, so an edit anywhere early
// renames spans and ids for everything after it, none of which changed. A host
// patching a rendered document then has to chase those renames, and the one it
// cannot chase without reimplementing the compiler is the def ids.
//
// A key is the fix: the host names each element, the compiler stamps the name
// instead of the offset, and an element compiles to the same bytes however far it
// has moved. The bookkeeping does not disappear — it moves *here*, where it belongs,
// and stops leaking into the output.

import type { SourceRange } from "./sourcemap";

/** Resolves a rendered element's identity attribute back to a source range. */
export interface NodeIndex {
  rangeOf(key: string): SourceRange | null;
}

/**
 * Names every element of a document and keeps those names attached to the same
 * elements as the text is edited.
 *
 * The contract the compiler asks for is "key every element, not just the top-level
 * ones" — nested elements are stamped too (warp children, paragraphs, table cells),
 * and an unkeyed one silently falls back to a byte span, reintroducing exactly the
 * dependence this removes. So keys come from `node_spans`, which reports every
 * element, rather than from anything the host guesses.
 */
export class NodeKeyStore implements NodeIndex {
  /** byte start → key. The compiler asks by start offset, so this is the hot path. */
  private byStart = new Map<number, string>();
  /** key → range, for projecting a rendered element back to its source. */
  private ranges = new Map<string, SourceRange>();
  private next = 0;

  /** The callback shape `compile_keyed_ext` wants. */
  readonly key = (byteStart: number): string | null => this.byStart.get(byteStart) ?? null;

  rangeOf(key: string): SourceRange | null {
    return this.ranges.get(key) ?? null;
  }

  /** The key of the innermost element whose range contains `byte`, or null. */
  keyAt(byte: number): string | null {
    let best: string | null = null;
    let width = Infinity;
    for (const [key, r] of this.ranges) {
      if (byte >= r.start && byte <= r.end && r.end - r.start < width) {
        best = key;
        width = r.end - r.start;
      }
    }
    return best;
  }

  /**
   * Adopt a document's element spans, keeping the name of every element already
   * known at that start offset and minting one for the rest.
   *
   * Call `shift` first when the text has been edited, so "already known at that
   * start offset" means the same element rather than whichever one has since slid
   * into its place.
   */
  sync(spans: [number, number][]): void {
    const byStart = new Map<number, string>();
    this.ranges = new Map();
    for (const [start, end] of spans) {
      const key = this.byStart.get(start) ?? `n${this.next++}`;
      byStart.set(start, key);
      this.ranges.set(key, { start, end });
    }
    // Elements that no longer exist drop out with the old map; their keys are never
    // reissued, so a stale reference can only fail to resolve, never resolve wrong.
    this.byStart = byStart;
  }

  /**
   * Carry every name across a contiguous edit: offsets at or before the pivot are
   * unmoved, later ones shift by the edit's length change.
   *
   * This is the same arithmetic the patch path does — the point of keys is not that
   * it goes away, but that it no longer reaches the output.
   */
  shift(pivot: number, delta: number): void {
    if (delta === 0) return;
    const moved = new Map<number, string>();
    for (const [start, key] of this.byStart) {
      const to = start <= pivot ? start : start + delta;
      // A deletion can slide one element's start onto another's. That means at least
      // one of them no longer exists; rather than guess which, let both be renamed by
      // the next `sync` — a fresh key costs a re-emit, a wrong key costs a wrong
      // element.
      if (moved.has(to)) {
        moved.delete(to);
        continue;
      }
      moved.set(to, key);
    }
    this.byStart = moved;
  }
}
