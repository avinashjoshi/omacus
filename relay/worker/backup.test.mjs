// A night's copy is the sealed bytes already on the server.
// Restoring writes them back. The index does not carry the sealed record.
import worker, { clearPlacement, runBackup } from "./worker.js";
import { Mailboxes, Config } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { memoryBackupStore, nightOf, openBackupStore, RETENTION_DAYS } from "./backup.js";
import { appendDevice, authHeaders, deviceChain, makeDevice, makeRecord } from "./auth.js";

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";
const text = new TextEncoder();
const MARKER = "marker-oak-page";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function b64(bytes) {
  const view = bytes instanceof Uint8Array ? bytes : text.encode(String(bytes));
  let bin = "";
  for (const byte of view) bin += String.fromCharCode(byte);
  return btoa(bin);
}

class MemoryBucket {
  constructor() {
    this.rows = new Map();
  }
  async put(key, value) {
    this.rows.set(key, String(value));
  }
  async get(key) {
    const value = this.rows.get(key);
    if (value === undefined) return null;
    return { text: async () => value };
  }
  async delete(key) {
    this.rows.delete(key);
  }
  async list({ prefix } = {}) {
    const objects = [...this.rows.keys()]
      .filter((key) => !prefix || key.startsWith(prefix))
      .sort()
      .map((key) => ({ key }));
    return { objects, truncated: false };
  }
}

function cluster() {
  clearPlacement();
  const storages = new Map();
  const bucket = new MemoryBucket();
  const env = {
    ACCESS_SECRET: SECRET,
    ADMIN_KEY: ADMIN,
    ALLOW_NO_ACCESS: "1",
    SERVE_APP: "1",
    BACKUPS: bucket,
  };
  const open = (key, build) => {
    if (!storages.has(key)) storages.set(key, new MemoryStorage());
    const object = build(storages.get(key));
    return { fetch: (request) => object.fetch(request) };
  };
  env.MAILBOXES = {
    idFromName: (name) => String(name),
    get: (name) => open(String(name), (storage) => new Mailboxes({ storage }, env)),
  };
  env.CONFIG = {
    idFromName: (name) => `config:${name}`,
    get: (name) => open(String(name), (storage) => new Config({ storage }, env)),
  };
  return { env, storages, bucket };
}

