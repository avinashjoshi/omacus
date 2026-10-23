// Runs on omacus.com. Sends this browser's saved pages to app.omacus.com and
// waits for that origin to confirm it stored them. Nothing here is deleted.

import { FLAG, NEW_ORIGIN, decideAck, isStoredKey } from "./migrate-protocol.js";

export function collectEntries(storage = localStorage) {
  const entries = [];
  try {
    for (let i = 0; i < storage.length; i++) {
      const key = storage.key(i);
      if (!isStoredKey(key)) continue;
      const value = storage.getItem(key);
      if (typeof value === "string") entries.push([key, value]);
    }
  } catch {
    return [];
  }
  return entries;
}

export async function migrateStoredPages() {
  let entries = [];
  try {
    if (localStorage.getItem(FLAG) === "1") return { status: "done" };
    entries = collectEntries();
  } catch {
    return { status: "empty" };
  }
  if (!entries.length) return { status: "empty" };
  const token = crypto.randomUUID?.() || `${Date.now()}-${Math.random()}`;
  const ok = await handoff(entries, token);
  if (!ok) return { status: "failed" };
  try {
    localStorage.setItem(FLAG, "1");
  } catch {
    // The new origin already confirmed. The flag only skips a second copy.
  }
  return { status: "migrated" };
}

function handoff(entries, token) {
  return new Promise((resolve) => {
    const iframe = document.createElement("iframe");
    iframe.hidden = true;
    iframe.title = "Move saved pages";
    iframe.src = `${NEW_ORIGIN}/migrate.html`;
    const timer = setTimeout(finish, 8000, false);
    function finish(ok) {
      clearTimeout(timer);
      removeEventListener("message", onMessage);
      iframe.remove();
      resolve(ok);
    }
    function onMessage(e) {
      if (e.source !== iframe.contentWindow || e.origin !== NEW_ORIGIN) return;
      if (e.data?.type === "omacus-migrate-ready") {
        iframe.contentWindow.postMessage({ type: "omacus-migrate", token, entries }, NEW_ORIGIN);
        return;
      }
      const decision = decideAck({ origin: e.origin, data: e.data, token });
      if (decision.drop) return;
      if (decision.mark) finish(true);
    }
    addEventListener("message", onMessage);
    document.body.append(iframe);
  });
}
