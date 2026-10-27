// The real sync worker, in memory, for the browser tests.
// One mailbox object per name, so a test can move that object's clock.
// Invite expiry stays on the wall clock. Only the wait clock moves.

import { createHash } from "node:crypto";
import { createServer } from "node:http";
import worker, { Mailboxes, bindLive, clearPlacement } from "../../relay/worker/worker.js";
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

function writeFrame(socket, text) {
  const payload = Buffer.from(String(text));
  const len = payload.length;
  let header;
  if (len < 126) header = Buffer.from([0x81, len]);
  else if (len < 65536) {
    header = Buffer.alloc(4);
    header[0] = 0x81;
    header[1] = 126;
    header.writeUInt16BE(len, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81;
    header[1] = 127;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  socket.write(Buffer.concat([header, payload]));
}

server.on("upgrade", async (req, socket) => {
  const key = req.headers["sec-websocket-key"];
  if (!key) {
    socket.destroy();
    return;
  }
  const url = new URL(req.url, "http://127.0.0.1");
  const headers = new Headers();
  for (const [name, value] of Object.entries(req.headers)) {
    if (value === undefined) continue;
    headers.set(name, Array.isArray(value) ? value.join(", ") : value);
  }
  headers.set("upgrade", "websocket");
  if (!headers.has("cf-connecting-ip")) headers.set("cf-connecting-ip", "127.0.0.1");
  let response;
  try {
    response = await worker.fetch(new Request("https://relay.example" + url.pathname + url.search, {
      method: "GET",
      headers,
    }), env);
  } catch {
    socket.destroy();
    return;
  }
  if (response.status !== 101 && response.headers.get("x-omacus-live") !== "1") {
    const body = Buffer.from(await response.arrayBuffer());
    socket.end(`HTTP/1.1 ${response.status} ${response.statusText}\r\ncontent-type: application/json\r\ncontent-length: ${body.length}\r\n\r\n`);
    socket.write(body);
    return;
  }
  const accept = createHash("sha1")
    .update(String(key) + "258EAFA5-E914-47DA-95CA-C5AB0DC5AB11")
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " +
      accept +
      "\r\n\r\n",
  );
  const account = (url.pathname.match(/mailboxes\/([0-9a-f]{64})/) || [])[1];
  if (account) bindLive(account, (data) => {
    if (!socket.destroyed) writeFrame(socket, data);
  });
  socket.on("error", () => {});
});

server.listen(8798, "127.0.0.1", () => {
  console.log("ready");
});
