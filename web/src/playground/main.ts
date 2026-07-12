// Playground bootstrap — CodeMirror editor on the left, live compiled preview on
// the right. Edits recompile (debounced); a sample picker seeds the editor; the
// document round-trips through the URL hash for shareable links.
//
//   /playground/?file=<name>       preload a bundled sample
//   /playground/#src=<base64>      preload a shared document
import "../base.css";
import "./playground.css";
import { CATALOG, SAMPLES, DEFAULT_SAMPLE, requestedSample } from "../core/samples";
import { createEditor } from "../core/editor";
import { createPreview } from "../core/preview";

function byId<T extends HTMLElement = HTMLElement>(id: string): T {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el as T;
}

// The live preview is the same component the /preview page uses, so multi-artboard
// documents get the slide deck (rail + nav) here for free. It keeps the last good
// preview on a compile error; we surface the error in our own #error box.
const preview = createPreview(byId("preview"));
const errorBox = byId("error");
const sampleSelect = byId<HTMLSelectElement>("sample");
const viewerLink = byId<HTMLAnchorElement>("open-viewer");

// ---- URL-hash sharing (utf-8 safe base64) ----------------------------------
function toBase64(s: string): string {
  const bytes = new TextEncoder().encode(s);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}
function fromBase64(b64: string): string {
  const bin = atob(b64);
  return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
}
function sharedDoc(): string | null {
  const m = location.hash.match(/^#src=(.*)$/);
  if (!m) return null;
  try {
    return fromBase64(decodeURIComponent(m[1]));
  } catch {
    return null;
  }
}

// ---- live compile ----------------------------------------------------------
async function render(source: string): Promise<void> {
  try {
    const result = await preview.render(source);
    if (result === "superseded") return; // a newer edit owns the final state
    errorBox.hidden = true;
  } catch (err) {
    errorBox.hidden = false;
    errorBox.textContent = String(err); // keep the last good preview visible
  }
}

let timer: number | undefined;
function scheduleRender(source: string): void {
  clearTimeout(timer);
  timer = window.setTimeout(() => void render(source), 200);
}

const editor = createEditor({
  parent: byId("editor"),
  onChange: (doc) => {
    scheduleRender(doc);
    sampleSelect.value = ""; // edits mean we're no longer on a named sample
  },
});

// ---- sample picker ---------------------------------------------------------
function buildPicker(): void {
  const placeholder = new Option("— pick a sample —", "");
  placeholder.disabled = true;
  sampleSelect.add(placeholder);
  for (const cat of CATALOG) {
    const group = document.createElement("optgroup");
    group.label = cat.name;
    for (const s of cat.samples) {
      if (SAMPLES[s.file]) group.appendChild(new Option(s.title, s.file));
    }
    if (group.childElementCount) sampleSelect.appendChild(group);
  }
}

function loadSample(file: string): void {
  if (!SAMPLES[file]) return;
  editor.setDoc(SAMPLES[file]);
  sampleSelect.value = file;
  viewerLink.href = `/viewer/?file=${encodeURIComponent(file)}`;
  void render(SAMPLES[file]);
}

sampleSelect.addEventListener("change", () => {
  if (sampleSelect.value) loadSample(sampleSelect.value);
});

// ---- share button ----------------------------------------------------------
byId("copy-link").addEventListener("click", async () => {
  const hash = `#src=${encodeURIComponent(toBase64(editor.getDoc()))}`;
  history.replaceState(null, "", location.pathname + location.search + hash);
  const btn = byId<HTMLButtonElement>("copy-link");
  try {
    await navigator.clipboard.writeText(location.href);
    flash(btn, "Copied!");
  } catch {
    flash(btn, "Link in URL");
  }
});

function flash(btn: HTMLButtonElement, text: string): void {
  const original = btn.textContent;
  btn.textContent = text;
  setTimeout(() => (btn.textContent = original), 1200);
}

// ---- boot ------------------------------------------------------------------
buildPicker();

const shared = sharedDoc();
const requested = requestedSample();
if (shared !== null) {
  editor.setDoc(shared);
  void render(shared);
} else if (requested) {
  loadSample(requested);
} else {
  loadSample(DEFAULT_SAMPLE);
}
