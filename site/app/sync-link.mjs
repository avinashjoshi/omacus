// A recovery or join picture is a link. The secret is only after "#".
// A browser does not send that part. This page removes it from the address bar.

const MAX_FRAGMENT = 2048;

export function encodeFragment(secret) {
  const bytes = new TextEncoder().encode(String(secret));
  let bin = "";
  for (const byte of bytes) bin += String.fromCharCode(byte);
  return btoa(bin).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

export function decodeFragment(text) {
  const cleaned = String(text || "").replaceAll("-", "+").replaceAll("_", "/");
  const pad = cleaned.length % 4 === 0 ? "" : "=".repeat((4 - (cleaned.length % 4)) % 4);
  const bin = atob(cleaned + pad);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

export function pageOrigin(loc = globalThis.location, root = globalThis.document?.documentElement) {
  const host = loc?.hostname || "";
  if (host === "app.omacus.com" || host === "omacus.com" || host === "www.omacus.com") return "https://app.omacus.com";
  return loc?.origin || "";
}

export function recoveryLink(page, phrase) {
  return fragmentLink(page, "recover", phrase);
}

export function joinLink(page, relay, account, code) {
  return fragmentLink(page, "join", `omacus-join\n${relay}\n${account}\n${code}\n`);
}

function fragmentLink(page, key, secret) {
  const base = String(page || "").trim().replace(/\/$/, "");
  return `${base}/#${key}=${encodeFragment(secret)}`;
}

function fragmentSecret(raw, key) {
  const text = String(raw || "").trim();
  if (!text) return null;
  let encoded = "";
  if (text.startsWith("#")) {
    const hash = text.slice(1);
    if (!hash.startsWith(`${key}=`)) return null;
    encoded = hash.slice(key.length + 1).split("&")[0];
  } else if (text.startsWith(`${key}=`)) {
    encoded = text.slice(key.length + 1).split("&")[0];
  } else {
    let url;
    try {
      url = new URL(text);
    } catch {
      return null;
    }
    if (url.search.includes(`${key}=`)) return null;
    const hash = url.hash.startsWith("#") ? url.hash.slice(1) : "";
    if (!hash.startsWith(`${key}=`)) return null;
    encoded = hash.slice(key.length + 1).split("&")[0];
  }
  if (!encoded || encoded.length > MAX_FRAGMENT) return null;
  let token = encoded;
  try {
    token = decodeURIComponent(encoded);
  } catch {
    return null;
  }
  if (!/^[A-Za-z0-9_-]+$/.test(token)) return null;
  try {
    const secret = decodeFragment(token);
    if (!secret || secret.includes("\u0000")) return null;
    return secret;
  } catch {
    return null;
  }
}

export function parseRecovery(raw) {
  const text = String(raw || "").trim();
  if (text.startsWith("omacus-recovery")) return text.replace(/^omacus-recovery\s*/, "");
  return fragmentSecret(text, "recover");
}

export function parseJoinPayload(raw) {
  const text = String(raw || "").trim();
  const secret = text.startsWith("omacus-join") ? text : fragmentSecret(text, "join");
  if (secret == null) return null;
  const lines = String(secret).split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
  if (lines[0] !== "omacus-join" || lines.length < 4) return null;
  return { origin: lines[1], account: lines[2], code: lines[3] };
}

let pending = null;

export function captureSyncLink(loc = globalThis.location, historyObj = globalThis.history) {
  const hash = String(loc?.hash || "");
  if (!hash.startsWith("#recover=") && !hash.startsWith("#join=")) return false;
  const path = `${loc.pathname || ""}${loc.search || ""}`;
  historyObj?.replaceState?.(null, "", path || "/");
  const kind = hash.startsWith("#recover=") ? "recover" : "join";
  const secret = fragmentSecret(hash, kind);
  if (secret) pending = { kind, secret };
  return true;
}

export function takeSyncLink() {
  const found = pending;
  pending = null;
  return found;
}
