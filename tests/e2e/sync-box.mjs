// A mailbox for the two-phone test. It checks the same signatures the relay does.
import { createHash } from "node:crypto";
import { appendFileSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { allowPut, authorize } from "../../relay/worker/auth.js";

const box = { revision: 0, record: "", objects: new Map(), nonces: {} };
const liveSockets = new Set();

function reset() {
  box.revision = 0;
  box.record = "";
  box.objects = new Map();
  box.nonces = {};
}

function writeFrame(socket, text) {
  if (socket.destroyed || !socket.writable) return;
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

function tellLive() {
  const note = JSON.stringify({ rev: box.revision });
  for (const socket of liveSockets) writeFrame(socket, note);
}

function send(res, status, value) {
  const revision = value && typeof value === "object" ? value.revision : "";
  appendFileSync("/tmp/sync-box.log", `  -> ${status} ${revision || ""}\n`);
  const body = status === 304 ? "" : JSON.stringify(value);
  res.writeHead(status, { "content-type": "application/json" });
  res.end(body);
}

function headersOf(req) {
  return new Headers(Object.entries(req.headers).map(([key, value]) => [key, String(value)]));
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => resolve(Buffer.concat(chunks).toString("utf8")));
    req.on("error", reject);
  });
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url || "/", "http://127.0.0.1");
  const path = url.pathname;
  const method = req.method || "GET";
  appendFileSync("/tmp/sync-box.log", `${method} ${path}\n`);
  const raw = method === "GET" || method === "HEAD" ? "" : await readBody(req);
  const headers = headersOf(req);
  if (method === "POST" && path === "/reset") {
    reset();
    send(res, 200, { ok: true });
    return;
  }
  if (method === "POST" && path === "/v1/access") {
    send(res, 200, { ok: true });
    return;
  }
  if (method === "POST" && path === "/v1/mailboxes") {
    const body = JSON.parse(raw || "{}");
    box.revision = 1;
    box.record = body.record;
    box.nonces = {};
    for (const object of body.objects || []) box.objects.set(object.id, { revision: 1, body: object.body });
    const objects = [...box.objects].map(([id, row]) => ({ id, revision: row.revision }));
    send(res, 200, { revision: 1, objects });
    tellLive();
    return;
  }
  const account = path.split("/")[3] || "";
  const mailboxPath = `/v1/mailboxes/${account}`;
  if (!box.record || !account) {
    send(res, 404, { message: "No pages for that yet." });
    return;
  }
  if (method === "GET" && path === mailboxPath) {
    const auth = await authorize("GET", path, "", headers, box.record, Date.now(), box);
    if (!auth) {
      send(res, 404, { message: "No pages for that yet." });
      return;
    }
    if (headers.get("if-none-match") === `"${box.revision}"`) {
      send(res, 304, {});
      return;
    }
    const objects = [...box.objects].map(([id, row]) => ({ id, revision: row.revision, bytes: 1 }));
    send(res, 200, { revision: box.revision, record: box.record, objects });
    return;
  }
  if (method === "PUT" && path === mailboxPath) {
    const auth = await authorize("PUT", path, raw, headers, box.record, Date.now(), box);
    if (!auth) {
      send(res, 404, { message: "No pages for that yet." });
      return;
    }
    const body = JSON.parse(raw || "{}");
    if (!(await allowPut(box.record, body.record, auth.recovery))) {
      send(res, 400, { message: "That copy could not be read." });
      return;
    }
    if (headers.get("if-match") !== `"${box.revision}"` || body.revision !== box.revision) {
      send(res, 409, { message: "That copy is out of date. Try again." });
      return;
    }
    box.revision += 1;
    box.record = body.record;
    const written = [];
    for (const object of body.objects || []) {
      const prev = box.objects.get(object.id);
      const revision = (prev?.revision || 0) + 1;
      box.objects.set(object.id, { revision, body: object.body });
      written.push({ id: object.id, revision });
    }
    for (const id of body.drop || []) box.objects.delete(id);
    send(res, 200, { revision: box.revision, objects: written });
    tellLive();
    return;
  }
  if (method === "POST" && path === `${mailboxPath}/objects`) {
    const auth = await authorize("POST", path, raw, headers, box.record, Date.now(), box);
    if (!auth) {
      send(res, 404, { message: "No pages for that yet." });
      return;
    }
    const body = JSON.parse(raw || "{}");
    const objects = (body.ids || []).flatMap((id) => {
      const row = box.objects.get(id);
      return row ? [{ id, revision: row.revision, body: row.body }] : [];
    });
    send(res, 200, { objects });
    return;
  }
  send(res, 404, { message: "No pages for that yet." });
});

// The status says Live only while this socket stays open. A new revision
// is the same short note the relay sends, so the other phone pulls at once.
server.on("upgrade", (req, socket) => {
  socket.on("error", () => {});
  const key = req.headers["sec-websocket-key"];
  const url = new URL(req.url || "/", "http://127.0.0.1");
  if (!key || !url.pathname.endsWith("/live")) {
    socket.destroy();
    return;
  }
  const accept = createHash("sha1")
    .update(String(key) + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11")
    .digest("base64");
  socket.write(
    "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: " +
      accept +
      "\r\n\r\n",
  );
  liveSockets.add(socket);
  socket.on("close", () => liveSockets.delete(socket));
  socket.on("data", () => {});
});

writeFileSync("/tmp/sync-box.log", "");
server.listen(8799, "127.0.0.1", () => {
  process.stdout.write("ready\n");
});
