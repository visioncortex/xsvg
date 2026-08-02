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
 * Values commit on `change`, not `input`: applying an edit recompiles the
 * document and rebuilds this panel, which would yank a control out from under a
 * drag in progress. Committing on release keeps every control usable. (Live
 * dragging needs the panel to survive a recompile — see docs/Extensibility.md §9.)
 */
export function renderParams(
  spec: ElementSpec,
  current: Map<string, AttrSlot>,
  onChange: (name: string, value: string) => void,
): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "xsvg-params";

  const params = spec.params ?? {};
  const names = Object.keys(params);
  if (names.length === 0) {
    const hint = document.createElement("span");
    hint.className = "hint";
    hint.textContent = "this element declares no parameters";
    wrap.append(hint);
    return wrap;
  }

  for (const name of names) {
    const p = params[name];
    const row = document.createElement("label");
    row.className = "xsvg-param";
    if (p.doc) row.title = p.doc;

    const label = document.createElement("span");
    label.className = "xsvg-param-name";
    label.textContent = name;
    row.append(label);

    const value = current.get(name)?.value ?? "";
    const control = makeControl(p, value);
    // An unset parameter shows the module's default as a placeholder, so the
    // panel distinguishes "inherited the default" from "explicitly set to it".
    control.addEventListener("change", () => onChange(name, control.value.trim()));
    row.append(control);
    wrap.append(row);
  }
  return wrap;
}

function makeControl(p: ParamSpec, value: string): HTMLInputElement | HTMLSelectElement {
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
    return sel;
  }

  const input = document.createElement("input");
  if (p.type === "color") {
    // Deliberately a text field, not `<input type="color">`: a paint value here is
    // often `var(accent)`, `currentColor`, or `none`, none of which a colour picker
    // can hold — and silently rewriting a theme token to a literal hex would
    // quietly sever the document from its palette.
    input.type = "text";
    input.value = value;
    input.placeholder = fallback;
    return input;
  }
  if (p.type === "number") {
    input.type = "number";
    if (p.min != null) input.min = String(p.min);
    if (p.max != null) input.max = String(p.max);
    input.step = String(p.step ?? 1);
  } else {
    input.type = "text";
  }
  input.value = value;
  input.placeholder = fallback;
  return input;
}
