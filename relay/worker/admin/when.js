// Times on the admin page. The words are how long ago. The exact time is the title.

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export function exactUtc(ms) {
  if (!ms) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n) => String(n).padStart(2, "0");
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())} UTC`;
}

export function relativeWhen(ms, now = Date.now()) {
  if (!ms) return "—";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "—";
  const delta = Math.max(0, now - ms);
  if (delta < MINUTE) return "Just now";
  if (delta < HOUR) {
    const n = Math.floor(delta / MINUTE);
    return n === 1 ? "1 minute ago" : `${n} minutes ago`;
  }
  if (delta < DAY) {
    const n = Math.floor(delta / HOUR);
    return n === 1 ? "1 hour ago" : `${n} hours ago`;
  }
  if (delta < 7 * DAY) {
    const n = Math.floor(delta / DAY);
    return n === 1 ? "1 day ago" : `${n} days ago`;
  }
  return `${MONTHS[date.getUTCMonth()]} ${date.getUTCDate()}, ${date.getUTCFullYear()}`;
}
