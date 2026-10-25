// Stats are counts and times. A sealed page, a name, and a code stay out.
import { readFile } from "node:fs/promises";
import worker, { Mailboxes } from "./worker.js";
import { activityOf, linkedDevices, noteActivity } from "./admin.js";
import { authHeaders, makeDevice, makeRecord } from "./auth.js";
import { memoryBackupStore, nightPrefix } from "./backup.js";
import { exactUtc, relativeWhen } from "./admin/when.js";
import init, {
  approveDevice,
  confirmSaved,
  deviceList,
  removeDevice,
  startJoin,
  startNew,
} from "./public/pkg/omacus_sync_web.js";

const wasm = await readFile(new URL("./public/pkg/omacus_sync_web_bg.wasm", import.meta.url));
await init({ module_or_path: wasm });

const SECRET = "omacus-access-test-secret";
const ADMIN = "admin-test-key-value";
const DAY = 24 * 60 * 60 * 1000;
const HOUR = 60 * 60 * 1000;
const TOP_KEYS = ["accounts", "activeDay", "activeWeek", "bytes", "copy", "devices", "errors", "paused", "signupsWeek", "storage"];
const ROW_KEYS = ["account", "bytes", "changed", "created", "devices", "requests", "used"];

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
function harness() {
  const storage = new Storage();
  const box = new Mailboxes({ storage }, {});
  const env = {
    ACCESS_SECRET: SECRET,
    ADMIN_KEY: ADMIN,
    SERVE_APP: "0",
    ACCESS_AUD: "",
    ALLOW_NO_ACCESS: "1",
    MAILBOXES: {
      idFromName: () => "test",
      get: () => box,
    },
  };
  return { storage, env };
}

