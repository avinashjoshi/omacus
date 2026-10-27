// An edit typed while a save is in flight must stay unsaved.
// Two phones that each edit offline both keep their lines: the one that was
// offline merges when its pages differ from the last synced copy.
import { readFileSync } from "node:fs";
import { approvalCard, beginSave, chipLabel, dedupeWelcome, declinedCovers, finishSave, keepUnlessTombstone, liveDeliveryMs, noteLocalEdit, noteTombstone, pagesDiffer, pagesLeftBehind, pagesOnlyHere, pollWait, removalSentence, showTurnOnSync } from "../site/app/sync-save.mjs";

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

const desktop = [["pantry", "oats\n"], ["kept", "ferry = 40\n"]];
const phone = [["kept", "ferry = 40\n"]];
const fromDesktop = keepUnlessTombstone(phone, desktop, []);
assert(fromDesktop.pages.some((row) => row[0] === "pantry"), "a page only on the desktop was dropped by the phone");
assert(fromDesktop.removed.length === 0, "a missing page was deleted on the phone");
const fromPhone = keepUnlessTombstone(desktop, phone, []);
assert(fromPhone.pages.some((row) => row[0] === "pantry") && fromPhone.pages.some((row) => row[0] === "kept"), "a page only on the phone was dropped by the desktop");
const oneDelete = keepUnlessTombstone(desktop, phone, [{ id: "pantry", rev: 1 }]);
assert(oneDelete.removed.length === 1 && oneDelete.removed[0] === "pantry", "a tombstone left the page in place");
assert(!oneDelete.pages.some((row) => row[0] === "pantry"), "a tombstoned page stayed in the list");
assert(oneDelete.pages.some((row) => row[0] === "kept"), "a tombstone removed the page that stayed");
const two = keepUnlessTombstone(
  [["pantry", "oats\n"], ["kept", "ferry = 40\n"], ["harbor", "cake\n"]],
  [["harbor", "cake\n"]],
  [{ id: "pantry", rev: 1 }, { id: "kept", rev: 2 }],
);
assert(two.held === false, "two signed deletes waited");
assert(two.removed.length === 2 && two.removed.includes("pantry") && two.removed.includes("kept"), "two signed deletes left the pages");
assert(!two.pages.some((row) => row[0] === "pantry" || row[0] === "kept"), "two signed deletes stayed in the list");
assert(two.pages.some((row) => row[0] === "harbor"), "two signed deletes removed the page that stayed");
const mass = keepUnlessTombstone(
  [["pantry", "oats\n"], ["kept", "ferry = 40\n"], ["harbor", "cake\n"]],
  [],
  [{ id: "pantry", rev: 1 }, { id: "kept", rev: 2 }, { id: "harbor", rev: 3 }],
);
assert(mass.held === true, "more than two deletes were applied");
assert(mass.removed.length === 0, "more than two deletes removed a page");
assert(mass.asked.length === 3, "more than two deletes did not ask");
assert(mass.pages.some((row) => row[0] === "pantry") && mass.pages.some((row) => row[0] === "kept") && mass.pages.some((row) => row[0] === "harbor"), "more than two deletes dropped the pages");
const keptAfterNo = keepUnlessTombstone(
  [["pantry", "oats\n"], ["kept", "ferry = 40\n"], ["harbor", "cake\n"]],
  [],
  [{ id: "pantry", rev: 1 }, { id: "kept", rev: 2 }, { id: "harbor", rev: 3 }],
  [{ id: "pantry", rev: 1 }, { id: "kept", rev: 2 }, { id: "harbor", rev: 3 }],
);
assert(keptAfterNo.held === false && keptAfterNo.removed.length === 0, "a declined delete was applied again");
assert(keptAfterNo.pages.length === 3, "a declined delete dropped a page");
assert(declinedCovers([{ id: "pantry", rev: 2 }], { id: "pantry", rev: 2 }) === true, "a declined delete was asked again");
assert(declinedCovers([{ id: "pantry", rev: 1 }], { id: "pantry", rev: 2 }) === false, "a newer delete was treated as already declined");
const welcome = "# Welcome to Omacus\nA note.\n";
const folded = dedupeWelcome([
  ["welcome-b", welcome],
  ["welcome-a", welcome],
  ["welcome-c", "# Welcome to Omacus\nferry\n"],
  ["pantry", "Pantry\noats\n"],
  ["kept", "Pantry\noats\n"],
], []);
assert(folded.pages.some((row) => row[0] === "welcome-a") && !folded.pages.some((row) => row[0] === "welcome-b"), "two identical welcome pages both stayed");
assert(folded.pages.some((row) => row[0] === "welcome-c"), "a different welcome page was folded away");
assert(folded.pages.some((row) => row[0] === "pantry") && folded.pages.some((row) => row[0] === "kept"), "two pages that only share a title were folded");
assert(folded.removed.length === 1 && folded.removed[0] === "welcome-b", "the welcome duplicate was not the later id");
assert(folded.tombstones.some((row) => row.id === "welcome-b"), "the welcome duplicate was not a signed delete");
assert(removalSentence(3).includes("3 pages") && removalSentence(3).includes("keep them"), "the prompt did not name the count");
const tombs = noteTombstone([], "pantry");
assert(tombs.length === 1 && tombs[0].id === "pantry" && tombs[0].rev === 1, "a delete was not recorded");
assert(noteTombstone(tombs, "pantry")[0].rev === 2, "deleting again did not advance the record");
const remoteKept = [["kept", "ferry = 40\n"]];
const localBoth = [["kept", "ferry = 40\n"], ["pantry", "oats\n"]];
const onlyHere = pagesOnlyHere(localBoth, remoteKept);
assert(onlyHere.length === 1 && onlyHere[0][0] === "pantry" && onlyHere[0][1] === "oats\n", "a page only on this phone was dropped");
assert(pagesOnlyHere(localBoth, localBoth).length === 0, "a shared page was treated as only here");
assert(pagesOnlyHere([], remoteKept).length === 0, "an empty phone invented a page");
const left = pagesLeftBehind(
  [["pantry", "oats\n"], ["kept", "1\n"]],
  [["kept", "1\n"]],
);
assert(left.length === 1 && left[0][0] === "pantry" && left[0][1] === "oats\n", "a synced page missing remotely was not kept for Recently deleted");
assert(pagesLeftBehind([["scratch", "local\n"]], [["scratch", "local\n"]]).length === 0, "a page still in the list was parked");
assert(pagesLeftBehind(null, []).length === 0, "a phone with no synced copy parked a page");
assert(showTurnOnSync({}) === true, "a fresh phone hid Turn on sync");
assert(showTurnOnSync({ account: "", removed: false, paused: false }) === true, "an empty account hid Turn on sync");
assert(showTurnOnSync({ account: "harbor", paused: false, removed: false }) === false, "a linked phone still offered Turn on sync");
assert(showTurnOnSync({ paused: true }) === false, "a paused phone was treated as not set up");
assert(showTurnOnSync({ removed: true }) === false, "a removed phone was treated as not set up");
assert(pollWait({ soon: true, hidden: true, dirty: true }) === 400, "an edit in a hidden tab waited a minute");
assert(pollWait({ dirty: true }) === 400, "an unsaved edit waited longer than the debounce");
assert(pollWait({ saving: true }) === 400, "a save in flight waited longer than the debounce");
assert(pollWait({ soon: true }) === 800, "a focus pull waited for the idle delay");
assert(pollWait({}) === 2000, "a quiet tab with no socket waited half a minute");
assert(pollWait({ live: true }) === 30000, "a live socket still polled every couple of seconds");
assert(pollWait({ hidden: true }) === 60000, "a hidden tab polled as if it were visible");
assert(liveDeliveryMs() === 400, "a desktop edit was not scheduled inside half a second");
assert(approvalCard([{ id: "pantry", state: "pending", created: 1_000 }], "pantry", 2_000, 1_000, 5 * 60 * 1000) === "open", "a fresh approval closed");
assert(approvalCard([], "pantry", 2_000, 1_000, 5 * 60 * 1000) === "expired", "a missing approval stayed open");
assert(approvalCard([{ id: "pantry", state: "rejected", created: 1_000 }], "pantry", 2_000, 1_000, 5 * 60 * 1000) === "expired", "a refusal stayed on the card");
assert(approvalCard([{ id: "pantry", state: "pending", created: 1_000 }], "pantry", 1_000 + 5 * 60 * 1000 + 1, 1_000, 5 * 60 * 1000) === "expired", "an old code stayed open");
assert(chipLabel({ live: true, member: true }) === "Live", "a connected device said Synced");
assert(chipLabel({ member: true }) === "Offline", "a device with no socket said Synced");
assert(chipLabel({ syncing: true, live: true, member: true }) === "Syncing", "a save hid behind Live");
assert(!chipLabel({}).includes("Synced"), "an empty status said Synced");

const appSource = readFileSync(new URL("../site/app/app.js", import.meta.url), "utf8");
const saveSettings = appSource.slice(appSource.indexOf("function saveSettings"), appSource.indexOf("function themeName"));
assert(saveSettings.includes("noteSyncEdit()"), "a settings change stayed on this device");

console.log("sync save ok");
