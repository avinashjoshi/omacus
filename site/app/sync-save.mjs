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

// Turn on sync is the entry before setup. A paused cloud is only for a phone
// that already turned sync off, or was removed.
export function showTurnOnSync(status) {
  const row = status || {};
  return !row.account && !row.removed && !row.paused;
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
