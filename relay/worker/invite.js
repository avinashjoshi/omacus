// The short code a linked device shows. The worker stores a slow hash of it, never the code.
// Same bytes as relay/src/invite.rs. 12 characters, 60 bits, grouped for typing.
// The hash is PBKDF2-HMAC-SHA256 so a hostile server cannot grind the stored hash.

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const INVITE_MS = 5 * 60 * 1000;
export const SECRET_LEN = 8;
export const CODE_CHARS = 12;
export const CODE_KDF_ITERS = 20_000;
export const JOIN_TRIES = 8;
const CODE_KDF_SALT = new TextEncoder().encode("omacus-join-v1");

export function canonicalSecret(secret) {
  const out = new Uint8Array(SECRET_LEN);
  const src = secret instanceof Uint8Array ? secret : Uint8Array.from(secret || []);
  out.set(src.subarray(0, SECRET_LEN));
  out[0] &= 0x0f;
  return out;
}

export function encodeCode(secret) {
  const bytes = canonicalSecret(secret);
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  const chars = Array(CODE_CHARS);
  for (let slot = CODE_CHARS - 1; slot >= 0; slot -= 1) {
    chars[slot] = ALPHABET[Number(value & 31n)];
    value >>= 5n;
  }
  return `${chars.slice(0, 4).join("")}-${chars.slice(4, 8).join("")}-${chars.slice(8).join("")}`;
}

export function decodeCode(text) {
  let cleaned = "";
  for (const ch of String(text || "")) {
    if (/\s/.test(ch) || ch === "-") continue;
    const up = ch.toUpperCase();
    cleaned += up === "I" || up === "L" ? "1" : up === "O" ? "0" : up;
  }
  if (cleaned.length !== CODE_CHARS) return null;
  let value = 0n;
  for (const ch of cleaned) {
    const index = ALPHABET.indexOf(ch);
    if (index < 0) return null;
    value = (value << 5n) | BigInt(index);
  }
  const out = new Uint8Array(SECRET_LEN);
  for (let slot = SECRET_LEN - 1; slot >= 0; slot -= 1) {
    out[slot] = Number(value & 255n);
    value >>= 8n;
  }
  out[0] &= 0x0f;
  return out;
}

export async function hashHex(secret) {
  const bytes = canonicalSecret(secret);
  const key = await crypto.subtle.importKey("raw", bytes, "PBKDF2", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits(
    { name: "PBKDF2", hash: "SHA-256", salt: CODE_KDF_SALT, iterations: CODE_KDF_ITERS },
    key,
    256,
  );
  return [...new Uint8Array(bits)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function hmacBytes(secret, device) {
  const key = await crypto.subtle.importKey("raw", secret, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", key, device));
}

export function sameBytes(left, right) {
  if (!left || !right || left.length !== right.length) return false;
  let diff = 0;
  for (let i = 0; i < left.length; i += 1) diff |= left[i] ^ right[i];
  return diff === 0;
}
