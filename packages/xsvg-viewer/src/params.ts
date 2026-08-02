// Manifest-driven parameter controls (docs/Extensibility.md §4).
//
// A plugin module declares its elements and their parameters — types, defaults,
// ranges, enum values, help text — and `xsvg_describe()` answers with that
// schema without generating anything. So a host can build an editing UI for an
// element the compiler has never heard of, from the module alone.
//
// This module is the schema→DOM half plus the small amount of source surgery an
// edit needs. It deliberately knows nothing about the inspector or the compiler:
// given a schema and a start tag, it produces controls and rewritten source.

/** One declared parameter. Unknown `type` values fall back to a text field. */
export interface ParamSpec {
  type?: string;
  default?: string | number | boolean;
  min?: number;
  max?: number;
  step?: number;
  /** Allowed values for `type: "enum"`. */
  values?: string[];
  /** Help text, shown as the control's tooltip. */
  doc?: string;
  /** Parameters whose presence makes this one have no effect — `<clip:arrow length>`
   *  is read only in the polar form, so `x2`/`y2` supersede it. Only the element
   *  knows this; the panel uses it to disable a control that would otherwise look
   *  live and do nothing. */
  supersededBy?: string[];
  /** Why this parameter must not be offered for editing — `font-family` selects a
   *  face that has to be loaded before anything can be measured in it, which is
   *  asynchronous and document-wide. Absent means it is editable. */
  readOnly?: string;
  /** Whether this parameter places or sizes the element (`x`/`y`, `width`/`height`,
   *  `cx`/`cy`/`r`, endpoints). A fact the element declares; what a host does with
   *  it is the host's call — this panel offers no control, leaving position and
   *  extent to the canvas and the source pane. */
  geometry?: boolean;
  /** The XML namespace the attribute lives in — `x:border-width` is `border-width`
   *  in the xsvg namespace. A host must resolve this to the prefix *this document*
   *  binds; writing a hard-coded prefix a document does not bind is malformed XML. */
  ns?: string;
}

/** One element a module provides. */
export interface ElementSpec {
  doc?: string;
  params?: Record<string, ParamSpec>;
  input?: string;
  children?: string;
}

/** Schema lookup across every module a document loaded. */
export interface PluginSchemas {
  spec(ns: string, tag: string): ElementSpec | null;
}

// ---- source surgery -------------------------------------------------------

export interface AttrSlot {
  value: string;
  /** UTF-16 offsets of the value text, relative to the start of the tag. */
  from: number;
  to: number;
}

export interface StartTag {
  /** Qualified name as written, e.g. `clip:arrow`. */
  name: string;
  prefix: string;
  local: string;
  attrs: Map<string, AttrSlot>;
  /** Where a new attribute should be inserted (before `/>` or `>`). */
  insertAt: number;
}

/**
 * Parse the start tag at the beginning of `text`.
 *
 * Hand-rolled rather than handed to `DOMParser` because we need **byte-accurate
 * offsets back into the author's own source** — the whole point is to edit the
 * document in place, preserving its formatting, not to round-trip it through a
 * serializer that would reflow everything.
 */
export function parseStartTag(text: string): StartTag | null {
  if (text[0] !== "<") return null;
  let i = 1;
  const nameStart = i;
  while (i < text.length && !/[\s/>]/.test(text[i])) i++;
  const name = text.slice(nameStart, i);
  if (!name) return null;
  const colon = name.indexOf(":");

  const attrs = new Map<string, AttrSlot>();
  for (;;) {
    while (i < text.length && /\s/.test(text[i])) i++;
    if (i >= text.length) return null;
    if (text[i] === ">" || (text[i] === "/" && text[i + 1] === ">")) {
      return {
        name,
        prefix: colon < 0 ? "" : name.slice(0, colon),
        local: colon < 0 ? name : name.slice(colon + 1),
        attrs,
        insertAt: i,
      };
    }
    const attrStart = i;
    while (i < text.length && !/[\s=/>]/.test(text[i])) i++;
    const attrName = text.slice(attrStart, i);
    while (i < text.length && /\s/.test(text[i])) i++;
    if (text[i] !== "=") {
      if (!attrName) return null; // not progressing — malformed
      continue; // valueless attribute; XML forbids it, but don't hang on it
    }
    i++;
    while (i < text.length && /\s/.test(text[i])) i++;
    const quote = text[i];
    if (quote !== '"' && quote !== "'") return null;
    i++;
    const from = i;
    while (i < text.length && text[i] !== quote) i++;
    if (i >= text.length) return null;
    attrs.set(attrName, { value: text.slice(from, i), from, to: i });
    i++;
  }
}

