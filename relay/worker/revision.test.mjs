// A quiet poll names the revision it has. The record comes back only when that revision changed.
import { Mailboxes } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { MAX_NONCES, appendDevice, authHeaders, authorize, deviceChain, makeDevice, makeRecord } from "./auth.js";

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
  const response = await box.fetch(new Request("https://relay.example" + path, {
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
  if (body && Number.isInteger(body.revision) && extra["if-match"] === undefined) {
    headers["if-match"] = `"${body.revision}"`;
  }
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

const path = `/v1/mailboxes/${account}`;
const barePut = await call(box, "PUT", path, { revision: 1, record }, await authHeaders(device, "PUT", path, JSON.stringify({ revision: 1, record })));
assert(barePut.status === 409 && barePut.json.message === "That copy is out of date. Try again.", `missing if-match ${barePut.status} ${barePut.raw}`);
const wrongTag = await authed(box, device, "PUT", path, { revision: 1, record }, { "if-match": '"0"' });
assert(wrongTag.status === 409, `wrong if-match ${wrongTag.status}`);
const saved = await authed(box, device, "PUT", path, { revision: 1, record: await makeRecord(device, "oats") });
assert(saved.status === 200 && saved.json.revision === 2, `named revision ${saved.status} ${saved.raw}`);

const nonceAccount = "cd".repeat(32);
const nonceDevice = await makeDevice(nonceAccount);
const nonceRecord = await makeRecord(nonceDevice, "barley");
const nonceBox = new Mailboxes({ storage: storage() }, {});
const nonceCreated = await call(nonceBox, "POST", "/v1/mailboxes", {
  account: nonceAccount,
  record: nonceRecord,
  objects: [{ id: object, base: 0, body: btoa(String.fromCharCode(...new Uint8Array(64).fill(3))) }],
}, access);
assert(nonceCreated.status === 201, `nonce mailbox ${nonceCreated.status}`);
const noncePath = `/v1/mailboxes/${nonceAccount}`;
const seen = [];
for (let n = 0; n < 70; n += 1) {
  const headers = await authHeaders(nonceDevice, "GET", noncePath, "");
  seen.push(headers);
  const read = await call(nonceBox, "GET", noncePath, undefined, headers);
  assert(read.status === 200, `nonce ${n} ${read.status} ${read.raw}`);
}
const replay = await call(nonceBox, "GET", noncePath, undefined, seen[0]);
assert(replay.status === 429, `a captured request was not busy ${replay.status} ${replay.raw}`);
assert(replay.json.message === "Sync is busy. Try again in a moment.", `busy message ${replay.raw}`);
assert(!replay.raw.includes("No pages for that yet"), "a replay looked like a missing mailbox");

const phones = [
  await makeDevice(nonceAccount),
  await makeDevice(nonceAccount),
  await makeDevice(nonceAccount),
];
let chain = await deviceChain(phones[0]);
chain = await appendDevice(phones[0], chain, phones[1]);
chain = await appendDevice(phones[0], chain, phones[2]);
assert(chain, "three phones were not listed");
const shared = await makeRecord(phones[0], "harbor", 2, chain);
const book = { record: shared, nonces: {} };
const start = 1_700_000_000_000;
const step = 4000;
const span = 15 * 60 * 1000;
let captured = null;
let capturedAt = start;
for (let t = start, n = 0; t < start + span; t += step, n += 1) {
  for (const phone of phones) {
    const headers = await authHeaders(phone, "GET", `/v1/mailboxes/${nonceAccount}`, "", t);
    const auth = await authorize("GET", `/v1/mailboxes/${nonceAccount}`, "", new Headers(headers), shared, t, book);
    assert(auth && !auth.busy, `poll ${n} was refused`);
    if (phone === phones[0]) {
      captured = headers;
      capturedAt = t;
    }
    if (n % 15 === 0) {
      const body = JSON.stringify({ revision: 1, record: shared });
      const put = await authHeaders(phone, "PUT", `/v1/mailboxes/${nonceAccount}`, body, t);
      const saved = await authorize("PUT", `/v1/mailboxes/${nonceAccount}`, body, new Headers(put), shared, t, book);
      assert(saved && !saved.busy, `save ${n} was refused`);
    }
  }
}
const again = await authorize(
  "GET",
  `/v1/mailboxes/${nonceAccount}`,
  "",
  new Headers(captured),
  shared,
  capturedAt,
  book,
);
assert(again && again.busy, "a poll still inside the window was accepted again");
const fresh = { record: shared, nonces: {} };
const now = start + span + 5 * 60 * 1000;
for (let i = 0; i < MAX_NONCES; i += 1) {
  const headers = await authHeaders(phones[0], "GET", `/v1/mailboxes/${nonceAccount}`, "", now);
  const auth = await authorize("GET", `/v1/mailboxes/${nonceAccount}`, "", new Headers(headers), shared, now, fresh);
  assert(auth && !auth.busy, `fill ${i} was refused`);
}
const overflow = await authorize(
  "GET",
  `/v1/mailboxes/${nonceAccount}`,
  "",
  new Headers(await authHeaders(phones[0], "GET", `/v1/mailboxes/${nonceAccount}`, "", now)),
  shared,
  now,
  fresh,
);
assert(overflow && overflow.busy, "a full phone list accepted another poll");
const other = await authorize(
  "GET",
  `/v1/mailboxes/${nonceAccount}`,
  "",
  new Headers(await authHeaders(phones[1], "GET", `/v1/mailboxes/${nonceAccount}`, "", now)),
  shared,
  now,
  fresh,
);
assert(other && !other.busy, "another phone was blocked by the full list");

console.log("revision poll ok");
