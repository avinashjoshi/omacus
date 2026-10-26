// Invite-only sync relay (1.0.0) as a Cloudflare Worker.
// Same HTTP API as relay/src/http.rs. It keeps only the sealed bytes.
// Each account lives in its own object. A directory object maps an invite
// hash to that account, so a join does not list every account.
// A new set of pages needs an invite code. The code is shown once, and
// only a hash is stored. A new phone joins with a one-time code, also
// stored only as a hash. An approval stays until the waiting phone says it
// finished, the other phone releases it, or the same window as the code
// has passed. A no is removed when it is read.
// The admin routes count and delete sealed copies. They never return one.
// This worker holds no keys and cannot open a page.
import { DurableObject } from "cloudflare:workers";
import { mint, openCode, spend } from "./access.js";
import {
  activityOf,
  adminKeyOk,
  adminPageAllowed,
  handleAdmin,
  isolateAllow,
  linkedDevices,
  noteActivity,
  noteError,
  summarize,
} from "./admin.js";
import { allowPut, authorize, chainIsLongerExtension, deviceListed } from "./auth.js";
import { HEARTBEAT_MS } from "./heartbeat.js";
import { INVITE_MS, JOIN_TRIES, decodeCode, hashHex, hmacBytes, sameBytes } from "./invite.js";
import {
  LIMIT_HEADER,
  LIMIT_KEY,
  defaultsFor,
  limitsFromHeader,
  normalizeLimits,
  rejectSignup,
} from "./limits.js";
import { nightOf, nightPrefix, openBackupStore } from "./backup.js";

const LABEL = "1.0.0";
const MAX_RECORD = 256 * 1024, MAX_MANIFEST = 1024 * 1024, MAX_OBJECT = 4 * 1024 * 1024 + 64;
const MAX_CHAIN = 64 * 1024, MAX_WRAP = 1024, MAX_BODY = 8 * 1024 * 1024;
const MAX_REQUESTS = 8, MAX_USES = 20;
const DIRECTORY = "directory";
const LEGACY = "test";
const homed = new Set();
const enc = new TextEncoder();
const RECORD_MAGIC = enc.encode("OMACUS-SYNC/1\n");
const RECORD_MAGIC_V2 = enc.encode("OMACUS-SYNC/2\n");
const CHAIN_MAGIC = enc.encode("OMACUS-DEVICES/1");
const ERR = {
  Unknown: [404, "No pages for that yet."],
  Conflict: [409, "That copy is already here."],
  Revision: [409, "That copy is out of date. Try again."],
  Full: [429, "Sync is full."],
  Unreadable: [400, "That copy could not be read."],
  BadInvite: [400, "That code didn't work. Check it on the other device."],
  BadAccess: [400, "That invite code didn't work. Check it and try again."],
  Paused: [403, "New signups are paused."],
  SignupFull: [403, "New signups are full."],
  Room: [429, "That account is out of room."],
  Devices: [429, "That account has as many phones and computers as it can."],
  Limited: [429, "Try again in a moment."],
  Busy: [429, "Sync is busy. Try again in a moment."],
  Waiting: [409, "That phone or computer stopped waiting."],
};
// Sync hosts never set a cookie. A browser on any origin may call them.
const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PUT, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type, If-None-Match, If-Match, X-Omacus-Device, X-Omacus-Time, X-Omacus-Nonce, X-Omacus-Signature, X-Omacus-Proof",
  "Access-Control-Expose-Headers": "ETag",
  "Cache-Control": "no-store",
};
const json = (status, value, extra) =>
  new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json", ...CORS, ...(extra || {}) },
  });
