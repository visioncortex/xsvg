//! The inspector feature of <xsvg-view-interactive>, in its own module so it is a
//! lazy `import()` chunk — CodeMirror (a heavy, optional peer dependency) loads only
//! when the `inspector` attribute is set, never in the base pan/zoom bundle.
//!
//! Composes the read-only CodeMirror source pane with the element↔source inspector and
//! wires both directions: click a rendered element → outline it + highlight its source;
//! click in the source → pin the element whose xsvg range is under the cursor.
import { createEditor } from "./editor";
import { createInspector } from "./inspector";
import type { PluginSchemas } from "./params";

export interface MountedInspector {
  clear(): void;
  /** Ask CodeMirror to re-measure — call after the source pane becomes visible. */
  remeasure(): void;
  /** Re-pin the element at a UTF-16 source offset (used to restore the selection
   *  after a parameter edit recompiles the document). */
  selectAtSourceOffset(offset: number): void;
  destroy(): void;
}

export function mountInspector(opts: {
  svgRoot: SVGSVGElement;
  panel: HTMLElement;
  sourcePane: HTMLElement;
  source: string;
  schemas?: PluginSchemas | null;
  onEdit?: (source: string, pinAt: number) => void;
}): MountedInspector {
  const editor = createEditor({ parent: opts.sourcePane, doc: opts.source, readOnly: true });
  const inspector = createInspector({
    svgRoot: opts.svgRoot,
    panel: opts.panel,
    source: opts.source,
    editor,
    schemas: opts.schemas,
    onEdit: opts.onEdit,
  });

  // Source → canvas: a click in the read-only source pins the element whose range is
  // under the pointer.
  const onSrcDown = (e: MouseEvent) => {
    const pos = editor.view.posAtCoords({ x: e.clientX, y: e.clientY });
    if (pos != null) inspector.selectAtSourceOffset(pos);
  };
  editor.view.dom.addEventListener("mousedown", onSrcDown);

  return {
    clear: () => inspector.clear(),
    remeasure: () => editor.view.requestMeasure(),
    selectAtSourceOffset: (offset: number) => inspector.selectAtSourceOffset(offset),
    destroy: () => {
      editor.view.dom.removeEventListener("mousedown", onSrcDown);
      inspector.destroy();
      editor.destroy();
    },
  };
}
