// The short code a linked device shows. The worker stores a hash of it, never the code.
// Same bytes as relay/src/invite.rs (code 0410-6105, hash and check value locked there).

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
export const INVITE_MS = 5 * 60 * 1000;

export function encodeCode(secret) {
  let value = 0n;
  for (const byte of secret) value = (value << 8n) | BigInt(byte);
  const chars = Array(8);
  for (let slot = 7; slot >= 0; slot -= 1) {
    chars[slot] = ALPHABET[Number(value & 31n)];
    value >>= 5n;
  }
  return `${chars.slice(0, 4).join("")}-${chars.slice(4).join("")}`;
}

export function decodeCode(text) {
  let cleaned = "";
  for (const ch of String(text || "")) {
    if (/\s/.test(ch) || ch === "-") continue;
    const up = ch.toUpperCase();
    cleaned += up === "I" || up === "L" ? "1" : up === "O" ? "0" : up;
  }
  if (cleaned.length !== 8) return null;
  let value = 0n;
  for (const ch of cleaned) {
    const index = ALPHABET.indexOf(ch);
    if (index < 0) return null;
    value = (value << 5n) | BigInt(index);
  }
  const out = new Uint8Array(5);
  for (let slot = 4; slot >= 0; slot -= 1) {
    out[slot] = Number(value & 255n);
    value >>= 8n;
  }
  return out;
}

export async function hashHex(secret) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", secret));
  return [...digest].map((byte) => byte.toString(16).padStart(2, "0")).join("");
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
