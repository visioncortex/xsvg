//! Single-file build entry — the zero-dependency `<script>` deliverable.
//!
//! Built by vite.embed.config.ts into one self-contained `dist/xsvg.js` with the WASM
//! inlined as base64. A host page (or an iframe in a docs site) then needs nothing but:
//!
//!   <script src="https://unpkg.com/@visioncortex/xsvg-viewer/dist/xsvg.js"></script>
//!   <xsvg-view src="diagram.xsvg"></xsvg-view>
//!
//! The WASM is initialized synchronously from the inlined bytes *before* the element is
//! registered, so the element's first render never touches the async fetch-a-sibling
//! `.wasm` path (which doesn't exist in a single-file bundle).
import { initFromBytes } from "./compiler";
import wasmBase64 from "virtual:xsvg-wasm";

function base64ToBytes(b64: string): Uint8Array {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

initFromBytes(base64ToBytes(wasmBase64));

// Register <xsvg-view> only after init. Dynamic import so its module body (which calls
// customElements.define) evaluates strictly after initFromBytes above.
void import("./element");
