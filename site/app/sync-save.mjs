// A save remembers how many local edits it started with.
// An edit that arrives while the save is in flight stays dirty, so the next save sends it.
// Local changes are the pages that differ from the last copy that synced.
// A phone that matches that copy takes the remote one, so a deleted line stays deleted.

export function noteLocalEdit(state) {
  state.editGen = (state.editGen || 0) + 1;
  state.dirty = true;
  return state.editGen;
}

export function beginSave(state) {
  return state.editGen || 0;
}

export function finishSave(state, started) {
  if ((state.editGen || 0) === started) state.dirty = false;
  return state.dirty === true;
}

function pageMap(pages) {
  const map = new Map();
  for (const row of pages || []) {
    if (Array.isArray(row) && row.length >= 2 && row[0]) map.set(String(row[0]), String(row[1] ?? ""));
  }
  return map;
}

// A follow-up after a save is soon. An edit waits under half a second.
// A live socket is the notice. Without one, a visible tab asks every couple of seconds.
export const POLL_SOON_MS = 800;
export const POLL_ACTIVE_MS = 4000;
export const PUSH_DEBOUNCE_MS = 400;
export const FALLBACK_POLL_MS = 2000;
export const POLL_IDLE_MS = 30000;
export const POLL_HIDDEN_MS = 60000;
export const FETCH_TIMEOUT_MS = 12000;

export function pollWait(flags) {
  const row = flags || {};
  if (row.dirty || row.saving) return PUSH_DEBOUNCE_MS;
  if (row.soon || row.waiting) return POLL_SOON_MS;
  if (row.hidden) return POLL_HIDDEN_MS;
  if (row.live) return POLL_IDLE_MS;
  return FALLBACK_POLL_MS;
}

// The edit waits, then the mailbox tells the other device in the same write.
export function liveDeliveryMs() {
  return PUSH_DEBOUNCE_MS;
}

// The approval card leaves when the request is gone, refused, or past its window.
export function approvalCard(requests, id, now, created, inviteMs) {
  const row = (requests || []).find((item) => item && item.id === id);
  if (!row) return "expired";
  if (row.state !== "pending" && row.state !== "approved") return "expired";
  const born = Number(row.created || created || 0);
  if (born && now > born + inviteMs) return "expired";
  return "open";
}

export function chipLabel(flags) {
  const row = flags || {};
  if (row.removed) return "Sync paused";
  if (row.waiting) return "Device waiting";
  if (row.error) return "Not saved";
  if (row.offline) return "Offline";
  if (row.syncing) return "Syncing";
  if (row.listChanged) return "List changed";
  if (row.live) return "Live";
  // The quick connection is down. Ordinary requests still save, so this is not Offline.
  if (row.member) return "Syncing";
  return "";
}

// Turn on sync is the entry before setup. A paused cloud is only for a phone
// that already turned sync off, or was removed.
export function showTurnOnSync(status) {
  const row = status || {};
  return !row.account && !row.removed && !row.paused;
}

// A missing page stays. A tombstone removes that one page.
// More than two signed deletes waits. One or two still apply.
export function massDelete(localPages, doomed) {
  void localPages;
  return doomed > 2;
}

export const WELCOME_TITLE = "Welcome to Omacus";

