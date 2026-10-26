// Account, storage, device, and rate limits are settings on the config object.
// The test worker starts lower than production. A saved setting replaces that.
// A signup past the account cap is refused, and the invite code is not used.
import { readFile } from "node:fs/promises";
import worker, { Config, Mailboxes, clearPlacement } from "./worker.js";
import { linkedDevices } from "./admin.js";
import { MemoryStorage } from "./storage.js";
import { authHeaders, makeDevice, makeRecord } from "./auth.js";
import { PRODUCTION, TEST } from "./limits.js";
import { formatLimits, planLimits } from "./admin.mjs";

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";
const text = new TextEncoder();

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

function u16(n) {
  return Uint8Array.of((n >> 8) & 255, n & 255);
}

function u32(n) {
  return Uint8Array.of((n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255);
}

function chunk(bytes) {
  return concat([u32(bytes.length), bytes]);
}

function b64(bytes) {
  const view = bytes instanceof Uint8Array ? bytes : text.encode(String(bytes));
  let bin = "";
  for (const byte of view) bin += String.fromCharCode(byte);
  return btoa(bin);
}

function twoDeviceRecord() {
  const chain = concat([
    text.encode("OMACUS-DEVICES/1"),
    u16(1),
    new Uint8Array(8 + 32 + 16 + 64),
    u16(2),
    new Uint8Array(160),
  ]);
  return concat([
    text.encode("OMACUS-SYNC/1\n"),
    Uint8Array.of(8),
    text.encode("personal"),
    new Uint8Array(8 + 32 + 16),
    chunk(chain),
    chunk(new Uint8Array()),
    u16(0),
    chunk(text.encode("note")),
  ]);
}

function bulkyRecord() {
  const body = new Uint8Array(2000);
  body.set(text.encode("OMACUS-SYNC/1\n"), 0);
  return body;
}

function cluster(serve) {
  clearPlacement();
  const storages = new Map();
  const env = {
    ACCESS_SECRET: SECRET,
    ADMIN_KEY: ADMIN,
    ALLOW_NO_ACCESS: "1",
    SERVE_APP: serve,
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
  return { env, storages };
}

let ips = 1;
async function call(env, method, path, body, headers = {}) {
  const request = new Request("https://relay.example" + path, {
    method,
    headers: {
      "content-type": "application/json",
      "cf-connecting-ip": `10.7.0.${(ips++ % 200) + 1}`,
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

const adminHeaders = { authorization: `Bearer ${ADMIN}` };

async function mint(env, uses) {
  const made = await call(env, "POST", "/v1/admin/codes", {
    uses,
    label: "Row",
    expires: "2099-01-01",
  }, adminHeaders);
  assert(made.status === 200, `mint ${made.status} ${made.text}`);
  return made.json;
}

function sameNumbers(got, expect) {
  for (const key of Object.keys(expect)) assert(got[key] === expect[key], `${key} ${got[key]}`);
}

const record = b64("OMACUS-SYNC/1\nsealed");
const listed = linkedDevices(twoDeviceRecord());
assert(listed === 2, `linked devices ${listed}`);

{
  const quiet = planLimits(["limits"], PRODUCTION);
  assert(quiet.changed === false, "printing limits is not a change");
  const planned = planLimits(["limits", "--max-accounts", "12", "--guess-rate", "3"], { ...PRODUCTION, saved: true });
  assert(planned.changed === true && planned.limits.maxAccounts === 12 && planned.limits.guessRate === 3, "one flag changes one field");
  assert(planned.limits.maxDevices === PRODUCTION.maxDevices && !Object.hasOwn(planned.limits, "saved"), "other fields stay, extras drop");
  const bad = planLimits(["limits", "--max-storage", "lots"], PRODUCTION);
  assert(bad.error === "That limit didn't work.", "a word is not a limit");
  const printed = formatLimits({ ...TEST, saved: false });
  assert(printed.includes("Max accounts: 3") && printed.includes("bytes") && printed.includes("Using the starting limits."), printed);
}

{
  const { env } = cluster("1");
  const opening = await call(env, "GET", "/v1/admin/limits", undefined, adminHeaders);
  assert(opening.status === 200 && opening.json.saved === false, `test defaults ${opening.text}`);
  sameNumbers(opening.json, TEST);
  const closed = await call(env, "GET", "/v1/admin/limits");
  assert(closed.status === 401, "limits without a key");
  const wrong = await call(env, "POST", "/v1/admin/limits", { maxDevices: 65 }, adminHeaders);
  assert(wrong.status === 400 && wrong.json.message === "That limit didn't work.", wrong.text);
  const still = await call(env, "GET", "/v1/admin/limits", undefined, adminHeaders);
  assert(still.json.saved === false && still.json.maxDevices === TEST.maxDevices, "a bad save did not stick");
  const saved = await call(env, "POST", "/v1/admin/limits", { maxAccounts: 9 }, adminHeaders);
  assert(saved.status === 200 && saved.json.saved === true && saved.json.maxAccounts === 9, saved.text);
  assert(saved.json.guessRate === TEST.guessRate, "an omitted rate stays");
  const again = await call(env, "GET", "/v1/admin/limits", undefined, adminHeaders);
  assert(again.json.maxAccounts === 9 && again.json.saved === true, "the config object kept the setting");
}

{
  const { env } = cluster("0");
  const opening = await call(env, "GET", "/v1/admin/limits", undefined, adminHeaders);
  assert(opening.status === 200 && opening.json.saved === false, opening.text);
  sameNumbers(opening.json, PRODUCTION);
  assert(opening.json.maxAccounts === 10_000 && opening.json.maxStorage === 50 * 1024 * 1024 && opening.json.maxDevices === 10, "production starts at 50 MB and 10 devices");
}

{
  const { env, storages } = cluster("1");
  const made = await mint(env, 4);
  await call(env, "POST", "/v1/admin/limits", { maxAccounts: 1 }, adminHeaders);
  const first = "11".repeat(32);
  const second = "22".repeat(32);
  const created = await call(env, "POST", "/v1/mailboxes", { account: first, record, code: made.code });
  assert(created.status === 201, `first signup ${created.text}`);
  const raised = { ...PRODUCTION, maxAccounts: 100 };
  const blocked = await call(env, "POST", "/v1/mailboxes", {
    account: second,
    record,
    code: made.code,
  }, { "x-omacus-limits": JSON.stringify(raised) });
  assert(blocked.status === 403 && blocked.json.message === "New signups are full.", blocked.text);
  const directory = storages.get("directory");
  const row = await directory.get("a:" + made.hash);
  assert(row.left === 3, `the refused signup spent the code (${row.left})`);
  assert(!(await directory.get("r:" + second)), "the refused signup reserved an account");
  assert(storages.get(`acct-${first}`) && await storages.get(`acct-${first}`).get("m:" + first), "the first account stayed");
  const home = await storages.get(`acct-${second}`);
  assert(!home || !(await home.get("m:" + second)), "the second account was not stored");
}

{
  const { env, storages } = cluster("1");
  const made = await mint(env, 2);
  await call(env, "POST", "/v1/admin/limits", { maxDevices: 1 }, adminHeaders);
  const account = "33".repeat(32);
  const blocked = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: b64(twoDeviceRecord()),
    code: made.code,
  });
  assert(blocked.status === 429 && blocked.json.message === "That account has as many phones and computers as it can.", blocked.text);
  const row = await storages.get("directory").get("a:" + made.hash);
  assert(row.left === 2, `a full device list spent the code (${row.left})`);
  assert(!(await storages.get("directory").get("r:" + account)), "a full device list reserved an account");
}

{
  const { env, storages } = cluster("1");
  const made = await mint(env, 2);
  await call(env, "POST", "/v1/admin/limits", { maxStorage: 1024 }, adminHeaders);
  const account = "44".repeat(32);
  const blocked = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: b64(bulkyRecord()),
    code: made.code,
  });
  assert(blocked.status === 429 && blocked.json.message === "That account is out of room.", blocked.text);
  const row = await storages.get("directory").get("a:" + made.hash);
  assert(row.left === 2, `an oversized copy spent the code (${row.left})`);
}

{
  const { env, storages } = cluster("1");
  const made = await mint(env, 1);
  const account = "55".repeat(32);
  const device = await makeDevice(account);
  const signed = await makeRecord(device, "mustard");
  const created = await call(env, "POST", "/v1/mailboxes", { account, record: signed, code: made.code });
  assert(created.status === 201, `signed create ${created.text}`);
  await call(env, "POST", "/v1/admin/limits", { maxStorage: 1024 }, adminHeaders);
  const objectId = "ab".repeat(16);
  const path = `/v1/mailboxes/${account}`;
  const payload = {
    revision: 1,
    record: signed,
    objects: [{ id: objectId, base: 0, body: b64(new Uint8Array(1500).fill(7)) }],
  };
  const raw = JSON.stringify(payload);
  const putHeaders = await authHeaders(device, "PUT", path, raw);
  putHeaders["if-match"] = `"${payload.revision}"`;
  const put = await call(env, "PUT", path, payload, putHeaders);
  assert(put.status === 429 && put.json.message === "That account is out of room.", put.text);
  const home = storages.get(`acct-${account}`);
  const mailbox = await home.get("m:" + account);
  assert(mailbox.revision === 1, `the save landed (${mailbox.revision})`);
  assert(!(await home.get(`o:${account}:${objectId}`)), "the oversized object was stored");
}

{
  const box = new Mailboxes({ storage: new MemoryStorage() }, { SERVE_APP: "0" });
  const limits = JSON.stringify({ ...PRODUCTION, guessRate: 2 });
  const headers = {
    "content-type": "application/json",
    "cf-connecting-ip": "203.0.113.80",
    "x-omacus-limits": limits,
  };
  const statuses = [];
  for (let n = 0; n < 3; n += 1) {
    const response = await box.fetch(new Request("https://relay.example/v1/access", {
      method: "POST",
      headers,
      body: "{}",
    }));
    statuses.push(response.status);
  }
  assert(statuses[0] === 400 && statuses[1] === 400 && statuses[2] === 429, `guess setting ${statuses}`);
}

{
  const wrangler = await readFile(new URL("./wrangler.jsonc", import.meta.url), "utf8");
  const page = await readFile(new URL("./admin/index.html", import.meta.url), "utf8");
  const script = await readFile(new URL("./admin/admin.js", import.meta.url), "utf8");
  assert(wrangler.includes('"class_name": "Config"'), "config object is not bound");
  assert((wrangler.match(/"tag": "v2"/g) || []).length === 1, "the worker needs the config migration");
  assert(page.includes("Save limits") && page.includes("data-limits") && !page.includes("innerHTML"), "limits form");
  assert(script.includes("/v1/admin/limits") && !script.includes("innerHTML"), "limits request");
  assert(page.includes('class="quiet" type="submit">Save limits'), "save stays quiet");
}

console.log("limits config ok");
