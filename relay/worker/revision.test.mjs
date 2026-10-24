// A quiet poll names the revision it has. The record comes back only when that revision changed.
import { Mailboxes } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { authHeaders, makeDevice, makeRecord } from "./auth.js";

const account = "ab".repeat(32);
const object = "0123456789abcdef0123456789abcdef";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function storage() {
  const store = new MemoryStorage();
  const view = store.view.bind(store);
  store.objectLists = 0;
  store.view = () => {
    const txn = view();
    const list = txn.list.bind(txn);
    txn.list = async (opts) => {
      if (opts && String(opts.prefix || "").startsWith("o:")) store.objectLists += 1;
      return list(opts);
    };
    return txn;
  };
  return store;
}

async function call(box, method, path, body, headers = {}) {
  const text = body === undefined ? undefined : (typeof body === "string" ? body : JSON.stringify(body));
  const response = await box.fetch(new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: { "content-type": "application/json", ...headers },
    body: text,
  }));
  const raw = await response.text();
  let json = {};
  if (raw) {
    try { json = JSON.parse(raw); } catch { json = { raw }; }
  }
  return { status: response.status, json, raw, etag: response.headers.get("etag") };
}

async function authed(box, device, method, path, body, extra = {}) {
  const text = body === undefined ? "" : JSON.stringify(body);
  const headers = await authHeaders(device, method, path, text);
  return call(box, method, path, body === undefined ? undefined : text, { ...headers, ...extra });
}

const access = {
  "x-omacus-access-hash": "cd".repeat(32),
  "x-omacus-access-uses": "1",
  "x-omacus-access-spend": "1",
};

const store = storage();
const box = new Mailboxes({ storage: store }, {});
const device = await makeDevice(account);
const record = await makeRecord(device, "oats");
const created = await call(box, "POST", "/v1/mailboxes", {
  account,
  record,
  objects: [{ id: object, base: 0, body: btoa(String.fromCharCode(...new Uint8Array(64).fill(7))) }],
}, access);
assert(created.status === 201, `create ${created.status}`);

const full = await authed(box, device, "GET", `/v1/mailboxes/${account}`);
assert(full.status === 200, `full ${full.status}`);
assert(full.etag === '"1"', `etag ${full.etag}`);
assert(full.json.record === record, "record");
assert(store.objectLists === 1, `lists ${store.objectLists}`);

const quiet = await authed(box, device, "GET", `/v1/mailboxes/${account}`, undefined, { "if-none-match": '"1"' });
assert(quiet.status === 304, `quiet ${quiet.status}`);
assert(quiet.raw === "", "quiet body");
assert(quiet.etag === '"1"', `quiet etag ${quiet.etag}`);
assert(!quiet.raw.includes(record), "quiet record");
assert(store.objectLists === 1, "quiet does not list objects");

const stale = await authed(box, device, "GET", `/v1/mailboxes/${account}`, undefined, { "if-none-match": '"0"' });
assert(stale.status === 200 && stale.json.record === record, "a different revision still downloads");
assert(store.objectLists === 2, "a miss lists objects");

const star = await authed(box, device, "GET", `/v1/mailboxes/${account}`, undefined, { "if-none-match": "*" });
assert(star.status === 200 && star.json.record === record, "a star still downloads");

const open = await call(box, "GET", `/v1/mailboxes/${account}`, undefined, { "if-none-match": '"1"' });
const unsigned = await call(box, "GET", `/v1/mailboxes/${account}`);
assert(open.status === 404 && unsigned.status === 404, "unsigned stays 404");
assert(open.json.message === unsigned.json.message, "unsigned messages match");
assert(open.raw !== "" && !open.raw.includes(record), "unsigned body is the error");

console.log("revision poll ok");
