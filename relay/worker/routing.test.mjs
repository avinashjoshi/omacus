// Each account has its own object. A join reads the invite index.
// It does not list every account. The 33rd account is not "full".
import worker, { Mailboxes, clearPlacement } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { authHeaders, makeDevice, makeRecord } from "./auth.js";
import { encodeCode, hashHex, hmacBytes } from "./invite.js";

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";
const text = new TextEncoder();

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function b64(bytes) {
  const view = bytes instanceof Uint8Array ? bytes : text.encode(String(bytes));
  let bin = "";
  for (const byte of view) bin += String.fromCharCode(byte);
  return btoa(bin);
}

function watch(storage) {
  storage.scans = 0;
  const view = storage.view.bind(storage);
  storage.view = () => {
    const txn = view();
    const list = txn.list.bind(txn);
    txn.list = async (opts) => {
      if (opts && opts.prefix === "m:") storage.scans += 1;
      return list(opts);
    };
    return txn;
  };
  const list = storage.list.bind(storage);
  storage.list = async (opts) => {
    if (opts && opts.prefix === "m:") storage.scans += 1;
    return list(opts);
  };
  return storage;
}

function cluster() {
  clearPlacement();
  const storages = new Map();
  const names = [];
  const env = {
    ACCESS_SECRET: SECRET,
    ADMIN_KEY: ADMIN,
    ALLOW_NO_ACCESS: "1",
    SERVE_APP: "0",
    MAILBOXES: {
      idFromName: (name) => {
        names.push(String(name));
        return String(name);
      },
      get: (name) => {
        const key = String(name);
        if (!storages.has(key)) storages.set(key, watch(new MemoryStorage()));
        const mailbox = new Mailboxes({ storage: storages.get(key) }, env);
        return { fetch: (request) => mailbox.fetch(request) };
      },
    },
  };
  return { env, storages, names };
}

