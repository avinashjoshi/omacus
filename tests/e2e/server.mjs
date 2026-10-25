// Serves site/ the way the app is opened in tests: /app/ is the web app, with the
// same Content-Security-Policy app.omacus.com sends. sw.js can change generation
// so a waiting update can be checked without publishing.

import { createServer } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(fileURLToPath(new URL("../../site/", import.meta.url)));
const headers = readFileSync(join(site, "app/_headers"), "utf8");
const csp = headers.match(/Content-Security-Policy:\s*(.+)/)[1].trim();
const genPath = process.env.OMACUS_SW_GEN || "/tmp/omacus-e2e-sw-gen";

const types = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".webp": "image/webp",
  ".webmanifest": "application/manifest+json",
};

function generation() {
  try {
    return readFileSync(genPath, "utf8").trim() || "1";
  } catch {
    return "1";
  }
}

const downPath = process.env.OMACUS_SW_DOWN || "/tmp/omacus-e2e-down";

const server = createServer((req, res) => {
  if (existsSync(downPath)) {
    req.socket.destroy();
    return;
  }
  const url = new URL(req.url, "http://127.0.0.1");
  let path = decodeURIComponent(url.pathname);
  if (path.endsWith("/")) path += "index.html";
  const file = normalize(join(site, path));
  const common = {
    "content-security-policy": csp,
    "x-content-type-options": "nosniff",
    "referrer-policy": "strict-origin-when-cross-origin",
  };
  if (!file.startsWith(site) || !existsSync(file)) {
    res.writeHead(404, common);
    res.end("not found");
    return;
  }
  let body = readFileSync(file);
  if (path.endsWith("/sw.js")) {
    const text = body.toString("utf8").replaceAll("omacus-app-14", `omacus-app-e2e-${generation()}`);
    body = Buffer.from(text);
    common["cache-control"] = "no-cache, no-store";
  }
  res.writeHead(200, { ...common, "content-type": types[extname(file)] || "application/octet-stream" });
  res.end(body);
});

server.listen(4173, "127.0.0.1");
