// Private sync test relay (1.0.0-beta.10) as a Cloudflare Worker.
// Same HTTP API as relay/src/http.rs. It keeps only the sealed bytes.
// A new set of pages needs an invite code. The code is shown once, and
// only a hash is stored. A new phone joins with a one-time code, also
// stored only as a hash. A join request is removed once the waiting phone
// has read the answer, or once the same window as the code has passed.
// The admin routes count and delete sealed copies. They never return one.
// This worker holds no keys and cannot open a page.
import { DurableObject } from "cloudflare:workers";
import { mint, openCode, spend } from "./access.js";
import { adminAuthorized, handleAdmin, isolateAllow, noteError } from "./admin.js";
import { INVITE_MS, decodeCode, hashHex, hmacBytes, sameBytes } from "./invite.js";

const LABEL = "1.0.0-beta.10";
const MAX_RECORD = 256 * 1024, MAX_CHAIN = 64 * 1024, MAX_WRAP = 1024;
const MAX_MAILBOXES = 32, MAX_REQUESTS = 8, MAX_USES = 20;
const enc = new TextEncoder();
const RECORD_MAGIC = enc.encode("OMACUS-SYNC/1\n");
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
};
const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Methods": "GET, POST, PUT, OPTIONS",
  "Access-Control-Allow-Headers": "Content-Type",
  "Cache-Control": "no-store",
};
const json = (status, value) =>
  new Response(JSON.stringify(value), { status, headers: { "Content-Type": "application/json", ...CORS } });
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
const sealed = (b64) => {
  const b = fromB64(b64);
  if (b.length > MAX_RECORD || !startsWith(b, RECORD_MAGIC)) fail("Unreadable");
  return b64.trim();
};
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
  if (text.length > 1024 * 1024) return json(400, { message: ERR.Unreadable[1] });
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
  async fetch(request) {
    const url = new URL(request.url);
    const access = {
      hash: (request.headers.get("x-omacus-access-hash") || "").toLowerCase(),
      uses: Number(request.headers.get("x-omacus-access-uses") || "0"),
      spend: request.headers.get("x-omacus-access-spend") === "1",
    };
    let body = {};
    if (request.method !== "GET") {
      const text = await request.text();
      if (text.length > 1024 * 1024) return json(400, { message: ERR.Unreadable[1] });
      try { body = JSON.parse(text); } catch { body = null; }
    }
    try {
      const admin = request.headers.get("x-omacus-admin") === "1";
      const [status, value] = await this.route(request.method, url.pathname, body, access, admin);
      return json(status, value);
    } catch (e) {
      const name = Object.hasOwn(ERR, e) ? e : "Other";
      try { await noteError(this.ctx.storage, name); } catch { /* the reply still names the failure */ }
      const [status, message] = ERR[e] || [500, "Try again in a moment."];
      return json(status, { message });
    }
  }
  async load(account) {
    return (await this.ctx.storage.get("m:" + account)) || fail("Unknown");
  }
  async save(account, m) { await this.ctx.storage.put("m:" + account, m); }
  // A put and an approval can overlap on a slow phone. One transaction keeps
  // both changes; the last write must not roll the saved pages back.
  async change(account, update) {
    let out;
    await this.ctx.storage.transaction(async (txn) => {
      const m = (await txn.get("m:" + account)) || fail("Unknown");
      dropExpired(m, Date.now());
      out = await update(m);
      await txn.put("m:" + account, m);
    });
    return out;
  }

  async useAccess(access, take) {
    const hash = access.hash;
    const uses = access.uses;
    if (!/^[0-9a-f]{64}$/.test(hash) || !Number.isInteger(uses) || uses < 1 || uses > MAX_USES) fail("BadAccess");
    const key = "a:" + hash;
    const row = await this.ctx.storage.get(key);
    if (row && row.revoked) fail("BadAccess");
    if (row && Number.isInteger(row.expires) && row.expires > 0 && row.expires <= Date.now()) fail("BadAccess");
    const table = {};
    if (row && Number.isInteger(row.left)) table[hash] = row.left;
    if (!spend(table, hash, uses, take)) fail("BadAccess");
    if (!take) return;
    const next = row && typeof row === "object" ? { ...row } : {};
    next.left = table[hash];
    next.usedAt = Date.now();
    await this.ctx.storage.put(key, next);
  }

  async route(method, path, body, access, admin) {
    if (path.startsWith("/v1/admin")) return handleAdmin(this.ctx.storage, method, path, body, admin === true);
    const need = () => (body && typeof body === "object" ? body : fail("Unreadable"));
    if (method === "POST" && path === "/v1/access") {
      await this.useAccess(access, false);
      return [200, { ok: true }];
    }
    if (method === "POST" && path === "/v1/join") return this.join(need());
    if (method === "POST" && path === "/v1/mailboxes") {
      const v = need();
      const account = accountId(typeof v.account === "string" ? v.account : fail("Unreadable"));
      const record = sealed(v.record);
      if (!access.spend) fail("BadAccess");
      const signups = await this.ctx.storage.get("admin:signups");
      if (signups && signups.paused) fail("Paused");
      await this.useAccess(access, true);
      await this.ctx.storage.transaction(async (txn) => {
        if (await txn.get("m:" + account)) fail("Conflict");
        if ((await txn.list({ prefix: "m:" })).size >= MAX_MAILBOXES) fail("Full");
        await txn.put("m:" + account, { revision: 1, record, requests: {}, created: Date.now() });
      });
      return [201, { revision: 1 }];
    }
    const rest = path.startsWith("/v1/mailboxes/") ? path.slice(14).split("/") : fail("Unknown");
    const account = accountId(rest[0]);
    const [sub, id, extra] = rest.slice(1);
    if (extra !== undefined) fail("Unknown");
    if (method === "GET" && sub === undefined) {
      return this.change(account, (m) => [200, { revision: m.revision, record: m.record }]);
    }
    if (method === "PUT" && sub === undefined) {
      const v = need();
      if (!Number.isInteger(v.revision) || v.revision < 0) fail("Unreadable");
      const record = sealed(v.record);
      return this.change(account, (m) => {
        if (m.revision !== v.revision) fail("Revision");
        m.revision += 1;
        m.record = record;
        return [200, { revision: m.revision }];
      });
    }
    if (method === "POST" && sub === "invites" && id === undefined) {
      const v = need();
      const hash = String(v.hash || "").trim().toLowerCase();
      if (!/^[0-9a-f]{64}$/.test(hash)) fail("Unreadable");
      return this.change(account, (m) => {
        m.invite = { hash, expires: Date.now() + INVITE_MS, used: false };
        return [200, { ok: true }];
      });
    }
    if (sub !== "requests") fail("Unknown");
    if (method === "GET" && id === undefined) {
      return this.change(account, (m) => [200, {
        requests: Object.entries(m.requests).filter(([, r]) => r.state === "pending")
          .map(([rid, r]) => ({ id: rid, device: r.device, name: r.name, proof: r.proof || "" })),
      }]);
    }
    if (method === "GET") {
      // Commit the removal first. An expired request must not come back
      // because the missing-request error rolled the write back.
      const [status, body] = await this.change(account, (m) => {
        const r = Object.hasOwn(m.requests, id) ? m.requests[id] : null;
        if (!r) return [404, null];
        const answer = { state: r.state, name: r.name, device: r.device, chain: r.chain, wrap: r.wrap };
        if (r.state !== "pending") delete m.requests[id];
        return [200, answer];
      });
      if (status === 404) fail("Unknown");
      return [status, body];
    }
    if (method === "POST") {
      const v = need();
      return this.change(account, (m) => {
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
    await this.ctx.storage.transaction(async (txn) => {
      const all = await txn.list({ prefix: "m:" });
      let account = "";
      let mailbox = null;
      for (const [key, stored] of all) {
        const invite = stored.invite;
        if (!invite || invite.used || !(invite.expires > now) || invite.hash !== hash) continue;
        account = key.slice(2);
        mailbox = stored;
        break;
      }
      if (!mailbox) fail("BadInvite");
      dropExpired(mailbox, now);
      const pending = Object.values(mailbox.requests).filter((request) => request.state === "pending").length;
      if (pending >= MAX_REQUESTS) fail("Full");
      mailbox.invite.used = true;
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
}

function mailbox(env) {
  return env.MAILBOXES.get(env.MAILBOXES.idFromName("test"));
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
  return mailbox(env).fetch(forward);
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
    if (url.pathname.startsWith("/v1/")) {
      const forwarded = await gate(request, env, url);
      if (forwarded instanceof Response) return forwarded;
      return mailbox(env).fetch(forwarded);
    }
    if (String(env.SERVE_APP) === "1" && env.ASSETS) return env.ASSETS.fetch(request);
    return notFound();
  },
};
