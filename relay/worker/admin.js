// Admin operations for the sync worker. Counts, sizes, and hashes only.
// A code, a key, and a sealed page never land in the log.

const enc = new TextEncoder();
const RECORD_MAGIC = enc.encode("OMACUS-SYNC/1\n");
const WINDOW_MS = 60_000;
const DAY_MS = 24 * 60 * 60 * 1000;
const MAX_USES = 20;
const LOG_CAP = 100;
const RATE_CAP = 30;
const hits = new Map();
const certs = new Map();

export function sameSecret(expected, presented) {
  const left = enc.encode(String(expected ?? ""));
  const right = enc.encode(String(presented ?? ""));
  if (left.length === 0) return false;
  let diff = left.length === right.length ? 0 : 1;
  const length = Math.max(left.length, right.length);
  for (let i = 0; i < length; i += 1) diff |= (left[i] || 0) ^ (right[i] || 0);
  return diff === 0;
}

export function isolateAllow(ip, now = Date.now()) {
  const key = ip || "local";
  const row = hits.get(key) || { start: now, count: 0 };
  if (now - row.start >= WINDOW_MS) {
    row.start = now;
    row.count = 0;
  }
  row.count += 1;
  hits.set(key, row);
  return row.count <= RATE_CAP;
}

export function clearAdminCaches() {
  hits.clear();
  certs.clear();
}

export function cleanLabel(raw) {
  let out = "";
  for (const ch of String(raw || "")) {
    if (/[A-Za-z0-9 ._-]/.test(ch)) out += ch;
    if (out.length >= 40) break;
  }
  return out.trim();
}

// A date string is the end of that UTC day. A number is used as given. Empty means no expiry.
export function expiryMs(value) {
  if (value === undefined || value === null || value === "") return 0;
  if (typeof value === "number" && Number.isInteger(value) && value >= 0) return value;
  if (typeof value === "string" && /^\d+$/.test(value)) {
    const asNumber = Number(value);
    return Number.isSafeInteger(asNumber) ? asNumber : null;
  }
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const [year, month, day] = value.split("-").map(Number);
  const ms = Date.UTC(year, month - 1, day, 23, 59, 59, 999);
  const check = new Date(ms);
  if (check.getUTCFullYear() !== year || check.getUTCMonth() !== month - 1 || check.getUTCDate() !== day) return null;
  return ms;
}

export function codeStatus(row, now = Date.now()) {
  if (row && row.revoked) return "Revoked";
  if (row && Number.isInteger(row.expires) && row.expires > 0 && row.expires <= now) return "Expired";
  const left = row && Number.isInteger(row.left) ? row.left : 0;
  return left <= 0 ? "Used" : "Active";
}

export function deviceCount(bytes) {
  if (!(bytes instanceof Uint8Array) || bytes.length < RECORD_MAGIC.length) return 0;
  for (let i = 0; i < RECORD_MAGIC.length; i += 1) if (bytes[i] !== RECORD_MAGIC[i]) return 0;
  let at = RECORD_MAGIC.length;
  if (at >= bytes.length) return 0;
  at += 1 + bytes[at] + 8 + 32 + 16;
  at = skipChunk(bytes, at);
  at = skipChunk(bytes, at);
  if (at < 0 || at + 2 > bytes.length) return 0;
  return (bytes[at] << 8) | bytes[at + 1];
}

export async function adminAuthorized(request, env) {
  const expected = String((env && env.ADMIN_KEY) || "");
  const header = request.headers.get("authorization") || "";
  const presented = header.startsWith("Bearer ") ? header.slice("Bearer ".length) : "";
  if (!sameSecret(expected, presented)) return false;
  return accessJwtOk(request.headers.get("cf-access-jwt-assertion") || "", env || {});
}

export async function noteError(storage, name) {
  const safe = /^[A-Za-z]+$/.test(name) ? name : "Other";
  const now = Date.now();
  await storage.transaction(async (txn) => {
    let row = await txn.get("admin:errors");
    if (!row || !Number.isInteger(row.start) || now - row.start >= DAY_MS) row = { start: now, counts: {} };
    row.counts[safe] = (row.counts[safe] || 0) + 1;
    await txn.put("admin:errors", row);
  });
}