/** The URI a prefix is bound to, from the document's `xmlns:` declarations. */
export function namespaceOf(source: string, prefix: string): string | null {
  const pattern = prefix
    ? `xmlns:${prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\s*=\\s*["']([^"']*)["']`
    : `xmlns\\s*=\\s*["']([^"']*)["']`;
  return new RegExp(pattern).exec(source)?.[1] ?? null;
}

const escapeAttr = (s: string) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);

/**
 * Set (or clear) one attribute on the element starting at `tagStart`, returning
 * the whole document with just that edit applied. An empty value removes the
 * attribute, so a control can fall back to the module's declared default rather
 * than baking it into the source.
 */
export function setAttribute(
  source: string,
  tagStart: number,
  name: string,
  value: string,
): string | null {
  const tag = parseStartTag(source.slice(tagStart));
  if (!tag) return null;
  const slot = tag.attrs.get(name);
  if (slot) {
    if (value === "") {
      // remove the attribute and the single space in front of it
      let from = tagStart + slot.from - name.length - 2; // `name="`
      const to = tagStart + slot.to + 1; // past the closing quote
      while (from > tagStart && /\s/.test(source[from - 1])) from--;
      return source.slice(0, from) + source.slice(to);
    }
    return source.slice(0, tagStart + slot.from) + escapeAttr(value) + source.slice(tagStart + slot.to);
  }
  if (value === "") return source;
  const at = tagStart + tag.insertAt;
  const pad = /\s/.test(source[at - 1]) ? "" : " ";
  return `${source.slice(0, at)}${pad}${name}="${escapeAttr(value)}"${source.slice(at)}`;
}

// ---- controls -------------------------------------------------------------

/**
 * Build the editing controls for one element from its declared schema.
 *
 * A bounded number gets a slider that commits **as it moves**: an edit now patches
 * the drawing in place instead of rebuilding the panel (docs/Incremental.md §4), so
 * the control survives its own edit and the drawing tracks the drag. Typed values
 * still commit on `change` — a half-written `var(acc` is not worth compiling.
 *
 * What it offers is the **styling** half of an element: paint, stroke, typography,
 * layout options. Geometry is deliberately absent — position and extent want handles
 * on the canvas, not a number field that fights the document's own arrangement, and
 * the attribute table above already shows those values. So this reads like a style
 * block, and moving something stays the source pane's job.
 *
 * `prefixFor` resolves a namespace to the prefix *this document* binds, for
 * attributes like `x:border-width` that live in the xsvg namespace. Without it (or
 * when a document binds no prefix for that namespace) such attributes are skipped
 * rather than guessed at: writing an unbound prefix would not merely be wrong, it
 * would make the document malformed.
 */
export function renderParams(
  spec: ElementSpec,
  current: Map<string, AttrSlot>,
  onChange: (name: string, value: string) => void,
  prefixFor?: (ns: string) => string | null,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "xsvg-params";

  const params = spec.params ?? {};
  const names = Object.keys(params);
  const hint = (text: string) => {
    const el = document.createElement("span");
    el.className = "hint";
    el.textContent = text;
    wrap.append(el);
    return wrap;
  };
  if (names.length === 0) return hint("this element declares no attributes");

  let offered = 0;
  for (const name of names) {
    const p = params[name];
    // Geometry is shown in the attribute table above, not offered here.
    if (p.geometry) continue;
    // The name as this document must spell it: a namespaced attribute takes the
    // document's own prefix, and is skipped when it binds none.
    let attr = name;
    if (p.ns) {
      const prefix = prefixFor?.(p.ns);
      if (!prefix) continue;
      attr = `${prefix}:${name}`;
    }
    offered++;

    const row = document.createElement("label");
    row.className = "xsvg-param";

    const label = document.createElement("span");
    label.className = "xsvg-param-name";
    label.textContent = attr;
    row.append(label);

    // Two reasons a control is shown but not offered, both of which are worse to
    // leave live than to disable: a parameter the element's own attributes have made
    // inert (drag it, nothing moves, and the editor looks broken), and one whose
    // effects a host cannot carry out from an attribute edit alone. Say which.
    const by = (p.supersededBy ?? []).filter((n) => current.has(n));
    const why = p.readOnly
      ? p.readOnly
      : by.length
        ? `no effect while ${by.join(" and ")} ${by.length > 1 ? "are" : "is"} set`
        : null;

    const value = current.get(attr)?.value ?? "";
    const control = makeControl(p, value, (v) => onChange(attr, v));
    if (why) {
      row.classList.add("superseded");
      for (const el of control.querySelectorAll("input, select")) {
        (el as HTMLInputElement | HTMLSelectElement).disabled = true;
      }
      if (control instanceof HTMLInputElement || control instanceof HTMLSelectElement) {
        control.disabled = true;
      }
      row.title = p.doc ? `${why} — ${p.doc}` : why;
    } else if (p.doc) {
      row.title = p.doc;
    }

    row.append(control);
    wrap.append(row);
  }
  // Everything this element declares turns out to be geometry (or unbindable), so
  // say why the block is empty rather than showing a blank panel.
  if (offered === 0) {
    return hint("this element declares only geometry — edit it in the source");
  }
  return wrap;
}

