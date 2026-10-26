// Each account has its own object. A join reads the invite index.
// It does not list every account. The 33rd account is not "full".
import worker, { Mailboxes, clearPlacement } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { HEARTBEAT_MS } from "./heartbeat.js";
import { appendDevice, authHeaders, deviceChain, makeDevice, makeRecord } from "./auth.js";
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
  const request = new Request("https://relay.example" + path, {
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
const secret = new Uint8Array([4, 5, 6, 7, 8, 1, 2, 3]);
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
assert(directory.rows.has(`i:${hash}`), "a waiting join removed the code");
const again = await call(env, "POST", "/v1/join", {
  code: encodeCode(secret),
  device: b64(phone),
  proof: b64(await hmacBytes(secret, phone)),
  name: "Kitchen",
});
assert(again.status === 400, `used code worked ${again.status}`);
assert(directory.rows.has(`i:${hash}`), "a second try while waiting removed the code");

function deviceBytes(owner) {
  const out = new Uint8Array(80);
  out.set(owner.id, 0);
  out.set(owner.pub, 16);
  out.set(owner.receive, 48);
  return out;
}

async function postInvite(owner, acct, inviteSecret) {
  const inviteHash = await hashHex(inviteSecret);
  const invitePath = `/v1/mailboxes/${acct}/invites`;
  const inviteBody = { hash: inviteHash };
  const postedInvite = await call(
    env,
    "POST",
    invitePath,
    inviteBody,
    await authHeaders(owner, "POST", invitePath, JSON.stringify(inviteBody)),
  );
  assert(postedInvite.status === 200, `invite ${postedInvite.status} ${postedInvite.text}`);
  return inviteHash;
}

async function joinWith(inviteSecret, rawDevice, name) {
  return call(env, "POST", "/v1/join", {
    code: encodeCode(inviteSecret),
    device: b64(rawDevice),
    proof: b64(await hmacBytes(inviteSecret, rawDevice)),
    name,
  });
}

// The same code works again after the first phone stops waiting.
// A used code, an expired code, and a finished approval still fail.
const reuse = "12".repeat(32);
const owner = await makeDevice(reuse);
const chain = await deviceChain(owner);
const sealed = await makeRecord(owner, "bread", 2, chain);
const openedReuse = await call(env, "POST", "/v1/mailboxes", { account: reuse, record: sealed, code: secondCode });
assert(openedReuse.status === 201, `reuse create ${openedReuse.status} ${openedReuse.text}`);
const reuseSecret = new Uint8Array([3, 3, 3, 3, 3, 3, 3, 3]);
const reuseHash = await postInvite(owner, reuse, reuseSecret);
const harbor = await makeDevice(reuse);
const harborBytes = deviceBytes(harbor);
const harborJoin = await joinWith(reuseSecret, harborBytes, "Harbor phone");
assert(harborJoin.status === 201 && harborJoin.json.account === reuse, `harbor ${harborJoin.status} ${harborJoin.text}`);
assert(directory.rows.has(`i:${reuseHash}`), "the first join deleted the lookup");
const ferry = await makeDevice(reuse);
const ferryWhileWaiting = await joinWith(reuseSecret, deviceBytes(ferry), "Ferry phone");
assert(ferryWhileWaiting.status === 400, `a live code was reused ${ferryWhileWaiting.status} ${ferryWhileWaiting.text}`);
assert(/didn't work/i.test(ferryWhileWaiting.json.message || ""), ferryWhileWaiting.text);
const home = storages.get(`acct-${reuse}`);
const waiting = await home.get(`m:${reuse}`);
for (const request of Object.values(waiting.requests)) {
  if (request.state === "pending") request.lastSeen = Date.now() - HEARTBEAT_MS - 1000;
}
await home.put(`m:${reuse}`, waiting);
const ferryJoin = await joinWith(reuseSecret, deviceBytes(ferry), "Ferry phone");
assert(ferryJoin.status === 201 && ferryJoin.json.account === reuse, `abandoned code ${ferryJoin.status} ${ferryJoin.text}`);
assert(ferryJoin.json.id !== harborJoin.json.id, "the abandoned request was reused");
assert(directory.rows.has(`i:${reuseHash}`), "the second join deleted the lookup");
const stillUsed = await joinWith(reuseSecret, deviceBytes(await makeDevice(reuse)), "Pantry phone");
assert(stillUsed.status === 400, `a code in use worked ${stillUsed.status}`);

const nextChain = await appendDevice(owner, chain, ferry);
assert(nextChain, "the list could not add Ferry phone");
const approvedRecord = await makeRecord(owner, "bread", 2, nextChain);
const approvePath = `/v1/mailboxes/${reuse}/requests/${ferryJoin.json.id}`;
const approveBody = {
  chain: b64(nextChain),
  wrap: b64(text.encode("wrap")),
  record: approvedRecord,
  revision: 1,
};
const approved = await call(env, "POST", approvePath, approveBody, {
  ...(await authHeaders(owner, "POST", approvePath, JSON.stringify(approveBody))),
  "if-match": '"1"',
});
assert(approved.status === 200, `approve ${approved.status} ${approved.text}`);
const donePath = approvePath;
const doneBody = { done: true };
const finished = await call(env, "POST", donePath, doneBody, await authHeaders(ferry, "POST", donePath, JSON.stringify(doneBody)));
assert(finished.status === 200, `done ${finished.status} ${finished.text}`);
assert(!directory.rows.has(`i:${reuseHash}`), "a finished code stayed findable");
assert(!home.rows.has(`i:${reuseHash}`), "a finished code stayed on the account");
const afterDone = await joinWith(reuseSecret, deviceBytes(await makeDevice(reuse)), "Harbor phone");
assert(afterDone.status === 400, `a finished code worked ${afterDone.status} ${afterDone.text}`);

const agedSecret = new Uint8Array([9, 9, 9, 9, 1, 1, 1, 1]);
const agedHash = await postInvite(owner, reuse, agedSecret);
const agedRow = await directory.get(`i:${agedHash}`);
agedRow.expires = Date.now() - 1000;
await directory.put(`i:${agedHash}`, agedRow);
const expired = await joinWith(agedSecret, deviceBytes(await makeDevice(reuse)), "Harbor phone");
assert(expired.status === 400, `an expired code worked ${expired.status} ${expired.text}`);
assert(/didn't work/i.test(expired.json.message || ""), expired.text);

const spentSecret = new Uint8Array([9, 9, 9, 9, 2, 2, 2, 2]);
const spentHash = await postInvite(owner, reuse, spentSecret);
const spentBox = await home.get(`m:${reuse}`);
spentBox.invite.expires = Date.now() - 1000;
await home.put(`m:${reuse}`, spentBox);
const spent = await joinWith(spentSecret, deviceBytes(await makeDevice(reuse)), "Harbor phone");
assert(spent.status === 400, `an expired account code worked ${spent.status} ${spent.text}`);
assert(directory.rows.has(`i:${spentHash}`), "an expired code was deleted before the account saw it");

const old = "cd".repeat(32);
const oldDevice = await makeDevice(old);
const oldRecord = await makeRecord(oldDevice, "lentils");
const objectId = "0123456789abcdef0123456789abcdef";
const secretOld = new Uint8Array([9, 8, 7, 6, 5, 4, 3, 2]);
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
