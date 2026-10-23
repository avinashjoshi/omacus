// The one-time handoff from omacus.com to app.omacus.com. Both sides import this
// so the origin check and the receipt check cannot drift apart.

export const NEW_ORIGIN = "https://app.omacus.com";
export const OLD_ORIGIN = "https://omacus.com";
export const FLAG = "omacus:migrated";

const PREFIX = "omacus:";

export function isStoredKey(key) {
  return typeof key === "string" && key.startsWith(PREFIX) && key !== FLAG;
}

export function validateIncoming(origin, data) {
  if (origin !== OLD_ORIGIN) return false;
  if (!data || data.type !== "omacus-migrate" || typeof data.token !== "string" || !data.token) return false;
  if (!Array.isArray(data.entries)) return false;
  return data.entries.every(
    (row) =>
      Array.isArray(row) &&
      row.length === 2 &&
      isStoredKey(row[0]) &&
      typeof row[1] === "string",
  );
}

// `drop` is always false: the old origin keeps its copy. `mark` is the flag that
// says the new origin has confirmed a copy, and only then.
export function decideAck({ origin, data, token }) {
  const drop = false;
  if (origin !== NEW_ORIGIN) return { mark: false, redirect: false, drop };
  if (!data || data.type !== "omacus-migrate-ack" || data.ok !== true || data.token !== token) {
    return { mark: false, redirect: false, drop };
  }
  return { mark: true, redirect: true, drop };
}

export function destinationUrl(loc = {}) {
  const url = new URL("/", NEW_ORIGIN);
  url.search = loc.search || "";
  url.hash = loc.hash || "";
  return url.href;
}
