// Dev harness for <xsvg-view>: registers the element via the normal async init
// (fetching the .wasm asset through Vite). Used by /embed-demo/ in `npm run dev`.
// Dogfoods the published element straight from the package (aliased to source in the
// monorepo). The shippable single-file bundle is src/embed/index.ts, built by
// web/vite.embed.config.ts into dist/xsvg.js.
import "@visioncortex/xsvg-viewer/element";