export async function handleAdmin(storage, method, path, body, allowed) {
  if (!(await bumpRate(storage))) {
    await audit(storage, "limited", {});
    return [429, { message: "Too many tries. Wait a minute." }];
  }
  if (!allowed) {
    await audit(storage, "denied", {});
    return [401, { message: "That key didn't work." }];
  }
  const value = body && typeof body === "object" && !Array.isArray(body) ? body : {};
  if (method === "GET" && path === "/v1/admin/codes") {
    await audit(storage, "list", {});
    return [200, { codes: await listCodes(storage) }];
  }
  if (method === "POST" && path === "/v1/admin/codes") return storeCode(storage, value);
  const revoke = /^\/v1\/admin\/codes\/([0-9a-f]{64})\/revoke$/.exec(path);
  if (method === "POST" && revoke) return revokeCode(storage, revoke[1]);
  if (method === "GET" && path === "/v1/admin/stats") {
    await audit(storage, "stats", {});
    return [200, await collectStats(storage)];
  }
  if (method === "POST" && path === "/v1/admin/signups") return setSignups(storage, value.paused);
  const remove = /^\/v1\/admin\/accounts\/([0-9a-f]{64})$/.exec(path);
  if (method === "DELETE" && remove) return deleteAccount(storage, remove[1], value.confirm);
  return [404, { message: "Not found." }];
}

async function listCodes(storage) {
  const rows = await storage.list({ prefix: "a:" });
  const codes = [];
  for (const [key, row] of rows) {
    const hash = key.slice(2);
    if (!/^[0-9a-f]{64}$/.test(hash) || !row || typeof row !== "object") continue;
    codes.push({
      hash,
      label: typeof row.label === "string" ? row.label : "",
      left: Number.isInteger(row.left) ? row.left : 0,
      created: Number.isInteger(row.created) ? row.created : 0,
      usedAt: Number.isInteger(row.usedAt) ? row.usedAt : 0,
      expires: Number.isInteger(row.expires) ? row.expires : 0,
      status: codeStatus(row),
    });
  }
  codes.sort((a, b) => b.created - a.created || a.hash.localeCompare(b.hash));
  return codes;
}

async function storeCode(storage, value) {
  const hash = String(value.hash || "").toLowerCase();
  const uses = Number.isInteger(value.uses) ? value.uses : Number(value.uses);
  if (!/^[0-9a-f]{64}$/.test(hash) || !Number.isInteger(uses) || uses < 1 || uses > MAX_USES) {
    return [400, { message: "Uses must be a number from 1 to 20." }];
  }
  const expires = expiryMs(value.expires);
  if (expires === null) return [400, { message: "That expiry didn't work." }];
  const label = cleanLabel(value.label);
  const key = "a:" + hash;
  if (await storage.get(key)) return [409, { message: "That code is already listed." }];
  const created = Date.now();
  await storage.put(key, { left: uses, uses, label, created, usedAt: 0, expires, revoked: false });
  await audit(storage, "mint", { hash });
  return [200, { hash, uses, label, created, expires }];
}

async function revokeCode(storage, hash) {
  const key = "a:" + hash;
  const row = await storage.get(key);
  if (!row) return [404, { message: "That code is not listed." }];
  row.revoked = true;
  await storage.put(key, row);
  await audit(storage, "revoke", { hash });
  return [200, { hash, status: "Revoked" }];
}

async function collectStats(storage) {
  const mailboxes = await storage.list({ prefix: "m:" });
  const signups = await storage.get("admin:signups");
  const storageRows = [];
  let devices = 0;
  for (const [key, mailbox] of mailboxes) {
    const account = key.slice(2);
    if (!/^[0-9a-f]{64}$/.test(account)) continue;
    const bytes = decodeRecord(mailbox && mailbox.record);
    const count = bytes ? deviceCount(bytes) : 0;
    devices += count;
    storageRows.push({ account, bytes: bytes ? bytes.length : 0, devices: count });
  }
  storageRows.sort((a, b) => a.account.localeCompare(b.account));
  return {
    accounts: storageRows.length,
    devices,
    paused: Boolean(signups && signups.paused),
    storage: storageRows,
    errors: await recentErrors(storage),
  };
}

