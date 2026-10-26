// Nightly copies of sealed accounts. A copy is the same sealed bytes the
// server already holds. It is not a way to open a page.
//
// A backup store is put, get, and list. R2 is one store. Tests pass an
// in-memory store. The worker does not delete old nights. The bucket's
// lifecycle rule removes a night after RETENTION_DAYS.

export const RETENTION_DAYS = 30;

export function nightOf(ms) {
  const date = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
}

export function nightPrefix(day) {
  return `nights/${day}/`;
}

export function dayFromKey(key) {
  const match = /^nights\/(\d{4}-\d{2}-\d{2})\//.exec(String(key || ""));
  return match ? match[1] : "";
}

export function memoryBackupStore(rows = new Map()) {
  return {
    kind: "backup",
    async put(key, text) {
      rows.set(key, String(text));
    },
    async get(key) {
      return rows.has(key) ? rows.get(key) : null;
    },
    async list(prefix) {
      return [...rows.keys()].filter((key) => !prefix || key.startsWith(prefix)).sort();
    },
  };
}

export function r2BackupStore(bucket) {
  return {
    async put(key, text) {
      await bucket.put(key, text);
    },
    async get(key) {
      const got = await bucket.get(key);
      if (!got) return null;
      if (typeof got === "string") return got;
      if (typeof got.text === "function") return got.text();
      return null;
    },
    async list(prefix) {
      const keys = [];
      let cursor = "";
      do {
        const page = await bucket.list(cursor ? { prefix, cursor } : { prefix });
        for (const object of page.objects || []) keys.push(object.key);
        cursor = page.truncated ? page.cursor || "" : "";
      } while (cursor);
      return keys;
    },
  };
}

export function openBackupStore(env) {
  const bucket = env && env.BACKUPS;
  if (!bucket || typeof bucket.put !== "function" || typeof bucket.get !== "function") return null;
  if (bucket.kind === "backup" && typeof bucket.list === "function") return bucket;
  if (typeof bucket.list !== "function") return null;
  return r2BackupStore(bucket);
}
