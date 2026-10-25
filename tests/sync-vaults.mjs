// The phone keeps a list of vaults. The older single slot is copied in once.
// A second vault does not replace the first. The account id and the sealed
// marker stay where they are.
import { readFileSync } from "node:fs";
import {
  activeVault,
  bookJson,
  legacyMirror,
  loadVaultBook,
  upsertVault,
} from "../site/app/sync-vaults.mjs";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

const legacy = {
  "device-file": "file-harbor",
  "sync-account": "harbor",
  "sync-revision": "4",
  "sync-slots": "[]",
  "sync-base": JSON.stringify([["trip", "bread $2\n"]]),
  "sync-paused": "1",
  "sync-vault-name": "Harbor pages",
  "sync-device-name": "Chrome on Linux",
  "sync-self-id": "self-harbor",
};
const fromSlot = loadVaultBook((key) => legacy[key] || "");
assert(fromSlot.migrated === true, "the single slot was not copied in");
assert(fromSlot.vaults.length === 1, "the single slot became more than one vault");
assert(fromSlot.vaults[0].deviceFile === "file-harbor", "the device file was dropped");
assert(fromSlot.vaults[0].revision === "4", "the revision was dropped");
assert(fromSlot.vaults[0].base.includes("bread"), "the last synced pages were dropped");
assert(fromSlot.vaults[0].paused === true, "the paused flag was dropped");
assert(fromSlot.vaults[0].name === "Harbor pages", "the name was dropped");
assert(fromSlot.active === "harbor", "the copied vault was not the open one");

const empty = loadVaultBook(() => "");
assert(empty.vaults.length === 0 && empty.migrated === false, "a fresh phone invented a vault");

let book = upsertVault({ active: "", vaults: [] }, {
  account: "harbor",
  deviceFile: "file-harbor",
  revision: "4",
  base: JSON.stringify([["trip", "bread $2\n"]]),
  paused: false,
  name: "Harbor pages",
});
book = upsertVault(book, {
  account: "lantern",
  deviceFile: "file-lantern",
  revision: "1",
  base: "[]",
  paused: false,
  name: "Lantern pages",
});
assert(book.vaults.length === 2, "the second vault replaced the first");
assert(book.vaults.find((row) => row.account === "harbor").deviceFile === "file-harbor", "harbor lost its device file");
assert(activeVault(book).account === "lantern", "the new vault did not become the open one");
book = { ...book, active: "harbor" };
assert(activeVault(book).revision === "4", "switching back lost harbor's revision");
assert(activeVault(book).name === "Harbor pages", "switching back lost harbor's name");

const saved = bookJson(book);
const again = loadVaultBook((key) => (key === "sync-vaults" ? saved : "stale-file"));
assert(again.migrated === false, "a saved list was copied from the old slot again");
assert(again.vaults.length === 2, "the saved list dropped a vault");
assert(activeVault(again).deviceFile === "file-harbor", "the saved list preferred the old slot");

const mirror = legacyMirror(activeVault(again));
assert(mirror.set["device-file"] === "file-harbor", "the open vault was not mirrored");
assert(mirror.set["sync-account"] === "harbor", "the account was not mirrored");
assert(mirror.set["sync-base"].includes("bread"), "the last synced pages were not mirrored");
assert(!mirror.set["sync-paused"], "a running vault was marked paused");
const paused = legacyMirror({ ...activeVault(again), deviceFile: "", paused: true });
assert(paused.set["sync-paused"] === "1", "turning sync off left the flag clear");
assert(paused.remove.includes("device-file"), "turning sync off kept the device file in the old slot");

const keys = readFileSync(new URL("../engine/src/sync_keys.rs", import.meta.url), "utf8");
assert(keys.includes('b"omacus/v1/account-sign"'), "the account id derivation changed");
assert(keys.includes('pub const VAULT_ID: &str = "personal"'), "the sealed marker changed");
const module = readFileSync(new URL("../site/app/sync-vaults.mjs", import.meta.url), "utf8");
assert(!module.includes("account-sign"), "the vault list started deriving account ids");

console.log("sync vaults ok");
