// Nightly copies of sealed accounts. A copy is the same sealed bytes the
// server already holds. It is not a way to open a page.

export const KEEP_NIGHTS = 14;

export function nightOf(ms) {
  const date = new Date(ms);
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`;
}

export function nightPrefix(day) {
  return `nights/${day}/`;
}

export function nightsToDrop(days, keep = KEEP_NIGHTS) {
  const sorted = [...new Set(days.filter((day) => /^\d{4}-\d{2}-\d{2}$/.test(day)))].sort();
  if (sorted.length <= keep) return [];
  return sorted.slice(0, sorted.length - keep);
}

export function dayFromKey(key) {
  const match = /^nights\/(\d{4}-\d{2}-\d{2})\//.exec(String(key || ""));
  return match ? match[1] : "";
}
