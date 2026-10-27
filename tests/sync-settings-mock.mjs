// The Sync tab uses the Settings cards. Demo words only.
import { readFileSync } from "node:fs";

const js = readFileSync(new URL("../site/app/sync-settings-mock.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../site/app/app.css", import.meta.url), "utf8");

const need = [
  "intro", "fresh", "words", "check", "join-words", "join-wait",
  "approve", "approve-timeout", "devices", "remove", "paused",
  "list-changed", "keyring", "off",
];

let failed = 0;
function check(ok, message) {
  if (!ok) {
    failed += 1;
    console.error(message);
  }
}

for (const name of need) check(js.includes(`"${name}"`), `missing ${name}`);
check(js.includes('class: "settings-group"'), "sync steps use a settings card");
check(js.includes("Join again") && js.includes("Check again") && js.includes('text: "Sync"'), "a paused sync keeps Sync, Join again, and Check again");
check(js.includes("List changed") && js.includes("Got it") && js.includes("does not stop pages from saving"), "a list change is a note, and sync keeps going");
check(js.includes("That took too long. Your pages are still here."), "a slow approval says the pages are still here");
check(js.includes("Set up your keyring"), "Linux can set up the key store from Sync");
check(js.includes("Yes, approve it") && js.includes(">No<") === false && js.includes('btn("No"'), "approval offers Yes and No");
check(js.includes("sync-mock-notice"), "a list change is a button you can dismiss");
check(js.includes("demoLibrary") && js.includes("Pantry") && js.includes("Kept"), "the sidebar behind the tab is the demo set");
check(!js.includes("jaimarchy") && !js.includes("Trip Budget") && !js.includes("Groceries"), "the mock uses the demo set");
check(!/#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})\b/.test(js), "the mock adds no hex color");
check(css.includes(".sync-mock-words") && css.includes("var(--app-warning)") && !/#(?:[0-9a-fA-F]{3}|[0-9a-fA-F]{6})\b/.test(css.slice(css.indexOf(".sync-mock-words"), css.indexOf(".sync-mock-alert") + 80)), "mock styles use tokens");

if (failed) {
  console.error(`${failed} failed`);
  process.exit(1);
}
console.log("sync settings mock ok");
