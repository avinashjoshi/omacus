// Request signatures for the sync test relay.
// A linked device signs the method, path, time, nonce, and body hash.
// This file does not open a page. It only checks public keys already in the record.

const enc = new TextEncoder();
const WINDOW_MS = 5 * 60 * 1000;
// One phone polling every few seconds, plus a save, stays well under this.
// The cap is per device. A nonce is still unique across the whole account.
export const MAX_NONCES = 512;
const RECOVERY_ID = enc.encode("recovery-signer\0");

const concat = (...parts) => {
  const out = new Uint8Array(parts.reduce((n, part) => n + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
};
const u16 = (n) => new Uint8Array([(n >> 8) & 255, n & 255]);
const u32 = (n) => new Uint8Array([(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255]);
const u64 = (n) => {
  let value = BigInt(n);
  const out = new Uint8Array(8);
  for (let i = 7; i >= 0; i -= 1) {
    out[i] = Number(value & 255n);
    value >>= 8n;
  }
  return out;
};
const hex = (bytes) => [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (text, size) => {
  if (typeof text !== "string" || text.length !== size * 2 || !/^[0-9a-f]+$/i.test(text)) return null;
  const out = new Uint8Array(size);
  for (let i = 0; i < size; i += 1) out[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
  return out;
};
const same = (a, b) => a.length === b.length && a.every((byte, i) => byte === b[i]);
const b64 = (bytes) => {
  let text = "";
  for (const byte of bytes) text += String.fromCharCode(byte);
  return btoa(text);
};
const fromB64 = (text) => {
  try {
    const raw = atob(String(text).trim());
    return Uint8Array.from(raw, (ch) => ch.charCodeAt(0));
  } catch {
    return null;
  }
};

function chunk(bytes) {
  return concat(u32(bytes.length), bytes);
}

async function verify(publicKey, signature, message) {
  const key = await crypto.subtle.importKey("raw", publicKey, { name: "Ed25519" }, false, ["verify"]);
  return crypto.subtle.verify("Ed25519", key, signature, message);
}

function listMessage(list) {
  const body = [u16(list.devices.length)];
  for (const device of list.devices) body.push(device.id, device.sign, device.receive);
  return concat(enc.encode("omacus-devices/1"), u64(list.version), list.prev, list.signer, ...body);
}

function changedByOne(previous, next) {
  if (next.length === previous.length + 1) {
    const added = next[next.length - 1];
    return next.slice(0, previous.length).every((device, i) => same(device.id, previous[i].id) && same(device.sign, previous[i].sign) && same(device.receive, previous[i].receive))
      && previous.every((device) => !same(device.id, added.id));
  }
  if (previous.length < 2 || next.length + 1 !== previous.length) return false;
  let skipped = false;
  let index = 0;
  for (const device of previous) {
    if (index < next.length && same(device.id, next[index].id) && same(device.sign, next[index].sign) && same(device.receive, next[index].receive)) {
      index += 1;
    } else if (!skipped) skipped = true;
    else return false;
  }
  return skipped && index === next.length;
}

async function openChain(bytes) {
  if (bytes.length < 18 || !same(bytes.slice(0, 16), enc.encode("OMACUS-DEVICES/1"))) return null;
  let at = 16;
  const take = (n) => {
    if (at + n > bytes.length) return null;
    const out = bytes.slice(at, at + n);
    at += n;
    return out;
  };
  const count = take(2);
  if (!count) return null;
  const lists = (count[0] << 8) | count[1];
  if (lists < 1 || lists > 64) return null;
  const out = [];
  for (let place = 0; place < lists; place += 1) {
    const versionBytes = take(8);
    const prev = take(32);
    const signer = take(16);
    const signature = take(64);
    const deviceCountBytes = take(2);
    if (!versionBytes || !prev || !signer || !signature || !deviceCountBytes) return null;
    const version = versionBytes.reduce((n, byte) => n * 256n + BigInt(byte), 0n);
    if (version !== BigInt(place + 1)) return null;
    const deviceCount = (deviceCountBytes[0] << 8) | deviceCountBytes[1];
    if (deviceCount < 1 || deviceCount > 64) return null;
    const devices = [];
    const seen = new Set();
    for (let i = 0; i < deviceCount; i += 1) {
      const id = take(16);
      const sign = take(32);
      const receive = take(32);
      if (!id || !sign || !receive) return null;
      const key = hex(id);
      if (seen.has(key)) return null;
      seen.add(key);
      devices.push({ id, sign, receive });
    }
    if (place === 0) {
      if (prev.some((byte) => byte !== 0) || devices.length !== 1 || !same(signer, devices[0].id)) return null;
    } else {
      const previous = out[place - 1];
      const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", listMessage(previous)));
      if (!same(prev, hash) || !changedByOne(previous.devices, devices)) return null;
      if (!previous.devices.some((device) => same(device.id, signer))) return null;
    }
    const list = { version: Number(version), prev, signer, devices };
    const row = devices.find((device) => same(device.id, signer));
    if (!row || !await verify(row.sign, signature, listMessage(list))) return null;
    out.push(list);
  }
  return at === bytes.length ? out : null;
}

async function openRecord(bytes) {
  if (!bytes || bytes.length < 15) return null;
  const magic = bytes.slice(0, 14);
  if (!same(magic, enc.encode("OMACUS-SYNC/1\n")) && !same(magic, enc.encode("OMACUS-SYNC/2\n"))) return null;
  let at = 14;
  const take = (n) => {
    if (at + n > bytes.length) return null;
    const out = bytes.slice(at, at + n);
    at += n;
    return out;
  };
  const vaultLenByte = take(1);
  if (!vaultLenByte) return null;
  const vault = take(vaultLenByte[0]);
  if (!vault || !same(vault, enc.encode("personal"))) return null;
  if (!take(8) || !take(32)) return null;
  const account = bytes.slice(at - 32, at);
  if (!take(16)) return null;
  const chainLenBytes = take(4);
  if (!chainLenBytes) return null;
  const chainLen = new DataView(chainLenBytes.buffer, chainLenBytes.byteOffset, 4).getUint32(0);
  const chain = take(chainLen);
  if (!chain) return null;
  const phraseLenBytes = take(4);
  if (!phraseLenBytes) return null;
  const phraseLen = new DataView(phraseLenBytes.buffer, phraseLenBytes.byteOffset, 4).getUint32(0);
  if (!take(phraseLen)) return null;
  const wrapCountBytes = take(2);
  if (!wrapCountBytes) return null;
  const wrapCount = (wrapCountBytes[0] << 8) | wrapCountBytes[1];
  for (let i = 0; i < wrapCount; i += 1) {
    const lenBytes = take(4);
    if (!lenBytes) return null;
    const len = new DataView(lenBytes.buffer, lenBytes.byteOffset, 4).getUint32(0);
    if (!take(len)) return null;
  }
  const pageLenBytes = take(4);
  if (!pageLenBytes) return null;
  const pageLen = new DataView(pageLenBytes.buffer, pageLenBytes.byteOffset, 4).getUint32(0);
  if (!take(pageLen)) return null;
  let signed = false;
  let signer = null;
  let signature = null;
  if (at !== bytes.length) {
    if (bytes.length - at !== 80) return null;
    signer = take(16);
    signature = take(64);
    signed = true;
  }
  const lists = await openChain(chain);
  if (!lists) return null;
  if (signed) {
    const prefix = bytes.slice(0, bytes.length - 80);
    if (!await recordOk(lists, account, signer, signature, prefix)) return null;
  }
  return { account, lists, signed };
}

async function recordOk(lists, account, signer, signature, prefix) {
  const latest = lists[lists.length - 1];
  let key = null;
  if (same(signer, RECOVERY_ID)) {
    const row = latest.devices.find((device) => same(device.id, RECOVERY_ID));
    if (row && !same(row.sign, account)) return false;
    key = account;
  } else {
    const row = latest.devices.find((device) => same(device.id, signer));
    if (!row) return false;
    key = row.sign;
  }
  return verify(key, signature, concat(enc.encode("omacus-record/1"), prefix));
}

function verifyKey(opened, device) {
  const latest = opened.lists[opened.lists.length - 1];
  if (same(device, RECOVERY_ID)) {
    const row = latest.devices.find((item) => same(item.id, RECOVERY_ID));
    if (row) return same(row.sign, opened.account) ? opened.account : null;
    return opened.account;
  }
  const row = latest.devices.find((item) => same(item.id, device));
  return row ? row.sign : null;
}

function authMessage(method, path, time, nonce, raw) {
  return crypto.subtle.digest("SHA-256", enc.encode(raw)).then((digest) => concat(
    enc.encode(`omacus-auth/1\n${method.toUpperCase()}\n${path}\n`),
    u64(time),
    nonce,
    new Uint8Array(digest),
  ));
}

function nonceBook(mailbox) {
  const raw = mailbox.nonces;
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    mailbox.nonces = {};
    return mailbox.nonces;
  }
  const values = Object.values(raw);
  // Older copies stored one flat map of nonce → time for the whole account.
  if (values.length && values.every((value) => typeof value === "number")) {
    mailbox.nonces = { "": raw };
  }
  return mailbox.nonces;
}

function pruneBook(book, now) {
  for (const device of Object.keys(book)) {
    const bucket = book[device];
    if (!bucket || typeof bucket !== "object") {
      delete book[device];
      continue;
    }
    for (const [nonce, at] of Object.entries(bucket)) {
      if (Number(at) + WINDOW_MS < now) delete bucket[nonce];
    }
    if (!Object.keys(bucket).length) delete book[device];
  }
}

function nonceSeen(book, nonce) {
  return Object.values(book).some((bucket) => bucket && typeof bucket === "object" && Object.hasOwn(bucket, nonce));
}

// "ok" remembers the nonce. "busy" is a replay, or that phone's list is full.
function rememberNonce(mailbox, device, nonce, time, now) {
  const book = nonceBook(mailbox);
  pruneBook(book, now);
  if (nonceSeen(book, nonce)) return "busy";
  const bucket = book[device] && typeof book[device] === "object" ? book[device] : {};
  book[device] = bucket;
  if (Object.keys(bucket).length >= MAX_NONCES) return "busy";
  bucket[nonce] = time;
  return "ok";
}

// The current device list is a longer extension of the older one.
export async function chainIsLongerExtension(currentB64, olderB64) {
  const current = fromB64(currentB64);
  const older = fromB64(olderB64);
  const next = current ? await openRecord(current) : null;
  const prev = older ? await openRecord(older) : null;
  if (!next || !prev || next.lists.length <= prev.lists.length) return false;
  for (let i = 0; i < prev.lists.length; i += 1) {
    if (!same(listMessage(next.lists[i]), listMessage(prev.lists[i]))) return false;
  }
  return true;
}

export async function authorize(method, path, raw, headers, recordB64, now, mailbox) {
  const device = unhex(headers.get("x-omacus-device") || "", 16);
  const nonce = unhex(headers.get("x-omacus-nonce") || "", 16);
  const signature = unhex(headers.get("x-omacus-signature") || "", 64);
  const time = Number(headers.get("x-omacus-time"));
  if (!device || !nonce || !signature || !Number.isSafeInteger(time)) return null;
  if (time + WINDOW_MS < now || now + WINDOW_MS < time) return null;
  const record = fromB64(recordB64);
  const opened = record ? await openRecord(record) : null;
  if (!opened) return null;
  const key = verifyKey(opened, device);
  if (!key) return null;
  const message = await authMessage(method, path, time, nonce, raw || "");
  if (!await verify(key, signature, message)) return null;
  if (rememberNonce(mailbox, hex(device), hex(nonce), time, now) !== "ok") return { busy: true };
  return { recovery: same(device, RECOVERY_ID) };
}

export async function allowPut(storedB64, incomingB64, recovery) {
  const incoming = fromB64(incomingB64);
  const next = incoming ? await openRecord(incoming) : null;
  if (!next || !next.signed) return false;
  if (recovery) return true;
  const stored = fromB64(storedB64);
  const previous = stored ? await openRecord(stored) : null;
  if (!previous) return false;
  const shared = Math.min(next.lists.length, previous.lists.length);
  for (let i = 0; i < shared; i += 1) {
    const left = listMessage(next.lists[i]);
    const right = listMessage(previous.lists[i]);
    if (!same(left, right)) return false;
  }
  return next.lists.length >= previous.lists.length;
}

export async function makeDevice(accountHex) {
  const key = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
  const pub = new Uint8Array(await crypto.subtle.exportKey("raw", key.publicKey));
  const id = crypto.getRandomValues(new Uint8Array(16));
  const receive = crypto.getRandomValues(new Uint8Array(32));
  const account = unhex(accountHex, 32);
  return { id, pub, receive, privateKey: key.privateKey, account };
}

async function chainBytes(device) {
  const prev = new Uint8Array(32);
  const devices = [{ id: device.id, sign: device.pub, receive: device.receive }];
  const list = { version: 1, prev, signer: device.id, devices };
  const signature = new Uint8Array(await crypto.subtle.sign("Ed25519", device.privateKey, listMessage(list)));
  const one = concat(device.id, device.pub, device.receive);
  const body = concat(u64(1), prev, device.id, signature, u16(1), one);
  return concat(enc.encode("OMACUS-DEVICES/1"), u16(1), body);
}

export async function deviceChain(device) {
  return chainBytes(device);
}

export async function deviceListed(recordB64, deviceB64) {
  const record = fromB64(recordB64);
  const device = fromB64(deviceB64);
  if (!record || !device || device.length < 16) return false;
  const opened = await openRecord(record);
  if (!opened || !opened.lists.length) return false;
  const id = device.slice(0, 16);
  const latest = opened.lists[opened.lists.length - 1];
  return latest.devices.some((row) => same(row.id, id));
}

export async function appendDevice(signer, chain, newcomer) {
  const lists = await openChain(chain);
  if (!lists || !lists.length) return null;
  const previous = lists[lists.length - 1];
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", listMessage(previous)));
  const devices = previous.devices.concat([{ id: newcomer.id, sign: newcomer.pub, receive: newcomer.receive }]);
  const list = { version: previous.version + 1, prev: hash, signer: signer.id, devices };
  const signature = new Uint8Array(await crypto.subtle.sign("Ed25519", signer.privateKey, listMessage(list)));
  const rows = devices.map((row) => concat(row.id, row.sign, row.receive));
  const body = concat(u64(list.version), hash, signer.id, signature, u16(devices.length), ...rows);
  return concat(chain.slice(0, 16), u16(lists.length + 1), chain.slice(18), body);
}

export async function dropDevice(signer, chain, removeId) {
  const lists = await openChain(chain);
  if (!lists || !lists.length) return null;
  const previous = lists[lists.length - 1];
  const devices = previous.devices.filter((row) => !same(row.id, removeId));
  if (devices.length + 1 !== previous.devices.length) return null;
  if (!devices.some((row) => same(row.id, signer.id))) return null;
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", listMessage(previous)));
  const list = { version: previous.version + 1, prev: hash, signer: signer.id, devices };
  const signature = new Uint8Array(await crypto.subtle.sign("Ed25519", signer.privateKey, listMessage(list)));
  const rows = devices.map((row) => concat(row.id, row.sign, row.receive));
  const body = concat(u64(list.version), hash, signer.id, signature, u16(devices.length), ...rows);
  return concat(chain.slice(0, 16), u16(lists.length + 1), chain.slice(18), body);
}

export async function makeRecord(device, text, version = 2, chain) {
  const used = chain || await chainBytes(device);
  const magic = enc.encode(version === 1 ? "OMACUS-SYNC/1\n" : "OMACUS-SYNC/2\n");
  const vault = enc.encode("personal");
  const page = enc.encode(text);
  const prefix = concat(
    magic,
    new Uint8Array([vault.length]),
    vault,
    u64(1),
    device.account,
    device.id,
    chunk(used),
    chunk(new Uint8Array()),
    u16(0),
    chunk(page),
  );
  const signature = new Uint8Array(await crypto.subtle.sign(
    "Ed25519",
    device.privateKey,
    concat(enc.encode("omacus-record/1"), prefix),
  ));
  return b64(concat(prefix, device.id, signature));
}

export async function authHeaders(device, method, path, raw, time = Date.now()) {
  const nonce = crypto.getRandomValues(new Uint8Array(16));
  const message = await authMessage(method, path, time, nonce, raw || "");
  const signature = new Uint8Array(await crypto.subtle.sign("Ed25519", device.privateKey, message));
  return {
    "x-omacus-device": hex(device.id),
    "x-omacus-time": String(time),
    "x-omacus-nonce": hex(nonce),
    "x-omacus-signature": hex(signature),
  };
}
