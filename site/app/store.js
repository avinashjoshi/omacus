// Pages, settings, and cached exchange rates. IndexedDB is the store. localStorage
// is where older versions kept the same keys, and the fallback when IndexedDB is
// missing or the browser will not grant persistent storage. The old copy is not
// removed until the new one has been read back.

function syncTest() {
  try {
    if (typeof location !== "undefined" && location.hostname === "sync-test.omacus.com") return true;
    if (typeof document !== "undefined" && document.documentElement?.dataset?.syncTest === "1") return true;
  } catch {
    // The published app keeps its own store.
  }
  return false;
}

const DB_NAME = syncTest() ? "omacus-sync-test" : "omacus";
const STORE = "kv";
export const PREFIX = syncTest() ? "omacus-sync-test:" : "omacus:";
export const CHANNEL = syncTest() ? "omacus-sync-test" : "omacus";

const memory = new Map();
let db = null;
let useIdb = false;
let hydrated = false;
let mirror = true;
let persistOk = false;
const writes = new Set();
let channel = null;
// Other tabs listen on their own channel. A write must not look like someone else's.
export const selfTab = Math.random().toString(36).slice(2);

function pageChannel() {
  if (channel || typeof window === "undefined" || typeof BroadcastChannel === "undefined") return channel;
  try {
    channel = new BroadcastChannel(CHANNEL);
  } catch {
    channel = null;
  }
  return channel;
}

export function persistGranted() {
  return persistOk;
}

export function ready() {
  return hydrate();
}

let hydrateOnce = null;
function hydrate() {
  if (!hydrateOnce) hydrateOnce = openAndImport();
  return hydrateOnce;
}

async function openAndImport() {
  try {
    db = await openDb();
    useIdb = true;
    for (const [key, value] of await idbAll(db)) {
      if (!memory.has(key)) memory.set(key, String(value));
    }
  } catch {
    useIdb = false;
    db = null;
  }
  const imported = readLocal();
  const hadPages = [...memory.keys()].some((key) => key.startsWith("page:"));
  for (const [key, value] of imported) {
    if (key === "index") continue;
    if (!memory.has(key)) {
      memory.set(key, value);
      if (useIdb && !(await confirmPut(db, key, value))) memory.set(key, value);
    }
  }
  const localIndex = imported.find(([key]) => key === "index");
  if (localIndex) {
    const existing = memory.has("index") ? memory.get("index") : null;
    const next = existing == null || !hadPages ? localIndex[1] : mergeIndex(existing, localIndex[1]);
    if (memory.get("index") !== next) {
      memory.set("index", next);
      if (useIdb) await confirmPut(db, "index", next);
    }
  }
  await migrateSheetKeys();
  hydrated = true;
  await requestPersist();
}

async function migrateSheetKeys() {
  for (const key of [...memory.keys()]) {
    if (!key.startsWith("sheet:")) continue;
    const id = "page:" + key.slice("sheet:".length);
    if (!memory.has(id)) {
      const value = memory.get(key);
      memory.set(id, value);
      if (useIdb && db) {
        const ok = await confirmPut(db, id, value);
        if (!ok) continue;
      }
    }
    memory.delete(key);
    if (useIdb && db) await idbDelete(db, key);
    try {
      localStorage.removeItem(PREFIX + key);
    } catch {
      // The page: copy is already confirmed. A leftover sheet: key is the old name.
    }
  }
}

export async function requestPersist() {
  try {
    persistOk = (await navigator.storage?.persist?.()) === true;
  } catch {
    persistOk = false;
  }
  // Denied, or the browser has no persistent-storage API: keep a localStorage
  // mirror so IndexedDB is not the only copy.
  mirror = !persistOk;
  return persistOk;
}

function readLocal() {
  const out = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const full = localStorage.key(i);
      if (!full?.startsWith(PREFIX) || full === PREFIX + "migrated") continue;
      const value = localStorage.getItem(full);
      if (typeof value === "string") out.push([full.slice(PREFIX.length), value]);
    }
  } catch {
    // Private mode, or storage turned off. IndexedDB may still be available.
  }
  return out;
}

