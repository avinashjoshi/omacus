// Sidebar folders for the web app. The shape matches the desktop `state.json`
// tree: `{type:"page",id}` or `{type:"folder",id,name,collapsed,children}`.
// A folder may contain one more folder. Anything deeper is refused.

export const MAX_FOLDER_DEPTH = 2;

export function pageIds(nodes) {
  const ids = [];
  const walk = (list) => {
    if (!Array.isArray(list)) return;
    for (const node of list) {
      if (!node || typeof node !== "object") continue;
      if (node.type === "page" && typeof node.id === "string") ids.push(node.id);
      else if (node.type === "folder") walk(node.children);
    }
  };
  walk(nodes);
  return ids;
}

export function fromPages(ids) {
  return (Array.isArray(ids) ? ids : [])
    .filter((id) => typeof id === "string")
    .map((id) => ({ type: "page", id }));
}

/// Pages in `ids` that the tree doesn't mention are added at the root, in order.
export function appendMissing(nodes, ids) {
  const have = new Set(pageIds(nodes));
  for (const id of Array.isArray(ids) ? ids : []) {
    if (typeof id === "string" && !have.has(id)) {
      nodes.push({ type: "page", id });
      have.add(id);
    }
  }
  return nodes;
}

/// Drop unknown or repeated pages. A folder past the depth limit is removed
/// and its pages stay at the deepest folder still allowed.
export function sanitize(nodes, known) {
  const have = known instanceof Set ? known : new Set(known ?? []);
  const seen = new Set();
  const walk = (list, depth) => {
    const out = [];
    if (!Array.isArray(list)) return out;
    for (const node of list) {
      if (!node || typeof node !== "object") continue;
      if (node.type === "page") {
        const id = node.id;
        if (typeof id === "string" && have.has(id) && !seen.has(id)) {
          seen.add(id);
          out.push({ type: "page", id });
        }
      } else if (node.type === "folder") {
        const id = typeof node.id === "string" ? node.id : "";
        if (!id || depth >= MAX_FOLDER_DEPTH || seen.has(id)) {
          out.push(...walk(node.children, depth));
          continue;
        }
        seen.add(id);
        const folder = {
          type: "folder",
          id,
          name: typeof node.name === "string" ? node.name : "Folder",
          collapsed: !!node.collapsed,
          children: walk(node.children, depth + 1),
        };
        const lock = folderLock(node.lock);
        if (lock) folder.lock = lock;
        out.push(folder);
      }
    }
    return out;
  };
  return walk(nodes, 0);
}

/// Rows the sidebar draws. Children of a collapsed folder are left out.
export function visible(nodes) {
  const out = [];
  const walk = (list, depth) => {
    for (const node of list ?? []) {
      if (node?.type === "page") out.push({ depth, type: "page", id: node.id });
      else if (node?.type === "folder") {
        out.push({
          depth,
          type: "folder",
          id: node.id,
          name: node.name,
          collapsed: !!node.collapsed,
          locked: !!node.lock,
        });
        if (!node.collapsed) walk(node.children, depth + 1);
      }
    }
  };
  walk(nodes, 0);
  return out;
}

export function folders(nodes) {
  const out = [];
  const walk = (list, depth) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      out.push({ id: node.id, name: node.name, depth: depth + 1 });
      walk(node.children, depth + 1);
    }
  };
  walk(nodes, 0);
  return out;
}

export function locate(nodes, id) {
  const walk = (list, parent) => {
    for (let i = 0; i < list.length; i++) {
      const node = list[i];
      if (!node) continue;
      if ((node.type === "page" || node.type === "folder") && node.id === id) {
        return { parent, index: i };
      }
      if (node.type === "folder" && Array.isArray(node.children)) {
        const found = walk(node.children, node.id);
        if (found) return found;
      }
    }
    return null;
  };
  return walk(nodes ?? [], null);
}

export function folderOf(nodes, id) {
  return locate(nodes, id)?.parent ?? null;
}

/// A lock object copied onto a folder, or null when the shape isn't one.
export function folderLock(lock) {
  if (!lock || typeof lock !== "object") return null;
  if (typeof lock.salt !== "string" || typeof lock.nonce !== "string" || typeof lock.wrapped !== "string") {
    return null;
  }
  return {
    salt: lock.salt,
    nonce: lock.nonce,
    wrapped: lock.wrapped,
    memory_kib: Number(lock.memory_kib) || 19 * 1024,
    iterations: Number(lock.iterations) || 2,
    lanes: Number(lock.lanes) || 1,
  };
}

