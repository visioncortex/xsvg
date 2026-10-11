#!/usr/bin/env node
// Measure page.html in headless Edge (Chromium). --dump-dom prints the page once the
// virtual-time budget is spent, which lets the font fetches and FontFace loads finish
// first; the page put its JSON in <body>. Same mechanism compare.mjs uses.
//
//   node scripts/font-measure/run-edge.mjs > edge.json
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const EDGE = "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge";
const page = resolve(dirname(fileURLToPath(import.meta.url)), "page.html");
const dom = execFileSync(
  EDGE,
  [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--virtual-time-budget=30000",
    "--dump-dom",
    `file://${page}`,
  ],
  { encoding: "utf8", maxBuffer: 64 << 20, stdio: ["ignore", "pipe", "ignore"] },
);
if (!/<title>done<\/title>/.test(dom)) throw new Error("the page had not finished measuring when the DOM was dumped");
const m = dom.match(/<body>([\s\S]*?)<\/body>/);
if (!m) throw new Error("no <body> in dumped DOM");
const text = m[1].replace(/&quot;/g, '"').replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").trim();
JSON.parse(text); // fail loudly here rather than in diff.mjs
process.stdout.write(text + "\n");
