// First launch, when there are no pages yet: a welcome page at the root and an
// Examples folder. `seeded` is remembered so deleting those pages does not
// bring them back, and an existing library is never added to.

import { PAGES } from "./examples.js";

export { PAGES };

// Keep in step with xref::allocate_handle in engine/src/xref.rs.
// The first word that starts with a letter, then the words joined, then trip2.
export function allocateHandle(title, taken) {
  const have = new Set([...taken].map((h) => String(h).toLowerCase()));
  const words = String(title ?? "").split(/\s+/).map((word) => word.replace(/[^A-Za-z0-9]/g, "").toLowerCase()).filter(Boolean);
  const start = words.findIndex((word) => /^[a-z]/.test(word));
  let base = "page";
  if (start >= 0) {
    const word = words[start].replace(/^[^a-z]+/, "");
    if (word) base = word;
  }
  if (!/^[a-z][a-z0-9]*$/.test(base)) base = "page";
  if (!have.has(base)) return base;
  if (start >= 0) {
    const longer = words.slice(start).join("").replace(/^[^a-z]+/, "");
    if (/^[a-z][a-z0-9]*$/.test(longer) && longer !== base && !have.has(longer)) return longer;
  }
  let n = 2;
  while (have.has(base + n)) n += 1;
  return base + n;
}

/// What a launch should do.
/// `pages` is how many pages are already saved. `trash` is Recently deleted.
/// `seeded` is the flag from the last save.
/// "examples" builds the welcome page and the Examples folder.
/// "welcome" opens a blank library with only the welcome page (the flag is already set).
/// "welcome-mark" is the same, and records the flag (there is old trash, so these are not new).
/// "mark" records the flag on a library that already has pages. Nothing is added.
/// "none" leaves the library alone.
export function seedAction({ pages, seeded, trash }) {
  if (pages > 0) return seeded ? "none" : "mark";
  if (seeded) return "welcome";
  if (trash > 0) return "welcome-mark";
  return "examples";
}

/// The library a first launch writes. Welcome stays at the root, first in the
/// sidebar, and is the open page. Example pages sit in Examples, under it,
/// in the order they're read.
export function freshLibrary(welcome) {
  const texts = {};
  const handles = {};
  const taken = new Set();
  const children = PAGES.map((page, i) => {
    const id = `ex${i + 1}`;
    texts[id] = page.text;
    const handle = allocateHandle(page.title, taken);
    handles[id] = handle;
    taken.add(handle);
    return { type: "page", id };
  });
  const welcomeId = "ex-welcome";
  texts[welcomeId] = welcome;
  handles[welcomeId] = allocateHandle(welcome.split("\n")[0].replace(/^#\s*/, ""), taken);
  const tree = [
    { type: "page", id: welcomeId },
    { type: "folder", id: "ex-folder", name: "Examples", collapsed: false, children },
  ];
  return { tree, texts, handles, active: welcomeId, seeded: true };
}