export function pageHeading(text) {
  const line = String(text ?? "").split(/\r?\n/).map((row) => row.trim()).find((row) => row) || "";
  const title = line.replace(/^[#/\s]+/, "").trim();
  return title || "Untitled";
}

// Two Welcome to Omacus pages with the same text become one.
// The lower id stays. Each other id is a signed delete of that page.
// A different text stays its own page. Two other pages that only share a title stay two pages.
export function dedupeWelcome(pages, tombstones) {
  const rows = [];
  const seen = new Set();
  for (const row of pages || []) {
    if (!Array.isArray(row) || !row[0] || seen.has(String(row[0]))) continue;
    seen.add(String(row[0]));
    rows.push([String(row[0]), String(row[1] ?? "")]);
  }
  const groups = new Map();
  for (const row of rows) {
    if (pageHeading(row[1]) !== WELCOME_TITLE) continue;
    const key = row[1];
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row[0]);
  }
  const drop = [];
  let tombs = (Array.isArray(tombstones) ? tombstones : [])
    .filter((row) => row && row.id)
    .map((row) => ({ id: String(row.id), rev: Number(row.rev) || 0 }));
  for (const ids of groups.values()) {
    if (ids.length < 2) continue;
    ids.sort();
    for (const id of ids.slice(1)) drop.push(id);
  }
  for (const id of drop) tombs = noteTombstone(tombs, id);
  const gone = new Set(drop);
  return {
    pages: rows.filter((row) => !gone.has(row[0])),
    tombstones: tombs,
    removed: drop,
  };
}

export function removalSentence(count) {
  const n = Number(count) || 0;
  const word = n === 1 ? "page" : "pages";
  return `Another phone or computer removed ${n} ${word}. They are still here. Remove them here too, or keep them?`;
}

export function declinedCovers(declined, tomb) {
  const id = String(tomb?.id || "");
  if (!id) return false;
  const rev = Number(tomb?.rev) || 0;
  return (declined || []).some((row) => row && row.id === id && (Number(row.rev) || 0) >= rev);
}

export function noteTombstone(tombstones, id) {
  const list = Array.isArray(tombstones) ? tombstones.filter((row) => row && row.id) : [];
  const rev = list.reduce((max, row) => Math.max(max, Number(row.rev) || 0), 0) + 1;
  const next = list.filter((row) => row.id !== id);
  if (id) next.push({ id: String(id), rev });
  return next;
}

export function keepUnlessTombstone(local, remote, tombstones, declined) {
  const localRows = [];
  for (const row of local || []) {
    if (Array.isArray(row) && row[0]) localRows.push([String(row[0]), String(row[1] ?? "")]);
  }
  const remoteRows = [];
  const remoteIds = new Set();
  for (const row of remote || []) {
    if (!Array.isArray(row) || !row[0]) continue;
    const id = String(row[0]);
    if (remoteIds.has(id)) continue;
    remoteIds.add(id);
    remoteRows.push([id, String(row[1] ?? "")]);
  }
  const named = [];
  const asked = [];
  for (const tomb of tombstones || []) {
    const id = String(tomb?.id || "");
    if (!id || named.includes(id) || declinedCovers(declined, tomb)) continue;
    if (!localRows.some((row) => row[0] === id)) continue;
    named.push(id);
    asked.push({ id, rev: Number(tomb.rev) || 0 });
  }
  const held = massDelete(localRows.length, named.length);
  const pages = remoteRows.slice();
  const removed = [];
  for (const row of localRows) {
    const doomed = named.includes(row[0]);
    if (remoteIds.has(row[0])) {
      if (!held && doomed) {
        const at = pages.findIndex((page) => page[0] === row[0]);
        if (at >= 0) pages.splice(at, 1);
        removed.push(row[0]);
      }
      continue;
    }
    if (!held && doomed) {
      removed.push(row[0]);
      continue;
    }
    pages.push(row);
  }
  const keptLocal = pages.some((row) => !remoteIds.has(row[0]));
  return { pages, removed, held, keptLocal, asked: held ? asked : [] };
}

// Pages in `local` whose id is not in `remote`. A shared page is left out,
// so a deleted line on that page is not put back.
export function pagesOnlyHere(local, remote) {
  const there = new Set();
  for (const row of remote || []) {
    if (Array.isArray(row) && row[0]) there.add(String(row[0]));
  }
  const out = [];
  for (const row of local || []) {
    if (!Array.isArray(row) || row.length < 2 || !row[0]) continue;
    const id = String(row[0]);
    if (!there.has(id)) out.push([id, String(row[1] ?? "")]);
  }
  return out;
}

// A page in the last synced copy that the remote list does not name.
// It goes to Recently deleted. A page that was never synced is not here.
export function pagesLeftBehind(baseline, remotePages) {
  const remote = new Set((remotePages || []).map((row) => row && row[0]).filter(Boolean));
  const out = [];
  for (const row of baseline || []) {
    if (!row || !row[0] || remote.has(row[0])) continue;
    out.push([row[0], String(row[1] ?? "")]);
  }
  return out;
}

// `baseline` is null until a copy has synced. Any page then counts as a local change.
export function pagesDiffer(baseline, local) {
  const here = pageMap(local);
  if (baseline == null) return here.size > 0;
  const base = pageMap(baseline);
  if (base.size !== here.size) return true;
  for (const [id, text] of here) {
    if (base.get(id) !== text) return true;
  }
  return false;
}
