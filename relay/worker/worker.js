// Private sync test relay (1.0.0-beta.1) as a Cloudflare Worker.
// Same HTTP API as relay/src/http.rs. It keeps only the sealed bytes and
// device approvals that arrive. A new device joins with a one-time code.
// The code is not stored — only a hash of it. This worker holds no keys
// and cannot open a page.
import { DurableObject } from "cloudflare:workers";
import { INVITE_MS, decodeCode, hashHex, hmacBytes, sameBytes } from "./invite.js";

const LABEL = "1.0.0-beta.1";
const MAX_RECORD = 256 * 1024, MAX_CHAIN = 64 * 1024, MAX_WRAP = 1024;
const MAX_MAILBOXES = 32, MAX_REQUESTS = 8;
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

export class Mailboxes extends DurableObject {
  async fetch(request) {
    const url = new URL(request.url);
    let body = {};
    if (request.method !== "GET") {
      const text = await request.text();
      if (text.length > 1024 * 1024) return json(400, { message: ERR.Unreadable[1] });
      try { body = JSON.parse(text); } catch { body = null; }
    }
    try {
      const [status, value] = await this.route(request.method, url.pathname, body);
      return json(status, value);
    } catch (e) {
      const [status, message] = ERR[e] || [500, "Try again in a moment."];
      return json(status, { message });
    }
  }
  async load(account) {
    return (await this.ctx.storage.get("m:" + account)) || fail("Unknown");
  }
  async save(account, m) { await this.ctx.storage.put("m:" + account, m); }

  async route(method, path, body) {
    const need = () => (body && typeof body === "object" ? body : fail("Unreadable"));
    if (method === "POST" && path === "/v1/join") return this.join(need());
    if (method === "POST" && path === "/v1/mailboxes") {
      const v = need();
      const account = accountId(typeof v.account === "string" ? v.account : fail("Unreadable"));
      const record = sealed(v.record);
      if (await this.ctx.storage.get("m:" + account)) fail("Conflict");
      if ((await this.ctx.storage.list({ prefix: "m:" })).size >= MAX_MAILBOXES) fail("Full");
      await this.save(account, { revision: 1, record, requests: {}, created: Date.now() });
      return [201, { revision: 1 }];
    }
    const rest = path.startsWith("/v1/mailboxes/") ? path.slice(14).split("/") : fail("Unknown");
    const account = accountId(rest[0]);
    const [sub, id, extra] = rest.slice(1);
    if (extra !== undefined) fail("Unknown");
    if (method === "GET" && sub === undefined) {
      const m = await this.load(account);
      return [200, { revision: m.revision, record: m.record }];
    }
    if (method === "PUT" && sub === undefined) {
      const v = need();
      if (!Number.isInteger(v.revision) || v.revision < 0) fail("Unreadable");
      const record = sealed(v.record);
      const m = await this.load(account);
      if (m.revision !== v.revision) fail("Revision");
      m.revision += 1; m.record = record;
      await this.save(account, m);
      return [200, { revision: m.revision }];
    }
    if (method === "POST" && sub === "invites" && id === undefined) {
      const v = need();
      const hash = String(v.hash || "").trim().toLowerCase();
      if (!/^[0-9a-f]{64}$/.test(hash)) fail("Unreadable");
      const m = await this.load(account);
      m.invite = { hash, expires: Date.now() + INVITE_MS, used: false };
      await this.save(account, m);
      return [200, { ok: true }];
    }
    if (sub !== "requests") fail("Unknown");
    if (method === "GET" && id === undefined) {
      const m = await this.load(account);
      return [200, { requests: Object.entries(m.requests).filter(([, r]) => r.state === "pending")
        .map(([rid, r]) => ({ id: rid, device: r.device, name: r.name, proof: r.proof || "" })) }];
    }
    const m = await this.load(account);
    const r = Object.hasOwn(m.requests, id) ? m.requests[id] : fail("Unknown");
    if (method === "GET") return [200, { state: r.state, name: r.name, device: r.device, chain: r.chain, wrap: r.wrap }];
    if (method === "POST") {
      const v = need();
      if (v.reject === true) {
        if (r.state !== "pending") fail("Conflict");
        r.state = "rejected";
      } else {
        const chain = fromB64(v.chain), wrap = fromB64(v.wrap);
        if (chain.length > MAX_CHAIN || !startsWith(chain, CHAIN_MAGIC) || !wrap.length || wrap.length > MAX_WRAP) fail("Unreadable");
        if (r.state !== "pending") fail("Conflict");
        r.state = "approved"; r.chain = v.chain.trim(); r.wrap = v.wrap.trim();
      }
      await this.save(account, m);
      return [200, { ok: true }];
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
    const all = await this.ctx.storage.list({ prefix: "m:" });
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
    if (!sameBytes(await hmacBytes(secret, device), given)) fail("BadInvite");
    const pending = Object.values(mailbox.requests).filter((request) => request.state === "pending").length;
    if (pending >= MAX_REQUESTS) fail("Full");
    mailbox.invite.used = true;
    const id = freshId();
    const proof = btoa(String.fromCharCode(...given));
    mailbox.requests[id] = {
      device: btoa(String.fromCharCode(...device)),
      name: cleanName(typeof v.name === "string" ? v.name : ""),
      state: "pending",
      chain: "",
      wrap: "",
      proof,
    };
    await this.save(account, mailbox);
    return [201, { id, account }];
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "OPTIONS") return new Response(null, { status: 204, headers: CORS });
    if (url.pathname === "/v1/health" && request.method === "GET") return json(200, { label: LABEL });
    if (url.pathname.startsWith("/v1/")) {
      return env.MAILBOXES.get(env.MAILBOXES.idFromName("test")).fetch(request);
    }
    return env.ASSETS.fetch(request);
  },
};