/**
 * The prefix `source` binds to `ns`, or null when it binds none.
 *
 * Only a real `xmlns:p=` binding counts. An unprefixed attribute is in *no*
 * namespace in XML — the default `xmlns` governs elements, not attributes — so a
 * document that binds the xsvg namespace only as its default cannot spell
 * `border-width` in it at all.
 */
export function prefixOf(source: string, ns: string): string | null {
  const quoted = ns.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`xmlns:([\\w.-]+)\\s*=\\s*["']${quoted}["']`).exec(source)?.[1] ?? null;
}

/** Build the control for one parameter, already wired to `commit`. */
function makeControl(p: ParamSpec, value: string, commit: (v: string) => void): HTMLElement {
  // An unset parameter shows the module's default as a placeholder, so the panel
  // distinguishes "inherited the default" from "explicitly set to it".
  const fallback = p.default == null ? "" : String(p.default);

  if (p.type === "enum" && p.values?.length) {
    const sel = document.createElement("select");
    // a blank option means "unset" — the module's own default applies
    for (const v of ["", ...p.values]) {
      const opt = document.createElement("option");
      opt.value = v;
      opt.textContent = v === "" ? `default${fallback ? ` (${fallback})` : ""}` : v;
      sel.append(opt);
    }
    sel.value = p.values.includes(value) ? value : "";
    sel.addEventListener("change", () => commit(sel.value.trim()));
    return sel;
  }

  if (p.type === "number" && p.min != null && p.max != null) return makeSlider(p, value, fallback, commit);

  const input = document.createElement("input");
  if (p.type === "number") {
    input.type = "number";
    if (p.min != null) input.min = String(p.min);
    if (p.max != null) input.max = String(p.max);
    input.step = String(p.step ?? 1);
  } else {
    // `color` is deliberately a text field, not `<input type="color">`: a paint
    // value here is often `var(accent)`, `currentColor`, or `none`, none of which a
    // colour picker can hold — and silently rewriting a theme token to a literal hex
    // would quietly sever the document from its palette.
    input.type = "text";
  }
  input.value = value;
  input.placeholder = fallback;
  input.addEventListener("change", () => commit(input.value.trim()));
  return input;
}

/**
 * A declared `min`/`max` is what makes a parameter draggable: the module has said
 * what the useful span is, so the host can offer a slider over exactly that range
 * rather than guessing one. The paired number box keeps precise entry available —
 * and stays the only way to clear the value back to the default, since a slider has
 * no position meaning "unset".
 */
function makeSlider(
  p: ParamSpec,
  value: string,
  fallback: string,
  commit: (v: string) => void,
): HTMLElement {
  const wrap = document.createElement("span");
  wrap.className = "xsvg-slider";

  const range = document.createElement("input");
  range.type = "range";
  range.min = String(p.min);
  range.max = String(p.max);
  range.step = String(p.step ?? guessStep(p.min!, p.max!));

  const box = document.createElement("input");
  box.type = "number";
  box.min = range.min;
  box.max = range.max;
  box.step = range.step;
  box.value = value;
  box.placeholder = fallback;

  // With nothing set, the slider sits at whatever the module says it will draw.
  range.value = value || fallback || range.min;

  range.addEventListener("input", () => {
    box.value = range.value;
    commit(range.value);
  });
  box.addEventListener("change", () => {
    const v = box.value.trim();
    if (v) range.value = v;
    commit(v);
  });

  wrap.append(range, box);
  return wrap;
}

/** A sane step for a range the module bounded but did not quantize: fine enough to
 *  feel continuous, coarse enough not to write noise into the source. */
function guessStep(min: number, max: number): number {
  const span = Math.abs(max - min);
  if (span <= 2) return 0.01;
  if (span <= 20) return 0.1;
  return 1;
}