let ips = 1;
async function call(env, method, path, body, headers = {}) {
  const request = new Request("https://relay.example" + path, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${ADMIN}`,
      "cf-connecting-ip": `10.8.0.${(ips++ % 200) + 1}`,
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

assert(RETENTION_DAYS === 30, "a night is kept for 30 days");
{
  const memory = memoryBackupStore();
  await memory.put("nights/2020-01-01/index.json", "{}");
  assert(await memory.get("nights/2020-01-01/index.json") === "{}", "the memory store dropped a night");
  assert(openBackupStore({ BACKUPS: memory }) === memory, "the memory store was wrapped");
}

{
  const { env } = cluster();
  const missing = await call(env, "GET", "/v1/internal/accounts");
  assert(missing.status === 404, "the account list is not public");
  const bare = cluster();
  delete bare.env.BACKUPS;
  const unready = await call(bare.env, "GET", "/v1/admin/backups");
  assert(unready.status === 200 && unready.json.ready === false, unready.text);
  const refused = await call(bare.env, "POST", "/v1/admin/backups", {});
  assert(refused.status === 400 && refused.json.message === "Backups are not set up.", refused.text);
}

{
  const { env, storages, bucket } = cluster();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 1, label: "Row", expires: "2099-01-01" });
  assert(made.status === 200, made.text);
  const account = "66".repeat(32);
  const objectId = "ab".repeat(16);
  const record = b64(`OMACUS-SYNC/1\n${MARKER}`);
  const created = await call(env, "POST", "/v1/mailboxes", {
    account,
    record,
    code: made.json.code,
    objects: [{ id: objectId, base: 0, body: b64(new Uint8Array(64).fill(4)) }],
  });
  assert(created.status === 201, `create ${created.text}`);
  for (let day = 1; day <= 14; day += 1) {
    const name = `2020-01-${String(day).padStart(2, "0")}`;
    await bucket.put(`nights/${name}/index.json`, JSON.stringify({ day: name, accounts: [], failed: 0 }));
    await bucket.put(`nights/${name}/accounts/${account}/mailbox.json`, "{}");
  }
  const when = Date.UTC(2026, 9, 25, 7, 15);
  const index = await runBackup(env, when);
  assert(index.day === nightOf(when) && index.at === when && index.accounts.length === 1 && index.failed === 0 && index.status === "saved", JSON.stringify(index));
  const saved = await bucket.get(`nights/${index.day}/accounts/${account}/mailbox.json`);
  assert(saved && (await saved.text()).includes(record), "the night copy kept the sealed record");
  const object = await bucket.get(`nights/${index.day}/accounts/${account}/objects/${objectId}.json`);
  assert(object, "the night copy kept the sealed object");
  assert(!JSON.stringify(index).includes(record), "the index carried the sealed bytes");
  assert(await bucket.get("nights/2020-01-01/index.json"), "an old night was deleted");
  assert(await bucket.get("nights/2020-01-02/index.json"), "a newer night was deleted");
  const listed = await call(env, "GET", "/v1/admin/backups");
  assert(listed.json.ready === true && listed.json.backups[0].day === index.day, listed.text);
  assert(listed.json.last && listed.json.last.status === "saved" && listed.json.last.at === when, listed.text);
  assert(listed.json.last.accounts.length === 1 && listed.json.last.accounts[0].ok === true, listed.text);
  assert(!listed.text.includes(record), "the backup list carried the sealed bytes");
  const wrong = await call(env, "POST", "/v1/admin/restore", { account, confirm: "11".repeat(32), date: index.day });
  assert(wrong.status === 400, wrong.text);
  const removed = await call(env, "DELETE", `/v1/admin/accounts/${account}`, { confirm: account });
  assert(removed.status === 200, removed.text);
  const home = storages.get(`acct-${account}`);
  assert(!(await home.get("m:" + account)), "the sealed copy stayed after removal");
  const restored = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: index.day });
  assert(restored.status === 200 && restored.json.objects === 1 && restored.json.revision === 1, restored.text);
  const every = await call(env, "POST", "/v1/admin/restore", { all: true, confirm: "nope", date: index.day });
  assert(every.status === 400 && every.json.message === "Type the day to confirm.", every.text);
  const all = await call(env, "POST", "/v1/admin/restore", { all: true, confirm: index.day, date: index.day });
  assert(all.status === 200 && all.json.restored === 1 && all.json.failed === 0, all.text);
  assert(!restored.text.includes(record), "restore returned the sealed bytes");
  const mailbox = await home.get("m:" + account);
  assert(mailbox && mailbox.record === record, "the restored record differs");
  const row = await home.get(`o:${account}:${objectId}`);
  assert(row && row.bytes === 64, "the restored object differs");
  const gone = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: "1999-01-01" });
  assert(gone.status === 404 && gone.json.message === "No backup from that day.", gone.text);
}

{
  const { env, storages } = cluster();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 1, label: "Row", expires: "2099-01-01" });
  assert(made.status === 200, made.text);
  const account = "77".repeat(32);
  const oak = "11".repeat(16);
  const elm = "22".repeat(16);
  const fir = "33".repeat(16);
  const device = await makeDevice(account);
  const oakBody = b64(new Uint8Array(64).fill(1));
  const elmBody = b64(new Uint8Array(64).fill(2));
  const firBody = b64(new Uint8Array(64).fill(3));
  const oakLater = b64(new Uint8Array(64).fill(9));
  const first = await makeRecord(device, "oak page\nelm page\n");
  const created = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: first,
    code: made.json.code,
    objects: [
      { id: oak, base: 0, body: oakBody },
      { id: elm, base: 0, body: elmBody },
    ],
  });
  assert(created.status === 201, `create ${created.text}`);
  const when = Date.UTC(2026, 9, 25, 7, 15);
  const index = await runBackup(env, when);
  assert(index.accounts.length === 1 && index.failed === 0, JSON.stringify(index));
  const later = await makeRecord(device, "oak page\nelm page\nfir page\n");
  const path = `/v1/mailboxes/${account}`;
  const edited = {
    revision: 1,
    record: later,
    objects: [
      { id: oak, base: 1, body: oakLater },
      { id: fir, base: 0, body: firBody },
    ],
    drop: [elm],
  };
  const editedHeaders = await authHeaders(device, "PUT", path, JSON.stringify(edited));
  editedHeaders["if-match"] = `"${edited.revision}"`;
  const put = await call(env, "PUT", path, edited, editedHeaders);
  assert(put.status === 200, `edit ${put.text}`);
  const restored = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: index.day });
  assert(restored.status === 200 && restored.json.revision === 3, restored.text);
  const home = storages.get(`acct-${account}`);
  const mailbox = await home.get("m:" + account);
  assert(mailbox.record === first && mailbox.revision === 3, "restore rewound the revision");
  const ids = [...(await home.list({ prefix: `o:${account}:` })).keys()];
  assert(ids.length === 2 && new Set(ids).size === 2, `restore duplicated or dropped a page ${ids.join()}`);
  assert(await home.get(`o:${account}:${oak}`) && (await home.get(`o:${account}:${oak}`)).body === oakBody, "the oak page was lost");
  assert((await home.get(`o:${account}:${elm}`)).body === elmBody, "the elm page was lost");
  assert(!(await home.get(`o:${account}:${fir}`)), "a page from after the night stayed");
  const mergedRecord = await makeRecord(device, "oak page later\nelm page\nfir page\n");
  const merged = {
    revision: 3,
    record: mergedRecord,
    objects: [
      { id: oak, base: 3, body: oakLater },
      { id: fir, base: 0, body: firBody },
    ],
  };
  const mergeHeaders = await authHeaders(device, "PUT", path, JSON.stringify(merged));
  mergeHeaders["if-match"] = `"${merged.revision}"`;
  const merge = await call(env, "PUT", path, merged, mergeHeaders);
  assert(merge.status === 200, `merge ${merge.text}`);
  const after = await home.get("m:" + account);
  assert(after.record === mergedRecord && after.revision === 4, "the merge did not land");
  const kept = [...(await home.list({ prefix: `o:${account}:` })).keys()].map((key) => key.slice(`o:${account}:`.length)).sort();
  assert(kept.join() === [oak, elm, fir].sort().join(), `pages after merge ${kept.join()}`);
  assert((await home.get(`o:${account}:${oak}`)).body === oakLater, "the later oak line was lost");
  assert((await home.get(`o:${account}:${elm}`)).body === elmBody, "the restored elm page was lost");
  assert((await home.get(`o:${account}:${fir}`)).body === firBody, "the fir page was lost");
  const fetched = await call(env, "POST", `${path}/objects`, { ids: [oak, elm, fir] }, await authHeaders(device, "POST", `${path}/objects`, JSON.stringify({ ids: [oak, elm, fir] })));
  assert(fetched.status === 200 && fetched.json.objects.length === 3, fetched.text);
  const seen = new Set(fetched.json.objects.map((row) => row.id));
  assert(seen.size === 3, "a page was returned twice");
}

{
  const { env, storages } = cluster();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 1, label: "Row", expires: "2099-01-01" });
  assert(made.status === 200, made.text);
  const account = "88".repeat(32);
  const oak = "44".repeat(16);
  const fir = "55".repeat(16);
  const device = await makeDevice(account);
  const second = await makeDevice(account);
  const chain = await deviceChain(device);
  const longer = await appendDevice(device, chain, second);
  assert(longer, "the longer list was not signed");
  const oakBody = b64(new Uint8Array(64).fill(4));
  const firBody = b64(new Uint8Array(64).fill(5));
  const first = await makeRecord(device, "oak page\n", 2, chain);
  const created = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: first,
    code: made.json.code,
    objects: [{ id: oak, base: 0, body: oakBody }],
  });
  assert(created.status === 201, `longer create ${created.text}`);
  const when = Date.UTC(2026, 9, 26, 7, 15);
  const index = await runBackup(env, when);
  assert(index.failed === 0, JSON.stringify(index));
  const later = await makeRecord(device, "oak page\nfir page\n", 2, longer);
  const path = `/v1/mailboxes/${account}`;
  const edited = {
    revision: 1,
    record: later,
    objects: [{ id: fir, base: 0, body: firBody }],
  };
  const headers = await authHeaders(device, "PUT", path, JSON.stringify(edited));
  headers["if-match"] = `"${edited.revision}"`;
  const put = await call(env, "PUT", path, edited, headers);
  assert(put.status === 200, `longer edit ${put.text}`);
  const restored = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: index.day });
  assert(restored.status === 200 && restored.json.keptList === true, restored.text);
  assert(restored.json.revision === 2 && restored.json.objects === 0, restored.text);
  const home = storages.get(`acct-${account}`);
  const mailbox = await home.get("m:" + account);
  assert(mailbox.record === later && mailbox.revision === 2, "restore replaced a longer device list");
  assert((await home.get(`o:${account}:${fir}`)).body === firBody, "a page from after the night was dropped");
  assert((await home.get(`o:${account}:${oak}`)).body === oakBody, "the earlier page was dropped");
}

console.log("backups ok");
