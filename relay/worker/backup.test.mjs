// A night's copy is the sealed bytes already on the server.
// Restoring writes them back. The index does not carry the sealed record.
import worker, { clearPlacement, runBackup } from "./worker.js";
import { Mailboxes, Config } from "./worker.js";
import { MemoryStorage } from "./storage.js";
import { nightOf, nightsToDrop } from "./backup.js";

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
  const request = new Request("https://sync-test.omacus.com" + path, {
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

assert(nightsToDrop(["2020-01-01", "2020-01-02"], 1).join() === "2020-01-01", "the oldest night is the one that goes");
assert(nightsToDrop(["2020-01-01"], 14).length === 0, "a short history is kept");

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
  assert(index.day === nightOf(when) && index.accounts.length === 1 && index.failed === 0, JSON.stringify(index));
  const saved = await bucket.get(`nights/${index.day}/accounts/${account}/mailbox.json`);
  assert(saved && (await saved.text()).includes(record), "the night copy kept the sealed record");
  const object = await bucket.get(`nights/${index.day}/accounts/${account}/objects/${objectId}.json`);
  assert(object, "the night copy kept the sealed object");
  assert(!JSON.stringify(index).includes(record), "the index carried the sealed bytes");
  assert(!(await bucket.get("nights/2020-01-01/index.json")), "the oldest night was kept");
  assert(await bucket.get("nights/2020-01-02/index.json"), "a newer night was dropped");
  const listed = await call(env, "GET", "/v1/admin/backups");
  assert(listed.json.ready === true && listed.json.backups[0].day === index.day, listed.text);
  assert(!listed.text.includes(record), "the backup list carried the sealed bytes");
  const wrong = await call(env, "POST", "/v1/admin/restore", { account, confirm: "11".repeat(32), date: index.day });
  assert(wrong.status === 400, wrong.text);
  const removed = await call(env, "DELETE", `/v1/admin/accounts/${account}`, { confirm: account });
  assert(removed.status === 200, removed.text);
  const home = storages.get(`acct-${account}`);
  assert(!(await home.get("m:" + account)), "the sealed copy stayed after removal");
  const restored = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: index.day });
  assert(restored.status === 200 && restored.json.objects === 1, restored.text);
  assert(!restored.text.includes(record), "restore returned the sealed bytes");
  const mailbox = await home.get("m:" + account);
  assert(mailbox && mailbox.record === record, "the restored record differs");
  const row = await home.get(`o:${account}:${objectId}`);
  assert(row && row.bytes === 64, "the restored object differs");
  const gone = await call(env, "POST", "/v1/admin/restore", { account, confirm: account, date: "1999-01-01" });
  assert(gone.status === 404 && gone.json.message === "No backup from that day.", gone.text);
}

console.log("backups ok");
