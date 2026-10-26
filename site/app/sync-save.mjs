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