const fail = (name) => { throw name; };
function dropExpired(m, now) {
  if (!m.requests) m.requests = {};
  for (const [id, request] of Object.entries(m.requests)) {
    const created = Number.isInteger(request.created) ? request.created : 0;
    if (created + INVITE_MS <= now) delete m.requests[id];
  }
  // A phone that stops waiting gives the code back. A phone that is still
  // waiting, or one that was already approved, keeps the code spent.
  const invite = m.invite;
  if (!invite || !invite.used || typeof invite.hash !== "string") return;
  const pending = Object.values(m.requests).filter((request) => request.state === "pending" && request.inviteHash === invite.hash);
  const live = pending.some((request) => stillWaiting(request, now));
  if (!live && pending.some((request) => !stillWaiting(request, now))) invite.used = false;
}
function seenAt(request) {
  if (Number.isInteger(request.lastSeen) && request.lastSeen > 0) return request.lastSeen;
  return Number.isInteger(request.created) ? request.created : 0;
}
function stillWaiting(request, now) {
  return !!request && request.state === "pending" && now - seenAt(request) < HEARTBEAT_MS;
}
const startsWith = (b, m) => b.length >= m.length && m.every((x, i) => b[i] === x);
const fromB64 = (s) => {
  if (typeof s !== "string") fail("Unreadable");
  try { return Uint8Array.from(atob(s.trim()), (c) => c.charCodeAt(0)); } catch { fail("Unreadable"); }
};
const accountId = (a) => {
  a = String(a).trim().toLowerCase();
  return /^[0-9a-f]{64}$/.test(a) ? a : fail("Unreadable");
};
// A join may name the account from the link. A bad id is ignored, not an error,
// so a guess that has no account still fails the same way.
function namedAccount(value) {
  const text = typeof value === "string" ? value.trim().toLowerCase() : "";
  return /^[0-9a-f]{64}$/.test(text) ? text : "";
}
function joinBlocked(mailbox, now) {
  const started = Number.isInteger(mailbox.joinFailAt) ? mailbox.joinFailAt : 0;
  if (!started || now - started >= INVITE_MS) return false;
  const fails = Number.isInteger(mailbox.joinFails) ? mailbox.joinFails : 0;
  return fails >= JOIN_TRIES;
}
// One failure for this account, and for its live invite. Eight of either retire the invite.
// The numbers live on the mailbox, so a new address or a new connection does not reset them.
function chargeJoinFailure(mailbox, now) {
  const started = Number.isInteger(mailbox.joinFailAt) ? mailbox.joinFailAt : 0;
  if (!started || now - started >= INVITE_MS) {
    mailbox.joinFails = 0;
    mailbox.joinFailAt = now;
  }
  mailbox.joinFails = (Number.isInteger(mailbox.joinFails) ? mailbox.joinFails : 0) + 1;
  const invite = mailbox.invite;
  if (!invite || invite.used || !(invite.expires > now)) return "";
  invite.tries = (Number.isInteger(invite.tries) ? invite.tries : 0) + 1;
  if (mailbox.joinFails >= JOIN_TRIES || invite.tries >= JOIN_TRIES) {
    invite.used = true;
    return typeof invite.hash === "string" ? invite.hash : "";
  }
  return "";
}
async function retireFailedInvite(txn, account, mailbox) {
  const invite = mailbox.invite;
  if (invite && !invite.used && invite.expires > 0 && typeof invite.hash === "string") {
    invite.used = true;
    await txn.delete("i:" + invite.hash);
  }
  await txn.put("m:" + account, mailbox);
}
function proofBytes(text) {
  if (typeof text !== "string" || !text.trim()) return null;
  try {
    return Uint8Array.from(atob(text.trim()), (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}
const sealed = (b64) => {
  const b = fromB64(b64);
  const v1 = startsWith(b, RECORD_MAGIC) && b.length <= MAX_RECORD;
  const v2 = startsWith(b, RECORD_MAGIC_V2) && b.length <= MAX_MANIFEST;
  if (b.length > 14 && (v1 || v2)) return b64.trim();
  fail("Unreadable");
};
const objectId = (id) => {
  id = String(id || "").trim().toLowerCase();
  return /^[0-9a-f]{32}$/.test(id) ? id : fail("Unreadable");
};
function objectWrites(v) {
  if (!v || v.objects === undefined) return [];
  if (!Array.isArray(v.objects) || v.objects.length > 2000) fail("Unreadable");
  return v.objects.map((item) => {
    const id = objectId(item && item.id);
    const base = item.base;
    if (!Number.isInteger(base) || base < 0) fail("Unreadable");
    const raw = fromB64(item.body);
    if (raw.length < 40 || raw.length > MAX_OBJECT) fail("Unreadable");
    return { id, base, body: String(item.body).trim(), bytes: raw.length };
  });
}
function idList(list) {
  if (list === undefined) return [];
  if (!Array.isArray(list) || list.length > 2000) fail("Unreadable");
  return list.map((item) => objectId(item));
}
async function applyObjects(txn, account, writes, dropping) {
  const seen = new Set();
  const planned = [];
  for (const write of writes) {
    if (seen.has(write.id)) fail("Unreadable");
    seen.add(write.id);
    const key = `o:${account}:${write.id}`;
    const current = await txn.get(key);
    if (write.base === 0) {
      if (current) fail("Revision");
      planned.push([key, { revision: 1, body: write.body, bytes: write.bytes }]);
    } else if (!current || current.revision !== write.base) {
      fail("Revision");
    } else {
      planned.push([key, { revision: current.revision + 1, body: write.body, bytes: write.bytes }]);
    }
  }
  for (const id of dropping) if (seen.has(id)) fail("Unreadable");
  const written = [];
  for (const [key, value] of planned) {
    await txn.put(key, value);
    written.push({ id: key.slice(key.lastIndexOf(":") + 1), revision: value.revision });
  }
  for (const id of dropping) await txn.delete(`o:${account}:${id}`);
  return written;
}
function deviceIdHex(deviceB64) {
  const raw = fromB64(deviceB64);
  return [...raw.subarray(0, 16)].map((b) => b.toString(16).padStart(2, "0")).join("");
}
const cleanName = (raw) => {
  let out = "";
  for (const ch of String(raw || "")) {
    if (/[A-Za-z0-9 -]/.test(ch)) out += ch;
    if (out.length >= 24) break;
  }
  out = out.trim();
  return out || "New device";
};
const freshId = () => [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");

function dropAccessHeaders(request) {
  const headers = new Headers(request.headers);
  headers.delete("x-omacus-access-hash");
  headers.delete("x-omacus-access-uses");
  headers.delete("x-omacus-access-spend");
  headers.delete("x-omacus-admin");
  return headers;
}

// The secret stays in the Worker. The mailbox only receives a hash, and only
// after the code checks out. A client cannot set those headers itself.
async function gate(request, env, url) {
  const create = request.method === "POST" && url.pathname === "/v1/mailboxes";
  const peek = request.method === "POST" && url.pathname === "/v1/access";
  if (!create && !peek) return new Request(request, { headers: dropAccessHeaders(request) });
  const text = await request.text();
  if (text.length > MAX_BODY) return json(400, { message: ERR.Unreadable[1] });
  let body;
  try { body = JSON.parse(text); } catch { body = null; }
  if (!body || typeof body !== "object" || Array.isArray(body)) return json(400, { message: ERR.Unreadable[1] });
  const opened = await openCode(String(env.ACCESS_SECRET || ""), String(body.code || ""));
  if (!opened) return json(400, { message: ERR.BadAccess[1] });
  const headers = dropAccessHeaders(request);
  headers.set("x-omacus-access-hash", opened.hash);
  headers.set("x-omacus-access-uses", String(opened.uses));
  headers.set("x-omacus-access-spend", create ? "1" : "0");
  headers.delete("content-length");
  const forward = create ? { ...body } : {};
  if (create) delete forward.code;
  return new Request(request.url, { method: "POST", headers, body: JSON.stringify(forward) });
}

export class Mailboxes extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.store = ctx.storage;
    this.ipHits = new Map();
    this.accountHits = new Map();
    this.guessHits = new Map();
    this.joinHits = new Map();
    this.readIpHits = new Map();
    this.readAccountHits = new Map();
    this.limits = defaultsFor(env);
  }
  now() {
    return typeof this.clock === "function" ? this.clock() : Date.now();
  }
  bump(map, key, limit) {
    if (!key) return;
    const now = this.now();
    const row = map.get(key);
    if (!row || now - row.start >= 60_000) {
      map.set(key, { start: now, count: 1 });
      return;
    }
    if (row.count >= limit) fail("Limited");
    row.count += 1;
  }
  charge(ip, account) {
    this.bump(this.ipHits, ip, this.limits.ipRate);
    this.bump(this.accountHits, account, this.limits.accountRate);
  }
  chargeGuess(ip) {
    this.bump(this.guessHits, ip, this.limits.guessRate);
  }
  chargeJoin(ip) {
    this.bump(this.joinHits, ip, this.limits.joinRate);
  }
  chargeRead(ip, account) {
    this.bump(this.readIpHits, ip, this.limits.readIpRate);
    this.bump(this.readAccountHits, account, this.limits.readAccountRate);
  }
  // A poll that the read cap then refuses still counts as waiting.
  async noteWaiting(account, id, proofText) {
    const given = proofBytes(proofText);
    if (!given) return;
    try {
      await this.change(account, (m) => {
        const request = Object.hasOwn(m.requests, id) ? m.requests[id] : null;
        const stored = request ? proofBytes(request.proof) : null;
        if (!request || !stored || !sameBytes(given, stored)) return [200, null];
        if (request.state === "pending") request.lastSeen = this.now();
        return [200, null];
      });
    } catch (error) {
      if (error === "Unknown") return;
      throw error;
    }
  }
  async guard(method, path, raw, headers, mailbox) {
    const auth = await authorize(method, path, raw, headers, mailbox.record, Date.now(), mailbox);
    if (auth && auth.busy) fail("Busy");
    if (!auth) fail("Unknown");
    noteActivity(mailbox);
    return auth;
  }
  async fetch(request) {
    const url = new URL(request.url);
    const access = {
      hash: (request.headers.get("x-omacus-access-hash") || "").toLowerCase(),
      uses: Number(request.headers.get("x-omacus-access-uses") || "0"),
      spend: request.headers.get("x-omacus-access-spend") === "1",
    };
    let raw = "";
    let body = {};
    if (request.method !== "GET" && request.method !== "HEAD") {
      raw = await request.text();
      if (raw.length > MAX_BODY) return json(400, { message: ERR.Unreadable[1] });
      try { body = JSON.parse(raw); } catch { body = null; }
    }
    try {
      this.limits = limitsFromHeader(request.headers) || defaultsFor(this.env);
      const admin = request.headers.get("x-omacus-admin") === "1";
      const ip = request.headers.get("cf-connecting-ip") || "local";
      const [status, value] = await this.route(request.method, url.pathname, body, access, admin, request.headers, raw, ip);
      if (status === 304) {
        const revision = value && Number.isInteger(value.revision) ? value.revision : 0;
        return new Response(null, {
          status: 304,
          headers: { ETag: `"${revision}"`, "Cache-Control": "no-store", ...CORS },
        });
      }
      const extra = {};
      if (value && typeof value.record === "string" && Number.isInteger(value.revision)) {
        extra.ETag = `"${value.revision}"`;
        extra["Cache-Control"] = "no-store";
      }
      return json(status, value, extra);
    } catch (e) {
      const name = Object.hasOwn(ERR, e) ? e : "Other";
      try { await noteError(this.store, name); } catch { /* the reply still names the failure */ }
      const [status, message] = ERR[e] || [500, "Try again in a moment."];
      return json(status, { message });
    }
  }
  async load(account) {
    return (await this.store.get("m:" + account)) || fail("Unknown");
  }
  async save(account, m) { await this.store.put("m:" + account, m); }
  // A put and an approval can overlap on a slow phone. One transaction keeps
  // both changes; the last write must not roll the saved pages back.
  async change(account, update) {
    let out;
    await this.store.transaction(async (txn) => {
      const m = (await txn.get("m:" + account)) || fail("Unknown");
      dropExpired(m, this.now());
      out = await update(m, txn);
      await txn.put("m:" + account, m);
    });
    return out;
  }

  async useAccess(access, take, store = this.store) {
    const hash = access.hash;
    const uses = access.uses;
    if (!/^[0-9a-f]{64}$/.test(hash) || !Number.isInteger(uses) || uses < 1 || uses > MAX_USES) fail("BadAccess");
    const key = "a:" + hash;
    const row = await store.get(key);
    if (row && row.revoked) fail("BadAccess");
    if (row && Number.isInteger(row.expires) && row.expires > 0 && row.expires <= Date.now()) fail("BadAccess");
    const table = {};
    if (row && Number.isInteger(row.left)) table[hash] = row.left;
    if (!spend(table, hash, uses, take)) fail("BadAccess");
    if (!take) return;
    const next = row && typeof row === "object" ? { ...row } : {};
    next.left = table[hash];
    next.usedAt = Date.now();
    await store.put(key, next);
  }

  async route(method, path, body, access, admin, headers, raw, ip) {
    if (path.startsWith("/v1/admin")) return handleAdmin(this.store, method, path, body, admin === true);
    if (path.startsWith("/v1/internal/")) return this.internal(method, path, body, headers, access);
    const need = () => (body && typeof body === "object" ? body : fail("Unreadable"));
    if (method === "POST" && path === "/v1/access") {
      this.charge(ip);
      this.chargeGuess(ip);
      await this.useAccess(access, false);
      return [200, { ok: true }];
    }
    if (method === "POST" && path === "/v1/join") {
      this.charge(ip);
      // A burst from one address is still refused. That is not the guess limit:
      // a wrong code is counted on the invite and on the account.
      this.chargeJoin(ip);
      return this.join(need());
    }
    if (method === "POST" && path === "/v1/mailboxes") {
      this.charge(ip);
      const v = need();
      const account = accountId(typeof v.account === "string" ? v.account : fail("Unreadable"));
      const record = sealed(v.record);
      const writes = objectWrites(v);
      const blocked = rejectSignup({ devices: linkedDevices(fromB64(record)), bytes: storedBytes(record, writes) }, this.limits);
      if (blocked) fail(blocked);
      const spent = headers && headers.get && headers.get("x-omacus-access-spent") === "1";
      if (!spent) {
        if (!access.spend) fail("BadAccess");
        const signups = await this.store.get("admin:signups");
        if (signups && signups.paused) fail("Paused");
        await this.useAccess(access, true);
      }
      let written = [];
      await this.store.transaction(async (txn) => {
        if (await txn.get("m:" + account)) fail("Conflict");
        const now = Date.now();
        await txn.put("m:" + account, {
          revision: 1,
          record,
          requests: {},
          created: now,
          changed: now,
          used: 0,
          hours: [],
        });
        written = await applyObjects(txn, account, writes, []);
      });
      return [201, { revision: 1, objects: written }];
    }
    const rest = path.startsWith("/v1/mailboxes/") ? path.slice(14).split("/") : fail("Unknown");
    const [sub, id, extra] = rest.slice(1);
    if (extra !== undefined) fail("Unknown");
    this.charge(ip);
    const account = accountId(rest[0]);
    this.charge(null, account);
    const header = (name) => (headers && headers.get ? headers.get(name) : "") || "";
    if (method === "GET" && sub === "requests" && id !== undefined) {
      await this.noteWaiting(account, id, header("x-omacus-proof"));
    }
    if (method === "GET" || (method === "POST" && sub === "objects" && id === undefined)) {
      this.chargeRead(ip, account);
    }
    if (method === "GET" && sub === "requests" && id !== undefined) {
      const given = proofBytes(header("x-omacus-proof"));
      const [status, answer] = await this.change(account, (m) => {
        const r = Object.hasOwn(m.requests, id) ? m.requests[id] : null;
        const stored = r ? proofBytes(r.proof) : null;
        if (!r || !given || !stored || !sameBytes(given, stored)) return [404, null];
        const reply = { state: r.state, name: r.name, device: r.device, chain: r.chain, wrap: r.wrap };
        if (r.state === "pending") r.lastSeen = this.now();
        else if (r.state !== "approved") delete m.requests[id];
        return [200, reply];
      });
      if (status === 404) fail("Unknown");
      return [status, answer];
    }
    const authed = (m) => this.guard(method, path, raw, headers, m);
    if (method === "GET" && sub === undefined) {
      return this.change(account, async (m, txn) => {
        await authed(m);
        if (header("if-none-match").trim() === `"${m.revision}"`) return [304, { revision: m.revision }];
        const listed = await txn.list({ prefix: `o:${account}:` });
        const objects = [...listed].map(([key, value]) => ({
          id: key.slice(`o:${account}:`.length),
          revision: value.revision,
          bytes: value.bytes,
        }));
        objects.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
        return [200, { revision: m.revision, record: m.record, objects }];
      });
    }
    if (method === "PUT" && sub === undefined) {
      return this.change(account, async (m, txn) => {
        const auth = await authed(m);
        const v = need();
        if (!Number.isInteger(v.revision) || v.revision < 0) fail("Unreadable");
        const record = sealed(v.record);
        if (!await allowPut(m.record, record, auth.recovery)) fail("Unreadable");
        if (m.revision !== v.revision) fail("Revision");
        if (header("if-match").trim() !== `"${m.revision}"`) fail("Revision");
        const writes = objectWrites(v);
        const dropping = idList(v.drop);
        const blocked = await rejectStored(txn, account, record, writes, dropping, this.limits);
        if (blocked) fail(blocked);
        const written = await applyObjects(txn, account, writes, dropping);
        m.revision += 1;
        m.record = record;
        m.changed = Date.now();
        return [200, { revision: m.revision, objects: written }];
      });
    }
    if (method === "POST" && sub === "objects" && id === undefined) {
      await this.change(account, async (m) => {
        await authed(m);
        return [200, { ok: true }];
      });
      const ids = idList(need().ids);
      const objects = [];
      for (const object of ids) {
        const cur = await this.store.get(`o:${account}:${object}`);
        if (!cur) fail("Unknown");
        objects.push({ id: object, revision: cur.revision, body: cur.body });
      }
      return [200, { objects }];
    }
    if (method === "POST" && sub === "invites" && id === undefined) {
      return this.change(account, async (m, txn) => {
        await authed(m);
        const v = need();
        const hash = String(v.hash || "").trim().toLowerCase();
        if (!/^[0-9a-f]{64}$/.test(hash)) fail("Unreadable");
        if (m.invite && m.invite.hash && m.invite.hash !== hash) await txn.delete("i:" + m.invite.hash);
        const expires = Date.now() + INVITE_MS;
        m.invite = { hash, expires, used: false, tries: 0 };
        await txn.put("i:" + hash, { account, expires });
        return [200, { ok: true }];
      });
    }
    if (sub !== "requests") fail("Unknown");
    if (method === "GET" && id === undefined) {
      return this.change(account, async (m) => {
        await authed(m);
        return [200, {
          requests: Object.entries(m.requests)
            .filter(([, r]) => r.state === "approved" || stillWaiting(r, this.now()))
            .map(([rid, r]) => ({
              id: rid,
              device: r.device,
              name: r.name,
              proof: r.proof || "",
              state: r.state,
              created: r.created || 0,
            })),
        }];
      });
    }
    if (method === "POST" && id !== undefined) {
      return this.change(account, async (m, txn) => {
        const auth = await authed(m);
        const v = need();
        const r = Object.hasOwn(m.requests, id) ? m.requests[id] : fail("Unknown");
        if (r.state === "pending" && !stillWaiting(r, this.now())) fail("Waiting");
        if (v.reject === true) {
          if (r.state !== "pending") fail("Conflict");
          r.state = "rejected";
          return [200, { ok: true }];
        }
        if (v.release === true) {
          if (r.state !== "approved") fail("Conflict");
          delete m.requests[id];
          if (m.invite && m.invite.hash) {
            m.invite.used = false;
            await txn.put("i:" + m.invite.hash, { account, expires: m.invite.expires });
          }
          return [200, { ok: true }];
        }
        if (v.done === true) {
          if (r.state !== "approved") fail("Conflict");
          const signer = header("x-omacus-device").trim().toLowerCase();
          if (!signer || signer !== deviceIdHex(r.device)) fail("Unknown");
          delete m.requests[id];
          if (m.invite && typeof m.invite.hash === "string") await txn.delete("i:" + m.invite.hash);
          return [200, { ok: true }];
        }
        const chain = fromB64(v.chain), wrap = fromB64(v.wrap);
        if (chain.length > MAX_CHAIN || !startsWith(chain, CHAIN_MAGIC) || !wrap.length || wrap.length > MAX_WRAP) fail("Unreadable");
        if (r.state !== "pending") fail("Conflict");
        const record = sealed(v.record);
        if (!await deviceListed(record, r.device)) fail("Unreadable");
        if (!await allowPut(m.record, record, auth.recovery)) fail("Unreadable");
        if (!Number.isInteger(v.revision) || v.revision !== m.revision) fail("Revision");
        if (header("if-match").trim() !== `"${m.revision}"`) fail("Revision");
        const writes = objectWrites(v);
        const dropping = idList(v.drop);
        const blocked = await rejectStored(txn, account, record, writes, dropping, this.limits);
        if (blocked) fail(blocked);
        const written = await applyObjects(txn, account, writes, dropping);
        m.revision += 1;
        m.record = record;
        m.changed = Date.now();
        r.state = "approved";
        r.chain = v.chain.trim();
        r.wrap = v.wrap.trim();
        return [200, { ok: true, revision: m.revision, objects: written }];
      });
    }
    fail("Unknown");
  }

  async noteJoinMiss(account) {
    const now = this.now();
    await this.store.transaction(async (txn) => {
      const mailbox = await txn.get("m:" + account);
      if (!mailbox) return;
      if (joinBlocked(mailbox, now)) {
        await retireFailedInvite(txn, account, mailbox);
        return;
      }
      const burned = chargeJoinFailure(mailbox, now);
      if (burned) await txn.delete("i:" + burned);
      await txn.put("m:" + account, mailbox);
    });
  }

  async join(v) {
    const named = namedAccount(v.account);
    const secret = decodeCode(v.code);
    if (!secret) {
      if (named) await this.noteJoinMiss(named);
      fail("BadInvite");
    }
    const device = fromB64(v.device);
    if (device.length !== 80) fail("Unreadable");
    const given = fromB64(v.proof);
    if (given.length !== 32) fail("Unreadable");
    const hash = await hashHex(secret);
    const now = this.now();
    const proofOk = sameBytes(await hmacBytes(secret, device), given);
    const id = freshId();
    const proof = btoa(String.fromCharCode(...given));
    let result = "bad";
    await this.store.transaction(async (txn) => {
      const indexed = await txn.get("i:" + hash);
      const account = indexed && typeof indexed.account === "string" ? indexed.account : "";
      const mailbox = /^[0-9a-f]{64}$/.test(account) ? await txn.get("m:" + account) : null;
      // A phone that stops waiting gives the code back before this check is a guess.
      if (mailbox) dropExpired(mailbox, now);
      const invite = mailbox && mailbox.invite;
      const live = !!(mailbox && invite && invite.expires > now && invite.hash === hash && !invite.used);
      if (mailbox && joinBlocked(mailbox, now)) {
        await retireFailedInvite(txn, account, mailbox);
        return;
      }
      if (!live) {
        // The proof matches a code that is only in use. That is not a guess.
        if (proofOk && mailbox && invite && invite.hash === hash) {
          await txn.put("m:" + account, mailbox);
          return;
        }
        if (mailbox && invite && invite.hash === hash) {
          const burned = chargeJoinFailure(mailbox, now);
          if (burned) await txn.delete("i:" + burned);
          await txn.put("m:" + account, mailbox);
          return;
        }
        if (!named) {
          if (mailbox) await txn.put("m:" + account, mailbox);
          return;
        }
        const target = await txn.get("m:" + named);
        if (!target) return;
        if (joinBlocked(target, now)) {
          await retireFailedInvite(txn, named, target);
          return;
        }
        const burned = chargeJoinFailure(target, now);
        if (burned) await txn.delete("i:" + burned);
        await txn.put("m:" + named, target);
        return;
      }
      const tries = Number.isInteger(mailbox.invite.tries) ? mailbox.invite.tries : 0;
      if (!mailbox.invite || mailbox.invite.used || tries >= JOIN_TRIES) return;
      if (!proofOk) {
        const burned = chargeJoinFailure(mailbox, now);
        if (burned) await txn.delete("i:" + burned);
        await txn.put("m:" + account, mailbox);
        return;
      }
      const pending = Object.values(mailbox.requests).filter((request) => request.state === "pending").length;
      if (pending >= MAX_REQUESTS) {
        result = "full";
        return;
      }
      mailbox.invite.used = true;
      mailbox.requests[id] = {
        device: btoa(String.fromCharCode(...device)),
        name: cleanName(typeof v.name === "string" ? v.name : ""),
        state: "pending",
        chain: "",
        wrap: "",
        proof,
        inviteHash: hash,
        created: now,
        lastSeen: now,
      };
      await txn.put("m:" + account, mailbox);
      result = [201, { id, account }];
    });
    if (result === "full") fail("Full");
    if (result === "bad") fail("BadInvite");
    return result;
  }

  async internal(method, path, body, headers, access) {
    if (!headers || !headers.get || headers.get("x-omacus-internal") !== "1") fail("Unknown");
    const value = body && typeof body === "object" ? body : {};
    const probe = /^\/v1\/internal\/probe\/([0-9a-f]{64})$/.exec(path);
    if (method === "GET" && probe) {
      if (!(await this.store.get("m:" + probe[1]))) fail("Unknown");
      return [200, { present: true }];
    }
    const copy = /^\/v1\/internal\/copy\/([0-9a-f]{64})$/.exec(path);
    if (method === "POST" && copy) {
      const account = copy[1];
      const mailbox = await this.store.get("m:" + account);
      if (!mailbox) fail("Unknown");
      const listed = await this.store.list({ prefix: `o:${account}:` });
      const objects = [...listed].map(([key]) => key.slice(`o:${account}:`.length));
      return [200, { mailbox, objects }];
    }
    const objectPath = /^\/v1\/internal\/object\/([0-9a-f]{64})\/([0-9a-f]{32})$/.exec(path);
    if (objectPath && method === "GET") {
      const row = await this.store.get(`o:${objectPath[1]}:${objectPath[2]}`);
      if (!row) fail("Unknown");
      return [200, row];
    }
    if (objectPath && method === "PUT") {
      const key = `o:${objectPath[1]}:${objectPath[2]}`;
      if (!Number.isInteger(value.revision) || typeof value.body !== "string") fail("Unreadable");
      if (value.replace === true || !(await this.store.get(key))) {
        await this.store.put(key, {
          revision: value.revision,
          body: value.body,
          bytes: Number.isInteger(value.bytes) ? value.bytes : 0,
        });
      }
      return [200, { ok: true }];
    }
    const importing = /^\/v1\/internal\/import\/([0-9a-f]{64})$/.exec(path);
    if (method === "POST" && importing) {
      const account = importing[1];
      const mailbox = value.mailbox;
      if (!mailbox || typeof mailbox !== "object" || typeof mailbox.record !== "string") fail("Unreadable");
      await this.store.transaction(async (txn) => {
        if (!(await txn.get("m:" + account))) await txn.put("m:" + account, mailbox);
        const invite = mailbox.invite;
        if (invite && /^[0-9a-f]{64}$/.test(invite.hash || "") && !invite.used) {
          await txn.put("i:" + invite.hash, { account, expires: invite.expires });
        }
      });
      return [200, { ok: true }];
    }
    const drop = /^\/v1\/internal\/drop\/([0-9a-f]{64})$/.exec(path);
    if (method === "POST" && drop) {
      const account = drop[1];
      const mailbox = await this.store.get("m:" + account);
      const objects = await this.store.list({ prefix: `o:${account}:` });
      for (const key of objects.keys()) await this.store.delete(key);
      if (mailbox && mailbox.invite && mailbox.invite.hash) await this.store.delete("i:" + mailbox.invite.hash);
      await this.store.delete("m:" + account);
      await this.store.delete("r:" + account);
      return [200, { ok: true }];
    }
    if (method === "POST" && path === "/v1/internal/claim") {
      const account = accountId(typeof value.account === "string" ? value.account : fail("Unreadable"));
      sealed(value.record);
      objectWrites(value);
      if (!access.spend) fail("BadAccess");
      await this.store.transaction(async (txn) => {
        if ((await txn.get("r:" + account)) || (await txn.get("m:" + account))) fail("Conflict");
        const signups = await txn.get("admin:signups");
        if (signups && signups.paused) fail("Paused");
        const listed = await txn.list({ prefix: "r:" });
        let count = 0;
        for (const key of listed.keys()) if (/^r:[0-9a-f]{64}$/.test(key)) count += 1;
        if (count >= this.limits.maxAccounts) fail("SignupFull");
        await this.useAccess(access, true, txn);
        await txn.put("r:" + account, { pending: true, bytes: 0, devices: 0, invite: "" });
      });
      return [201, { ok: true }];
    }
    if (method === "POST" && path === "/v1/internal/release") {
      const account = accountId(typeof value.account === "string" ? value.account : fail("Unreadable"));
      const row = await this.store.get("r:" + account);
      if (row && row.pending) await this.store.delete("r:" + account);
      return [200, { ok: true }];
    }
    if (method === "POST" && path === "/v1/internal/registry") {
      const account = accountId(typeof value.account === "string" ? value.account : fail("Unreadable"));
      const prev = (await this.store.get("r:" + account)) || {};
      await this.store.put("r:" + account, {
        pending: false,
        bytes: Number.isInteger(value.bytes) ? value.bytes : 0,
        devices: Number.isInteger(value.devices) ? value.devices : 0,
        invite: typeof prev.invite === "string" ? prev.invite : "",
      });
      return [200, { ok: true }];
    }
    const inviteHash = /^\/v1\/internal\/invite-hash\/([0-9a-f]{64})$/.exec(path);
    if (method === "GET" && inviteHash) {
      const mailbox = await this.store.get("m:" + inviteHash[1]);
      const hash = mailbox && mailbox.invite && typeof mailbox.invite.hash === "string" ? mailbox.invite.hash : "";
      if (!/^[0-9a-f]{64}$/.test(hash)) fail("Unknown");
      return [200, { hash }];
    }
    const invite = /^\/v1\/internal\/invite\/([0-9a-f]{64})$/.exec(path);
    if (invite && method === "PUT") {
      const account = accountId(typeof value.account === "string" ? value.account : fail("Unreadable"));
      const hash = invite[1];
      const prev = (await this.store.get("r:" + account)) || { bytes: 0, devices: 0 };
      if (prev.invite && prev.invite !== hash) await this.store.delete("i:" + prev.invite);
      prev.invite = hash;
      prev.pending = false;
      await this.store.put("r:" + account, prev);
      await this.store.put("i:" + hash, { account, expires: Number(value.expires) || 0 });
      return [200, { ok: true }];
    }
    if (invite && method === "GET") {
      const account = await indexedAccount(this.store, invite[1]);
      if (!account) fail("BadInvite");
      return [200, { account }];
    }
    if (invite && method === "DELETE") {
      await this.store.delete("i:" + invite[1]);
      return [200, { ok: true }];
    }
    const size = /^\/v1\/internal\/size\/([0-9a-f]{64})$/.exec(path);
    if (method === "GET" && size) {
      const foot = await footprint(this.store, size[1]);
      if (!foot) fail("Unknown");
      return [200, foot];
    }
    if (method === "GET" && path === "/v1/internal/accounts") {
      const ids = new Set();
      const registered = await this.store.list({ prefix: "r:" });
      for (const [key, row] of registered) {
        const account = key.slice(2);
        if (/^[0-9a-f]{64}$/.test(account) && row && !row.pending) ids.add(account);
      }
      const mailboxes = await this.store.list({ prefix: "m:" });
      for (const key of mailboxes.keys()) {
        const account = key.slice(2);
        if (/^[0-9a-f]{64}$/.test(account)) ids.add(account);
      }
      return [200, { accounts: [...ids].sort() }];
    }
    const revising = /^\/v1\/internal\/revise\/([0-9a-f]{64})$/.exec(path);
    if (method === "POST" && revising) return this.revise(revising[1], value);
    const replacing = /^\/v1\/internal\/replace\/([0-9a-f]{64})$/.exec(path);
    if (method === "POST" && replacing) {
      const account = replacing[1];
      const mailbox = value.mailbox;
      if (!mailbox || typeof mailbox !== "object" || typeof mailbox.record !== "string") fail("Unreadable");
      const keep = new Set(
        Array.isArray(value.keep) ? value.keep.filter((id) => /^[0-9a-f]{32}$/.test(id)) : [],
      );
      await this.store.put("m:" + account, mailbox);
      const objects = await this.store.list({ prefix: `o:${account}:` });
      for (const key of objects.keys()) {
        const id = key.slice(`o:${account}:`.length);
        if (!keep.has(id)) await this.store.delete(key);
      }
      return [200, { ok: true }];
    }
    fail("Unknown");
  }

  // The same compare-and-swap a phone uses when it saves. The revision
  // moves forward. Objects in the night are written. Objects that are not
  // in the night are dropped in that same commit. A current device list
  // that already extends the restored one stays, and so do its pages.
  async revise(account, value) {
    const record = sealed(value.record);
    const incoming = Array.isArray(value.objects) ? value.objects : fail("Unreadable");
    if (incoming.length > 2000) fail("Unreadable");
    let revision = 0;
    let written = 0;
    let keptList = false;
    await this.store.transaction(async (txn) => {
      const current = await txn.get("m:" + account);
      if (current && await chainIsLongerExtension(current.record, record)) {
        revision = current.revision;
        keptList = true;
        return;
      }
      const listed = await txn.list({ prefix: `o:${account}:` });
      const have = new Map();
      for (const [key, row] of listed) have.set(key.slice(`o:${account}:`.length), row);
      const writes = [];
      const keep = new Set();
      for (const item of incoming) {
        const id = objectId(item && item.id);
        if (keep.has(id)) fail("Unreadable");
        keep.add(id);
        const raw = fromB64(item.body);
        if (!raw || raw.length < 40 || raw.length > MAX_OBJECT) fail("Unreadable");
        const existing = have.get(id);
        writes.push({
          id,
          base: existing ? existing.revision : 0,
          body: String(item.body).trim(),
          bytes: raw.length,
        });
      }
      const dropping = [...have.keys()].filter((id) => !keep.has(id));
      await applyObjects(txn, account, writes, dropping);
      written = keep.size;
      const invite = value.invite && typeof value.invite === "object" ? value.invite : null;
      if (!current) {
        const now = Date.now();
        const created = { revision: 1, record, requests: {}, created: now, changed: now, used: 0, hours: [] };
        if (invite && /^[0-9a-f]{64}$/.test(invite.hash || "")) {
          created.invite = { hash: invite.hash, expires: Number(invite.expires) || 0, used: invite.used === true, tries: Number.isInteger(invite.tries) ? invite.tries : 0 };
        }
        await txn.put("m:" + account, created);
        revision = 1;
        return;
      }
      current.revision += 1;
      current.record = record;
      current.changed = Date.now();
      if (invite && /^[0-9a-f]{64}$/.test(invite.hash || "")) {
        current.invite = { hash: invite.hash, expires: Number(invite.expires) || 0, used: invite.used === true, tries: Number.isInteger(invite.tries) ? invite.tries : 0 };
      }
      await txn.put("m:" + account, current);
      revision = current.revision;
    });
    return [200, { ok: true, revision, objects: written, keptList }];
  }
}

export function clearPlacement() {
  homed.clear();
}

function box(env, name) {
  return env.MAILBOXES.get(env.MAILBOXES.idFromName(name));
}

function accountObject(account) {
  return `acct-${account}`;
}

function storedBytes(record, writes) {
  let bytes = fromB64(record).length;
  for (const write of writes) bytes += write.bytes;
  return bytes;
}

async function rejectStored(txn, account, record, writes, dropping, limits) {
  const drop = new Set(dropping);
  const replacing = new Map();
  for (const write of writes) {
    if (drop.has(write.id)) return "Unreadable";
    replacing.set(write.id, write.bytes);
  }
  let bytes = fromB64(record).length;
  const objects = await txn.list({ prefix: `o:${account}:` });
  for (const [key, row] of objects) {
    const id = key.slice(key.lastIndexOf(":") + 1);
    if (drop.has(id)) continue;
    if (replacing.has(id)) {
      bytes += replacing.get(id);
      replacing.delete(id);
    } else {
      bytes += row && Number.isInteger(row.bytes) ? row.bytes : 0;
    }
  }
  for (const size of replacing.values()) bytes += size;
  return rejectSignup({ devices: linkedDevices(fromB64(record)), bytes }, limits);
}

function signupFits(body, limits) {
  if (!body || typeof body.record !== "string") return "";
  let record;
  try { record = fromB64(body.record); } catch { return ""; }
  let bytes = record.length;
  if (Array.isArray(body.objects)) {
    for (const item of body.objects) {
      if (!item || typeof item.body !== "string") return "";
      try { bytes += fromB64(item.body).length; } catch { return ""; }
    }
  }
  return rejectSignup({ devices: linkedDevices(record), bytes }, limits);
}

export class Config extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.store = ctx.storage;
    this.env = env;
  }

  async fetch(request) {
    if (request.headers.get("x-omacus-internal") !== "1") return json(404, { message: ERR.Unknown[1] });
    const url = new URL(request.url);
    if (url.pathname !== "/v1/internal/limits") return json(404, { message: "Not found." });
    if (request.method === "GET") return json(200, await this.effective());
    if (request.method !== "PUT") return json(404, { message: "Not found." });
    let body = {};
    try { body = await request.json(); } catch { return json(400, { message: "That limit didn't work." }); }
    const next = normalizeLimits(body, await this.numbers());
    if (!next) return json(400, { message: "That limit didn't work." });
    const stored = { ...next, updated: Date.now() };
    await this.store.put(LIMIT_KEY, stored);
    return json(200, { ...stored, saved: true });
  }

  async numbers() {
    const stored = await this.store.get(LIMIT_KEY);
    const base = defaultsFor(this.env);
    if (!stored) return base;
    return normalizeLimits(stored, base) || base;
  }

  async effective() {
    const stored = await this.store.get(LIMIT_KEY);
    const numbers = await this.numbers();
    const saved = Boolean(stored) && Boolean(normalizeLimits(stored, defaultsFor(this.env)));
    return {
      ...numbers,
      saved,
      updated: stored && Number.isInteger(stored.updated) ? stored.updated : 0,
    };
  }
}