export function get(key) {
  if (memory.has(key)) return memory.get(key);
  if (hydrated) return null;
  try {
    return localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export function keys() {
  if (!hydrated) {
    try {
      for (let i = 0; i < localStorage.length; i++) {
        const full = localStorage.key(i);
        if (full?.startsWith(PREFIX)) memory.set(full.slice(PREFIX.length), localStorage.getItem(full));
      }
    } catch {
      // Fall through to whatever is already in memory.
    }
  }
  return [...memory.keys()];
}

export function adopt(key, value) {
  if (value == null) memory.delete(key);
  else memory.set(key, value);
}

function track(job) {
  writes.add(job);
  job.finally(() => writes.delete(job));
  return job;
}

export function set(key, value) {
  memory.set(key, value);
  const job = track(writeThrough(key, value));
  return job;
}

async function writeThrough(key, value) {
  let idbOk = false;
  if (useIdb && db) {
    try {
      idbOk = await confirmPut(db, key, value);
    } catch {
      idbOk = false;
    }
  }
  let localOk = false;
  if (mirror || !idbOk) {
    try {
      localStorage.setItem(PREFIX + key, value);
      localOk = true;
    } catch {
      localOk = false;
    }
  } else {
    try {
      pageChannel()?.postMessage({ key, value, tab: selfTab });
    } catch {
      // Another tab will not hear this write. This tab's copy is in IndexedDB.
    }
  }
  if (!idbOk && !localOk) throw new Error("storage");
  return true;
}

export function remove(key) {
  memory.delete(key);
  const dropped = useIdb && db ? idbDelete(db, key) : Promise.resolve(false);
  try {
    localStorage.removeItem(PREFIX + key);
  } catch {
    // The in-memory copy is already gone.
  }
  try {
    pageChannel()?.postMessage({ key, value: null, tab: selfTab });
  } catch {
    // See writeThrough.
  }
  return dropped;
}

export function drained() {
  return Promise.all([...writes]).then(() => {});
}

// Copy `entries` ([full localStorage key, value]) into this origin. A key that
// already has a value is left alone, except the page index, which gains any
// pages the sender has and this origin does not. Returns true only after every
// new value has been read back.
export async function acceptEntries(entries) {
  if (!Array.isArray(entries)) return false;
  let database = db;
  if (!database) {
    try {
      database = await openDb();
      db = database;
      useIdb = true;
    } catch {
      return false;
    }
  }
  const rows = [];
  for (const row of entries) {
    if (!Array.isArray(row) || row.length !== 2) return false;
    const [full, value] = row;
    if (typeof full !== "string" || !full.startsWith(PREFIX) || full === PREFIX + "migrated") continue;
    if (typeof value !== "string") return false;
    rows.push([full.slice(PREFIX.length), value]);
  }
  const before = await idbAll(database);
  const hadPages = before.some(([key]) => key.startsWith("page:"));
  const have = new Map(before);
  for (const [key, value] of rows) {
    if (key === "index") continue;
    if (have.has(key)) continue;
    if (!(await confirmPut(database, key, value))) return false;
    have.set(key, value);
    memory.set(key, value);
  }
  const incomingIndex = rows.find(([key]) => key === "index");
  if (incomingIndex) {
    const existing = have.get("index");
    const next = existing == null || !hadPages ? incomingIndex[1] : mergeIndex(existing, incomingIndex[1]);
    if (have.get("index") !== next) {
      if (!(await confirmPut(database, "index", next))) return false;
      memory.set("index", next);
    }
  }
  return true;
}

export function mergeIndex(existingRaw, incomingRaw) {
  const existing = parseIndex(existingRaw);
  const incoming = parseIndex(incomingRaw);
  const tree = Array.isArray(existing.tree) ? existing.tree.map(cloneNode) : [];
  const have = new Set(pagesIn(tree));
  for (const id of pagesIn(incoming.tree)) {
    if (!have.has(id)) {
      tree.push({ type: "page", id });
      have.add(id);
    }
  }
  for (const id of [...(existing.order || []), ...(incoming.order || [])]) {
    if (typeof id === "string" && id && !have.has(id)) {
      tree.push({ type: "page", id });
      have.add(id);
    }
  }
  const handles = { ...(incoming.handles || {}), ...(existing.handles || {}) };
  const order = pagesIn(tree);
  const active = order.includes(existing.active) ? existing.active : order.includes(incoming.active) ? incoming.active : order[0] ?? null;
  return JSON.stringify({ order, active, tree, handles });
}

function parseIndex(raw) {
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" ? value : {};
  } catch {
    return {};
  }
}

function cloneNode(node) {
  if (!node || typeof node !== "object") return node;
  if (node.type === "folder") return { ...node, children: (node.children || []).map(cloneNode) };
  return { ...node };
}

function pagesIn(tree, out = []) {
  for (const node of tree || []) {
    if (node?.type === "page" && typeof node.id === "string") out.push(node.id);
    else if (node?.type === "folder") pagesIn(node.children, out);
  }
  return out;
}

function openDb() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const database = req.result;
      if (!database.objectStoreNames.contains(STORE)) database.createObjectStore(STORE);
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function confirmPut(database, key, value) {
  return new Promise((resolve) => {
    const tx = database.transaction(STORE, "readwrite");
    tx.objectStore(STORE).put(value, key);
    tx.oncomplete = () => {
      const read = database.transaction(STORE, "readonly");
      const get = read.objectStore(STORE).get(key);
      get.onsuccess = () => resolve(get.result === value);
      get.onerror = () => resolve(false);
    };
    tx.onerror = () => resolve(false);
    tx.onabort = () => resolve(false);
  });
}

function idbDelete(database, key) {
  return new Promise((resolve) => {
    try {
      const tx = database.transaction(STORE, "readwrite");
      tx.objectStore(STORE).delete(key);
      tx.oncomplete = () => resolve(true);
      tx.onerror = () => resolve(false);
      tx.onabort = () => resolve(false);
    } catch {
      resolve(false);
    }
  });
}

function idbAll(database) {
  return new Promise((resolve, reject) => {
    const tx = database.transaction(STORE, "readonly");
    const req = tx.objectStore(STORE).getAllKeys();
    const values = tx.objectStore(STORE).getAll();
    tx.oncomplete = () => {
      const keys = req.result || [];
      const rows = values.result || [];
      resolve(keys.map((key, i) => [String(key), rows[i]]));
    };
    tx.onerror = () => reject(tx.error);
  });
}
