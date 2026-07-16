// Compile an xsvg document to clean plain SVG and save it as a file. Shared by
// the viewer and playground "Download SVG" buttons so they stay identical.
import { compileXsvg } from "./compiler";

/** Compile `source` to plain SVG with no source map (so the file carries no
 *  data-xsvg-* noise) and save it as `<basename>.svg` via a transient blob URL.
 *  Silently no-ops if the document doesn't compile — the caller's live preview
 *  already surfaces the error. */
export async function downloadSvg(source: string, basename: string): Promise<void> {
  let svg: string;
  try {
    svg = await compileXsvg(source);
  } catch {
    return;
  }
  const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = basename.replace(/\.[^./]+$/, "") + ".svg";
  a.click();
  URL.revokeObjectURL(url);
}