async function currentLimits(env) {
  const fallback = defaultsFor(env);
  if (!env.CONFIG || typeof env.CONFIG.get !== "function" || typeof env.CONFIG.idFromName !== "function") {
    return fallback;
  }
  try {
    const response = await env.CONFIG.get(env.CONFIG.idFromName("index")).fetch(new Request(
      "https://sync.omacus.com/v1/internal/limits",
      { headers: internalHeaders() },
    ));
    if (!response.ok) return fallback;
    return normalizeLimits(await response.json(), fallback) || fallback;
  } catch {
    return fallback;
  }
}

async function adminLimits(env, method, payload) {
  if (!env.CONFIG || typeof env.CONFIG.get !== "function") return json(404, { message: "Not found." });
  const headers = internalHeaders();
  const request = method === "GET"
    ? new Request("https://sync.omacus.com/v1/internal/limits", { headers })
    : new Request("https://sync.omacus.com/v1/internal/limits", {
      method: "PUT",
      headers,
      body: JSON.stringify(payload || {}),
    });
  const response = await env.CONFIG.get(env.CONFIG.idFromName("index")).fetch(request);
  const body = await response.json().catch(() => ({ message: "Try again in a moment." }));
  return json(response.status, body);
}

function internalJson(path, method = "GET", body) {
  return new Request("https://sync.omacus.com" + path, {
    method,
    headers: internalHeaders(),
    body: body === undefined ? undefined : JSON.stringify(body),
  });
}

