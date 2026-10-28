// A write tells every open device the new revision, and nothing else.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { performance } from "node:perf_hooks";
import { Mailboxes } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { authHeaders, makeDevice, makeRecord } from "./auth.js";

const account = "ab".repeat(32);
const object = "0123456789abcdef0123456789abcdef";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

const sockets = [];
const store = new MemoryStorage();
const box = new Mailboxes({
  storage: store,
  acceptWebSocket(socket) { sockets.push(socket); },
  getWebSockets() { return sockets; },
}, {});

const device = await makeDevice(account);
const record = await makeRecord(device, "oats");
const access = {
  "x-omacus-access-hash": "cd".repeat(32),
  "x-omacus-access-uses": "1",
  "x-omacus-access-spend": "1",
};

async function call(method, path, body, headers = {}) {
  const text = body === undefined ? undefined : (typeof body === "string" ? body : JSON.stringify(body));
  const response = await box.fetch(new Request("https://relay.example" + path, {
    method,
    headers: { "content-type": "application/json", upgrade: headers.upgrade || "", ...headers },
    body: text,
  }));
  const raw = await response.text();
  let json = {};
  if (raw) {
    try { json = JSON.parse(raw); } catch { json = { raw }; }
  }
  return { status: response.status, json, raw };
}

const created = await call("POST", "/v1/mailboxes", {
  account,
  record,
  objects: [{ id: object, base: 0, body: btoa(String.fromCharCode(...new Uint8Array(64).fill(7))) }],
}, access);
assert(created.status === 201, `create ${created.status} ${created.raw}`);

const path = `/v1/mailboxes/${account}/live`;
const headers = await authHeaders(device, "GET", path, "");
headers.upgrade = "websocket";
const opened = await call("GET", path, undefined, headers);
assert(opened.status === 101 || opened.status === 200, `live ${opened.status} ${opened.raw}`);
assert(opened.raw === '{"rev":1}', `hello ${opened.raw}`);
assert(!opened.raw.includes(record), "the hello carried the sealed library");
assert(sockets.length === 1, "the socket was not kept");

const next = await makeRecord(device, "ferry");
const body = { revision: 1, record: next };
const putHeaders = await authHeaders(device, "PUT", `/v1/mailboxes/${account}`, JSON.stringify(body));
putHeaders["if-match"] = '"1"';
const started = performance.now();
const saved = await call("PUT", `/v1/mailboxes/${account}`, body, putHeaders);
const elapsed = performance.now() - started;
assert(saved.status === 200 && saved.json.revision === 2, `put ${saved.status} ${saved.raw}`);
const pullPath = `/v1/mailboxes/${account}`;
const pullHeaders = await authHeaders(device, "GET", pullPath, "");
const pulled = await call("GET", pullPath, undefined, pullHeaders);
assert(pulled.status === 200 && pulled.json.revision === 2 && pulled.json.record === next, "the saved copy did not come back");
const notes = sockets[0].sent;
assert(notes.includes('{"rev":2}'), `notice ${notes.join(" | ")}`);
assert(notes.every((note) => note === '{"rev":1}' || note === '{"rev":2}'), `extra ${notes.join(" | ")}`);
assert(notes.every((note) => !note.includes(record) && !note.includes(next)), "a notice carried ciphertext");
assert(elapsed < 50, `the notice took ${elapsed.toFixed(1)}ms`);
console.log(`live notify ${elapsed.toFixed(1)}ms`);

const query = new URLSearchParams({
  device: headers["x-omacus-device"],
  time: headers["x-omacus-time"],
  nonce: "00".repeat(16),
  signature: headers["x-omacus-signature"],
});
const refused = await call("GET", `${path}?${query}`, undefined, { upgrade: "websocket" });
assert(refused.status === 404, `a reused nonce opened a socket ${refused.status}`);

// RFC 6455 section 1.3. A swapped guid makes every real socket fail the handshake.
const guid = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const relayBox = readFileSync(new URL("../../tests/e2e/relay-box.mjs", import.meta.url), "utf8");
assert(relayBox.includes(guid), "the test relay uses the websocket accept guid");
assert(!relayBox.includes(guid.replace("DC85B", "DC5AB")), "the old accept guid is gone");
assert(relayBox.includes("socket.end(Buffer.concat([head, body]))"), "a refused upgrade is one write");
assert(!relayBox.includes("socket.write(body)"), "a refused upgrade does not write after the socket ended");
const accept = createHash("sha1").update(`dGhlIHNhbXBsZSBub25jZQ==${guid}`).digest("base64");
assert(accept === "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=", `accept ${accept}`);

console.log("live ok");