/// The nearest locked folder at or above `id`, including `id` when it is a locked folder.
export function coveringLock(nodes, id) {
  if (id == null) return null;
  let found = null;
  const walk = (list, ancestor) => {
    for (const node of list ?? []) {
      if (!node) continue;
      if (node.type === "folder") {
        const here = node.lock ? { id: node.id, lock: node.lock } : ancestor;
        if (node.id === id) {
          found = here;
          return true;
        }
        if (walk(node.children, here)) return true;
      } else if (node.type === "page" && node.id === id) {
        found = ancestor;
        return true;
      }
    }
    return false;
  };
  walk(nodes, null);
  return found;
}

/// Page ids inside a folder, including pages in nested folders.
export function pagesUnder(nodes, folderId) {
  const out = [];
  const walk = (list, inside) => {
    for (const node of list ?? []) {
      if (node?.type === "page" && inside) out.push(node.id);
      else if (node?.type === "folder") walk(node.children, inside || node.id === folderId);
    }
  };
  walk(nodes, false);
  return out;
}

export function folderDepth(nodes, id) {
  const walk = (list, depth) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (node.id === id) return depth + 1;
      const found = walk(node.children, depth + 1);
      if (found) return found;
    }
    return null;
  };
  return walk(nodes, 0);
}

export function canNest(nodes, parent) {
  if (parent == null) return true;
  const depth = folderDepth(nodes, parent);
  return depth != null && depth < MAX_FOLDER_DEPTH;
}

export function freshName(nodes) {
  const names = [];
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      names.push(node.name);
      walk(node.children);
    }
  };
  walk(nodes);
  if (!names.includes("New folder")) return "New folder";
  let n = 2;
  for (;;) {
    const candidate = `New folder ${n}`;
    if (!names.includes(candidate)) return candidate;
    n += 1;
  }
}

function newFolderId() {
  return "f" + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

/// A folder at the top of `parent` (`null` is the root). `null` when that
/// parent is already nested as deep as a folder may go.
export function createFolder(nodes, parent, id) {
  if (parent != null && folderDepth(nodes, parent) == null) parent = null;
  if (!canNest(nodes, parent)) return null;
  const folderId = id || newFolderId();
  const node = {
    type: "folder",
    id: folderId,
    name: freshName(nodes),
    collapsed: false,
    children: [],
  };
  if (!insertNode(nodes, parent, 0, node)) return null;
  return folderId;
}

export function renameFolder(nodes, id, name) {
  const trimmed = String(name ?? "").trim();
  if (!trimmed) return false;
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (node.id === id) {
        node.name = trimmed;
        return true;
      }
      if (walk(node.children)) return true;
    }
    return false;
  };
  return walk(nodes);
}

function setCollapsed(nodes, id, collapsed) {
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (node.id === id) {
        const next = collapsed == null ? !node.collapsed : collapsed;
        if (!!node.collapsed === !!next) return false;
        node.collapsed = !!next;
        return true;
      }
      if (walk(node.children)) return true;
    }
    return false;
  };
  return walk(nodes);
}

export function toggleFolder(nodes, id) {
  return setCollapsed(nodes, id, null);
}

export function expandFolder(nodes, id) {
  return setCollapsed(nodes, id, false);
}

export function expandTo(nodes, pageId) {
  let changed = false;
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (pageIds(node.children).includes(pageId)) {
        if (node.collapsed) {
          node.collapsed = false;
          changed = true;
        }
        walk(node.children);
        return true;
      }
    }
    return false;
  };
  walk(nodes);
  return changed;
}

export function collectPageIds(node) {
  if (!node) return [];
  if (node.type === "page" && typeof node.id === "string") return [node.id];
  return pageIds(node.children);
}

