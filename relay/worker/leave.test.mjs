// Turning sync off stores a list that drops this phone. A later request from
// that phone is refused. The phone that stayed can still read.
import { Mailboxes } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { appendDevice, authHeaders, deviceChain, deviceListed, dropDevice, makeDevice, makeRecord } from "./auth.js";

const account = "ab".repeat(32);

function assert(cond, message) {
  if (!cond) throw new Error(message || "assertion failed");
}

function b64(bytes) {
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text);
}

function deviceBytes(device) {
  const out = new Uint8Array(80);
  out.set(device.id, 0);
  out.set(device.pub, 16);
  out.set(device.receive, 48);
  return out;
}

async function call(box, method, path, body, headers = {}) {
  const text = body === undefined ? undefined : (typeof body === "string" ? body : JSON.stringify(body));
  const response = await box.fetch(new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: { "content-type": "application/json", "cf-connecting-ip": "198.51.100.8", ...headers },
    body: text,
  }));
  const raw = await response.text();
  let json = {};
  if (raw) {
    try { json = JSON.parse(raw); } catch { json = { raw }; }
  }
  return { status: response.status, json };
}

async function authed(box, device, method, path, body) {
  const text = body === undefined ? "" : JSON.stringify(body);
  const headers = await authHeaders(device, method, path, text);
  if (body && Number.isInteger(body.revision)) headers["if-match"] = `"${body.revision}"`;
  return call(box, method, path, body === undefined ? undefined : text, headers);
}

const harbor = await makeDevice(account);
const ferry = await makeDevice(account);
let chain = await deviceChain(harbor);
chain = await appendDevice(harbor, chain, ferry);
assert(chain, "the second phone was not added");
const stored = await makeRecord(harbor, "ferry = 40", 2, chain);
const leftChain = await dropDevice(ferry, chain, ferry.id);
assert(leftChain, "this phone could not leave the list");
const left = await makeRecord(ferry, "ferry = 40", 2, leftChain);
assert(await deviceListed(stored, b64(deviceBytes(ferry))), "the phone was missing before it left");
assert(!(await deviceListed(left, b64(deviceBytes(ferry)))), "the phone is still on the list it signed");
assert(await deviceListed(left, b64(deviceBytes(harbor))), "the other phone dropped off the list");

const box = new Mailboxes({ storage: new MemoryStorage() }, {});
const created = await call(box, "POST", "/v1/mailboxes", { account, record: stored }, {
  "x-omacus-access-hash": "ab".repeat(32),
  "x-omacus-access-uses": "2",
  "x-omacus-access-spend": "1",
});
assert(created.status === 201, `create ${created.status} ${JSON.stringify(created.json)}`);

const saved = await authed(box, ferry, "PUT", `/v1/mailboxes/${account}`, {
  revision: created.json.revision,
  record: left,
});
assert(saved.status === 200, `leave ${saved.status} ${JSON.stringify(saved.json)}`);

const still = await authed(box, harbor, "GET", `/v1/mailboxes/${account}`);
assert(still.status === 200, `the other phone could not read ${still.status}`);
assert(!(await deviceListed(still.json.record, b64(deviceBytes(ferry)))), "the stored list still has the phone that left");

const again = await authed(box, ferry, "PUT", `/v1/mailboxes/${account}`, {
  revision: saved.json.revision,
  record: left,
});
assert(again.status === 404, `a phone that left could still write ${again.status}`);
console.log("leave ok");
