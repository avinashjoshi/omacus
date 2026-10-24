// A save remembers how many local edits it started with.
// An edit that arrives while the save is in flight stays dirty, so the next save sends it.

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
