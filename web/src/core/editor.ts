// Shared CodeMirror 6 factory: an XML-highlighted editor used by the playground
// (editable) and the interactive viewer's source pane (read-only). Also exposes a
// range-highlight facility for the viewer's element→source projection.
import { EditorState, StateEffect, StateField, type Extension } from "@codemirror/state";
import { EditorView, Decoration, type DecorationSet } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { xml } from "@codemirror/lang-xml";

// A single mark decoration highlighting the element that maps to the pinned source
// range. `.xsvg-src-hl` is styled by the host app's CSS.
const rangeMark = Decoration.mark({ class: "xsvg-src-hl" });
const setHighlight = StateEffect.define<{ from: number; to: number } | null>();

const highlightField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (e.is(setHighlight)) {
        deco =
          e.value && e.value.to > e.value.from
            ? Decoration.set([rangeMark.range(e.value.from, e.value.to)])
            : Decoration.none;
      }
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export interface EditorHandle {
  view: EditorView;
  getDoc(): string;
  setDoc(text: string): void;
  /** Highlight [from, to) (UTF-16 indices) and scroll it into view; clears if empty. */
  highlight(from: number, to: number): void;
  destroy(): void;
}

export interface EditorOptions {
  parent: HTMLElement;
  doc?: string;
  readOnly?: boolean;
  onChange?: (doc: string) => void;
}

export function createEditor(opts: EditorOptions): EditorHandle {
  const { parent, doc = "", readOnly = false, onChange } = opts;

  const extensions: Extension[] = [
    basicSetup,
    xml(),
    highlightField,
    EditorView.theme({
      "&": { height: "100%", fontSize: "13px" },
      ".cm-scroller": { overflow: "auto", fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace" },
    }),
  ];
  if (readOnly) extensions.push(EditorState.readOnly.of(true), EditorView.editable.of(false));
  if (onChange) {
    extensions.push(
      EditorView.updateListener.of((u) => {
        if (u.docChanged) onChange(u.state.doc.toString());
      }),
    );
  }

  const view = new EditorView({ doc, parent, extensions });

  return {
    view,
    getDoc: () => view.state.doc.toString(),
    setDoc: (text) =>
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text } }),
    highlight: (from, to) => {
      const len = view.state.doc.length;
      const a = Math.max(0, Math.min(from, len));
      const b = Math.max(0, Math.min(to, len));
      const value = b > a ? { from: a, to: b } : null;
      view.dispatch({
        effects: value ? [setHighlight.of(value), EditorView.scrollIntoView(a, { y: "center" })] : [setHighlight.of(null)],
      });
    },
    destroy: () => view.destroy(),
  };
}
