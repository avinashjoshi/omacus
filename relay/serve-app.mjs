// Serves the built private-test app and forwards /v1 to the relay.
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const port = Number(process.argv[2] || 8800);
const relay = process.argv[3] || "http://127.0.0.1:8791";
const root = path.resolve(process.argv[4] || new URL("./worker/public", import.meta.url).pathname);

const types = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".webmanifest": "application/manifest+json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".woff2": "font/woff2",
  ".txt": "text/plain; charset=utf-8",
};

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${port}`);
  if (url.pathname === "/v1" || url.pathname.startsWith("/v1/")) {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const body = Buffer.concat(chunks);
    const headers = {};
    if (req.headers["content-type"]) headers["content-type"] = req.headers["content-type"];
    const method = req.method || "GET";
    const upstream = await fetch(relay + url.pathname + url.search, {
      method,
      headers,
      body: method === "GET" || method === "HEAD" ? undefined : body,
    });
    const buf = Buffer.from(await upstream.arrayBuffer());
    const out = {
      "content-type": upstream.headers.get("content-type") || "application/json",
      "cache-control": "no-store",
      "access-control-allow-origin": "*",
    };
    res.writeHead(upstream.status, out);
    res.end(buf);
    return;
  }
  const decoded = decodeURIComponent(url.pathname);
  let file = path.join(root, decoded);
  if (decoded.endsWith("/")) file = path.join(file, "index.html");
  if (!file.startsWith(root)) {
    res.writeHead(403);
    res.end();
    return;
  }
  if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    if (!path.extname(decoded)) file = path.join(root, "index.html");
  }
  if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) {
    res.writeHead(404);
    res.end("missing");
    return;
  }
  res.writeHead(200, {
    "content-type": types[path.extname(file)] || "application/octet-stream",
    "cache-control": "no-store",
  });
  fs.createReadStream(file).pipe(res);
});

server.listen(port, "127.0.0.1", () => {
  console.log(`app http://127.0.0.1:${port}  relay ${relay}`);
});
