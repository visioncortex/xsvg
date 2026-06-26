// v0 dev playground: edit xsvg on the left, see the compiled SVG render on the
// right, plus the emitted SVG source. Also instantiates an <xsvg-view> to
// exercise the embeddable-component path.
import "./xsvg-view"; // registers <xsvg-view>
import { compileXsvg } from "./xsvg";

const DEMO = `<svg xmlns="http://www.w3.org/2000/svg"
     xmlns:x="https://xsvg.dev/ns"
     viewBox="0 0 220 140">
  <!-- this <rect> is lowered to a <path> by the compiler -->
  <rect x="10" y="10" width="200" height="120" fill="#eef2ff" stroke="#8899cc"/>
  <text x="110" y="74" text-anchor="middle" font-family="sans-serif" font-size="14">
    edit me — hello xsvg
  </text>
</svg>`;

const app = document.getElementById("app")!;
app.innerHTML = `
  <header>
    <h1>xsvg <span class="tag">v0</span></h1>
    <p>Pure-Rust core → WASM, compiled in your browser. Edit the source; output updates live.</p>
  </header>
  <main>
    <section class="pane">
      <h2>source (xsvg)</h2>
      <textarea id="src" spellcheck="false"></textarea>
    </section>
    <section class="pane">
      <h2>rendered (browser draws the compiled SVG)</h2>
      <div id="preview" class="preview"></div>
    </section>
    <section class="pane">
      <h2>output (compiled SVG)</h2>
      <pre id="out"></pre>
    </section>
  </main>
  <footer>
    <h2>&lt;xsvg-view&gt; component</h2>
    <xsvg-view>
      <script type="application/xsvg+xml"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 60"><rect x="5" y="5" width="110" height="50" fill="#fff7ed" stroke="#fb923c"/><text x="60" y="35" text-anchor="middle" font-family="sans-serif" font-size="12">via component</text></svg></script>
    </xsvg-view>
  </footer>`;

const srcEl = document.getElementById("src") as HTMLTextAreaElement;
const preview = document.getElementById("preview")!;
const out = document.getElementById("out")!;
srcEl.value = DEMO;

async function run() {
  try {
    const svg = await compileXsvg(srcEl.value, "balanced");
    preview.innerHTML = svg;
    out.textContent = svg;
  } catch (err) {
    preview.innerHTML = "";
    out.textContent = String(err);
  }
}

let timer: number | undefined;
srcEl.addEventListener("input", () => {
  window.clearTimeout(timer);
  timer = window.setTimeout(run, 150);
});

void run();
