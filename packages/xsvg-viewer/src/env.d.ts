/// <reference types="vite/client" />

// opentype.js ships no bundled types; the compiler only narrows `parse` to a local
// shape, so an untyped ambient module suffices.
declare module "opentype.js";