let ips = 1;
async function call(env, method, path, body, headers = {}) {
  const request = new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: {
      "content-type": "application/json",
      "cf-connecting-ip": `10.4.0.${ips++}`,
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const response = await worker.fetch(request, env);
  const payload = await response.text();
  let json = {};
  try { json = JSON.parse(payload); } catch { json = {}; }
  return { status: response.status, json, text: payload };
}

async function mint(env, uses) {
  const made = await call(env, "POST", "/v1/admin/codes", {
    uses,
    label: "Row",
    expires: "2099-01-01",
  }, { authorization: `Bearer ${ADMIN}` });
  assert(made.status === 200, `mint ${made.status} ${made.text}`);
  return made.json.code;
}

const record = b64("OMACUS-SYNC/1\nsealed");
const recordBytes = text.encode("OMACUS-SYNC/1\nsealed").length;
const { env, storages, names } = cluster();
const firstCode = await mint(env, 20);
const secondCode = await mint(env, 20);

for (let n = 0; n < 33; n += 1) {
  const account = n.toString(16).padStart(64, "0");
  const code = n < 20 ? firstCode : secondCode;
  const created = await call(env, "POST", "/v1/mailboxes", { account, record, code });
  assert(created.status === 201, `create ${n} ${created.status} ${created.text}`);
  const home = storages.get(`acct-${account}`);
  assert(home && home.rows.has(`m:${account}`), `object ${n} did not keep the account`);
}

const legacy = storages.get("test");
if (legacy) {
  for (const key of legacy.rows.keys()) assert(!key.startsWith("m:"), `shared object kept ${key}`);
}
const directory = storages.get("directory");
assert(directory, "directory object was not used");
for (const key of directory.rows.keys()) assert(!key.startsWith("m:"), `directory kept ${key}`);
assert(names.includes("directory"), "router did not name a directory");
assert(new Set(names.filter((name) => name.startsWith("acct-"))).size >= 33, "accounts shared one object");

const stats = await call(env, "GET", "/v1/admin/stats", undefined, { authorization: `Bearer ${ADMIN}` });
assert(stats.json.accounts === 33, `stats ${stats.json.accounts}`);
assert(stats.json.storage[0].bytes === recordBytes, `bytes ${stats.json.storage[0].bytes}`);
assert(!stats.text.includes(record), "stats returned the sealed bytes");

const account = "ab".repeat(32);
const device = await makeDevice(account);
const signed = await makeRecord(device, "manifest");
const created = await call(env, "POST", "/v1/mailboxes", { account, record: signed, code: secondCode });
assert(created.status === 201, `signed create ${created.status} ${created.text}`);
const secret = new Uint8Array([4, 5, 6, 7, 8]);
const hash = await hashHex(secret);
const invitePath = `/v1/mailboxes/${account}/invites`;
const inviteBody = { hash };
const posted = await call(
  env,
  "POST",
  invitePath,
  inviteBody,
  await authHeaders(device, "POST", invitePath, JSON.stringify(inviteBody)),
);
assert(posted.status === 200, `invite ${posted.status} ${posted.text}`);
directory.scans = 0;
storages.get(`acct-${account}`).scans = 0;
const phone = new Uint8Array(80).fill(4);
const joined = await call(env, "POST", "/v1/join", {
  code: encodeCode(secret),
  device: b64(phone),
  proof: b64(await hmacBytes(secret, phone)),
  name: "Kitchen",
});
assert(joined.status === 201 && joined.json.account === account, `join ${joined.status} ${joined.text}`);
assert(directory.scans === 0, `join listed accounts on the directory (${directory.scans})`);
assert(storages.get(`acct-${account}`).scans === 0, "join listed accounts on the mailbox");
const again = await call(env, "POST", "/v1/join", {
  code: encodeCode(secret),
  device: b64(phone),
  proof: b64(await hmacBytes(secret, phone)),
  name: "Kitchen",
});
assert(again.status === 400, `used code worked ${again.status}`);

const old = "cd".repeat(32);
const oldDevice = await makeDevice(old);
const oldRecord = await makeRecord(oldDevice, "lentils");
const objectId = "0123456789abcdef0123456789abcdef";
const secretOld = new Uint8Array([9, 8, 7, 6, 5]);
const hashOld = await hashHex(secretOld);
legacy.rows.set(`m:${old}`, {
  revision: 1,
  record: oldRecord,
  requests: {},
  created: 1,
  invite: { hash: hashOld, expires: Date.now() + 60_000, used: false },
});
legacy.rows.set(`o:${old}:${objectId}`, {
  revision: 1,
  body: b64(new Uint8Array(64).fill(7)),
  bytes: 64,
});
clearPlacement();
const readPath = `/v1/mailboxes/${old}`;
const opened = await call(
  env,
  "GET",
  readPath,
  undefined,
  await authHeaders(oldDevice, "GET", readPath, ""),
);
assert(opened.status === 200, `moved read ${opened.status} ${opened.text}`);
const moved = storages.get(`acct-${old}`);
assert(moved && moved.rows.has(`m:${old}`), "legacy account was not copied");
assert(moved.rows.has(`o:${old}:${objectId}`), "legacy object was not copied");
assert(!legacy.rows.has(`m:${old}`), "legacy copy stayed");
directory.scans = 0;
moved.scans = 0;
const joinedOld = await call(env, "POST", "/v1/join", {
  code: encodeCode(secretOld),
  device: b64(phone),
  proof: b64(await hmacBytes(secretOld, phone)),
  name: "Pantry",
});
assert(joinedOld.status === 201 && joinedOld.json.account === old, `legacy join ${joinedOld.status} ${joinedOld.text}`);
assert(directory.scans === 0, `legacy join listed accounts (${directory.scans})`);

const removed = await call(
  env,
  "DELETE",
  `/v1/admin/accounts/${account}`,
  { confirm: account },
  { authorization: `Bearer ${ADMIN}` },
);
assert(removed.status === 200, `delete ${removed.status} ${removed.text}`);
assert(!storages.get(`acct-${account}`).rows.has(`m:${account}`), "account object kept the copy");
assert(!directory.rows.has(`r:${account}`), "directory kept the account");

const fresh = "ef".repeat(32);
const skipped = await call(env, "POST", "/v1/mailboxes", {
  account: fresh,
  record,
}, { "x-omacus-access-spent": "1" });
assert(skipped.status === 400, `header skipped the invite code ${skipped.status}`);
assert(!storages.has(`acct-${fresh}`), "a header created an account");
const hidden = await call(env, "POST", `/v1/internal/copy/${account}`, {});
assert(hidden.status === 404, `internal route was public ${hidden.status}`);