async function sealedAccounts(env) {
  const ids = new Set();
  for (const name of [DIRECTORY, LEGACY]) {
    const response = await box(env, name).fetch(internalJson("/v1/internal/accounts"));
    if (!response.ok) continue;
    const body = await response.json().catch(() => ({}));
    for (const account of body.accounts || []) {
      if (/^[0-9a-f]{64}$/.test(account)) ids.add(account);
    }
  }
  return [...ids].sort();
}

async function snapshotAccount(env, account) {
  for (const name of [accountObject(account), LEGACY]) {
    const copy = await box(env, name).fetch(internalJson(`/v1/internal/copy/${account}`, "POST", {}));
    if (!copy.ok) continue;
    const payload = await copy.json();
    if (!payload || typeof payload.mailbox !== "object" || !payload.mailbox) continue;
    const objects = [];
    let complete = true;
    for (const id of payload.objects || []) {
      if (!/^[0-9a-f]{32}$/.test(id)) continue;
      const object = await box(env, name).fetch(internalJson(`/v1/internal/object/${account}/${id}`));
      if (!object.ok) {
        complete = false;
        break;
      }
      objects.push({ id, row: await object.json() });
    }
    if (!complete) return null;
    return { mailbox: payload.mailbox, objects };
  }
  return null;
}

