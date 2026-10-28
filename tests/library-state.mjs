// The web sidebar follows the same shared tree the desktop writes.
import { applyPresent, structuralState } from "../site/app/library-state.mjs";

function assert(cond, message) {
  if (!cond) throw new Error(message);
}

const flat = {
  tree: [
    { type: "page", id: "oats" },
    { type: "page", id: "ferry" },
    { type: "page", id: "cake" },
    { type: "page", id: "bread" },
  ],
  handles: new Map([["ferry", "dock"]]),
  trash: [],
  settings: { theme: "light", hideAnswers: false, lockMinutes: 5 },
};
const noted = JSON.parse(structuralState(flat));
assert(noted.sidebar[3].id === "bread", "the flat list did not keep bread last");
assert(noted.prefs.theme === "light", "a web theme was dropped");
assert(!Object.hasOwn(noted.prefs, "exportDir"), "a path was shared");

const present = {
  sidebar: [{
    type: "folder",
    id: "pantry",
    name: "Pantry",
    collapsed: false,
    lock: { salt: "s", nonce: "n", wrapped: "w", memory_kib: 19456, iterations: 2, lanes: 1 },
    children: [
      { type: "page", id: "oats" },
      { type: "page", id: "ferry" },
      { type: "page", id: "bread" },
      { type: "page", id: "cake" },
    ],
  }],
  handles: [{ id: "ferry", handle: "pier" }],
  trash: [{ id: "kept", text: "Kept\nold\n", handle: "kept" }],
  prefs: { theme: "dusk", hideAnswers: true, lockMinutes: 15 },
};
const applied = applyPresent({
  tree: flat.tree,
  handles: flat.handles,
  trash: [],
}, present, [["harbor", "notes"]]);
assert(applied.tree.length === 2, "a page outside the folder was dropped");
assert(applied.tree[0].name === "Pantry", "the folder was flattened");
assert(applied.tree[0].lock.wrapped === "w", "the folder password was dropped");
const ids = applied.tree[0].children.map((node) => node.id);
assert(ids.join() === "oats,ferry,bread,cake", `order ${ids.join()}`);
assert(applied.tree[1].id === "harbor", "an extra page was not kept in order");
assert(applied.handles.get("ferry") === "pier", "the handle stayed dock");
const cleared = applyPresent(applied, {
  sidebar: present.sidebar,
  handles: [{ id: "ferry", handle: "" }],
  trash: present.trash,
  prefs: present.prefs,
}, []);
assert(!cleared.handles.has("ferry"), "a cleared handle stayed");
assert(applied.trash[0].id === "kept", "Recently deleted was empty");
const keptLocal = applyPresent({
  tree: [{ type: "folder", id: "bag", name: "Bag", collapsed: false, children: [{ type: "page", id: "loaf" }] }],
  handles: new Map(),
  trash: [{ id: "old", text: "Old\nloaf\n", handle: "old" }],
}, {
  sidebar: [{ type: "page", id: "ferry" }],
  handles: [],
  trash: [],
  prefs: {},
}, [["loaf", "slice"], ["ferry", "pier"]]);
assert(keptLocal.tree.some((node) => node.type === "folder" && node.id === "bag"), "a local folder was dropped");
assert(keptLocal.tree.some((node) => node.type === "page" && node.id === "ferry"), "the other page was dropped");
assert(keptLocal.trash.some((item) => item.id === "old"), "a local Recently deleted page was dropped");
assert(applied.prefs.theme === "dusk", "the theme stayed light");

console.log("library state ok");
