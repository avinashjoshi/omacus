// Folder tree: existing pages stay at the root, nesting stops at two levels,
// and a move survives a save/load of the same JSON the web app stores.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  appendMissing,
  canNest,
  collectPageIds,
  createFolder,
  expandTo,
  folderOf,
  folders,
  fromPages,
  insertNode,
  pageIds,
  place,
  removeNode,
  renameFolder,
  sanitize,
  toggleFolder,
  visible,
} from "../site/app/tree.js";

const failures = [];
function check(cond, message) {
  if (!cond) failures.push(message);
}

const order = ["weekend", "trip", "groceries"];
const migrated = fromPages(order);
check(pageIds(migrated).join() === order.join(), "pages without a tree stay at the root in that order");
check(folders(migrated).length === 0, "a migrated list has no folders");
check(folderOf(migrated, "trip") == null, "a migrated page is not inside a folder");

const round = fromPages(["groceries"]);
const folder = createFolder(round, null, "folder-trip");
check(folder === "folder-trip", "a folder can be created at the root");
check(renameFolder(round, folder, "Trip Budget"), "a folder can be renamed");
insertNode(round, null, 0, { type: "page", id: "weekend" });
check(place(round, "weekend", { kind: "into", id: folder }), "a page can move into a folder");
check(toggleFolder(round, folder), "a folder can collapse");
const saved = JSON.parse(JSON.stringify(round));
const loaded = sanitize(saved, new Set(["groceries", "weekend"]));
check(
  visible(loaded).some((row) => row.type === "folder" && row.name === "Trip Budget" && row.collapsed),
  "a collapsed folder comes back collapsed",
);
check(
  visible(loaded).every((row) => row.id !== "weekend"),
  "a page inside a collapsed folder is not a visible row",
);
check(pageIds(loaded).includes("weekend"), "that page is still stored");
check(folderOf(loaded, "weekend") === folder, "the page is still in the folder");
check(expandTo(loaded, "weekend"), "opening the page expands the folder");
check(visible(loaded).some((row) => row.id === "weekend"), "the page is visible once expanded");

const trash = fromPages(["groceries", "weekend"]);
const bag = createFolder(trash, null, "bag");
renameFolder(trash, bag, "Groceries");
place(trash, "groceries", { kind: "into", id: bag });
place(trash, "weekend", { kind: "into", id: bag });
const removed = removeNode(trash, bag);
check(removed?.node?.type === "folder", "deleting a folder takes the folder node");
check(collectPageIds(removed.node).join() === "weekend,groceries", "its pages are what move to Recently deleted");
check(pageIds(trash).join() === "", "those pages leave the sidebar");

const deep = fromPages(["trip"]);
const outer = createFolder(deep, null, "outer");
const inner = createFolder(deep, outer, "inner");
check(createFolder(deep, inner, "too-deep") == null, "a folder cannot be nested three deep");
check(!canNest(deep, inner), "the nested folder cannot hold another folder");
const other = createFolder(deep, null, "other");
check(!place(deep, outer, { kind: "into", id: other }), "a folder that holds a folder cannot move inside another");
check(place(deep, inner, { kind: "into", id: other }), "a folder with only pages can move inside another");
check(place(deep, "trip", { kind: "into", id: inner }), "a page can sit in a nested folder");
check(folderOf(deep, "trip") === inner, "the page's folder is the nested one");
check(folders(deep).length === 3, "three folders remain");

const moved = fromPages(["groceries"]);
insertNode(moved, null, 0, { type: "page", id: "weekend" });
const plans = createFolder(moved, null, "plans");
renameFolder(moved, plans, "Weekend Plans");
check(place(moved, "groceries", { kind: "into", id: plans }), "groceries move into the folder");
check(place(moved, "weekend", { kind: "before", id: "groceries" }), "another page can be placed just before it");
check(folderOf(moved, "weekend") === plans, "that page is in the folder too");
check(place(moved, "groceries", { kind: "root" }), "a page can move back out to the root");
check(folderOf(moved, "groceries") == null, "it is no longer in a folder");
check(folderOf(JSON.parse(JSON.stringify(moved)), "weekend") === plans, "the folder membership survives a save");

const fresh = fromPages(["groceries"]);
const home = createFolder(fresh, null, "home");
insertNode(fresh, home, 0, { type: "page", id: "trip" });
insertNode(fresh, home, 0, { type: "page", id: "weekend" });
const homeNode = fresh.find((node) => node.id === home);
check(pageIds(homeNode.children).join() === "weekend,trip", "a new page lands above the open page, inside the folder");

const hoisted = sanitize(
  [
    {
      type: "folder",
      id: "a",
      name: "Trip Budget",
      children: [
        {
          type: "folder",
          id: "b",
          name: "Groceries",
          children: [
            {
              type: "folder",
              id: "c",
              name: "Weekend Plans",
              children: [{ type: "page", id: "p1" }],
            },
          ],
        },
      ],
    },
    { type: "page", id: "missing" },
    { type: "page", id: "p1" },
  ],
  new Set(["p1"]),
);
check(pageIds(hoisted).join() === "p1", "an unknown page is dropped and a duplicate is not kept twice");
check(folders(hoisted).map((f) => f.name).join() === "Trip Budget,Groceries", "a third-level folder is removed");
check(folderOf(hoisted, "p1") === "b", "its page stays in the deepest folder still allowed");

const rooted = sanitize(null, new Set(order));
appendMissing(rooted, order);
check(pageIds(rooted).join() === order.join(), "a missing tree is the saved page order");
appendMissing(rooted, ["weekend", "extra"]);
check(pageIds(rooted).at(-1) === "extra", "a page the tree doesn't mention is appended");

const here = dirname(fileURLToPath(import.meta.url));
const appJs = readFileSync(join(here, "../site/app/app.js"), "utf8");
const appHtml = readFileSync(join(here, "../site/app/index.html"), "utf8");
const appCss = readFileSync(join(here, "../site/app/app.css"), "utf8");
check(appHtml.includes('data-action="new-folder"'), "the sidebar has a New folder button");
check(appHtml.includes("data-switcher"), "the page switcher is in the page");
check(appHtml.includes("data-move"), "Move to folder has a dialog");
check(appJs.includes("tree: state.tree"), "the folder tree is saved with the pages");
check(appJs.includes("openFolderRename"), "renaming a folder does not replace Rename variable");
check(appJs.includes("function openRename(view, utf16)"), "Rename variable still has its own field");
check(appJs.includes("Move to folder"), "a page can be moved from its menu");
check(appJs.includes('e.key.toLowerCase() === "j"'), "Ctrl+J opens the switcher");
check(appCss.includes("text-overflow: ellipsis"), "long names truncate");
check(appCss.includes(".side-row.drop-into"), "a drop onto a folder is marked");
check(appCss.includes("touch-action: pan-y"), "the list can still scroll on a phone");

if (failures.length) {
  console.error(failures.map((f) => "  - " + f).join("\n"));
  process.exit(1);
}
console.log("folder checks ok");
