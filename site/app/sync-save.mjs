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

// A follow-up after a save is soon. A quiet visible tab waits. A hidden tab waits longer.
export const POLL_SOON_MS = 800;
export const POLL_ACTIVE_MS = 4000;
export const POLL_IDLE_MS = 30000;
export const POLL_HIDDEN_MS = 60000;

export function pollWait(flags) {
  const row = flags || {};
  if (row.soon) return POLL_SOON_MS;
  if (row.hidden) return POLL_HIDDEN_MS;
  if (row.dirty || row.saving) return POLL_ACTIVE_MS;
  return POLL_IDLE_MS;
}

// Turn on sync is the entry before setup. A paused cloud is only for a phone
// that already turned sync off, or was removed.
export function showTurnOnSync(status) {
  const row = status || {};
  return !row.account && !row.removed && !row.paused;
}

// A missing page stays. A tombstone removes that one page.
// Two or more, and more than half of the pages here, removes none.
export function massDelete(localPages, doomed) {
  return doomed >= 2 && doomed * 2 > localPages;
}

export function noteTombstone(tombstones, id) {
  const list = Array.isArray(tombstones) ? tombstones.filter((row) => row && row.id) : [];
  const rev = list.reduce((max, row) => Math.max(max, Number(row.rev) || 0), 0) + 1;
  const next = list.filter((row) => row.id !== id);
  if (id) next.push({ id: String(id), rev });
  return next;
}

export function keepUnlessTombstone(local, remote, tombstones) {
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
  for (const tomb of tombstones || []) {
    const id = String(tomb?.id || "");
    if (!id || named.includes(id)) continue;
    if (localRows.some((row) => row[0] === id)) named.push(id);
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
  return { pages, removed, held, keptLocal };
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

// `baseline` is null until a copy has synced. Any page then counts as a local change.
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
