// Private sync test relay (1.0.0-beta.16) as a Cloudflare Worker.
// Same HTTP API as relay/src/http.rs. It keeps only the sealed bytes.
// Each account lives in its own object. A directory object maps an invite
// hash to that account, so a join does not list every account.
// A new set of pages needs an invite code. The code is shown once, and
// only a hash is stored. A new phone joins with a one-time code, also
// stored only as a hash. A join request is removed once the waiting phone
// has read the answer, or once the same window as the code has passed.
// The admin routes count and delete sealed copies. They never return one.
// This worker holds no keys and cannot open a page.
import { DurableObject } from "cloudflare:workers";
import { mint, openCode, spend } from "./access.js";
import { adminAuthorized, deviceCount, handleAdmin, isolateAllow, noteError } from "./admin.js";
import { allowPut, authorize } from "./auth.js";
import { INVITE_MS, decodeCode, hashHex, hmacBytes, sameBytes } from "./invite.js";

const LABEL = "1.0.0-beta.16";
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
  Unknown: [404, "This test server has no pages for that yet."],
  Conflict: [409, "This test server already has that copy."],
  Revision: [409, "That copy is out of date. Try again."],
  Full: [429, "This test server is full."],
  Unreadable: [400, "That copy could not be read."],
  BadInvite: [400, "That code didn't work. Check it on the other device."],
  BadAccess: [400, "That invite code didn't work. Check it and try again."],
  Paused: [403, "New signups are paused."],
  Limited: [429, "Try again in a moment."],
};
const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PUT, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type, If-None-Match, X-Omacus-Device, X-Omacus-Time, X-Omacus-Nonce, X-Omacus-Signature, X-Omacus-Proof",
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
  }
  charge(ip, account) {
    const now = Date.now();
    const hit = (map, key, limit) => {
      if (!key) return;
      const row = map.get(key);
      if (!row || now - row.start >= 60_000) {
        map.set(key, { start: now, count: 1 });
        return;
      }
      if (row.count >= limit) fail("Limited");
      row.count += 1;
    };
    hit(this.ipHits, ip, 800);
    hit(this.accountHits, account, 240);
  }
  async guard(method, path, raw, headers, mailbox) {
    const auth = await authorize(method, path, raw, headers, mailbox.record, Date.now(), mailbox);
    if (!auth) fail("Unknown");
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
      dropExpired(m, Date.now());
      out = await update(m, txn);
      await txn.put("m:" + account, m);
    });
    return out;
  }

  async useAccess(access, take) {
    const hash = access.hash;
    const uses = access.uses;
    if (!/^[0-9a-f]{64}$/.test(hash) || !Number.isInteger(uses) || uses < 1 || uses > MAX_USES) fail("BadAccess");
    const key = "a:" + hash;
    const row = await this.store.get(key);
    if (row && row.revoked) fail("BadAccess");
    if (row && Number.isInteger(row.expires) && row.expires > 0 && row.expires <= Date.now()) fail("BadAccess");
    const table = {};
    if (row && Number.isInteger(row.left)) table[hash] = row.left;
    if (!spend(table, hash, uses, take)) fail("BadAccess");
    if (!take) return;
    const next = row && typeof row === "object" ? { ...row } : {};
    next.left = table[hash];
    next.usedAt = Date.now();
    await this.store.put(key, next);
  }

  async route(method, path, body, access, admin, headers, raw, ip) {
    if (path.startsWith("/v1/admin")) return handleAdmin(this.store, method, path, body, admin === true);
    if (path.startsWith("/v1/internal/")) return this.internal(method, path, body, headers, access);
    const need = () => (body && typeof body === "object" ? body : fail("Unreadable"));
    if (method === "POST" && path === "/v1/access") {
      this.charge(ip);
      await this.useAccess(access, false);
      return [200, { ok: true }];
    }
    if (method === "POST" && path === "/v1/join") {
      this.charge(ip);
      return this.join(need());
    }
    if (method === "POST" && path === "/v1/mailboxes") {
      this.charge(ip);
      const v = need();
      const account = accountId(typeof v.account === "string" ? v.account : fail("Unreadable"));
      const record = sealed(v.record);
      const writes = objectWrites(v);
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
        await txn.put("m:" + account, { revision: 1, record, requests: {}, created: Date.now() });
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
      const given = proofBytes(header("x-omacus-proof"));
      const [status, answer] = await this.change(account, (m) => {
        const r = Object.hasOwn(m.requests, id) ? m.requests[id] : null;
        const stored = r ? proofBytes(r.proof) : null;
        if (!r || !given || !stored || !sameBytes(given, stored)) return [404, null];
        const reply = { state: r.state, name: r.name, device: r.device, chain: r.chain, wrap: r.wrap };
        if (r.state !== "pending") delete m.requests[id];
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
        const writes = objectWrites(v);
        const dropping = idList(v.drop);
        const written = await applyObjects(txn, account, writes, dropping);
        m.revision += 1;
        m.record = record;
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
        m.invite = { hash, expires, used: false };
        await txn.put("i:" + hash, { account, expires });
        return [200, { ok: true }];
      });
    }
    if (sub !== "requests") fail("Unknown");
    if (method === "GET" && id === undefined) {
      return this.change(account, async (m) => {
        await authed(m);
        return [200, {
          requests: Object.entries(m.requests).filter(([, r]) => r.state === "pending")
            .map(([rid, r]) => ({ id: rid, device: r.device, name: r.name, proof: r.proof || "" })),
        }];
      });
    }
    if (method === "POST" && id !== undefined) {
      return this.change(account, async (m) => {
        await authed(m);
        const v = need();
        const r = Object.hasOwn(m.requests, id) ? m.requests[id] : fail("Unknown");
        if (v.reject === true) {
          if (r.state !== "pending") fail("Conflict");
          r.state = "rejected";
        } else {
          const chain = fromB64(v.chain), wrap = fromB64(v.wrap);
          if (chain.length > MAX_CHAIN || !startsWith(chain, CHAIN_MAGIC) || !wrap.length || wrap.length > MAX_WRAP) fail("Unreadable");
          if (r.state !== "pending") fail("Conflict");
          r.state = "approved"; r.chain = v.chain.trim(); r.wrap = v.wrap.trim();
        }
        return [200, { ok: true }];
      });
    }
    fail("Unknown");
  }

  async join(v) {
    const secret = decodeCode(v.code);
    if (!secret) fail("BadInvite");
    const device = fromB64(v.device);
    if (device.length !== 80) fail("Unreadable");
    const given = fromB64(v.proof);
    if (given.length !== 32) fail("Unreadable");
    const hash = await hashHex(secret);
    const now = Date.now();
    const proofBytes = await hmacBytes(secret, device);
    if (!sameBytes(proofBytes, given)) fail("BadInvite");
    const id = freshId();
    const proof = btoa(String.fromCharCode(...given));
    let result;
    await this.store.transaction(async (txn) => {
      const indexed = await txn.get("i:" + hash);
      const account = indexed && typeof indexed.account === "string" ? indexed.account : "";
      const mailbox = /^[0-9a-f]{64}$/.test(account) ? await txn.get("m:" + account) : null;
      const invite = mailbox && mailbox.invite;
      if (!mailbox || !invite || invite.used || !(invite.expires > now) || invite.hash !== hash) fail("BadInvite");
      dropExpired(mailbox, now);
      const pending = Object.values(mailbox.requests).filter((request) => request.state === "pending").length;
      if (pending >= MAX_REQUESTS) fail("Full");
      mailbox.invite.used = true;
      await txn.delete("i:" + hash);
      mailbox.requests[id] = {
        device: btoa(String.fromCharCode(...device)),
        name: cleanName(typeof v.name === "string" ? v.name : ""),
        state: "pending",
        chain: "",
        wrap: "",
        proof,
        created: now,
      };
      await txn.put("m:" + account, mailbox);
      result = [201, { id, account }];
    });
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
      if (!(await this.store.get(key))) {
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
      const signups = await this.store.get("admin:signups");
      if (signups && signups.paused) fail("Paused");
      if ((await this.store.get("r:" + account)) || (await this.store.get("m:" + account))) fail("Conflict");
      await this.useAccess(access, true);
      await this.store.put("r:" + account, { pending: true, bytes: 0, devices: 0, invite: "" });
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
    fail("Unknown");
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

function internalHeaders() {
  const headers = new Headers();
  headers.set("x-omacus-internal", "1");
  headers.set("content-type", "application/json");
  return headers;
}

function mailbox(env) {
  return box(env, DIRECTORY);
}

async function footprint(store, account) {
  const mailbox = await store.get("m:" + account);
  if (!mailbox || typeof mailbox.record !== "string") return null;
  const record = fromB64(mailbox.record);
  let bytes = record.length;
  const objects = await store.list({ prefix: `o:${account}:` });
  for (const [, row] of objects) bytes += row && Number.isInteger(row.bytes) ? row.bytes : 0;
  return { bytes, devices: deviceCount(record) };
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
  const secret = body && decodeCode(body.code);
  if (!secret) return json(400, { message: ERR.BadInvite[1] });
  const hash = await hashHex(secret);
  const found = await lookupInvite(env, request, hash);
  if (!found) return json(400, { message: ERR.BadInvite[1] });
  await migrateIfAbsent(env, request, found.account);
  const response = await box(env, accountObject(found.account)).fetch(new Request(request.url, {
    method: "POST",
    headers,
    body: raw,
  }));
  if (response.ok) {
    const drop = new URL(`/v1/internal/invite/${hash}`, request.url);
    await box(env, DIRECTORY).fetch(new Request(drop, { method: "DELETE", headers: internalHeaders() }));
    await box(env, LEGACY).fetch(new Request(drop, { method: "DELETE", headers: internalHeaders() }));
  }
  return response;
}

async function routeClient(env, request) {
  const url = new URL(request.url);
  const { pathname: path } = url;
  const { method } = request;
  if (path.startsWith("/v1/internal")) return json(404, { message: ERR.Unknown[1] });
  const headers = new Headers(request.headers);
  headers.delete("x-omacus-internal");
  headers.delete("x-omacus-access-spent");
  headers.delete("content-length");
  let raw = "";
  if (method !== "GET" && method !== "HEAD") raw = await request.text();
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
    return response;
  }
  return box(env, DIRECTORY).fetch(forward());
}

function notFound() {
  return new Response("Not found", { status: 404, headers: { "cache-control": "no-store" } });
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
  const allowed = await adminAuthorized(request, env);
  const headers = new Headers();
  headers.set("content-type", "application/json");
  if (allowed) headers.set("x-omacus-admin", "1");
  let payload = {};
  if (request.method !== "GET" && request.method !== "HEAD") {
    const text = await request.text();
    if (text.length > 4096) return json(400, { message: "That request was too large." });
    if (text) {
      try { payload = JSON.parse(text); } catch { return json(400, { message: "That request could not be read." }); }
    }
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
    if (url.pathname === "/admin" || url.pathname === "/admin/") return adminAsset(request, env, "/admin/index.html");
    if (url.pathname.startsWith("/admin/")) return adminAsset(request, env, url.pathname);
    if (url.pathname === "/css/tokens.css") return adminAsset(request, env, url.pathname);
    if (url.pathname.startsWith("/v1/admin")) return adminApi(request, env, url);
    if (url.pathname.startsWith("/v1/internal")) return json(404, { message: ERR.Unknown[1] });
    if (url.pathname.startsWith("/v1/")) {
      const forwarded = await gate(request, env, url);
      if (forwarded instanceof Response) return forwarded;
      return routeClient(env, forwarded);
    }
    if (String(env.SERVE_APP) === "1" && env.ASSETS) return env.ASSETS.fetch(request);
    return notFound();
  },
};