export function removeNode(nodes, id) {
  const walk = (list, parent) => {
    for (let i = 0; i < list.length; i++) {
      const node = list[i];
      if (!node) continue;
      if ((node.type === "page" || node.type === "folder") && node.id === id) {
        list.splice(i, 1);
        return { parent, index: i, node };
      }
      if (node.type === "folder" && Array.isArray(node.children)) {
        const found = walk(node.children, node.id);
        if (found) return found;
      }
    }
    return null;
  };
  return walk(nodes ?? [], null);
}

export function insertNode(nodes, parent, index, node) {
  const list = childList(nodes, parent);
  if (!list) return false;
  const at = Math.max(0, Math.min(index, list.length));
  list.splice(at, 0, node);
  return true;
}

function childList(nodes, parent) {
  if (parent == null) return nodes;
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (node.id === parent) return node.children;
      const found = walk(node.children);
      if (found) return found;
    }
    return null;
  };
  return walk(nodes);
}

function childrenLen(nodes, parent) {
  const list = childList(nodes, parent);
  return list ? list.length : 0;
}

function contains(node, id) {
  if (!node) return false;
  if (node.id === id) return true;
  if (node.type !== "folder") return false;
  return (node.children || []).some((child) => contains(child, id));
}

function hasSubfolder(node) {
  return (node.children || []).some((child) => child?.type === "folder");
}

function canPlace(nodes, node, parent) {
  if (node?.type === "page") {
    if (parent == null) return true;
    const depth = folderDepth(nodes, parent);
    return depth != null && depth <= MAX_FOLDER_DEPTH;
  }
  if (node?.type !== "folder") return false;
  if (parent != null && (parent === node.id || contains(node, parent))) return false;
  let depth;
  if (parent == null) depth = 1;
  else {
    const parentDepth = folderDepth(nodes, parent);
    if (parentDepth == null) return false;
    depth = parentDepth + 1;
  }
  return depth <= MAX_FOLDER_DEPTH && (depth < MAX_FOLDER_DEPTH || !hasSubfolder(node));
}

/// `index` is among the destination's children after this item is taken out.
export function moveItem(nodes, id, parent, index) {
  const taken = removeNode(nodes, id);
  if (!taken) return false;
  const parentExists = parent == null || folderDepth(nodes, parent) != null;
  if (!parentExists || !canPlace(nodes, taken.node, parent)) {
    insertNode(nodes, taken.parent, taken.index, taken.node);
    return false;
  }
  const len = childrenLen(nodes, parent);
  if (taken.parent === parent && taken.index === Math.min(index, len)) {
    insertNode(nodes, taken.parent, taken.index, taken.node);
    return true;
  }
  if (!insertNode(nodes, parent, index, taken.node)) {
    insertNode(nodes, taken.parent, taken.index, taken.node);
    return false;
  }
  return true;
}

/// `spot` is `{kind:"before"|"after"|"into", id}` or `{kind:"root"}`.
export function place(nodes, id, spot) {
  const src = locate(nodes, id);
  if (!src || !spot) return false;
  let parent;
  let index;
  if (spot.kind === "before" || spot.kind === "after") {
    if (spot.id === id) return true;
    const sib = locate(nodes, spot.id);
    if (!sib) return false;
    parent = sib.parent;
    index = spot.kind === "after" ? sib.index + 1 : sib.index;
  } else if (spot.kind === "into") {
    if (spot.id === id) return false;
    parent = spot.id;
    index = 0;
  } else if (spot.kind === "root") {
    parent = null;
    index = Number.MAX_SAFE_INTEGER;
  } else {
    return false;
  }
  if (src.parent === parent && src.index < index) index -= 1;
  return moveItem(nodes, id, parent, index);
}

export function validDrop(nodes, id, spot) {
  try {
    return place(structuredClone(nodes), id, spot);
  } catch {
    return false;
  }
}

export function moveSibling(nodes, id, step) {
  const found = locate(nodes, id);
  if (!found) return false;
  const dest = found.index + step;
  const len = childrenLen(nodes, found.parent);
  if (dest < 0 || dest >= len) return false;
  const list = childList(nodes, found.parent);
  const [item] = list.splice(found.index, 1);
  list.splice(dest, 0, item);
  return true;
}

export function siblingEnds(nodes, id) {
  const found = locate(nodes, id);
  if (!found) return { first: true, last: true };
  const len = childrenLen(nodes, found.parent);
  return { first: found.index === 0, last: found.index + 1 >= len };
}
