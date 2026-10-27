// Serves site/ the way the app is opened in tests: /app/ is the web app, with the
// same Content-Security-Policy app.omacus.com sends. sw.js can change generation
// so a waiting update can be checked without publishing.

import { createServer } from "node:http";
import { connect as connectNet } from "node:net";
import { readFileSync, existsSync } from "node:fs";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(fileURLToPath(new URL("../../site/", import.meta.url)));
const headers = readFileSync(join(site, "app/_headers"), "utf8");
const csp = headers.match(/Content-Security-Policy:\s*(.+)/)[1].trim();
const genPath = process.env.OMACUS_SW_GEN || "/tmp/omacus-e2e-sw-gen";
const sourceVersion = readFileSync(join(site, "app/version.js"), "utf8").match(/APP_VERSION = "([^"]+)"/)[1];

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
const port = Number(process.env.OMACUS_PREVIEW_PORT || 4173);

const quieted = new WeakSet();

function quiet(socket) {
  if (!socket || quieted.has(socket)) return;
  quieted.add(socket);
  socket.on("error", () => {});
}

const server = createServer((req, res) => {
  quiet(req.socket);
  quiet(res);
  try {
    if (existsSync(downPath)) {
      // One dropped page must not take the listener down with it.
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
      "cache-control": "no-cache, no-store",
    };
    // The published host answers a missing file with the app page. The worker
    // has to refuse that for anything that is not the page itself.
    if (path.endsWith("/words.txt")) {
      const page = readFileSync(join(site, "app/index.html"));
      res.writeHead(200, { ...common, "content-type": "text/html; charset=utf-8" });
      res.end(page);
      return;
    }
    if (!file.startsWith(site) || !existsSync(file)) {
      res.writeHead(404, common);
      res.end("not found");
      return;
    }
    let body = readFileSync(file);
    if (path.endsWith("/sw.js") || path.endsWith("/version.js") || path.endsWith("/version.json")) {
      const stamped = `${sourceVersion}-e2e${generation()}`;
      const text = body.toString("utf8").replaceAll(sourceVersion, stamped);
      body = Buffer.from(text);
      common["cache-control"] = "no-cache, no-store";
    }
    res.writeHead(200, { ...common, "content-type": types[extname(file)] || "application/octet-stream" });
    res.end(body);
  } catch {
    quiet(res);
    if (!res.headersSent) res.writeHead(500);
    res.end();
  }
});

server.on("clientError", (_err, socket) => {
  quiet(socket);
  socket.destroy();
});

// The page's socket is for the mailbox. The test relay is on 8798.
server.on("upgrade", (req, socket, head) => {
  quiet(socket);
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (!url.pathname.startsWith("/v1/")) {
    socket.destroy();
    return;
  }
  const upstream = connectNet(8798, "127.0.0.1", () => {
    let raw = `GET ${req.url} HTTP/1.1\r\n`;
    for (const [name, value] of Object.entries(req.headers)) {
      if (value === undefined) continue;
      raw += `${name}: ${Array.isArray(value) ? value.join(", ") : value}\r\n`;
    }
    raw += "\r\n";
    upstream.write(raw);
    if (head && head.length) upstream.write(head);
    upstream.pipe(socket);
    socket.pipe(upstream);
  });
  const drop = () => {
    socket.destroy();
    upstream.destroy();
  };
  upstream.on("error", drop);
  socket.on("error", drop);
});

server.listen(port, "127.0.0.1");
