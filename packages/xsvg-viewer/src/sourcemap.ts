// Source-map helpers for the interactive viewer's element→source projection.
//
// The WASM compiler tags emitted elements with `data-xsvg-pos="START-END"` — byte
// offsets into the xsvg source (UTF-8). CodeMirror positions are UTF-16 string
// indices, so `ByteIndex` converts between the two. Synthesized subtrees tag only
// their root element, so `nearestSource` walks up to the nearest tagged ancestor.

export interface SourceRange {
  /** UTF-8 byte offset into the xsvg source. */
  start: number;
  end: number;
}

/**
 * Byte-offset → UTF-16-index converter for one source string. Built once
 * (O(n) over code points); lookups are O(log n) via binary search over code-point
 * boundaries. Element ranges begin/end on ASCII delimiters, so they always land on
 * a boundary — but this stays correct for any UTF-8 offset.
 */
export class ByteIndex {
  private bytes: number[] = [0]; // byte offset of each code-point boundary
  private units: number[] = [0]; // matching UTF-16 index

  constructor(source: string) {
    let b = 0;
    let u = 0;
    for (const ch of source) {
      b += utf8Len(ch.codePointAt(0)!);
      u += ch.length; // 1 for BMP, 2 for a surrogate pair
      this.bytes.push(b);
      this.units.push(u);
    }
  }

  /** UTF-16 string index at (or just before) the given byte offset. */
  toStr(byte: number): number {
    return this.units[this.seek(this.bytes, byte)];
  }

  /** Byte offset at (or just before) the given UTF-16 string index — the inverse of
   *  `toStr`, for handing an edit position back to the compiler's byte-based API. */
  toByte(unit: number): number {
    return this.bytes[this.seek(this.units, unit)];
  }

  /** Total length of the source in UTF-8 bytes. */
  get byteLength(): number {
    return this.bytes[this.bytes.length - 1];
  }

  // Largest index whose value is ≤ `at`, over an ascending table.
  private seek(table: number[], at: number): number {
    let lo = 0;
    let hi = table.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (table[mid] <= at) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }
}

/** One contiguous replacement turning `a` into `b`: `a.slice(from, to)` became
 *  `b.slice(from, insertEnd)`. */
export interface SingleEdit {
  from: number;
  /** End of the replaced span in the **old** string. */
  to: number;
  /** End of the replacement in the **new** string. */
  insertEnd: number;
}

/**
 * Describe the difference between two versions of a source as one contiguous
 * replacement — the common prefix and suffix pared away.
 *
 * This is what lets the viewer patch instead of recompile: with the change
 * localized to `[from, to)`, every source offset outside it maps between the two
 * versions by a constant shift, so the DOM's `data-xsvg-pos` anchors stay findable.
 * Any edit *is* representable this way; a scattered multi-site change just yields a
 * span wide enough to cover them all, which the caller will find spans several
 * fragments and fall back on.
 *
 * Returns null when the strings are identical.
 */
export function singleEdit(a: string, b: string): SingleEdit | null {
  if (a === b) return null;
  const max = Math.min(a.length, b.length);
  let from = 0;
  while (from < max && a[from] === b[from]) from++;
  // Stop the suffix scan at `from` so the two spans can never overlap it.
  let tail = 0;
  while (tail < max - from && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  return { from, to: a.length - tail, insertEnd: b.length - tail };
}

function utf8Len(cp: number): number {
  if (cp < 0x80) return 1;
  if (cp < 0x800) return 2;
  if (cp < 0x10000) return 3;
  return 4;
}

/** Parse `data-xsvg-pos="a-b"` on an element, or null if absent/malformed. */
export function posAttr(el: Element): SourceRange | null {
  const v = el.getAttribute("data-xsvg-pos");
  if (!v) return null;
  const dash = v.indexOf("-");
  if (dash < 0) return null;
  const start = parseInt(v.slice(0, dash), 10);
  const end = parseInt(v.slice(dash + 1), 10);
  if (!Number.isFinite(start) || !Number.isFinite(end)) return null;
  return { start, end };
}

/**
 * Walk up from `el` (inclusive) to the nearest ancestor carrying `data-xsvg-pos`,
 * not going above `root`. Returns the tagged element and its source range, or null.
 */
export function nearestSource(
  el: Element | null,
  root: Element,
): { el: Element; range: SourceRange } | null {
  let cur: Element | null = el;
  while (cur) {
    const range = posAttr(cur);
    if (range) return { el: cur, range };
    if (cur === root) break;
    cur = cur.parentElement;
  }
  return null;
}
