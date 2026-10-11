#!/usr/bin/env node
// Measure page.html in Safari through safaridriver (WebDriver over HTTP, no npm
// dependency). Safari has no headless mode: a Safari window opens for the few seconds
// the session lives, unattended, and closes when the session is deleted. Needs
// `safaridriver --enable` once (Safari ▸ Develop ▸ Allow Remote Automation).
//
// Safari under automation refuses file:// pages ("Failed to open page"), so the page
// is served from a throwaway local HTTP server for the length of the session.
//
//   node scripts/font-measure/run-safari.mjs > safari.json
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const PORT = 4444 + Math.floor(Math.random() * 1000);
const base = `http://127.0.0.1:${PORT}`;
const page = resolve(dirname(fileURLToPath(import.meta.url)), "page.html");

const html = readFileSync(page);
const server = createServer((_req, res) => {
  res.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  res.end(html);
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const pageUrl = `http://127.0.0.1:${server.address().port}/page.html`;

const driver = spawn("safaridriver", ["-p", String(PORT)], { stdio: ["ignore", "ignore", "inherit"] });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const call = async (method, path, body) => {
  const res = await fetch(base + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json();
  if (!res.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json)}`);
  return json.value;
};

try {
  for (let i = 0; i < 50; i++) {
    try { await fetch(base + "/status"); break; } catch { await sleep(100); }
  }
  const session = await call("POST", "/session", { capabilities: { alwaysMatch: { browserName: "safari" } } });
  const sid = session.sessionId;
  try {
    await call("POST", `/session/${sid}/url`, { url: pageUrl });
    let text = "";
    for (let i = 0; i < 300; i++) {
      const title = await call("POST", `/session/${sid}/execute/sync`, { script: "return document.title;", args: [] });
      if (title === "done") {
        text = await call("POST", `/session/${sid}/execute/sync`, { script: "return document.body.textContent;", args: [] });
        break;
      }
      await sleep(100);
    }
    if (!text) throw new Error("the page did not finish measuring within 30s");
    JSON.parse(text);
    process.stdout.write(text.trim() + "\n");
  } finally {
    await call("DELETE", `/session/${sid}`);
  }
} finally {
  driver.kill();
  server.close();
}
