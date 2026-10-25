// Admin on the sync worker: codes, stats, signups, and sealed-copy removal.
// The code is shown once. Storage keeps a hash. Stats keep sizes, not pages.
import { readFile } from "node:fs/promises";
import worker, { Mailboxes } from "./worker.js";
import { clearAdminCaches, deviceCount, expiryMs, sameSecret } from "./admin.js";
import { DEFAULT_SYNC_URL } from "./admin.mjs";

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";
const text = new TextEncoder();

class Storage {
  constructor() {
    this.rows = new Map();
    this.chain = Promise.resolve();
  }
  async transaction(fn) {
    const run = this.chain.then(async () => {
      const snapshot = structuredClone(this.rows);
      const txn = {
        get: async (key) => {
          const value = this.rows.get(key);
          return value === undefined ? undefined : structuredClone(value);
        },
        put: async (key, value) => {
          this.rows.set(key, structuredClone(value));
        },
        delete: async (key) => {
          this.rows.delete(key);
        },
        list: async ({ prefix } = {}) => {
          const out = new Map();
          for (const [key, value] of this.rows) {
            if (!prefix || key.startsWith(prefix)) out.set(key, structuredClone(value));
          }
          return out;
        },
      };
      try {
        await fn(txn);
      } catch (error) {
        this.rows = snapshot;
        throw error;
      }
    });
    this.chain = run.then(() => {}, () => {});
    return run;
  }
  async get(key) {
    const value = this.rows.get(key);
    return value === undefined ? undefined : structuredClone(value);
  }
  async put(key, value) {
    this.rows.set(key, structuredClone(value));
  }
  async delete(key) {
    this.rows.delete(key);
  }
  async list({ prefix } = {}) {
    const out = new Map();
    for (const [key, value] of this.rows) {
      if (!prefix || key.startsWith(prefix)) out.set(key, structuredClone(value));
    }
    return out;
  }
}

let ips = 1;
function harness(extra = {}) {
  const storage = new Storage();
  const box = new Mailboxes({ storage }, {});
  const seen = [];
  const raw = box.fetch.bind(box);
  const env = {
    ACCESS_SECRET: extra.ACCESS_SECRET ?? SECRET,
    ADMIN_KEY: Object.hasOwn(extra, "ADMIN_KEY") ? extra.ADMIN_KEY : ADMIN,
    SERVE_APP: extra.SERVE_APP ?? "0",
    ACCESS_AUD: extra.ACCESS_AUD || "",
    ACCESS_TEAM_DOMAIN: extra.ACCESS_TEAM_DOMAIN || "",
    MAILBOXES: {
      idFromName: () => "test",
      get: () => ({
        fetch: async (request) => {
          if (request.method !== "GET" && request.method !== "HEAD") seen.push(await request.clone().text());
          return raw(request);
        },
      }),
    },
  };
  return { storage, box, env, seen };
}