async function call(env, method, path, body, headers = {}) {
  const request = new Request("https://sync-test.omacus.com" + path, {
    method,
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${ADMIN}`,
      "cf-connecting-ip": headers["cf-connecting-ip"] || `10.4.0.${ips++}`,
      ...headers,
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const response = await worker.fetch(request, env);
  const text = await response.text();
  let json = {};
  try { json = JSON.parse(text); } catch { json = {}; }
  return { status: response.status, json, text, headers: response.headers };
}

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

function parse(raw) {
  const value = JSON.parse(raw);
  if (!value.ok) throw new Error(value.message || raw);
  return value;
}

function fromB64(value) {
  return Uint8Array.from(atob(value), (ch) => ch.charCodeAt(0));
}

function shape(stats) {
  assert(Object.keys(stats.json).sort().join() === TOP_KEYS.join(), Object.keys(stats.json).sort().join());
  for (const row of stats.json.storage) {
    assert(Object.keys(row).sort().join() === ROW_KEYS.join(), Object.keys(row).sort().join());
  }
  if (stats.json.copy) {
    assert(Object.keys(stats.json.copy).sort().join() === "at,bytes,status", Object.keys(stats.json.copy).join());
  }
}

{
  const start = Date.UTC(2026, 9, 20, 1, 0, 0);
  const mailbox = { requests: { keep: { name: "kept aside" } } };
  noteActivity(mailbox, start);
  noteActivity(mailbox, start + 1000);
  assert(activityOf(mailbox, start + 1000).requests === 2, "same hour");
  assert(activityOf(mailbox, start + 25 * HOUR).requests === 0, "a day later");
  assert(mailbox.used === start + 1000, "last use stayed");
  noteActivity(mailbox, start, true);
  assert(mailbox.changed === start, "a write stamps the change");
  assert(mailbox.requests.keep.name === "kept aside", "the join map stayed");
  assert(!JSON.stringify(mailbox.hours).includes("kept"), "the hour rows kept a name");
  const later = { hours: [{ h: Math.floor(start / HOUR), n: 2 }] };
  noteActivity(later, start + 25 * HOUR);
  assert(activityOf(later, start + 25 * HOUR).requests === 1, "only the new hour");
  assert(later.hours.length === 1 && later.hours[0].n === 1, "the old hour was dropped");
  const capped = { hours: [{ h: Math.floor(start / HOUR), n: 1_000_000_000_000_000 }] };
  noteActivity(capped, start + 5);
  assert(capped.hours[0].n === 100_000, `cap ${capped.hours[0].n}`);
  assert(Object.keys(capped.hours[0]).sort().join() === "h,n", "hour row keys");
}

{
  const now = Date.UTC(2026, 9, 25, 15, 4, 0);
  assert(relativeWhen(0, now) === "—", "empty time");
  assert(relativeWhen(now - 20_000, now) === "Just now", "just now");
  assert(relativeWhen(now - 60_000, now) === "1 minute ago", "one minute");
  assert(relativeWhen(now - 5 * 60_000, now) === "5 minutes ago", "minutes");
  assert(relativeWhen(now - HOUR, now) === "1 hour ago", "one hour");
  assert(relativeWhen(now - 3 * HOUR, now) === "3 hours ago", "hours");
  assert(relativeWhen(now - DAY, now) === "1 day ago", "one day");
  assert(relativeWhen(now - 6 * DAY, now) === "6 days ago", "days");
  assert(relativeWhen(Date.UTC(2026, 0, 2, 8, 30), now) === "Jan 2, 2026", "older date");
  assert(exactUtc(Date.UTC(2026, 9, 25, 3, 4)) === "2026-10-25 03:04 UTC", "exact title");
  const page = await readFile(new URL("./admin/admin.js", import.meta.url), "utf8");
  const html = await readFile(new URL("./admin/index.html", import.meta.url), "utf8");
  const css = await readFile(new URL("./admin/admin.css", import.meta.url), "utf8");
  const build = await readFile(new URL("./build.sh", import.meta.url), "utf8");
  assert(page.includes("title: exactUtc(ms)"), "times need an exact title");
  assert(page.includes("relativeWhen"), "times need the relative words");
  assert(html.includes("data-activity") && html.includes("data-copy"), "stats lines");
  assert(!/#[0-9a-fA-F]{3,8}\b/.test(css), "admin css color");
  assert(build.includes("admin/when.js"), "the build copies the time helper");
}

{
  const started = parse(startNew());
  const words = started.words;
  const saved = parse(confirmSaved(started.id, JSON.stringify([[3, words[2]], [7, words[6]], [11, words[10]]])));
  const alone = parse(deviceList(started.id));
  assert(linkedDevices(fromB64(saved.record)) === 1, `one device counted ${linkedDevices(fromB64(saved.record))}`);
  assert(alone.devices.length === 1 && alone.devices[0].self === true, JSON.stringify(alone.devices));
  const first = parse(startJoin());
  parse(approveDevice(started.id, first.device));
  const second = parse(startJoin());
  parse(approveDevice(started.id, second.device));
  const before = parse(deviceList(started.id));
  assert(before.devices.length === 3, `before remove ${before.devices.length}`);
  const gone = before.devices.find((row) => !row.self);
  const kept = before.devices.find((row) => !row.self && row.id !== gone.id);
  const removed = parse(removeDevice(started.id, gone.id));
  const after = parse(deviceList(started.id));
  const counted = linkedDevices(fromB64(removed.record));
  assert(counted === after.devices.length && counted === 2, `count ${counted} list ${after.devices.length}`);
  assert(!after.devices.some((row) => row.id === gone.id), "removed id still listed");
  assert(after.devices.filter((row) => row.self).length === 1, "self count");
  assert(after.devices.some((row) => row.id === kept.id), "the other phone dropped out");
}

{
  const { storage, env } = harness();
  const now = Date.now();
  const aged = "11".repeat(32);
  const account = "22".repeat(32);
  const agedDevice = await makeDevice(aged);
  const agedRecord = await makeRecord(agedDevice, "Trip Budget\nferry");
  await storage.put("m:" + aged, {
    revision: 1,
    record: agedRecord,
    requests: {},
    created: now - 8 * DAY,
    used: now - 2 * DAY,
    changed: now - 2 * DAY,
    hours: [],
  });
  const made = await call(env, "POST", "/v1/admin/codes", { uses: 1, label: "Row" });
  assert(made.status === 200, made.text);
  const device = await makeDevice(account);
  const record = await makeRecord(device, "Trip Budget\nferry");
  const created = await call(env, "POST", "/v1/mailboxes", { account, record, code: made.json.code });
  assert(created.status === 201, created.text);
  const quiet = await call(env, "GET", "/v1/admin/stats");
  shape(quiet);
  assert(quiet.json.copy === null, "no copy yet");
  assert(quiet.json.activeDay === 0 && quiet.json.activeWeek === 1 && quiet.json.signupsWeek === 1, JSON.stringify(quiet.json));
  const fresh = quiet.json.storage.find((row) => row.account === account);
  const old = quiet.json.storage.find((row) => row.account === aged);
  assert(fresh && fresh.created > 0 && fresh.changed > 0 && fresh.used === 0 && fresh.requests === 0, JSON.stringify(fresh));
  assert(fresh.devices === 1 && old.devices === 1, "one phone each");
  assert(old.created === now - 8 * DAY && old.used === now - 2 * DAY, "aged stamps");
  const changedAt = fresh.changed;

  const path = `/v1/mailboxes/${account}`;
  const read = await call(env, "GET", path, undefined, await authHeaders(device, "GET", path, ""));
  assert(read.status === 200, read.text);
  const once = await call(env, "GET", "/v1/admin/stats");
  const used = once.json.storage.find((row) => row.account === account);
  assert(used.requests === 1 && used.used > 0 && used.changed === changedAt, JSON.stringify(used));
  assert(once.json.activeDay === 1 && once.json.activeWeek === 2 && once.json.signupsWeek === 1, JSON.stringify(once.json));

  const again = await call(env, "GET", path, undefined, await authHeaders(device, "GET", path, ""));
  assert(again.status === 200, again.text);
  const twice = await call(env, "GET", "/v1/admin/stats");
  assert(twice.json.storage.find((row) => row.account === account).requests === 2, JSON.stringify(twice.json.storage));

  const badHeaders = await authHeaders(device, "GET", path, "");
  badHeaders["x-omacus-signature"] = badHeaders["x-omacus-signature"].replace(/.$/, (ch) => (ch === "a" ? "b" : "a"));
  const bad = await call(env, "GET", path, undefined, badHeaders);
  assert(bad.status === 404, `bad signature ${bad.status}`);
  const afterBad = await call(env, "GET", "/v1/admin/stats");
  assert(afterBad.json.storage.find((row) => row.account === account).requests === 2, "a bad signature counted");

  const staleBody = { revision: 99, record };
  const stale = await call(env, "PUT", path, staleBody, await authHeaders(device, "PUT", path, JSON.stringify(staleBody)));
  assert(stale.status === 409, stale.text);
  const afterStale = await call(env, "GET", "/v1/admin/stats");
  assert(afterStale.json.storage.find((row) => row.account === account).requests === 2, "a rejected write counted");

  const next = await makeRecord(device, "Trip Budget\nferry");
  const body = { revision: read.json.revision, record: next };
  const putHeaders = await authHeaders(device, "PUT", path, JSON.stringify(body));
  putHeaders["if-match"] = `"${body.revision}"`;
  const put = await call(env, "PUT", path, body, putHeaders);
  assert(put.status === 200, put.text);
  const saved = await call(env, "GET", "/v1/admin/stats");
  const written = saved.json.storage.find((row) => row.account === account);
  assert(written.requests === 3 && written.changed > changedAt, JSON.stringify(written));

  const follow = await call(env, "GET", path, undefined, {
    ...await authHeaders(device, "GET", path, ""),
    "if-none-match": `"${put.json.revision}"`,
  });
  assert(follow.status === 304, `not modified ${follow.status}`);
  const finalStats = await call(env, "GET", "/v1/admin/stats");
  shape(finalStats);
  assert(finalStats.json.storage.find((row) => row.account === account).requests === 4, "a quiet read counted");
  assert(!finalStats.text.includes("Trip Budget") && !finalStats.text.includes("ferry"), "stats kept a page");
  assert(!finalStats.text.includes(record) && !finalStats.text.includes(next), "stats returned the sealed bytes");
  assert(!finalStats.text.toLowerCase().includes(ADMIN.toLowerCase()), "stats kept the admin key");
  assert(!finalStats.text.includes("hours"), "stats returned the hour buckets");
  assert(!finalStats.text.includes(made.json.code), "stats kept the code");
}

{
  const { env } = harness();
  const store = memoryBackupStore();
  const marker = "marker-oak-page";
  const account = "ab".repeat(32);
  const older = {
    day: "2026-10-20",
    at: Date.UTC(2026, 9, 20, 3, 0, 0),
    accounts: [{ account, objects: 1, bytes: 40, ok: true }],
    failed: 0,
    status: "saved",
    note: marker,
  };
  const newer = {
    day: "2026-10-21",
    at: Date.UTC(2026, 9, 21, 3, 15, 0),
    accounts: [
      { account, objects: 1, bytes: 80, ok: true },
      { account: "cd".repeat(32), objects: 0, bytes: 0, ok: false },
    ],
    failed: 1,
    status: "incomplete",
    note: marker,
  };
  await store.put(`${nightPrefix(older.day)}index.json`, JSON.stringify(older));
  await store.put(`${nightPrefix(newer.day)}index.json`, JSON.stringify(newer));
  await store.put(`${nightPrefix(newer.day)}accounts/${account}/mailbox.json`, JSON.stringify({ record: marker }));
  env.BACKUPS = store;
  const stats = await call(env, "GET", "/v1/admin/stats");
  assert(stats.status === 200, stats.text);
  assert(stats.json.copy.at === newer.at, JSON.stringify(stats.json.copy));
  assert(stats.json.copy.status === "incomplete" && stats.json.copy.bytes === 80, JSON.stringify(stats.json.copy));
  assert(Object.keys(stats.json.copy).sort().join() === "at,bytes,status", "copy shape");
  assert(!stats.text.includes(marker), "stats returned a sealed copy");
}

console.log("stats checks passed");