function snapshotBytes(mailbox, objects) {
  let bytes = 0;
  try { bytes += fromB64(mailbox.record).length; } catch { bytes += 0; }
  for (const object of objects) {
    bytes += object.row && Number.isInteger(object.row.bytes) ? object.row.bytes : 0;
  }
  return bytes;
}

export async function runBackup(env, when = Date.now()) {
  const store = openBackupStore(env);
  if (!store) return null;
  const day = nightOf(when);
  const prefix = nightPrefix(day);
  const accounts = [];
  const errors = [];
  let failed = 0;
  for (const account of await sealedAccounts(env)) {
    const shot = await snapshotAccount(env, account);
    if (!shot) {
      failed += 1;
      errors.push({ account, message: "That copy didn't save." });
      accounts.push({ account, objects: 0, bytes: 0, ok: false });
      continue;
    }
    await store.put(`${prefix}accounts/${account}/mailbox.json`, JSON.stringify({
      record: shot.mailbox.record,
      invite: shot.mailbox.invite || null,
    }));
    for (const object of shot.objects) {
      await store.put(
        `${prefix}accounts/${account}/objects/${object.id}.json`,
        JSON.stringify(object.row),
      );
    }
    accounts.push({
      account,
      objects: shot.objects.length,
      bytes: snapshotBytes(shot.mailbox, shot.objects),
      ok: true,
    });
  }
  const index = {
    day,
    at: when,
    accounts,
    failed,
    errors,
    status: failed === 0 ? "saved" : "incomplete",
  };
  await store.put(`${prefix}index.json`, JSON.stringify(index));
  return index;
}

