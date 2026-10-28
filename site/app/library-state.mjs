// Sidebar, trash, handles, and preferences inside the sealed library.
// The same shape is what the desktop writes. A missing key stays as it was.

export const WEB_THEMES = ["auto", "light", "dark", "paper", "dusk"];

export function structuralState({ tree, handles, trash, settings }) {
  const sidebar = [];
  const walk = (nodes, parent) => {
    (nodes || []).forEach((node, index) => {
      if (!node || !node.id) return;
      if (node.type === "folder") {
        const row = {
          id: node.id,
          kind: "folder",
          name: node.name || "",
          parent: parent || "",
          index,
          collapsed: !!node.collapsed,
        };
        if (node.lock && typeof node.lock === "object") row.lock = node.lock;
        sidebar.push(row);
        walk(node.children, node.id);
      } else if (node.type === "page") {
        sidebar.push({ id: node.id, kind: "page", name: "", parent: parent || "", index });
      }
    });
  };
  walk(tree, "");
  const handleRows = [];
  const source = handles instanceof Map ? [...handles.entries()] : Object.entries(handles || {});
  for (const [id, handle] of source) {
    if (id && handle) handleRows.push({ id, handle });
  }
  const trashRows = (trash || [])
    .map((item) => ({ id: item.id, text: item.text || "", handle: item.handle || "" }))
    .filter((item) => item.id);
  const prefs = {};
  const theme = settings && settings.theme;
  if (WEB_THEMES.includes(theme)) prefs.theme = theme;
  if (settings && typeof settings.hideAnswers === "boolean") prefs.hideAnswers = settings.hideAnswers;
  if (settings && Number.isFinite(settings.lockMinutes)) prefs.lockMinutes = settings.lockMinutes;
  return JSON.stringify({ sidebar, handles: handleRows, trash: trashRows, prefs });
}

// `pages` are `[id, text]` rows already on this device. A page the shared tree
// does not name, and that is not in Recently deleted, stays at the root in id order.
export function applyPresent(current, present, pages) {
  const shown = typeof present === "string" ? JSON.parse(present) : present;
  if (!shown || typeof shown !== "object") return current;
  const next = current;
  const localTree = Array.isArray(current.tree) ? current.tree : [];
  const localTrash = Array.isArray(current.trash) ? current.trash : [];
  if (Array.isArray(shown.sidebar)) next.tree = shown.sidebar.slice();
  const folderIds = new Set();
  const collectFolders = (nodes) => {
    for (const node of nodes || []) {
      if (!node || node.type !== "folder" || !node.id) continue;
      folderIds.add(node.id);
      collectFolders(node.children);
    }
  };
  collectFolders(next.tree);
  for (const node of localTree) {
    if (node && node.type === "folder" && node.id && !folderIds.has(node.id)) {
      next.tree.push(node);
      folderIds.add(node.id);
    }
  }
  const known = new Set();
  const walk = (nodes) => {
    for (const node of nodes || []) {
      if (!node) continue;
      if (node.type === "page" && node.id) known.add(node.id);
      if (node.type === "folder") walk(node.children);
    }
  };
  walk(next.tree);
  const trashed = new Set(
    [...(shown.trash || []), ...localTrash].map((item) => item && item.id).filter(Boolean),
  );
  const extras = [];
  for (const row of pages || []) {
    const id = Array.isArray(row) ? row[0] : row;
    if (!id || known.has(id) || trashed.has(id)) continue;
    extras.push(String(id));
  }
  extras.sort();
  for (const id of extras) next.tree.push({ type: "page", id });
  if (!(next.handles instanceof Map)) next.handles = new Map(Object.entries(next.handles || {}));
  if (Array.isArray(shown.handles)) {
    for (const row of shown.handles) {
      if (!row || !row.id) continue;
      if (row.handle) next.handles.set(row.id, row.handle);
      else next.handles.delete(row.id);
    }
  }
  const trash = new Map();
  for (const item of localTrash) {
    if (item && item.id) trash.set(item.id, { id: item.id, text: item.text || "", handle: item.handle || "" });
  }
  for (const item of shown.trash || []) {
    if (item && item.id) trash.set(item.id, { id: item.id, text: item.text || "", handle: item.handle || "" });
  }
  for (const id of known) trash.delete(id);
  next.trash = [...trash.values()];
  next.prefs = shown.prefs && typeof shown.prefs === "object" ? shown.prefs : {};
  return next;
}
