// Export everything: a zip of pages, folders, and settings. A locked page stays
// ciphertext, and bringing a backup back never drops a page that is already here.

import { backupEntries, mergeBackup, parseBackup, unzipNamed, zipBytes } from "../site/app/backup.js";

const failures = [];
function check(cond, message) {
  if (!cond) failures.push(message);
}

const secret = "ferry = 40";
const sealed = "OMACUS-LOCK/1\nsalt=abc\ndata=not-the-page\n";
const entries = backupEntries({
  version: "0.2.14",
  library: {
    order: ["trip", "groceries"],
    tree: [
      { type: "folder", id: "trips", name: "Trips", children: [{ type: "page", id: "trip" }] },
      { type: "page", id: "groceries" },
    ],
    handles: { trip: "trip", groceries: "groceries" },
  },
  settings: { theme: "paper" },
  trash: [],
  pages: [
    { id: "trip", text: sealed },
    { id: "groceries", text: "# Groceries\nmilk $3\n" },
    { id: "../nope", text: secret },
  ],
});
check(entries.some((file) => file.name === "pages/trip.txt"), "the locked page is in the backup");
check(!entries.some((file) => file.name.includes("..")), "a backup name cannot leave the folder");
const bytes = zipBytes(entries);
const asText = new TextDecoder().decode(bytes);
check(!asText.includes(secret), "a rejected page is not in the zip");
check(!asText.includes("correct horse"), "a password is not in the zip");

const parsed = parseBackup(await unzipNamed(bytes));
check(parsed, "the zip reads back as a backup");
check(parsed.pages.length === 2, "two pages came back");
const locked = parsed.pages.find((page) => page.id === "trip");
check(locked.text === sealed, "the locked page came back as ciphertext");
check(!locked.text.includes(secret), "the locked page does not contain the open text");
const open = parsed.pages.find((page) => page.id === "groceries");
check(open.text.includes("milk $3"), "an open page comes back whole");

const merged = mergeBackup(
  {
    pages: [{ id: "groceries", text: "# Groceries\nmilk $4\n" }],
    tree: [{ type: "page", id: "groceries" }],
    trash: [],
    settings: { theme: "dark" },
  },
  parsed,
);
check(merged.added.length === 1 && merged.added[0] === "trip", "only the missing page is added");
check(
  merged.pages.find((page) => page.id === "groceries").text.includes("milk $4"),
  "a page already here is left as it is",
);
check(merged.pages.find((page) => page.id === "trip").text === sealed, "the added page stays sealed");
check(merged.settings.theme === "dark", "settings already chosen are kept");

const fresh = mergeBackup({ pages: [], tree: [], trash: [], settings: {} }, parsed);
check(fresh.pages.length === 2, "an empty library keeps both pages");
check(JSON.stringify(fresh.tree).includes("Trips"), "an empty library keeps the folders");

const plain = parseBackup([{ name: "notes.txt", text: "hello" }]);
check(plain === null, "a zip of notes is not a backup");

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}
console.log("backup ok");