async function readBackup(store, account, day) {
  const prefix = `${nightPrefix(day)}accounts/${account}/`;
  const mailboxText = await store.get(`${prefix}mailbox.json`);
  if (!mailboxText) return null;
  const mailbox = JSON.parse(mailboxText);
  const keys = (await store.list(`${prefix}objects/`)).sort();
  const objects = [];
  for (const key of keys) {
    const id = key.slice(key.lastIndexOf("/") + 1).replace(/\.json$/, "");
    if (!/^[0-9a-f]{32}$/.test(id)) continue;
    const text = await store.get(key);
    if (!text) continue;
    objects.push({ id, row: JSON.parse(text) });
  }
  return { mailbox, objects };
}

async function restoreSealed(env, account, day) {
  const store = openBackupStore(env);
  if (!store) return null;
  const shot = await readBackup(store, account, day);
  if (!shot || !shot.mailbox || typeof shot.mailbox.record !== "string") return null;
  const home = box(env, accountObject(account));
  const revised = await home.fetch(internalJson(`/v1/internal/revise/${account}`, "POST", {
    record: shot.mailbox.record,
    invite: shot.mailbox.invite,
    objects: shot.objects.map((object) => ({
      id: object.id,
      body: object.row && object.row.body,
      bytes: object.row && object.row.bytes,
    })),
  }));
  if (!revised.ok) return null;
  const body = await revised.json().catch(() => ({}));
  await box(env, LEGACY).fetch(internalJson(`/v1/internal/drop/${account}`, "POST", {}));
  const invite = shot.mailbox.invite;
  if (invite && /^[0-9a-f]{64}$/.test(invite.hash || "") && !invite.used) {
    await box(env, DIRECTORY).fetch(internalJson(`/v1/internal/invite/${invite.hash}`, "PUT", {
      account,
      expires: invite.expires,
    }));
  }
  const size = await home.fetch(internalJson(`/v1/internal/size/${account}`));
  if (size.ok) {
    const foot = await size.json();
    await box(env, DIRECTORY).fetch(internalJson("/v1/internal/registry", "POST", {
      account,
      bytes: foot.bytes,
      devices: foot.devices,
    }));
  }
  homed.add(account);
  return { objects: body.keptList ? 0 : shot.objects.length, revision: body.revision, keptList: body.keptList === true };
}

function nightSummary(row) {
  const accounts = Array.isArray(row.accounts) ? row.accounts : [];
  const failed = Number.isInteger(row.failed) ? row.failed : accounts.filter((item) => item && item.ok === false).length;
  return {
    day: row.day,
    at: Number.isInteger(row.at) ? row.at : 0,
    accounts: accounts.length,
    failed,
    status: row.status === "incomplete" || failed > 0 ? "incomplete" : "saved",
    rows: accounts.filter((item) => item && /^[0-9a-f]{64}$/.test(item.account || "")).map((item) => ({
      account: item.account,
      objects: item.objects || 0,
      bytes: item.bytes || 0,
      ok: item.ok !== false,
    })),
    errors: Array.isArray(row.errors) ? row.errors.filter((item) => item && /^[0-9a-f]{64}$/.test(item.account)).map((item) => ({
      account: item.account,
      message: "That copy didn't save.",
    })) : [],
  };
}

function copySummary(row) {
  const accounts = Array.isArray(row.accounts) ? row.accounts : [];
  let bytes = 0;
  for (const item of accounts) {
    if (item && Number.isInteger(item.bytes) && item.bytes > 0) bytes += item.bytes;
  }
  const failed = Number.isInteger(row.failed) ? row.failed : accounts.filter((item) => item && item.ok === false).length;
  return {
    at: Number.isInteger(row.at) ? row.at : 0,
    status: row.status === "incomplete" || failed > 0 ? "incomplete" : "saved",
    bytes,
  };
}

async function latestCopy(env) {
  try {
    const store = openBackupStore(env);
    if (!store) return null;
    const keys = (await store.list("nights/")).filter((key) => key.endsWith("/index.json"));
    let best = null;
    let bestDay = "";
    for (const key of keys) {
      const text = await store.get(key);
      if (!text) continue;
      let row;
      try { row = JSON.parse(text); } catch { continue; }
      if (!row || typeof row.day !== "string") continue;
      if (!best || row.day > bestDay) {
        best = row;
        bestDay = row.day;
      }
    }
    if (!best) return null;
    const summary = copySummary(best);
    return summary.at ? summary : null;
  } catch {
    return null;
  }
}

async function listBackups(env) {
  const store = openBackupStore(env);
  if (!store) return { ready: false, backups: [], last: null };
  const keys = (await store.list("nights/")).filter((key) => key.endsWith("/index.json"));
  const backups = [];
  for (const key of keys) {
    const text = await store.get(key);
    if (!text) continue;
    let row;
    try { row = JSON.parse(text); } catch { continue; }
    if (!row || typeof row.day !== "string") continue;
    backups.push(nightSummary(row));
  }
  backups.sort((a, b) => (a.day < b.day ? 1 : a.day > b.day ? -1 : 0));
  const newest = backups[0];
  const last = newest ? {
    day: newest.day,
    at: newest.at,
    accounts: newest.rows,
    failed: newest.failed,
    status: newest.status,
    errors: newest.errors,
  } : null;
  return {
    ready: true,
    last,
    backups: backups.map((row) => ({
      day: row.day,
      at: row.at,
      accounts: row.accounts,
      failed: row.failed,
      status: row.status,
    })),
  };
}

async function adminBackups(env, method, payload) {
  if (method === "GET") return json(200, await listBackups(env));
  if (!env.BACKUPS || typeof env.BACKUPS.put !== "function") {
    return json(400, { message: "Backups are not set up." });
  }
  const index = await runBackup(env, Date.now());
  return json(200, {
    ready: true,
    day: index.day,
    accounts: index.accounts.length,
    failed: index.failed,
  });
}

async function adminRestore(env, payload) {
  const value = payload && typeof payload === "object" ? payload : {};
  const date = String(value.date || "");
  const confirm = String(value.confirm || "").trim().toLowerCase();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return json(400, { message: "That day didn't work." });
  if (!openBackupStore(env)) return json(400, { message: "Backups are not set up." });
  if (value.all === true) {
    if (confirm !== date) return json(400, { message: "Type the day to confirm." });
    const store = openBackupStore(env);
    const text = await store.get(`${nightPrefix(date)}index.json`);
    if (!text) return json(404, { message: "No backup from that day." });
    let index;
    try { index = JSON.parse(text); } catch { return json(404, { message: "No backup from that day." }); }
    const ids = (Array.isArray(index.accounts) ? index.accounts : [])
      .filter((row) => row && row.ok !== false && /^[0-9a-f]{64}$/.test(row.account || ""))
      .map((row) => row.account);
    let restored = 0;
    let failed = 0;
    for (const account of ids) {
      if (await restoreSealed(env, account, date)) restored += 1;
      else failed += 1;
    }
    return json(200, { ok: true, restored, failed });
  }
  const account = String(value.account || "").trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(account) || confirm !== account) {
    return json(400, { message: "Type the account id to confirm." });
  }
  const restored = await restoreSealed(env, account, date);
  if (!restored) return json(404, { message: "No backup from that day." });
  return json(200, {
    ok: true,
    objects: restored.objects,
    revision: restored.revision,
    keptList: restored.keptList === true,
  });
}

