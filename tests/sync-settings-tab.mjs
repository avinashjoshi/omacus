import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const sync = readFileSync(new URL("../site/app/sync.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../site/app/app.css", import.meta.url), "utf8");
const help = readFileSync(new URL("../src/help.rs", import.meta.url), "utf8");
const app = readFileSync(new URL("../src/app.rs", import.meta.url), "utf8");

assert(sync.includes('tabs.insertBefore(tab, about)'), "Sync sits beside Settings, before About");
assert(sync.includes('class: "settings-group"'), "every sync step is a Settings card");
assert(sync.includes('get("syncCard")'), "a card preview can open the real sync screens");
{
  const preview = sync.slice(sync.indexOf("function showSyncCard"), sync.indexOf("function cloudIcon"));
  assert(!preview.includes("recovery words\n") && !/\b(meadow|lantern|cedar|maple)\b/.test(preview), "the card preview repeats recovery words");
}
assert(sync.includes("Sync is paused") && sync.includes("Join again") && sync.includes("Check again"), "a paused card keeps Join again and Check again");
assert(sync.includes("The Sync button in the sidebar stays"), "a pause keeps the Sync button");
assert(sync.includes('class: "sync-list-notice"') && sync.includes("data-sync-list"), "the list change is a button");
assert(sync.includes("This note does not stop pages from saving."), "the list change says saving continues");

const tick = sync.slice(sync.indexOf("async function tick"), sync.indexOf("function schedule"));
assert(!tick.includes("listNote"), "a list change stopped the save loop");

assert(!css.includes(".settings-card:has(.sync-panel:not([hidden]))"), "the Sync tab still becomes a bottom sheet");
assert(css.includes(".sync-sheet"), "the sheet class name stays for the layout check");
assert(css.includes(".sync-list-notice"), "the list change notice uses the warning token");

assert(help.includes("Tab::Sync") && help.includes("fn rail()"), "the desktop rail has a Sync tab");
assert(!app.includes("omacus-list-changed"), "list changed is still a blocking window");

console.log("sync settings tab ok");
