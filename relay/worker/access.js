// Invite codes for a new set of pages. Same bytes as relay/src/access.rs.
// The secret stays in the environment. A stored row is the hash and the uses left.

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const MARK = "omacus-access/1";
const MIN_SECRET = 16;
const MAX_USES = 20;
const text = new TextEncoder();

export async function mint(secret, uses = 1, id) {
  if (typeof secret !== "string" || secret.length < MIN_SECRET) return null;
  if (!Number.isInteger(uses) || uses < 1 || uses > MAX_USES) return null;
  const rawId = id ? new Uint8Array(id) : crypto.getRandomValues(new Uint8Array(6));
  if (rawId.length !== 6) return null;
  const mac = await tag(secret, rawId, uses);
  const raw = new Uint8Array(15);
  raw.set(rawId, 0);
  raw[6] = uses;
  raw.set(mac, 7);
  return group(encode(raw));
}

export async function openCode(secret, code) {
  if (typeof secret !== "string" || secret.length < MIN_SECRET) return null;
  const raw = decode(code);
  if (!raw) return null;
  const id = raw.subarray(0, 6);
  const uses = raw[6];
  if (uses < 1 || uses > MAX_USES) return null;
  const given = raw.subarray(7);
  const expect = await tag(secret, id, uses);
  if (!same(expect, given)) return null;
  const normalized = normalize(code);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", text.encode(normalized)));
  const hash = [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
  return { uses, hash };
}

export function spend(table, hash, uses, take) {
  const left = Object.hasOwn(table, hash) ? table[hash] : uses;
  if (left <= 0) return false;
  if (take) table[hash] = left - 1;
  return true;
}

async function tag(secret, id, uses) {
  const key = await crypto.subtle.importKey("raw", text.encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const body = new Uint8Array(MARK.length + id.length + 1);
  body.set(text.encode(MARK), 0);
  body.set(id, MARK.length);
  body[body.length - 1] = uses;
  const sig = new Uint8Array(await crypto.subtle.sign("HMAC", key, body));
  return sig.subarray(0, 8);
}

function encode(bytes) {
  let bits = 0n;
  for (const byte of bytes) bits = (bits << 8n) | BigInt(byte);
  const chars = Array(24);
  for (let slot = 23; slot >= 0; slot -= 1) {
    chars[slot] = ALPHABET[Number(bits & 31n)];
    bits >>= 5n;
  }
  return chars.join("");
}

function decode(text) {
  const cleaned = normalize(text);
  if (!cleaned) return null;
  let bits = 0n;
  for (const ch of cleaned) {
    const index = ALPHABET.indexOf(ch);
    if (index < 0) return null;
    bits = (bits << 5n) | BigInt(index);
  }
  const out = new Uint8Array(15);
  for (let slot = 14; slot >= 0; slot -= 1) {
    out[slot] = Number(bits & 255n);
    bits >>= 8n;
  }
  return out;
}

function normalize(value) {
  let cleaned = "";
  for (const ch of String(value || "")) {
    if (/\s/.test(ch) || ch === "-") continue;
    const up = ch.toUpperCase();
    cleaned += up === "I" || up === "L" ? "1" : up === "O" ? "0" : up;
  }
  return cleaned.length === 24 ? cleaned : null;
}

function group(raw) {
  return raw.match(/.{4}/g).join("-");
}

function same(left, right) {
  if (left.length !== right.length) return false;
  let diff = 0;
  for (let i = 0; i < left.length; i += 1) diff |= left[i] ^ right[i];
  return diff === 0;
}