function internalHeaders() {
  const headers = new Headers();
  headers.set("x-omacus-internal", "1");
  headers.set("content-type", "application/json");
  return headers;
}

function mailbox(env) {
  return box(env, DIRECTORY);
}

async function accountFoot(env, request, account) {
  const size = await box(env, accountObject(account)).fetch(new Request(
    new URL(`/v1/internal/size/${account}`, request.url),
    { headers: internalHeaders() },
  ));
  if (!size.ok) return null;
  const foot = await size.json().catch(() => null);
  if (!foot || !Number.isInteger(foot.devices)) return null;
  return foot;
}

async function fillDeviceCounts(env, request, body) {
  const rows = Array.isArray(body.storage) ? body.storage : [];
  for (const row of rows) {
    const account = row && typeof row.account === "string" ? row.account : "";
    if (!/^[0-9a-f]{64}$/.test(account)) continue;
    const foot = await accountFoot(env, request, account);
    if (!foot) continue;
    row.devices = foot.devices;
    row.created = Number.isInteger(foot.created) ? foot.created : 0;
    row.used = Number.isInteger(foot.used) ? foot.used : 0;
    row.changed = Number.isInteger(foot.changed) ? foot.changed : 0;
    row.requests = Number.isInteger(foot.requests) ? foot.requests : 0;
  }
  summarize(body);
}

async function footprint(store, account) {
  const mailbox = await store.get("m:" + account);
  if (!mailbox || typeof mailbox.record !== "string") return null;
  const record = fromB64(mailbox.record);
  let bytes = record.length;
  const objects = await store.list({ prefix: `o:${account}:` });
  for (const [, row] of objects) bytes += row && Number.isInteger(row.bytes) ? row.bytes : 0;
  return { bytes, devices: linkedDevices(record), ...activityOf(mailbox) };
}

async function indexedAccount(store, hash) {
  const row = await store.get("i:" + hash);
  if (row && typeof row.account === "string" && /^[0-9a-f]{64}$/.test(row.account)) {
    if (!Number.isInteger(row.expires) || row.expires > Date.now()) return row.account;
  }
  const all = await store.list({ prefix: "m:" });
  for (const [key, stored] of all) {
    const invite = stored && stored.invite;
    const account = key.slice(2);
    if (!/^[0-9a-f]{64}$/.test(account) || !invite || invite.used || invite.hash !== hash) continue;
    if (!(invite.expires > Date.now())) continue;
    await store.put("i:" + hash, { account, expires: invite.expires });
    return account;
  }
  return "";
}

async function remember(env, request, account) {
  const size = await box(env, accountObject(account)).fetch(new Request(
    new URL(`/v1/internal/size/${account}`, request.url),
    { headers: internalHeaders() },
  ));
  if (!size.ok) return;
  const foot = await size.json();
  await box(env, DIRECTORY).fetch(new Request(new URL("/v1/internal/registry", request.url), {
    method: "POST",
    headers: internalHeaders(),
    body: JSON.stringify({ account, bytes: foot.bytes, devices: foot.devices }),
  }));
}

async function migrateIfAbsent(env, request, account) {
  if (homed.has(account)) return;
  const home = box(env, accountObject(account));
  const probe = await home.fetch(new Request(new URL(`/v1/internal/probe/${account}`, request.url), {
    headers: internalHeaders(),
  }));
  if (probe.ok) {
    homed.add(account);
    return;
  }
  const legacy = box(env, LEGACY);
  const copy = await legacy.fetch(new Request(new URL(`/v1/internal/copy/${account}`, request.url), {
    method: "POST",
    headers: internalHeaders(),
    body: "{}",
  }));
  if (!copy.ok) return;
  const payload = await copy.json();
  const imported = await home.fetch(new Request(new URL(`/v1/internal/import/${account}`, request.url), {
    method: "POST",
    headers: internalHeaders(),
    body: JSON.stringify({ mailbox: payload.mailbox }),
  }));
  if (!imported.ok) return;
  for (const id of payload.objects || []) {
    const object = await legacy.fetch(new Request(
      new URL(`/v1/internal/object/${account}/${id}`, request.url),
      { headers: internalHeaders() },
    ));
    if (!object.ok) return;
    const saved = await home.fetch(new Request(
      new URL(`/v1/internal/object/${account}/${id}`, request.url),
      {
        method: "PUT",
        headers: internalHeaders(),
        body: JSON.stringify(await object.json()),
      },
    ));
    if (!saved.ok) return;
  }
  await legacy.fetch(new Request(new URL(`/v1/internal/drop/${account}`, request.url), {
    method: "POST",
    headers: internalHeaders(),
    body: "{}",
  }));
  const invite = payload.mailbox && payload.mailbox.invite;
  if (invite && invite.hash && !invite.used) {
    await box(env, DIRECTORY).fetch(new Request(new URL(`/v1/internal/invite/${invite.hash}`, request.url), {
      method: "PUT",
      headers: internalHeaders(),
      body: JSON.stringify({ account, expires: invite.expires }),
    }));
  }
  await remember(env, request, account);
  homed.add(account);
}

async function dropCopies(env, request, account) {
  homed.delete(account);
  const url = new URL(`/v1/internal/drop/${account}`, request.url);
  await box(env, accountObject(account)).fetch(new Request(url, { method: "POST", headers: internalHeaders(), body: "{}" }));
  await box(env, LEGACY).fetch(new Request(url, { method: "POST", headers: internalHeaders(), body: "{}" }));
}

async function publishInvite(env, request, account, raw) {
  let body;
  try { body = JSON.parse(raw); } catch { return; }
  const hash = String((body && body.hash) || "").trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(hash)) return;
  await box(env, DIRECTORY).fetch(new Request(new URL(`/v1/internal/invite/${hash}`, request.url), {
    method: "PUT",
    headers: internalHeaders(),
    body: JSON.stringify({ account, expires: Date.now() + INVITE_MS }),
  }));
}

async function createAccount(env, request, raw, headers) {
  let body;
  try { body = JSON.parse(raw); } catch { body = null; }
  const account = body && typeof body.account === "string" ? body.account.trim().toLowerCase() : "";
  if (!/^[0-9a-f]{64}$/.test(account)) {
    return box(env, DIRECTORY).fetch(new Request(request.url, { method: "POST", headers, body: raw }));
  }
  const blocked = signupFits(body, limitsFromHeader(headers) || defaultsFor(env));
  if (blocked) return json(ERR[blocked][0], { message: ERR[blocked][1] });
  const legacy = await box(env, LEGACY).fetch(new Request(
    new URL(`/v1/internal/probe/${account}`, request.url),
    { headers: internalHeaders() },
  ));
  if (legacy.ok) return json(409, { message: ERR.Conflict[1] });
  const claimHeaders = new Headers(headers);
  claimHeaders.set("x-omacus-internal", "1");
  const claim = await box(env, DIRECTORY).fetch(new Request(new URL("/v1/internal/claim", request.url), {
    method: "POST",
    headers: claimHeaders,
    body: raw,
  }));
  if (!claim.ok) return claim;
  const createHeaders = new Headers(headers);
  createHeaders.delete("x-omacus-internal");
  createHeaders.set("x-omacus-access-spent", "1");
  const created = await box(env, accountObject(account)).fetch(new Request(request.url, {
    method: "POST",
    headers: createHeaders,
    body: raw,
  }));
  if (!created.ok) {
    await box(env, DIRECTORY).fetch(new Request(new URL("/v1/internal/release", request.url), {
      method: "POST",
      headers: internalHeaders(),
      body: JSON.stringify({ account }),
    }));
    return created;
  }
  homed.add(account);
  await remember(env, request, account);
  return created;
}

async function lookupInvite(env, request, hash) {
  const url = new URL(`/v1/internal/invite/${hash}`, request.url);
  const directory = await box(env, DIRECTORY).fetch(new Request(url, { headers: internalHeaders() }));
  if (directory.ok) return directory.json();
  const legacy = await box(env, LEGACY).fetch(new Request(url, { headers: internalHeaders() }));
  if (legacy.ok) return legacy.json();
  return null;
}

async function joinAccount(env, request, raw, headers) {
  let body;
  try { body = JSON.parse(raw); } catch { body = null; }
  const named = body && typeof body.account === "string" && /^[0-9a-f]{64}$/.test(body.account.trim().toLowerCase())
    ? body.account.trim().toLowerCase()
    : "";
  const secret = body && decodeCode(body.code);
  const hash = secret ? await hashHex(secret) : "";
  const found = hash ? await lookupInvite(env, request, hash) : null;
  // A wrong code still has to reach the account it named, so the miss is stored
  // there. An address change cannot spend a fresh set of tries.
  const account = found && typeof found.account === "string" && /^[0-9a-f]{64}$/.test(found.account) ? found.account : named;
  if (!account) return json(400, { message: ERR.BadInvite[1] });
  await migrateIfAbsent(env, request, account);
  // The lookup stays while a phone is still joining. It comes off when the
  // approval finishes, so a phone that stops waiting can use the same code.
  return box(env, accountObject(account)).fetch(new Request(request.url, {
    method: "POST",
    headers,
    body: raw,
  }));
}

