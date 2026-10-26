// The real sync worker, in memory, for the browser tests.
// One mailbox object per name, so a test can move that object's clock.
// Invite expiry stays on the wall clock. Only the wait clock moves.

import { createServer } from "node:http";
import worker, { Mailboxes, clearPlacement } from "../../relay/worker/worker.js";
import { MemoryStorage } from "../../relay/worker/storage.js";
import { HEARTBEAT_MS } from "../../relay/worker/heartbeat.js";

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";

clearPlacement();
let offset = 0;
const boxes = new Map();
const env = {
  ACCESS_SECRET: SECRET,
  ADMIN_KEY: ADMIN,
  ALLOW_NO_ACCESS: "1",
  SERVE_APP: "0",
  MAILBOXES: {
    idFromName: (name) => String(name),
    get: (name) => {
      const key = String(name);
      let box = boxes.get(key);
      if (!box) {
        box = new Mailboxes({ storage: new MemoryStorage() }, env);
        box.clock = () => Date.now() + offset;
        boxes.set(key, box);
      }
      return { fetch: (request) => box.fetch(request) };
    },
  },
};

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => resolve(Buffer.concat(chunks)));
    req.on("error", reject);
  });
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  const body = await readBody(req);
  if (req.method === "POST" && url.pathname === "/__clock") {
    offset += HEARTBEAT_MS + 1000;
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ offset }));
    return;
  }
  const headers = new Headers();
  for (const [key, value] of Object.entries(req.headers)) {
    if (value === undefined) continue;
    headers.set(key, Array.isArray(value) ? value.join(", ") : value);
  }
  if (!headers.has("cf-connecting-ip")) headers.set("cf-connecting-ip", "127.0.0.1");
  const request = new Request("https://relay.example" + url.pathname + url.search, {
    method: req.method,
    headers,
    body: req.method === "GET" || req.method === "HEAD" ? undefined : body,
  });
  try {
    const response = await worker.fetch(request, env);
    const bytes = Buffer.from(await response.arrayBuffer());
    const out = {};
    response.headers.forEach((value, key) => {
      out[key] = value;
    });
    res.writeHead(response.status, out);
    res.end(bytes);
  } catch (error) {
    res.writeHead(500, { "content-type": "text/plain; charset=utf-8" });
    res.end(String(error && error.stack ? error.stack : error));
  }
});

server.listen(8798, "127.0.0.1", () => {
  console.log("ready");
});