async function recentErrors(storage) {
  const row = await storage.get("admin:errors");
  if (!row || !Number.isInteger(row.start) || Date.now() - row.start >= DAY_MS || !row.counts) return [];
  return Object.entries(row.counts)
    .filter(([name, count]) => /^[A-Za-z]+$/.test(name) && Number.isInteger(count))
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

async function setSignups(storage, paused) {
  if (typeof paused !== "boolean") return [400, { message: "Say whether new signups are paused." }];
  await storage.put("admin:signups", { paused });
  await audit(storage, paused ? "pause" : "resume", {});
  return [200, { paused }];
}

async function deleteAccount(storage, account, confirm) {
  if (confirm !== account) return [400, { message: "Type the account id to confirm." }];
  const key = "m:" + account;
  const row = await storage.get(key);
  if (!row) return [404, { message: "No sealed copy is stored for that account." }];
  await storage.delete(key);
  await audit(storage, "delete", { account });
  return [200, { ok: true }];
}

async function bumpRate(storage) {
  let count = 0;
  await storage.transaction(async (txn) => {
    const now = Date.now();
    const row = (await txn.get("admin:rate")) || { start: now, count: 0 };
    if (!Number.isInteger(row.start) || now - row.start >= WINDOW_MS) {
      row.start = now;
      row.count = 0;
    }
    row.count += 1;
    count = row.count;
    await txn.put("admin:rate", row);
  });
  return count <= RATE_CAP;
}

async function audit(storage, action, detail) {
  const known = ["mint", "revoke", "pause", "resume", "delete", "list", "stats", "denied", "limited"];
  if (!known.includes(action)) return;
  const entry = { at: Date.now(), action };
  if (detail && /^[0-9a-f]{64}$/.test(detail.hash || "")) entry.hash = detail.hash;
  if (detail && /^[0-9a-f]{64}$/.test(detail.account || "")) entry.account = detail.account;
  await storage.transaction(async (txn) => {
    const row = (await txn.get("admin:log")) || [];
    row.push(entry);
    while (row.length > LOG_CAP) row.shift();
    await txn.put("admin:log", row);
  });
}

function skipChunk(bytes, at) {
  if (at < 0 || at + 4 > bytes.length) return -1;
  const len = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(at);
  const next = at + 4 + len;
  if (next > bytes.length) return -1;
  return next;
}

function decodeRecord(record) {
  if (typeof record !== "string" || !record) return null;
  try {
    const bin = atob(record.trim());
    return Uint8Array.from(bin, (ch) => ch.charCodeAt(0));
  } catch {
    return null;
  }
}

async function accessJwtOk(token, env) {
  const team = String(env.CF_ACCESS_TEAM || "").trim();
  const aud = String(env.CF_ACCESS_AUD || "").trim();
  if (!team || !aud) return true;
  if (!token) return false;
  const parts = token.split(".");
  if (parts.length !== 3) return false;
  let header;
  let payload;
  try {
    header = JSON.parse(new TextDecoder().decode(b64urlDecode(parts[0])));
    payload = JSON.parse(new TextDecoder().decode(b64urlDecode(parts[1])));
  } catch {
    return false;
  }
  if (!header || header.alg !== "RS256" || typeof header.kid !== "string") return false;
  const audiences = Array.isArray(payload.aud) ? payload.aud : [payload.aud];
  if (!audiences.includes(aud)) return false;
  const host = team.includes(".") ? team : `${team}.cloudflareaccess.com`;
  if (payload.iss !== `https://${host}`) return false;
  if (typeof payload.exp !== "number" || payload.exp * 1000 <= Date.now()) return false;
  if (typeof payload.nbf === "number" && payload.nbf * 1000 > Date.now()) return false;
  const keys = await accessKeys(team);
  if (!keys) return false;
  const jwk = keys.find((key) => key && key.kid === header.kid);
  if (!jwk) return false;
  try {
    const key = await crypto.subtle.importKey(
      "jwk",
      jwk,
      { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
      false,
      ["verify"],
    );
    const data = enc.encode(`${parts[0]}.${parts[1]}`);
    return crypto.subtle.verify("RSASSA-PKCS1-v1_5", key, b64urlDecode(parts[2]), data);
  } catch {
    return false;
  }
}

async function accessKeys(team) {
  const hit = certs.get(team);
  if (hit && hit.until > Date.now()) return hit.keys;
  const host = team.includes(".") ? team : `${team}.cloudflareaccess.com`;
  const response = await fetch(`https://${host}/cdn-cgi/access/certs`);
  if (!response.ok) return null;
  const body = await response.json();
  const keys = Array.isArray(body.keys) ? body.keys : [];
  certs.set(team, { until: Date.now() + 60 * 60 * 1000, keys });
  return keys;
}

function b64urlDecode(text) {
  const pad = text.length % 4 === 0 ? "" : "=".repeat(4 - (text.length % 4));
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/") + pad;
  const bin = atob(b64);
  return Uint8Array.from(bin, (ch) => ch.charCodeAt(0));
}