async function call(env, method, path, body, headers = {}) {
  const ip = headers["cf-connecting-ip"] || `10.1.0.${ips++}`;
  const request = new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${ADMIN}`,
      "cf-connecting-ip": ip,
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

function chunk(bytes) {
  const out = new Uint8Array(4 + bytes.length);
  new DataView(out.buffer).setUint32(0, bytes.length);
  out.set(bytes, 4);
  return out;
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

function sealedRecord() {
  const page = text.encode("Trip Budget\nHarbor phone");
  return concat([
    text.encode("OMACUS-SYNC/1\n"),
    Uint8Array.of(8),
    text.encode("personal"),
    new Uint8Array(8 + 32 + 16),
    chunk(new Uint8Array(0)),
    chunk(new Uint8Array(0)),
    Uint8Array.of(0, 2),
    chunk(text.encode("wrap-one")),
    chunk(text.encode("wrap-two")),
    chunk(page),
  ]);
}

function b64(bytes) {
  let bin = "";
  for (const byte of bytes) bin += String.fromCharCode(byte);
  return btoa(bin);
}

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function noSecrets(blob, code) {
  const folded = blob.toLowerCase();
  assert(!blob.includes(code), "response kept the code");
  assert(!blob.includes(code.replaceAll("-", "")), "response kept the raw code");
  assert(!folded.includes(ADMIN.toLowerCase()), "response kept the admin key");
  assert(!blob.includes("Trip Budget"), "response kept a page name");
  assert(!blob.includes("Harbor phone"), "response kept a device name");
}

if (!sameSecret(ADMIN, ADMIN)) throw new Error("same secret");
if (sameSecret(ADMIN, "admin-test-key-valu")) throw new Error("short secret matched");
if (sameSecret(ADMIN, ADMIN + "x")) throw new Error("long secret matched");
if (sameSecret("", "")) throw new Error("empty secret matched");
if (expiryMs("2026-12-31") !== Date.UTC(2026, 11, 31, 23, 59, 59, 999)) throw new Error("expiry day");
if (expiryMs("") !== 0) throw new Error("empty expiry");
if (expiryMs("2026-13-40") !== null) throw new Error("bad date");

const record = sealedRecord();
assert(deviceCount(record) === 2, `devices ${deviceCount(record)}`);
const recordB64 = b64(record);
const account = "ab".repeat(32);
const other = "cd".repeat(32);

{
  const { storage, env, seen } = harness();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 3, label: "Friends", expires: "2099-01-01" });
  assert(made.status === 200, `mint ${made.status} ${made.text}`);
  assert(/^[0-9A-Z]{4}(?:-[0-9A-Z]{4}){5}$/.test(made.json.code), `code shape ${made.json.code}`);
  const code = made.json.code;
  const row = await storage.get("a:" + made.json.hash);
  assert(row && row.left === 3 && row.label === "Friends" && row.revoked === false, "stored row");
  assert(row.expires === Date.UTC(2099, 0, 1, 23, 59, 59, 999), "stored expiry");
  const stored = JSON.stringify([...storage.rows]);
  assert(!stored.includes(code), "storage kept the code");
  assert(!JSON.stringify(row).includes(code.replaceAll("-", "")), "stored row kept the raw code");
  assert(!stored.includes(ADMIN), "storage kept the admin key");
  assert(seen.every((body) => !body.includes(code)), "the stored request carried the code");
  const listed = await call(env, "GET", "/v1/admin/codes");
  noSecrets(listed.text, code);
  assert(listed.json.codes.length === 1, "one code");
  const item = listed.json.codes[0];
  assert(item.label === "Friends" && item.left === 3 && item.status === "Active", `list ${JSON.stringify(item)}`);
  assert(item.created > 0 && item.usedAt === 0 && item.hash === made.json.hash, "list times");
  const log = await storage.get("admin:log");
  const logText = JSON.stringify(log);
  assert(log.some((entry) => entry.action === "mint" && entry.hash === item.hash), "mint was logged");
  assert(!logText.includes(code) && !logText.includes(ADMIN), "log kept a secret");

  const revoked = await call(env, "POST", `/v1/admin/codes/${item.hash}/revoke`, {});
  assert(revoked.status === 200 && revoked.json.status === "Revoked", "revoke");
  const after = await call(env, "GET", "/v1/admin/codes");
  assert(after.json.codes[0].status === "Revoked" && after.json.codes[0].left === 3, "revoked still lists uses");
  const spent = await call(env, "POST", "/v1/mailboxes", { account, record: b64(text.encode("OMACUS-SYNC/1\nsealed")), code });
  assert(spent.status === 400, `revoked code was accepted ${spent.text}`);
  assert((await storage.get("a:" + item.hash)).left === 3, "revoked code was spent");
}

{
  const { storage, env } = harness();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 2, label: "Weekend", expires: Date.UTC(2030, 0, 1) });
  const code = made.json.code;
  const paused = await call(env, "POST", "/v1/admin/signups", { paused: true });
  assert(paused.status === 200 && paused.json.paused === true, "pause");
  const blocked = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: b64(text.encode("OMACUS-SYNC/1\nsealed")),
    code,
  });
  assert(blocked.status === 403 && blocked.json.message === "New signups are paused.", blocked.text);
  assert((await storage.get("a:" + made.json.hash)).left === 2, "pause spent the code");
  await call(env, "POST", "/v1/admin/signups", { paused: false });
  const created = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: b64(text.encode("OMACUS-SYNC/1\nsealed")),
    code,
  });
  assert(created.status === 201, `resume ${created.text}`);
  assert((await storage.get("a:" + made.json.hash)).left === 1, "one use remains");
  const second = await call(env, "POST", "/v1/mailboxes", {
    account: other,
    record: b64(text.encode("OMACUS-SYNC/1\nsealed")),
    code,
  });
  assert(second.status === 201, `second use ${second.text}`);
  assert((await storage.get("a:" + made.json.hash)).left === 0, "uses ran out");
  const listed = await call(env, "GET", "/v1/admin/codes");
  assert(listed.json.codes[0].status === "Used" && listed.json.codes[0].usedAt > 0, "used status");
}

{
  const { storage, env } = harness();
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 1, label: "Old", expires: Date.now() - 1000 });
  const blocked = await call(env, "POST", "/v1/mailboxes", {
    account,
    record: b64(text.encode("OMACUS-SYNC/1\nsealed")),
    code: made.json.code,
  });
  assert(blocked.status === 400, `expired code worked ${blocked.text}`);
  assert((await storage.get("a:" + made.json.hash)).left === 1, "expired code was spent");
  const listed = await call(env, "GET", "/v1/admin/codes");
  assert(listed.json.codes[0].status === "Expired", listed.text);
}

{
  const { storage, box, env } = harness();
  await storage.put("m:" + account, {
    revision: 1,
    record: recordB64,
    requests: { abcd: { name: "Harbor phone", state: "pending" } },
    created: 1,
  });
  await storage.put("a:" + "12".repeat(32), { left: 4 });
  const bad = await box.fetch(new Request("https://sync-test.omacus.com/v1/access", {
    method: "POST",
    headers: { "x-omacus-access-hash": "ab".repeat(32), "x-omacus-access-uses": "0", "x-omacus-access-spend": "0" },
    body: "{}",
  }));
  assert(bad.status === 400, "bad access did not fail");
  const stats = await call(env, "GET", "/v1/admin/stats");
  noSecrets(stats.text, "no-code");
  assert(!stats.text.includes(recordB64), "stats returned the sealed bytes");
  assert(stats.json.accounts === 1 && stats.json.devices === 2, JSON.stringify(stats.json));
  assert(stats.json.storage[0].account === account && stats.json.storage[0].devices === 2, "storage row");
  assert(stats.json.storage[0].bytes === record.length, `bytes ${stats.json.storage[0].bytes}`);
  assert(stats.json.errors.some((row) => row.name === "BadAccess" && row.count >= 1), JSON.stringify(stats.json.errors));
  const legacy = await call(env, "GET", "/v1/admin/codes");
  const old = legacy.json.codes.find((row) => row.hash === "12".repeat(32));
  assert(old && old.label === "" && old.left === 4 && old.created === 0 && old.status === "Active", JSON.stringify(old));

  const wrong = await call(env, "DELETE", `/v1/admin/accounts/${account}`, { confirm: other });
  assert(wrong.status === 400, wrong.text);
  assert(await storage.get("m:" + account), "wrong confirm deleted the copy");
  const removed = await call(env, "DELETE", `/v1/admin/accounts/${account}`, { confirm: account });
  assert(removed.status === 200 && removed.json.ok === true, removed.text);
  noSecrets(removed.text, "no-code");
  assert(!removed.text.includes(recordB64), "delete returned the sealed bytes");
  assert(!(await storage.get("m:" + account)), "sealed copy stayed");
  const again = await call(env, "GET", "/v1/admin/stats");
  assert(again.json.accounts === 0 && again.text.includes("[]"), again.text);
}

{
  const { env } = harness();
  const missing = await call(env, "GET", "/v1/admin/stats", undefined, { authorization: "" });
  assert(missing.status === 401 && missing.json.message === "That key didn't work.", missing.text);
  const wrong = await call(env, "GET", "/v1/admin/stats", undefined, { authorization: "Bearer nope" });
  assert(wrong.status === 401, "wrong key");
  assert(!wrong.text.includes("nope") && !wrong.text.includes(ADMIN), wrong.text);
  const closed = harness({ ADMIN_KEY: "" });
  const refused = await call(closed.env, "GET", "/v1/admin/stats");
  assert(refused.status === 401, "empty admin key was accepted");
  const home = await worker.fetch(new Request("https://sync.omacus.com/"), closed.env);
  assert(home.status === 404, "production served the app");
  const health = await worker.fetch(new Request("https://sync-test.omacus.com/v1/health"), closed.env);
  assert((await health.json()).label === "1.0.0-beta.22", "label");
}

{
  const { box } = harness();
  const ip = "10.9.9.9";
  let last = 0;
  for (let n = 0; n < 31; n += 1) {
    const response = await box.fetch(new Request("https://sync-test.omacus.com/v1/admin/stats", {
      headers: { "x-omacus-admin": "1", "cf-connecting-ip": ip },
    }));
    last = response.status;
  }
  assert(last === 429, `do rate ${last}`);
  const { env } = harness();
  let edge = 0;
  for (let n = 0; n < 31; n += 1) {
    const response = await call(env, "GET", "/v1/admin/stats", undefined, { "cf-connecting-ip": "10.8.8.8" });
    edge = response.status;
  }
  assert(edge === 429, `edge rate ${edge}`);
}

{
  clearAdminCaches();
  const pair = await crypto.subtle.generateKey(
    { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
    true,
    ["sign", "verify"],
  );
  const jwk = await crypto.subtle.exportKey("jwk", pair.publicKey);
  jwk.kid = "test-key";
  jwk.alg = "RS256";
  jwk.use = "sig";
  const realFetch = globalThis.fetch;
  const certs = "https://omacus.cloudflareaccess.com/cdn-cgi/access/certs";
  globalThis.fetch = async (url) => {
    if (String(url) === certs) {
      return new Response(JSON.stringify({ keys: [jwk] }), { status: 200 });
    }
    throw new Error(`unexpected fetch ${url}`);
  };
  async function jwt(claims) {
    const header = Buffer.from(JSON.stringify({ alg: "RS256", kid: "test-key" })).toString("base64url");
    const payload = Buffer.from(JSON.stringify(claims)).toString("base64url");
    const sig = Buffer.from(await crypto.subtle.sign(
      "RSASSA-PKCS1-v1_5",
      pair.privateKey,
      text.encode(`${header}.${payload}`),
    )).toString("base64url");
    return `${header}.${payload}.${sig}`;
  }
  const iss = "https://omacus.cloudflareaccess.com";
  const aud = "aud-tag";
  const exp = Math.floor(Date.now() / 1000) + 3600;
  try {
    const open = harness();
    const plain = await call(open.env, "GET", "/v1/admin/stats");
    assert(plain.status === 200, "unconfigured access rejected a key");
    const pageOpen = await call(open.env, "GET", "/admin");
    assert(pageOpen.status !== 401, "unconfigured access refused the admin page");
    const noTeam = harness({ ACCESS_AUD: aud });
    const noTeamRes = await call(noTeam.env, "GET", "/v1/admin/stats");
    assert(noTeamRes.status === 401, "audience without a team domain was accepted");
    const locked = harness({ ACCESS_AUD: aud, ACCESS_TEAM_DOMAIN: "omacus.cloudflareaccess.com" });
    locked.env.ASSETS = {
      fetch: async (request) => new Response(new URL(request.url).pathname, { status: 200 }),
    };
    const bare = await call(locked.env, "GET", "/v1/admin/stats");
    assert(bare.status === 401, "missing access token was accepted");
    const barePage = await call(locked.env, "GET", "/admin", undefined, { authorization: "" });
    assert(barePage.status === 401 && barePage.text.includes("didn't work"), "admin page skipped the access check");
    const bareScript = await call(locked.env, "GET", "/admin/admin.js");
    assert(bareScript.status === 401, "admin script skipped the access check");
    const other = await jwt({ aud: "other", exp, iss });
    const mismatch = await call(locked.env, "GET", "/v1/admin/stats", undefined, { "cf-access-jwt-assertion": other });
    assert(mismatch.status === 401, "wrong audience was accepted");
    const foreign = await jwt({ aud, exp, iss: "https://other.cloudflareaccess.com" });
    const wrongIss = await call(locked.env, "GET", "/v1/admin/stats", undefined, { "cf-access-jwt-assertion": foreign });
    assert(wrongIss.status === 401, "wrong issuer was accepted");
    const stale = await jwt({ aud, exp: Math.floor(Date.now() / 1000) - 10, iss });
    const expired = await call(locked.env, "GET", "/v1/admin/stats", undefined, { "cf-access-jwt-assertion": stale });
    assert(expired.status === 401, "expired access token was accepted");
    const good = await jwt({ aud, exp, iss });
    const keyless = await call(locked.env, "GET", "/v1/admin/stats", undefined, {
      "cf-access-jwt-assertion": good,
      authorization: "",
    });
    assert(keyless.status === 401, "access token replaced the admin key");
    const ok = await call(locked.env, "GET", "/v1/admin/stats", undefined, { "cf-access-jwt-assertion": good });
    assert(ok.status === 200, `valid access token failed ${ok.text}`);
    const page = await call(locked.env, "GET", "/admin", undefined, {
      "cf-access-jwt-assertion": good,
      authorization: "",
    });
    assert(page.status === 200 && page.text === "/admin/index.html", `admin page rejected a valid sign-in ${page.status}`);
  } finally {
    globalThis.fetch = realFetch;
    clearAdminCaches();
  }
}

{
  const headers = await readFile(new URL("../../site/app/_headers", import.meta.url), "utf8");
  const config = await readFile(new URL("../../site/app/sync-config.js", import.meta.url), "utf8");
  const sync = await readFile(new URL("../../site/app/sync.js", import.meta.url), "utf8");
  const web = await readFile(new URL("../../.github/scripts/build-web.sh", import.meta.url), "utf8");
  const testBuild = await readFile(new URL("./build.sh", import.meta.url), "utf8");
  const wrangler = await readFile(new URL("./wrangler.jsonc", import.meta.url), "utf8");
  const cli = await readFile(new URL("./admin.mjs", import.meta.url), "utf8");
  const mint = await readFile(new URL("./mint.mjs", import.meta.url), "utf8");
  const page = await readFile(new URL("./admin/admin.js", import.meta.url), "utf8");
  assert(config.includes('export const SYNC_URL = "";'), "source sync url was filled in");
  assert(!sync.includes("sync.omacus.com") && !sync.includes("sync-test.omacus.com"), "sync.js hardcodes a host");
  assert(sync.includes("SYNC_URL"), "sync.js ignores the setting");
  assert(web.includes("https://sync.omacus.com"), "production build has no sync host");
  assert(testBuild.includes("https://sync-test.omacus.com"), "test build has no sync host");
  assert(headers.includes("connect-src") && headers.includes("https://sync.omacus.com") && headers.includes("https://sync-test.omacus.com"), "csp omits the sync host");
  assert(testBuild.includes("site/app/_headers") && testBuild.includes("Content-Security-Policy") && testBuild.includes("Cache-Control: no-store"), "test build drops the content security policy");
  assert(!testBuild.includes("printf"), "test build replaces the headers file");
  assert(wrangler.includes('"name": "omacus-sync-test"') && wrangler.includes("sync-test.omacus.com"), "test worker");
  assert(wrangler.includes('"name": "omacus-sync"') && wrangler.includes("sync.omacus.com"), "production worker");
  assert(wrangler.includes("omacus-sync-test-backups") && wrangler.includes("omacus-sync-backups"), "backup buckets");
  assert(wrangler.includes('"binding": "BACKUPS"') && wrangler.includes("15 7 * * *"), "backup binding");
  assert(wrangler.includes("ACCESS_AUD") && !wrangler.includes("CF_ACCESS_AUD"), "access audience");
  assert((wrangler.match(/omacus\.cloudflareaccess\.com/g) || []).length >= 2, "access team on both workers");
  const access = await readFile(new URL("./admin.js", import.meta.url), "utf8");
  assert(access.includes("ACCESS_TEAM_DOMAIN") && access.includes("/cdn-cgi/access/certs"), "access certs");
  assert(!access.includes("damp-band"), "old access team");
  assert(wrangler.includes('"SERVE_APP": "0"') && wrangler.includes('"SERVE_APP": "1"'), "serve flag");
  assert(DEFAULT_SYNC_URL === "https://sync-test.omacus.com", "cli default host");
  assert(!cli.includes("ACCESS_SECRET"), "cli still mints locally");
  assert(!mint.includes("ACCESS_SECRET") && mint.includes("admin.mjs"), "mint.mjs still mints locally");
  assert(!page.includes("innerHTML"), "admin page builds html from strings");
}

console.log("admin checks passed");
