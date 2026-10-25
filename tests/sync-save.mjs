// An edit typed while a save is in flight must stay unsaved.
// Two phones that each edit offline both keep their lines: the one that was
// offline merges when its pages differ from the last synced copy.
import { beginSave, finishSave, noteLocalEdit, pagesDiffer } from "../site/app/sync-save.mjs";

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

const synced = [["trip", "ferry = 40\n"]];
const phoneA = [["trip", "ferry = 40\nbread $2\n"]];
const phoneB = [["trip", "ferry = 40\ncake $5\n"]];
const fresh = [["trip", "ferry = 40\n"], ["market", "milk $3\n"]];
assert(pagesDiffer(synced, phoneB) === true, "an offline edit was treated as already synced");
assert(pagesDiffer(synced, synced) === false, "an unchanged phone would merge a deleted line back");
assert(pagesDiffer(synced, fresh) === true, "a new page edited offline was treated as already synced");
assert(pagesDiffer(null, synced) === true, "a phone with no synced copy ignored its pages");
assert(pagesDiffer([], []) === false, "two empty copies differed");
assert(pagesDiffer(phoneA, phoneB) === true, "bread and cake were treated as the same page");

console.log("sync save ok");