function finishedJoin(raw) {
  try {
    const body = JSON.parse(raw);
    return !!(body && body.done === true);
  } catch {
    return false;
  }
}

async function retireInvite(env, request, account) {
  const looked = await box(env, accountObject(account)).fetch(new Request(
    new URL(`/v1/internal/invite-hash/${account}`, request.url),
    { headers: internalHeaders() },
  ));
  if (!looked.ok) return;
  const payload = await looked.json().catch(() => ({}));
  const hash = String(payload.hash || "").trim().toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(hash)) return;
  const drop = new URL(`/v1/internal/invite/${hash}`, request.url);
  await box(env, DIRECTORY).fetch(new Request(drop, { method: "DELETE", headers: internalHeaders() }));
  await box(env, LEGACY).fetch(new Request(drop, { method: "DELETE", headers: internalHeaders() }));
}

async function routeClient(env, request) {
  const url = new URL(request.url);
  const { pathname: path } = url;
  const { method } = request;
  if (path.startsWith("/v1/internal")) return json(404, { message: ERR.Unknown[1] });
  const headers = new Headers(request.headers);
  headers.delete("x-omacus-internal");
  headers.delete("x-omacus-access-spent");
  headers.delete(LIMIT_HEADER);
  headers.delete("content-length");
  let raw = "";
  if (method !== "GET" && method !== "HEAD") raw = await request.text();
  headers.set(LIMIT_HEADER, JSON.stringify(await currentLimits(env)));
  const forward = () => new Request(request.url, {
    method,
    headers,
    body: method === "GET" || method === "HEAD" ? undefined : raw,
  });
  if (method === "POST" && path === "/v1/access") return box(env, DIRECTORY).fetch(forward());
  if (method === "POST" && path === "/v1/mailboxes") return createAccount(env, request, raw, headers);
  if (method === "POST" && path === "/v1/join") return joinAccount(env, request, raw, headers);
  const mailboxPath = /^\/v1\/mailboxes\/([0-9a-f]{64})(\/.*)?$/.exec(path);
  if (mailboxPath) {
    const account = mailboxPath[1];
    await migrateIfAbsent(env, request, account);
    const response = await box(env, accountObject(account)).fetch(forward());
    if (response.ok && method === "POST" && mailboxPath[2] === "/invites") await publishInvite(env, request, account, raw);
    if (response.ok && method === "PUT" && !mailboxPath[2]) await remember(env, request, account);
    if (response.ok && method === "POST" && /^\/requests\/[^/]+$/.test(mailboxPath[2] || "") && finishedJoin(raw)) {
      await retireInvite(env, request, account);
    }
    return response;
  }
  return box(env, DIRECTORY).fetch(forward());
}

const DEFAULT_APP_URL = "https://app.omacus.com/";

function appUrl(env) {
  const configured = String((env && env.APP_URL) || "").trim();
  if (!configured) return DEFAULT_APP_URL;
  try {
    const url = new URL(configured);
    if (url.protocol !== "https:") return DEFAULT_APP_URL;
    return url.toString();
  } catch {
    return DEFAULT_APP_URL;
  }
}

function notFound() {
  return new Response("Not found", { status: 404, headers: { "cache-control": "no-store" } });
}

function redirectHome(env) {
  return new Response(null, {
    status: 302,
    headers: { location: appUrl(env), "cache-control": "no-store" },
  });
}

async function adminAsset(request, env, pathname) {
  if (!env.ASSETS) return notFound();
  const url = new URL(request.url);
  url.pathname = pathname;
  return env.ASSETS.fetch(new Request(url, { method: "GET" }));
}

// The code is minted here, where the secret lives, and is not sent to storage.
async function adminApi(request, env, url) {
  if (!isolateAllow(request.headers.get("cf-connecting-ip") || "")) {
    return json(429, { message: "Too many tries. Wait a minute." });
  }
  if (!(await adminPageAllowed(request, env))) return json(403, { message: "Sign in again." });
  const allowed = adminKeyOk(request, env);
  const headers = new Headers();
  headers.set("content-type", "application/json");
  if (allowed) headers.set("x-omacus-admin", "1");
  let payload = {};
  if (request.method !== "GET" && request.method !== "HEAD") {
    const text = await request.text();
    if (enc.encode(text).length > 4096) return json(400, { message: "That request was too large." });
    if (text) {
      try { payload = JSON.parse(text); } catch { return json(400, { message: "That request could not be read." }); }
    }
  }
  if (url.pathname === "/v1/admin/limits") {
    if (!allowed) return json(401, { message: "That key didn't work." });
    if (request.method !== "GET" && request.method !== "POST") return json(404, { message: "Not found." });
    return adminLimits(env, request.method, payload);
  }
  if (url.pathname === "/v1/admin/backups") {
    if (!allowed) return json(401, { message: "That key didn't work." });
    if (request.method !== "GET" && request.method !== "POST") return json(404, { message: "Not found." });
    return adminBackups(env, request.method, payload);
  }
  if (url.pathname === "/v1/admin/restore") {
    if (!allowed) return json(401, { message: "That key didn't work." });
    if (request.method !== "POST") return json(404, { message: "Not found." });
    return adminRestore(env, payload);
  }
  if (allowed && request.method === "POST" && url.pathname === "/v1/admin/codes") {
    const uses = Number(payload && payload.uses);
    const code = await mint(String(env.ACCESS_SECRET || ""), Number.isInteger(uses) ? uses : NaN);
    if (!code) return json(400, { message: "Uses must be a number from 1 to 20." });
    const opened = await openCode(String(env.ACCESS_SECRET || ""), code);
    if (!opened) return json(400, { message: "That invite code could not be created." });
    const forward = new Request(request.url, {
      method: "POST",
      headers,
      body: JSON.stringify({
        hash: opened.hash,
        uses: opened.uses,
        label: payload && payload.label,
        expires: payload && payload.expires,
      }),
    });
    const stored = await mailbox(env).fetch(forward);
    const body = await stored.json();
    if (!stored.ok) return json(stored.status, body);
    return json(200, { code, ...body });
  }
  const forward = new Request(request.url, {
    method: request.method,
    headers,
    body: request.method === "GET" || request.method === "HEAD" ? undefined : JSON.stringify(payload),
  });
  if (allowed && request.method === "GET" && url.pathname === "/v1/admin/stats") {
    const response = await mailbox(env).fetch(forward);
    const body = await response.json().catch(() => ({}));
    if (!response.ok) return json(response.status, body);
    await fillDeviceCounts(env, request, body);
    body.copy = await latestCopy(env);
    return json(200, body);
  }
  const response = await mailbox(env).fetch(forward);
  if (request.method === "DELETE" && response.ok) {
    const remove = /^\/v1\/admin\/accounts\/([0-9a-f]{64})$/.exec(url.pathname);
    if (remove) await dropCopies(env, request, remove[1]);
  }
  return response;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
    if (url.pathname === "/v1/health" && request.method === "GET") return json(200, { label: LABEL });
    if (url.pathname === "/admin" || url.pathname === "/admin/" || url.pathname.startsWith("/admin/")) {
      if (!(await adminPageAllowed(request, env))) {
        return new Response("Sign in again.", {
          status: 403,
          headers: { "cache-control": "no-store", "content-type": "text/plain; charset=utf-8" },
        });
      }
      const pathname = url.pathname === "/admin" || url.pathname === "/admin/" ? "/admin/index.html" : url.pathname;
      return adminAsset(request, env, pathname);
    }
    if (url.pathname === "/css/tokens.css") return adminAsset(request, env, url.pathname);
    if (url.pathname.startsWith("/v1/admin")) return adminApi(request, env, url);
    if (url.pathname.startsWith("/v1/internal")) return json(404, { message: ERR.Unknown[1] });
    if (url.pathname.startsWith("/v1/")) {
      const forwarded = await gate(request, env, url);
      if (forwarded instanceof Response) return forwarded;
      return routeClient(env, forwarded);
    }
    if (url.pathname === "/" && (request.method === "GET" || request.method === "HEAD") && String(env.SERVE_APP) !== "1") {
      return redirectHome(env);
    }
    if (String(env.SERVE_APP) === "1" && env.ASSETS) return env.ASSETS.fetch(request);
    return notFound();
  },
  async scheduled(event, env, ctx) {
    const when = event && Number.isFinite(event.scheduledTime) ? event.scheduledTime : Date.now();
    const job = runBackup(env, when);
    if (ctx && typeof ctx.waitUntil === "function") ctx.waitUntil(job);
    await job;
  },
};
