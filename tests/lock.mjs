// Folder locks survive a save, and the web app keeps plaintext out of storage.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  coveringLock,
  createFolder,
  folderOf,
  fromPages,
  insertNode,
  pagesUnder,
  sanitize,
  visible,
} from "../site/app/tree.js";

const failures = [];
function check(cond, message) {
  if (!cond) failures.push(message);
}

const nodes = fromPages(["groceries"]);
const trip = createFolder(nodes, null, "trip");
insertNode(nodes, trip, 0, { type: "page", id: "weekend" });
const tripNode = nodes.find((node) => node.id === trip);
tripNode.lock = {
  salt: "salt",
  nonce: "nonce",
  wrapped: "wrapped",
  memory_kib: 19456,
  iterations: 2,
  lanes: 1,
};
check(coveringLock(nodes, "weekend")?.id === trip, "a page inside a locked folder uses that folder's lock");
check(coveringLock(nodes, trip)?.id === trip, "a locked folder covers itself");
check(pagesUnder(nodes, trip).includes("weekend"), "pages under a folder include the page inside it");
check(visible(nodes).find((row) => row.id === trip)?.locked === true, "the sidebar row knows the folder is locked");

const saved = JSON.parse(JSON.stringify(nodes));
const loaded = sanitize(saved, new Set(["groceries", "weekend"]));
const again = loaded.find((node) => node.id === trip);
check(again?.lock?.wrapped === "wrapped", "a folder lock survives sanitize");
check(folderOf(loaded, "weekend") === trip, "the page stays in the locked folder");

const junk = sanitize(
  [{ type: "folder", id: "bag", name: "Bag", lock: "nope", children: [{ type: "page", id: "groceries" }] }],
  new Set(["groceries"]),
);
check(junk[0].lock == null, "a lock that is not a wrap is dropped");

const root = dirname(fileURLToPath(import.meta.url));
const app = readFileSync(join(root, "../site/app/app.js"), "utf8");
check(app.includes("lockCreate"), "the web app creates locks through the wasm engine");
check(app.includes("lockSeal"), "the web app seals page text through the wasm engine");
check(app.includes("lockOpenPassword"), "the web app opens a lock through the wasm engine");
check(app.includes("A forgotten password can't be recovered."), "the lock dialog warns that a password can't be recovered");
check(app.includes("lockMinutes"), "idle relock is a setting");
check(!app.includes('store.set("plains"'), "open page text is not written to browser storage");
check(!app.includes("localStorage.setItem(PREFIX + \"plains\""), "content keys are not written to browser storage");

if (failures.length) {
  for (const message of failures) console.error("FAIL " + message);
  process.exit(1);
}
console.log("lock checks passed");
