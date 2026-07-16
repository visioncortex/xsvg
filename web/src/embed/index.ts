// Single-file embed entry — the "vanilla viewer" deliverable.
//
// Built by web/vite.embed.config.ts into one self-contained `dist/xsvg.js` with the
// WASM inlined as base64. A host page then needs nothing but:
//
//   <script src="xsvg.js"></script>
//   <xsvg-view src="diagram.xsvg"></xsvg-view>
//
// We initialize the WASM synchronously from the inlined bytes *before* registering
// the custom element, so the element's first render never touches the async
// fetch-a-sibling-.wasm path (which does not exist in a single-file bundle).
import { initFromBytes } from "@visioncortex/xsvg-viewer";
import wasmBase64 from "virtual:xsvg-wasm";

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

initFromBytes(base64ToBytes(wasmBase64));

// Register <xsvg-view> only after init. Dynamic import so its module body (which
// calls customElements.define) evaluates strictly after initFromBytes above.
void import("./xsvg-view");
