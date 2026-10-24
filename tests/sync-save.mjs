// An edit typed while a save is in flight must stay unsaved.
import { beginSave, finishSave, noteLocalEdit } from "../site/app/sync-save.mjs";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

const inflight = { dirty: true, editGen: 2 };
const started = beginSave(inflight);
noteLocalEdit(inflight);
finishSave(inflight, started);
assert(inflight.dirty === true, "an edit during a save was marked saved");
assert(inflight.editGen === 3, "the edit was not counted");

const quiet = { dirty: true, editGen: 4 };
const at = beginSave(quiet);
assert(finishSave(quiet, at) === false, "a quiet save stayed dirty");
assert(quiet.dirty === false, "a finished save stayed marked unsaved");

const earlier = { dirty: true, editGen: 1 };
noteLocalEdit(earlier);
const again = beginSave(earlier);
finishSave(earlier, again);
assert(earlier.dirty === false, "the edit that started the save was left unsaved");

console.log("sync save ok");
