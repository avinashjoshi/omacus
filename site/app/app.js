// app.omacus.com: the notepad calculator in the browser. The math is the desktop
// app's own engine, compiled to WebAssembly (pkg/, built from web/ in the repo).
// Pages live in this browser's IndexedDB; a share link carries a page in the
// part after "#", which browsers never send to the server.

import init, {
  Calc, shareEncode, shareDecode, layoutAnswers, realignMap, pageMarks, previewBlocks, variables, wordKind,
  bindingAt, renameVariable, definitionChange, pageRefAt, rewriteVarRefs, renameBlurb, propagateRename, propagateHandle,
  lockCreate, lockWrap, lockSeal, lockOpen, lockUnwrap, lockOpenPassword, lockIs,
  docMigrate, docSnapshot, docApply, docMergeDisk, docConflicts, docPick, docRemove, docSetFolders, docUnion,
} from "./pkg/omacus_web.js";
import {
  EditorState, StateField, StateEffect, RangeSetBuilder, Prec, EditorView, Decoration, WidgetType, keymap, hoverTooltip,
  defaultKeymap, history as undoHistory, historyKeymap, autocompletion, acceptCompletion, closeCompletion,
  moveCompletionSelection, startCompletion, completionStatus,
} from "./editor.js";
import {
  appendMissing, canNest, collectPageIds, coveringLock, createFolder, expandFolder, expandTo, folderOf, folders, fromPages,
  insertNode, moveSibling, pageIds, pagesUnder, place, removeNode, renameFolder, sanitize, siblingEnds, toggleFolder, validDrop,
  visible,
} from "./tree.js";
import {
  ready as storeReady, get as storeGet, set as storeSet, remove as storeRemove, adopt as storeAdopt,
  drained as storeDrained, keys as storeKeys, persistGranted, selfTab, PREFIX, CHANNEL,
} from "./store.js";
import { mountSync } from "./sync.js";
import { allocateHandle, freshLibrary, seedAction } from "./seed.js";
import { backupEntries, mergeBackup, parseBackup, unzipNamed, zipBytes } from "./backup.js";

// Set before any await, so a failed import never looks like a slow start.
globalThis.__omacusBooted = true;
const bootFail = document.querySelector("[data-boot-fail]");
if (bootFail) bootFail.hidden = true;
const bootLoading = document.querySelector("[data-loading]");
if (bootLoading) bootLoading.hidden = false;

const WASM = new URL("pkg/omacus_web_bg.wasm", import.meta.url).href;
const WORKER = new URL("./worker.js", import.meta.url).href;
const RATES_URL = "https://open.er-api.com/v6/latest/USD";
const RATES_MAX_AGE = 12 * 60 * 60 * 1000;
const KEYWORDS = new Set(["prev", "sum", "total", "avg", "average", "mean", "min", "max", "today", "now",
  "tomorrow", "yesterday", "plus", "minus", "times", "mod"]);
const HINT_IDLE = 600;

const WELCOME = `# Welcome to Omacus
// Write math the way you'd write a note. Answers appear on the right.
// The Examples folder has a few short pages. Monthly budget reads @trip.total.

Flights: 4 * $1,180
Hotel ¥68,000 * 5 in USD
total
each = prev / 4

6pm PST in Tokyo
15% off $1,299
18% tip on $62
$84 split 3 ways
3.5 km in miles
4 GB at 100 Mbps
days until Christmas
#ff8800 in hsl

// Click an answer to copy it. Share link sends this page to anyone.
// Private: your pages are saved in this browser only. Nothing is uploaded.
`;

// Embedded in omacus.com's homepage (/app/?embed#v1.…): just the editor, on the sheet
// in the link, with nothing read from or saved to this browser's sheets.
const EMBED = new URLSearchParams(location.search).has("embed");

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const el = {
  page: document.body,
  host: $("[data-text]"),
  editor: $("[data-editor]"),
  sheets: $("[data-sheets]"),
  title: $("[data-title]"),
  shared: $("[data-shared]"),
  toast: $("[data-toast]"),
  menu: $("[data-menu]"),
  menuButton: $("[data-menu-button]"),
  importInput: $("[data-import]"),
  loading: $("[data-loading]"),
  empty: $("[data-empty]"),
  deleted: $("[data-deleted]"),
  confirm: $("[data-confirm]"),
  confirmText: $("[data-confirm-text]"),
  pick: $("[data-pick]"),
  pickTitle: $("[data-pick-title]"),
  pickDetail: $("[data-pick-detail]"),
  pickLines: $("[data-pick-lines]"),
  pickOurs: $("[data-pick-ours]"),
  pickTheirs: $("[data-pick-theirs]"),
  pickKeep: $("[data-pick-keep]"),
  pickOther: $("[data-pick-other]"),
  handleDialog: $("[data-handle]"),
  handleTitle: $("[data-handle-title]"),
  handleInput: $("[data-handle-input]"),
  handleNote: $("[data-handle-note]"),
  workspace: $("[data-workspace]"),
  editor: $("[data-editor]"),
  preview: $("[data-preview]"),
  previewTitle: $("[data-preview-title]"),
  previewBody: $("[data-preview-body]"),
  settings: $("[data-settings]"),
  pagePreview: $("[data-page-preview]"),
  pagePreviewBody: $("[data-page-preview-body]"),
  move: $("[data-move]"),
  moveList: $("[data-move-list]"),
  rename: $("[data-rename]"),
  renameInput: $("[data-rename-input]"),
  switcher: $("[data-switcher]"),
  switcherQuery: $("[data-switcher-query]"),
  switcherHits: $("[data-switcher-hits]"),
  rowMenu: $("[data-row-menu]"),
  lock: $("[data-lock]"),
  lockTitle: $("[data-lock-title]"),
  lockDetail: $("[data-lock-detail]"),
  lockCurrent: $("[data-lock-current]"),
  lockPassword: $("[data-lock-password]"),
  lockConfirm: $("[data-lock-confirm]"),
  lockError: $("[data-lock-error]"),
  lockYes: $("[data-lock-yes]"),
  gate: $("[data-lock-gate]"),
  gatePassword: $("[data-gate-password]"),
  gateError: $("[data-gate-error]"),
};

// pageMarks and variables throw until init() finishes. The editor is created first.
let engineReady = false;

let calc;
let results = [];
// The text `results` were worked out for (the editor may be ahead of it), and whether
// they're all of that text's answers, fresh (not some still on their way).
let resultsText = "";
let resultsExact = false;
const state = { order: [], active: null, texts: new Map(), shared: null, tree: [], handles: new Map() };
// Password lock is a hook for a feature that has not landed. Nothing sets this.
// A reference to a locked page shows "🔒 locked" instead of the value.
// Plaintext and content keys stay in memory. Storage only ever sees ciphertext
// for a locked page. `plains` is the open text; `pageKeys` is an individual lock.
const plains = new Map();
const pageKeys = new Map();
const folderKeys = new Map();
let lastInput = Date.now();
let pendingNew = null;

const LOCKED_PAGE = "Locked page";
// Sidebar glyphs are real SVG nodes. Parsing a markup string is a TrustedHTML
// sink, and the page has no policy that would accept one.

function noteActivity() {
  lastInput = Date.now();
}

function isLock(text) {
  return typeof text === "string" && text.startsWith("OMACUS-LOCK/1\n");
}

function wrapOfStored(text) {
  if (!isLock(text)) return null;
  const fields = {};
  for (const line of text.split("\n")) {
    const at = line.indexOf("=");
    if (at > 0) fields[line.slice(0, at)] = line.slice(at + 1);
  }
  if (!fields.salt || !fields.wrap || !fields.key) return null;
  return {
    salt: fields.salt,
    nonce: fields.wrap,
    wrapped: fields.key,
    memory_kib: Number(fields.m) || 19 * 1024,
    iterations: Number(fields.t) || 2,
    lanes: Number(fields.p) || 1,
  };
}

function sameWrap(a, b) {
  return !!a && !!b && a.salt === b.salt && a.nonce === b.nonce && a.wrapped === b.wrapped;
}

function treeHasWrap(wrap) {
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (sameWrap(wrap, node.lock)) return true;
      if (walk(node.children)) return true;
    }
    return false;
  };
  return walk(state.tree);
}

function individualLock(id) {
  const wrap = wrapOfStored(state.texts.get(id) ?? "");
  if (!wrap) return false;
  const cover = coveringLock(state.tree, id);
  if (cover) return !sameWrap(wrap, cover.lock);
  return !treeHasWrap(wrap);
}

function keyFor(id) {
  if (!individualLock(id)) {
    const cover = coveringLock(state.tree, id);
    if (cover && folderKeys.has(cover.id)) return folderKeys.get(cover.id);
  }
  return pageKeys.get(id) ?? null;
}

function wrapFor(id) {
  if (!individualLock(id)) {
    const cover = coveringLock(state.tree, id);
    if (cover?.lock) return cover.lock;
  }
  return wrapOfStored(state.texts.get(id) ?? "");
}

function pageConcealed(id) {
  if (!id || id === "shared") return false;
  if (plains.has(id)) return false;
  if (isLock(state.texts.get(id) ?? "")) return true;
  return !!coveringLock(state.tree, id);
}

function shownPlain(id) {
  if (plains.has(id)) return plains.get(id);
  const stored = state.texts.get(id) ?? "";
  return isLock(stored) ? "" : stored;
}

function pageLabel(id) {
  if (id === "shared") return heading(state.shared?.text ?? "");
  if (pageConcealed(id)) return LOCKED_PAGE;
  return heading(shownPlain(id));
}

function anyUnlocked() {
  if (folderKeys.size > 0) return true;
  for (const id of plains.keys()) {
    if (isLock(state.texts.get(id) ?? "") || coveringLock(state.tree, id)) return true;
  }
  return false;
}

// What Lock does for this page. A page with no password must ask for one.
// "relock" closes a lock this session already opened. "unlock" is a closed lock.
function pageLockChoice(id) {
  if (!id || id === "shared" || !pageIds(state.tree).includes(id)) return "none";
  const covered = !!(coveringLock(state.tree, id) && !individualLock(id));
  const own = isLock(state.texts.get(id) ?? "") && !covered;
  if (pageConcealed(id)) return "unlock";
  if (own || covered) return "relock";
  return "create";
}

function ownPageLock(id) {
  if (!id || id === "shared") return false;
  return isLock(state.texts.get(id) ?? "") && !(coveringLock(state.tree, id) && !individualLock(id));
}

function parseLockResult(raw) {
  try {
    return JSON.parse(raw);
  } catch {
    return { ok: false, error: "That lock could not be read." };
  }
}

function sealWith(key, wrap, plain, id) {
  if (!engineReady || !key || !wrap) return null;
  const out = parseLockResult(lockSeal(key, plain, JSON.stringify(wrap), id));
  return out.ok ? out.text : null;
}

function reseal(id, plain) {
  return sealWith(keyFor(id), wrapFor(id), plain, id);
}

function revealPage(id, key) {
  const stored = state.texts.get(id) ?? "";
  if (!isLock(stored)) {
    plains.set(id, stored);
    return true;
  }
  if (!key) return false;
  const out = parseLockResult(lockOpen(key, stored, id));
  if (!out.ok) return false;
  plains.set(id, out.text);
  return true;
}

function setFolderLock(id, lock) {
  const walk = (list) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      if (node.id === id) {
        if (lock) node.lock = lock;
        else delete node.lock;
        return true;
      }
      if (walk(node.children)) return true;
    }
    return false;
  };
  return walk(state.tree);
}

function forgetPage(id) {
  plains.delete(id);
  pageKeys.delete(id);
  editorStates.delete(id);
}

function lockNow(quiet = false) {
  flushSave();
  plains.clear();
  pageKeys.clear();
  folderKeys.clear();
  editorStates.clear();
  shownId = null;
  closePagePreview();
  if (!quiet) {
    showSheet();
    renderSidebar();
    toast("Locked");
  }
  noteActivity();
}

function refuseConcealed() {
  if (state.active && state.active !== "shared" && pageConcealed(state.active)) {
    toast("Unlock this page first.");
    return true;
  }
  return false;
}

function spotCover(spot) {
  if (!spot || spot.kind === "root") return null;
  if (spot.kind === "into") return coveringLock(state.tree, spot.id);
  const parent = folderOf(state.tree, spot.id);
  return parent ? coveringLock(state.tree, parent) : null;
}

function movingPages(id) {
  return folders(state.tree).some((folder) => folder.id === id) ? pagesUnder(state.tree, id) : [id];
}

function canMoveLocked(id, spot) {
  const cover = spotCover(spot);
  if (!cover || folderKeys.has(cover.id)) return true;
  return movingPages(id).every((pid) => individualLock(pid) || (isLock(state.texts.get(pid) ?? "") && !plains.has(pid)));
}

function rebindPage(id, oldId) {
  const next = coveringLock(state.tree, id);
  const nextId = next?.id ?? null;
  if (nextId === oldId || individualLock(id)) return;
  if (next && folderKeys.has(next.id)) {
    const plain = plains.has(id) ? plains.get(id) : isLock(state.texts.get(id) ?? "") ? null : state.texts.get(id) ?? "";
    if (plain == null) return;
    const sealed = sealWith(folderKeys.get(next.id), next.lock, plain, id);
    if (!sealed) return;
    state.texts.set(id, sealed);
    plains.set(id, plain);
    pageKeys.delete(id);
    saveSheet(id);
  } else if (!next && plains.has(id)) {
    state.texts.set(id, plains.get(id));
    plains.delete(id);
    pageKeys.delete(id);
    saveSheet(id);
  }
}

function applyPlace(id, spot) {
  if (!canMoveLocked(id, spot)) {
    toast("Unlock the folder before moving a page into it.");
    return false;
  }
  const before = movingPages(id).map((pid) => [pid, coveringLock(state.tree, pid)?.id ?? null]);
  if (!place(state.tree, id, spot)) return false;
  for (const [pid, oldId] of before) rebindPage(pid, oldId);
  syncOrder();
  saveIndex();
  if (state.active && pageIds(state.tree).includes(state.active)) showSheet();
  return true;
}

function closeLock() {
  if (el.lock) el.lock.hidden = true;
  pendingNew = null;
}

const PASSWORD_NUDGE = "Save this password in a password manager. A forgotten password can't be recovered.";

function openLock(mode, id) {
  if (!el.lock) return;
  closeMenu();
  closeRowMenu();
  closeSwitcher();
  el.lock.dataset.mode = mode;
  el.lock.dataset.id = id;
  const copy = {
    "create-page": ["Lock page", PASSWORD_NUDGE, "Lock"],
    "create-folder": ["Lock folder", `One password locks every page in this folder. ${PASSWORD_NUDGE}`, "Lock"],
    "unlock-folder": ["Unlock folder", "Enter the password to open the pages in this folder.", "Unlock"],
    "change-page": ["Change password", PASSWORD_NUDGE, "Change"],
    "change-folder": ["Change password", PASSWORD_NUDGE, "Change"],
    "remove-page": ["Remove lock", "Enter the password. The page stays on this device, without a lock.", "Remove lock"],
    "remove-folder": ["Remove lock", "Pages in this folder will be stored without a password.", "Remove lock"],
  }[mode] || ["Lock", PASSWORD_NUDGE, "Lock"];
  if (el.lockTitle) el.lockTitle.textContent = copy[0];
  if (el.lockDetail) el.lockDetail.textContent = copy[1];
  if (el.lockYes) {
    el.lockYes.textContent = copy[2];
    const remove = mode.startsWith("remove");
    el.lockYes.classList.toggle("primary", !remove);
    el.lockYes.classList.toggle("danger", remove);
  }
  const change = mode.startsWith("change");
  const remove = mode.startsWith("remove");
  if (el.lockCurrent) el.lockCurrent.hidden = !change;
  if (el.lockPassword) el.lockPassword.hidden = false;
  if (el.lockConfirm) el.lockConfirm.hidden = remove || mode === "unlock-folder";
  for (const input of [el.lockCurrent, el.lockPassword, el.lockConfirm]) {
    if (input) input.value = "";
  }
  if (el.lockError) {
    el.lockError.hidden = true;
    el.lockError.textContent = "";
  }
  el.lock.hidden = false;
  // Drop a value card that was already open. The lock dialog is the only popup.
  view.dispatch({});
  const focus = change ? el.lockCurrent : el.lockPassword;
  focus?.focus();
}

function lockError(message) {
  if (!el.lockError) return;
  el.lockError.hidden = false;
  el.lockError.textContent = message;
}

function submitLock() {
  const mode = el.lock?.dataset.mode;
  const id = el.lock?.dataset.id;
  if (!mode || !id) return;
  if (!engineReady) {
    lockError("Still loading the engine.");
    return;
  }
  const password = el.lockPassword?.value ?? "";
  const confirm = el.lockConfirm?.value ?? "";
  const current = el.lockCurrent?.value ?? "";
  const needsMatch = mode === "create-page" || mode === "create-folder" || mode.startsWith("change");
  if (needsMatch && password !== confirm) {
    lockError("Those passwords don't match.");
    return;
  }
  let error = null;
  if (mode === "create-page") error = lockPage(id, password);
  else if (mode === "create-folder") error = lockFolder(id, password);
  else if (mode === "unlock-folder") error = unlockFolder(id, password);
  else if (mode === "change-page") error = changePagePassword(id, current, password);
  else if (mode === "change-folder") error = changeFolderPassword(id, current, password);
  else if (mode === "remove-page") error = removePageLock(id, password);
  else if (mode === "remove-folder") error = removeFolderLock(id);
  if (error) {
    lockError(error);
    return;
  }
  const folder = pendingNew;
  const created = mode === "create-page" || mode === "create-folder";
  closeLock();
  if (created) {
    lockNow();
    return;
  }
  if (mode === "unlock-folder" && folder) {
    pendingNew = null;
    addSheet("", { parent: folder.parent });
  }
  saveIndex();
  showSheet();
  renderSidebar();
  noteActivity();
}

function lockPage(id, password) {
  if (coveringLock(state.tree, id)) return "This page is inside a locked folder.";
  const stored = state.texts.get(id) ?? "";
  if (isLock(stored)) return "This page is already locked.";
  const created = parseLockResult(lockCreate(password));
  if (!created.ok) return created.error;
  const plain = plains.get(id) ?? stored;
  const sealed = sealWith(created.key, created.wrap, plain, id);
  if (!sealed) return "That lock could not be read.";
  state.texts.set(id, sealed);
  plains.set(id, plain);
  pageKeys.set(id, created.key);
  saveSheet(id);
  return null;
}

function unlockPage(id, password) {
  const cover = coveringLock(state.tree, id);
  const wrap = wrapOfStored(state.texts.get(id) ?? "");
  if (cover && (!wrap || sameWrap(wrap, cover.lock))) return unlockFolder(cover.id, password);
  const opened = parseLockResult(lockOpenPassword(password, state.texts.get(id) ?? "", id));
  if (!opened.ok) return opened.error;
  plains.set(id, opened.text);
  pageKeys.set(id, opened.key);
  return null;
}

function changePagePassword(id, current, next) {
  if (!plains.has(id)) return "Unlock this page first.";
  const wrap = wrapOfStored(state.texts.get(id) ?? "");
  if (!wrap) return "This page isn't locked.";
  const check = parseLockResult(lockUnwrap(current, JSON.stringify(wrap)));
  if (!check.ok) return check.error;
  const key = pageKeys.get(id);
  if (!key) return "Unlock this page first.";
  const wrapped = parseLockResult(lockWrap(next, key));
  if (!wrapped.ok) return wrapped.error;
  const sealed = sealWith(key, wrapped.wrap, plains.get(id), id);
  if (!sealed) return "That lock could not be read.";
  state.texts.set(id, sealed);
  saveSheet(id);
  return null;
}

function removePageLock(id, password) {
  const stored = state.texts.get(id) ?? "";
  const cover = coveringLock(state.tree, id);
  if (cover && sameWrap(wrapOfStored(stored), cover.lock)) {
    return "The folder is locked. Remove that lock instead.";
  }
  if (!isLock(stored)) return "This page isn't locked.";
  const opened = parseLockResult(lockOpenPassword(password, stored, id));
  if (!opened.ok) return opened.error;
  // A wrong password never reaches here. An open page may have an edit that
  // is newer than the sealed copy; keep that text once the password checks out.
  const plain = plains.has(id) ? plains.get(id) : opened.text;
  state.texts.set(id, plain);
  forgetPage(id);
  saveSheet(id);
  return null;
}

function lockFolder(id, password) {
  if (!folders(state.tree).some((folder) => folder.id === id)) return "That folder is gone.";
  const self = coveringLock(state.tree, id);
  if (self?.id === id) return "This folder is already locked.";
  if (self) return "A folder around this one is already locked.";
  const created = parseLockResult(lockCreate(password));
  if (!created.ok) return created.error;
  setFolderLock(id, created.wrap);
  folderKeys.set(id, created.key);
  for (const pid of pagesUnder(state.tree, id)) {
    if (individualLock(pid)) continue;
    const stored = state.texts.get(pid) ?? "";
    if (isLock(stored) && !plains.has(pid)) continue;
    const plain = plains.has(pid) ? plains.get(pid) : stored;
    const sealed = sealWith(created.key, created.wrap, plain, pid);
    if (!sealed) continue;
    state.texts.set(pid, sealed);
    plains.set(pid, plain);
    pageKeys.delete(pid);
    saveSheet(pid);
  }
  return null;
}

function unlockFolder(id, password) {
  const cover = coveringLock(state.tree, id);
  if (!cover || cover.id !== id) return "This folder isn't locked.";
  const opened = parseLockResult(lockUnwrap(password, JSON.stringify(cover.lock)));
  if (!opened.ok) return opened.error;
  folderKeys.set(id, opened.key);
  for (const pid of pagesUnder(state.tree, id)) {
    if (individualLock(pid)) continue;
    revealPage(pid, opened.key);
  }
  return null;
}

function changeFolderPassword(id, current, next) {
  const cover = coveringLock(state.tree, id);
  if (!cover || cover.id !== id) return "This folder isn't locked.";
  const check = parseLockResult(lockUnwrap(current, JSON.stringify(cover.lock)));
  if (!check.ok) return check.error;
  const key = folderKeys.get(id);
  if (!key) return "Unlock this folder first.";
  const wrapped = parseLockResult(lockWrap(next, key));
  if (!wrapped.ok) return wrapped.error;
  setFolderLock(id, wrapped.wrap);
  for (const pid of pagesUnder(state.tree, id)) {
    if (individualLock(pid) || !plains.has(pid)) continue;
    const sealed = sealWith(key, wrapped.wrap, plains.get(pid), pid);
    if (!sealed) continue;
    state.texts.set(pid, sealed);
    saveSheet(pid);
  }
  return null;
}

function removeFolderLock(id) {
  const cover = coveringLock(state.tree, id);
  if (!cover || cover.id !== id) return "This folder isn't locked.";
  if (!folderKeys.has(id)) return "Unlock this folder first.";
  for (const pid of pagesUnder(state.tree, id)) {
    if (!sameWrap(wrapOfStored(state.texts.get(pid) ?? ""), cover.lock)) continue;
    if (!plains.has(pid)) continue;
    state.texts.set(pid, plains.get(pid));
    forgetPage(pid);
    saveSheet(pid);
  }
  folderKeys.delete(id);
  setFolderLock(id, null);
  return null;
}

function submitGate() {
  const id = state.active;
  if (!id || id === "shared") return;
  const error = unlockPage(id, el.gatePassword?.value ?? "");
  if (error) {
    if (el.gateError) {
      el.gateError.hidden = false;
      el.gateError.textContent = error;
    }
    return;
  }
  if (el.gatePassword) el.gatePassword.value = "";
  if (el.gateError) el.gateError.hidden = true;
  noteActivity();
  showSheet();
  renderSidebar();
  view?.focus();
}

function showGate() {
  if (el.gate) el.gate.hidden = false;
  if (el.editor) el.editor.hidden = true;
  if (el.pagePreview) el.pagePreview.hidden = true;
  pagePreviewOpen = false;
  el.gatePassword?.focus();
}

function hideGate() {
  if (el.gate) el.gate.hidden = true;
}

function onLockKey(e) {
  if (e.key !== "Enter") return;
  e.preventDefault();
  submitLock();
}
el.lockPassword?.addEventListener("keydown", onLockKey);
el.lockConfirm?.addEventListener("keydown", onLockKey);
el.lockCurrent?.addEventListener("keydown", onLockKey);
el.gatePassword?.addEventListener("keydown", (e) => {
  if (e.key !== "Enter") return;
  e.preventDefault();
  submitGate();
});
addEventListener("pointerdown", noteActivity, true);
addEventListener("keydown", noteActivity, true);
setInterval(() => {
  const minutes = settings.lockMinutes ?? 5;
  if (minutes > 0 && anyUnlocked() && Date.now() - lastInput >= minutes * 60 * 1000) lockNow();
}, 5000);

// ---------------------------------------------------------------------------------
// Storage: IndexedDB, with localStorage as the previous store and the fallback
// when persistent storage is denied. `storeReady()` runs before the first read.

const store = {
  get: (key) => storeGet(key),
  set(key, value) {
    return storeSet(key, value).catch(() => {
      toast("Couldn't save: this browser's storage is full or turned off.");
      return false;
    });
  },
  remove: (key) => storeRemove(key),
};

// Colors live in site/css/tokens.css (data-app-theme). Auto follows this device.
const THEME_NOTES = {
  auto: "Follows this device's light or dark mode.",
  light: "A plain light theme.",
  dark: "A dark theme.",
  paper: "Warm paper, for reading in daylight.",
  dusk: "A blue night theme.",
};

function loadSettings() {
  try {
    const saved = JSON.parse(store.get("settings") || "null");
    const theme = ["auto", "light", "dark", "paper", "dusk"].includes(saved?.theme) ? saved.theme : "auto";
    let lockMinutes = Number(saved?.lockMinutes);
    if (!Number.isFinite(lockMinutes)) lockMinutes = 5;
    lockMinutes = Math.max(0, Math.min(240, Math.round(lockMinutes)));
    return { theme, hideAnswers: !!saved?.hideAnswers, lockMinutes };
  } catch {
    return { theme: "auto", hideAnswers: false, lockMinutes: 5 };
  }
}

let settings = loadSettings();

function saveSettings() {
  store.set("settings", JSON.stringify(settings));
}

function themeName() {
  if (settings.theme !== "auto") return settings.theme;
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

// The first matching theme-color wins, so Auto keeps the light and dark tags
// and a chosen theme replaces them with the page background.
const themeColorDefaults = [...document.querySelectorAll('meta[name="theme-color"]')].map((meta) => ({
  content: meta.getAttribute("content") || "",
  media: meta.getAttribute("media") || "",
}));

function syncThemeChrome() {
  for (const meta of document.querySelectorAll('meta[name="theme-color"]')) meta.remove();
  const explicit = settings.theme !== "auto";
  const specs = explicit
    ? [{ content: getComputedStyle(document.documentElement).getPropertyValue("--app-bg").trim(), media: "" }]
    : themeColorDefaults;
  for (const spec of specs) {
    const meta = document.createElement("meta");
    meta.setAttribute("name", "theme-color");
    if (spec.media) meta.setAttribute("media", spec.media);
    meta.setAttribute("content", spec.content);
    document.head.appendChild(meta);
  }
  // black-translucent lets the page paint under the status bar and the home
  // indicator. The bar itself is a solid colour over that, so the clock is
  // not a blur. Dropping this tag insets the web view and leaves a band.
  let status = document.querySelector('meta[name="apple-mobile-web-app-status-bar-style"]');
  if (!status) {
    status = document.createElement("meta");
    status.setAttribute("name", "apple-mobile-web-app-status-bar-style");
    document.head.appendChild(status);
  }
  status.setAttribute("content", "black-translucent");
}

function applySettings() {
  const name = themeName();
  document.documentElement.dataset.appTheme = name;
  document.body.classList.toggle("answers-inline", settings.hideAnswers);
  syncThemeChrome();
}

function updateThemeNote() {
  const note = $("[data-theme-note]");
  if (note) note.textContent = THEME_NOTES[settings.theme] || THEME_NOTES.auto;
}

function showSettingsTab(name) {
  const panels = [...document.querySelectorAll("[data-settings-panel]")];
  const known = new Set(panels.map((panel) => panel.dataset.settingsPanel));
  const tab = known.has(name) ? name : "settings";
  for (const button of document.querySelectorAll("[data-settings-tab]")) {
    button.setAttribute("aria-selected", String(button.dataset.settingsTab === tab));
  }
  for (const panel of panels) {
    panel.hidden = panel.dataset.settingsPanel !== tab;
  }
  const title = document.getElementById("settings-title");
  const titles = { about: "About", sync: "Sync" };
  if (title) title.textContent = titles[tab] || "Settings";
}

function showBuildHash() {
  const node = $("[data-build-hash]");
  if (!node) return;
  const hash = (document.querySelector('meta[name="omacus-build"]')?.content || "").trim();
  node.textContent = /^[0-9a-f]{64}$/i.test(hash) ? hash : "Not a published build.";
}

function openSettings(tab = "settings") {
  closeMenu();
  if (!el.settings) return;
  const select = $("[data-setting=theme]", el.settings);
  const hide = $("[data-setting=hide-answers]", el.settings);
  if (select) select.value = settings.theme;
  if (hide) hide.checked = settings.hideAnswers;
  const lockAfter = $("[data-setting=lock-minutes]", el.settings);
  if (lockAfter) lockAfter.value = String(settings.lockMinutes ?? 5);
  updateThemeNote();
  showBuildHash();
  showSettingsTab(tab);
  el.settings.hidden = false;
}

function closeSettings() {
  if (el.settings) el.settings.hidden = true;
}

function toggleAnswers() {
  settings.hideAnswers = !settings.hideAnswers;
  saveSettings();
  applySettings();
  const hide = el.settings && $("[data-setting=hide-answers]", el.settings);
  if (hide) hide.checked = settings.hideAnswers;
  toast(settings.hideAnswers ? "Answers sit after each line." : "Answers are in the column.");
}

applySettings();
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
  if (settings.theme === "auto") applySettings();
});
$("[data-setting=theme]")?.addEventListener("change", (e) => {
  settings.theme = e.target.value;
  saveSettings();
  applySettings();
  updateThemeNote();
});
$("[data-setting=hide-answers]")?.addEventListener("change", (e) => {
  settings.hideAnswers = e.target.checked;
  saveSettings();
  applySettings();
});
$("[data-setting=lock-minutes]")?.addEventListener("change", (e) => {
  const minutes = Number(e.target.value);
  settings.lockMinutes = Number.isFinite(minutes) ? Math.max(0, Math.min(240, minutes)) : 5;
  saveSettings();
  noteActivity();
});

// Older builds stored each page under `sheet:`. Read those once and write `page:`.
function migratePageKeys() {
  for (const key of storeKeys()) {
    if (!key.startsWith("sheet:")) continue;
    const id = key.slice("sheet:".length);
    if (store.get("page:" + id) == null) {
      const value = store.get(key);
      if (value != null) store.set("page:" + id, value);
    }
    store.remove(key);
  }
}

function storedPage(id) {
  return typeof id === "string" && !!id && store.get("page:" + id) !== null;
}

function load() {
  migratePageKeys();
  let index = {};
  try {
    index = JSON.parse(store.get("index") ?? "{}");
  } catch {}
  const order = Array.isArray(index.order) ? index.order.filter((id) => storedPage(id)) : [];
  const fromTree = pageIds(index.tree).filter((id) => storedPage(id));
  const known = [...new Set([...order, ...fromTree])];
  state.texts = new Map(known.map((id) => [id, store.get("page:" + id)]));
  if (Array.isArray(index.tree)) {
    state.tree = sanitize(index.tree, new Set(known));
    appendMissing(state.tree, order);
    appendMissing(state.tree, known);
  } else {
    state.tree = fromPages(order);
  }
  syncOrder();
  state.active = state.order.includes(index.active) ? index.active : state.order[0] ?? null;
  librarySeeded = index.seeded === true;
  state.handles = new Map();
  const saved = index.handles && typeof index.handles === "object" ? index.handles : {};
  for (const [id, handle] of Object.entries(saved)) {
    if (typeof handle === "string" && handle) state.handles.set(id, handle);
  }
  ensureHandles();
}

function syncOrder() {
  state.order = pageIds(state.tree);
}

let librarySeeded = false;

function saveIndex() {
  syncOrder();
  store.set("index", JSON.stringify({
    order: state.order,
    active: state.active,
    tree: state.tree,
    handles: Object.fromEntries(state.handles),
    seeded: librarySeeded,
  }));
}

function installLibrary(lib) {
  state.texts = new Map(Object.entries(lib.texts));
  state.tree = lib.tree;
  state.handles = new Map(Object.entries(lib.handles));
  state.active = lib.active;
  librarySeeded = true;
  syncOrder();
  for (const [id, text] of state.texts) store.set("page:" + id, text);
  saveIndex();
}

let syncTouched = () => {};
let applyingRemote = false;
function noteSyncEdit() {
  if (!applyingRemote) syncTouched();
}

function saveSheet(id) {
  if (id === "shared" || !id) return Promise.resolve(false);
  const saved = store.set("page:" + id, state.texts.get(id) ?? "");
  const noted = noteDocPage(id);
  noteSyncEdit();
  return Promise.all([saved, noted]).then(([ok]) => ok);
}

// The library document is the source of truth. Pages stay in IndexedDB either way,
// and a failed backup leaves them there without a document.
let docBytes = null;
let docSaved = "";
let docHeld = false;
let docPromise = null;

function bytesToB64(bytes) {
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
  }
  return btoa(binary);
}

function b64ToBytes(text) {
  const binary = atob(text);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function folderSeeds() {
  const out = [];
  const walk = (list, parent) => {
    for (const node of list ?? []) {
      if (node?.type !== "folder") continue;
      const row = { id: node.id, name: node.name || "" };
      if (parent) row.parent = parent;
      out.push(row);
      walk(node.children, node.id);
    }
  };
  walk(state.tree, null);
  return out;
}

function libraryPages() {
  const pages = [];
  for (const id of state.order) {
    if (!id || id === "shared") continue;
    pages.push({ id, text: state.texts.get(id) ?? "" });
  }
  return pages;
}

function ensureLibraryDoc() {
  if (docPromise) return docPromise;
  docPromise = ensureLibraryDocOnce();
  return docPromise;
}

async function ensureLibraryDocOnce() {
  if (EMBED || docHeld || !engineReady) return null;
  const existing = store.get("library-doc");
  if (existing) {
    try {
      docBytes = b64ToBytes(existing);
      docSaved = existing;
      showDocConflicts();
      return docBytes;
    } catch {
      docHeld = true;
      docBytes = null;
      return null;
    }
  }
  const version = document.querySelector('meta[name="omacus-version"]')?.content || "";
  let index = {};
  try {
    index = JSON.parse(store.get("index") || "{}");
  } catch {
    index = {};
  }
  const pages = libraryPages();
  let zip;
  try {
    zip = zipBytes(backupEntries({
      version,
      library: index,
      settings,
      trash: loadTrash(),
      pages,
    }));
  } catch {
    docHeld = true;
    return null;
  }
  const backup = bytesToB64(zip);
  if (!store.get("sync-backup")) {
    try {
      await store.set("sync-backup", backup);
    } catch {
      docHeld = true;
      return null;
    }
    if (store.get("sync-backup") !== backup) {
      docHeld = true;
      return null;
    }
  }
  try {
    docBytes = docMigrate(JSON.stringify({ pages, folders: folderSeeds() }));
  } catch {
    docHeld = true;
    docBytes = null;
    return null;
  }
  const encoded = bytesToB64(docBytes);
  try {
    await store.set("library-doc", encoded);
  } catch {
    docHeld = true;
    docBytes = null;
    return null;
  }
  docSaved = encoded;
  return docBytes;
}

function showDocConflicts() {
  if (!docBytes || !el.pick) return;
  let rows = [];
  try {
    rows = JSON.parse(docConflicts(docBytes) || "[]");
  } catch {
    return;
  }
  if (!Array.isArray(rows) || rows.length === 0) {
    el.pick.hidden = true;
    return;
  }
  const next = rows[0];
  if (!el.pick.hidden && el.pick.dataset.page === next.page_id && el.pick.dataset.line === next.line_id) return;
  openPick(next);
}

function openPick(conflict) {
  if (!el.pick || !conflict) return;
  const sealed = conflict.line_id === "body" || isLock(conflict.ours) || isLock(conflict.theirs);
  el.pick.dataset.page = conflict.page_id;
  el.pick.dataset.line = conflict.line_id;
  el.pickTitle.textContent = sealed ? "Pick one copy" : "Pick one line";
  el.pickDetail.textContent = sealed
    ? "This locked page was changed in two places."
    : "This line was changed in two places.";
  if (el.pickLines) el.pickLines.hidden = !!sealed;
  if (!sealed) {
    el.pickOurs.textContent = conflict.ours ?? "";
    el.pickTheirs.textContent = conflict.theirs ?? "";
  }
  el.pickKeep.textContent = sealed ? "Keep this copy" : "Keep this line";
  el.pickOther.textContent = sealed ? "Use the other copy" : "Use the other line";
  el.pick.hidden = false;
}

async function noteDocPage(id) {
  if (EMBED || docHeld || !engineReady || !id || id === "shared") return;
  if (!docBytes) {
    const made = await ensureLibraryDoc();
    if (!made) return;
  }
  const text = state.texts.get(id) ?? "";
  let bytes = docBytes;
  try {
    const stored = store.get("library-doc");
    if (stored === docSaved && docSnapshot(bytes, id) === text) {
      bytes = docSetFolders(bytes, JSON.stringify(folderSeeds()));
      docBytes = bytes;
      const encoded = bytesToB64(bytes);
      docSaved = encoded;
      await store.set("library-doc", encoded);
      showDocConflicts();
      return;
    }
    if (stored && stored !== docSaved) {
      const ancestor = docSnapshot(bytes, id);
      bytes = docUnion(bytes, b64ToBytes(stored));
      const disk = docSnapshot(bytes, id);
      if (disk !== ancestor) {
        // The lines already stored are the last common copy. Diff against that,
        // then fold this editor and the other copy.
        bytes = docApply(bytes, id, ancestor);
        bytes = docMergeDisk(bytes, id, text, disk);
      } else {
        bytes = docApply(bytes, id, text);
      }
    } else {
      bytes = docApply(bytes, id, text);
    }
    bytes = docSetFolders(bytes, JSON.stringify(folderSeeds()));
  } catch {
    return;
  }
  docBytes = bytes;
  const encoded = bytesToB64(bytes);
  docSaved = encoded;
  await store.set("library-doc", encoded);
  showDocConflicts();
}

async function dropDocPage(id) {
  if (EMBED || docHeld || !docBytes || !id || id === "shared") return;
  try {
    docBytes = docRemove(docBytes, id);
  } catch {
    return;
  }
  const encoded = bytesToB64(docBytes);
  docSaved = encoded;
  await store.set("library-doc", encoded);
}

async function choosePick(theirs) {
  const page = el.pick?.dataset.page;
  const line = el.pick?.dataset.line;
  if (!docBytes || !page || !line) {
    if (el.pick) el.pick.hidden = true;
    return;
  }
  try {
    docBytes = docPick(docBytes, page, line, !!theirs);
  } catch {
    if (el.pick) el.pick.hidden = true;
    return;
  }
  const snapshot = docSnapshot(docBytes, page);
  if (isLock(snapshot) || line === "body") {
    state.texts.set(page, snapshot);
    plains.delete(page);
    pageKeys.delete(page);
  } else {
    if (plains.has(page)) plains.set(page, snapshot);
    if (!isLock(state.texts.get(page) ?? "")) state.texts.set(page, snapshot);
  }
  const encoded = bytesToB64(docBytes);
  docSaved = encoded;
  await store.set("library-doc", encoded);
  await saveSheet(page);
  if (state.active === page) showSheet();
  if (el.pick) el.pick.hidden = true;
  showDocConflicts();
}

function newId() {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

function addSheet(text, { select = true, parent } = {}) {
  let folder = parent;
  if (parent === undefined) {
    folder = state.active && state.active !== "shared" ? folderOf(state.tree, state.active) : null;
  }
  const cover = folder ? coveringLock(state.tree, folder) : null;
  if (cover && !folderKeys.has(cover.id) && !isLock(text)) {
    pendingNew = { parent: folder };
    openLock("unlock-folder", cover.id);
    toast("Unlock the folder before adding a page.");
    return null;
  }
  const id = newId();
  let stored = text;
  if (cover && folderKeys.has(cover.id) && !isLock(text)) {
    const sealed = sealWith(folderKeys.get(cover.id), cover.lock, text, id);
    if (!sealed) {
      toast("That lock could not be read.");
      return null;
    }
    stored = sealed;
    plains.set(id, text);
  }
  state.texts.set(id, stored);
  if (!insertNode(state.tree, folder, 0, { type: "page", id })) state.tree.unshift({ type: "page", id });
  syncOrder();
  const taken = new Set([...state.handles.values()].map((h) => h.toLowerCase()));
  state.handles.set(id, allocateHandle(heading(text), taken));
  saveSheet(id);
  if (select) {
    state.active = id;
    expandTo(state.tree, id);
  }
  saveIndex();
  return id;
}

// Existing pages keep the handle already stored. A title change does not allocate again.
function ensureHandles() {
  const taken = new Set([...state.handles.values()].map((h) => h.toLowerCase()));
  let dirty = false;
  const assign = (id, title) => {
    if (state.handles.has(id)) return;
    const handle = allocateHandle(title, taken);
    state.handles.set(id, handle);
    taken.add(handle.toLowerCase());
    dirty = true;
  };
  for (const id of state.order) assign(id, heading(state.texts.get(id) || ""));
  for (const item of loadTrash()) {
    if (item.handle && !state.handles.has(item.id)) {
      const handle = String(item.handle).toLowerCase();
      if (/^[a-z][a-z0-9]*$/.test(handle) && !taken.has(handle)) {
        state.handles.set(item.id, handle);
        taken.add(handle);
        dirty = true;
      }
    }
    assign(item.id, heading(item.text || ""));
  }
  if (dirty) saveIndex();
}

function currentHandle() {
  return state.handles.get(state.active) || "";
}

// Every page a `@handle.name` can read. A locked page contributes no text.
function pageCatalog() {
  const pages = [];
  const doc = typeof view !== "undefined" && view ? view.state.doc.toString() : null;
  for (const id of state.order) {
    const concealed = pageConcealed(id);
    const text = concealed ? "" : (id === state.active && doc != null ? doc : shownPlain(id));
    pages.push({
      id,
      handle: state.handles.get(id) || "",
      title: concealed ? LOCKED_PAGE : heading(text),
      text,
      locked: concealed,
      deleted: false,
      elsewhere: false,
    });
  }
  for (const item of loadTrash()) {
    if (state.order.includes(item.id)) continue;
    const locked = isLock(item.text);
    const text = locked ? "" : item.text;
    pages.push({
      id: item.id,
      handle: item.handle || state.handles.get(item.id) || "",
      title: locked ? LOCKED_PAGE : heading(text),
      text,
      locked,
      deleted: true,
      elsewhere: false,
    });
  }
  return pages;
}

function pageIndex(id) {
  return pageCatalog().findIndex((page) => page.id === id);
}

let crossUndo = null;

function writePageEdit(edit) {
  if (state.texts.has(edit.id)) {
    state.texts.set(edit.id, edit.text);
    state.handles.set(edit.id, edit.handle);
    saveSheet(edit.id);
    if (edit.id !== state.active) editorStates.delete(edit.id);
    return;
  }
  const items = loadTrash();
  const item = items.find((t) => t.id === edit.id);
  if (item) {
    item.text = edit.text;
    item.handle = edit.handle;
    saveTrash(items);
  }
  if (edit.handle) state.handles.set(edit.id, edit.handle);
}

function snapshotOthers(edits) {
  const others = [];
  for (const edit of edits) {
    if (edit.id === state.active) continue;
    if (state.texts.has(edit.id)) {
      others.push({ id: edit.id, text: state.texts.get(edit.id), handle: state.handles.get(edit.id) || "" });
    } else {
      const item = loadTrash().find((t) => t.id === edit.id);
      if (item) others.push({ id: edit.id, text: item.text, handle: item.handle || state.handles.get(edit.id) || "" });
    }
  }
  return others;
}

// One undo restores every page a rename rewrote. When the page on screen
// changed, the editor's own undo is that step; otherwise the toast is.
function applyEdits(edits, blurb, cursor) {
  if (!edits?.length) return;
  const id = state.active;
  const beforeText = view.state.doc.toString();
  const beforeHandle = state.handles.get(id) || "";
  const others = snapshotOthers(edits);
  const mine = edits.find((edit) => edit.id === id);
  const afterText = mine ? mine.text : beforeText;
  crossUndo = { id, beforeText, afterText, beforeHandle, others, armed: true };
  for (const edit of edits) {
    if (edit.id === id) {
      if (edit.handle) state.handles.set(id, edit.handle);
    } else {
      writePageEdit(edit);
    }
  }
  saveIndex();
  if (afterText !== beforeText) {
    applyingRename = true;
    const change = { changes: { from: 0, to: view.state.doc.length, insert: afterText } };
    if (cursor != null) change.selection = { anchor: cursor };
    view.dispatch(change);
    applyingRename = false;
  } else {
    toast(blurb || "Updated", { label: "Undo", run: restoreCrossUndo });
    evaluate();
  }
}

function restoreCrossUndo() {
  const undo = crossUndo;
  crossUndo = null;
  if (!undo) return;
  if (state.active === undo.id && undo.beforeText !== undo.afterText && view.state.doc.toString() === undo.afterText) {
    applyingRename = true;
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: undo.beforeText } });
    applyingRename = false;
  }
  if ((state.handles.get(undo.id) || "") !== undo.beforeHandle) state.handles.set(undo.id, undo.beforeHandle);
  for (const other of undo.others) writePageEdit(other);
  saveIndex();
  renderSidebar();
  evaluate();
}

function noteCrossUndo(update) {
  if (!crossUndo?.armed || crossUndo.id !== state.active) return;
  const undone = update.transactions.some((tr) => tr.isUserEvent("undo"));
  if (!undone || update.state.doc.toString() !== crossUndo.beforeText) return;
  const undo = crossUndo;
  crossUndo = null;
  if ((state.handles.get(undo.id) || "") !== undo.beforeHandle) state.handles.set(undo.id, undo.beforeHandle);
  for (const other of undo.others) writePageEdit(other);
  saveIndex();
  renderSidebar();
}

function openHandle(handle) {
  const want = String(handle || "").toLowerCase();
  if (!want) return;
  for (const id of state.order) {
    if ((state.handles.get(id) || "").toLowerCase() === want) {
      if (id !== state.active) select(id);
      return;
    }
  }
  const trashed = loadTrash().find((item) => (item.handle || state.handles.get(item.id) || "").toLowerCase() === want);
  if (trashed) showPreview(trashed);
}

function rewriteSpans(text, spans, name) {
  const ordered = [...(spans || [])].sort((a, b) => b[0] - a[0]);
  let out = text;
  for (const [from, to] of ordered) out = out.slice(0, from) + name + out.slice(to);
  return out;
}

function withForeign(edit) {
  if (!edit || edit.collision) return edit;
  const handle = currentHandle();
  if (!handle) return edit;
  let elsewhere = 0;
  let locked = 0;
  let away = 0;
  for (const page of pageCatalog()) {
    if (page.handle.toLowerCase() === handle.toLowerCase()) continue;
    const again = JSON.parse(rewriteVarRefs(page.text, handle, edit.oldName, edit.newName));
    if (!again.count) continue;
    if (page.locked) locked += 1;
    else if (page.elsewhere) away += 1;
    else elsewhere += again.count;
  }
  if (!elsewhere && !locked && !away) return edit;
  edit.message = renameBlurb(edit.oldName, edit.newName, edit.references, elsewhere, locked, away);
  edit.references += elsewhere;
  return edit;
}

function applyDefinition(view, edit) {
  const handle = currentHandle();
  let text = rewriteSpans(view.state.doc.toString(), edit.spans, edit.newName);
  if (handle) text = JSON.parse(rewriteVarRefs(text, handle, edit.oldName, edit.newName)).text;
  const edits = [];
  if (text !== view.state.doc.toString()) edits.push({ id: state.active, text, handle });
  if (!edit.collision && handle) {
    for (const page of pageCatalog()) {
      if (page.id === state.active || page.locked || page.elsewhere) continue;
      const again = JSON.parse(rewriteVarRefs(page.text, handle, edit.oldName, edit.newName));
      if (again.count) edits.push({ id: page.id, text: again.text, handle: page.handle });
    }
  }
  applyEdits(edits, edit.message);
}

function heading(text) {
  const line = (text ?? "").split("\n").map((l) => l.trim()).find((l) => l) ?? "";
  const t = line.replace(/^[#/\s]+/, "").trim();
  return t || "Untitled";
}

function title(text) {
  const t = heading(text);
  return t.length > 32 ? t.slice(0, 32) + "…" : t;
}

// Other tabs: pick up their changes, unless this tab is mid-edit on the same sheet.
function onStoreWrite(key, value) {
  if (EMBED || typeof key !== "string") return;
  storeAdopt(key, value);
  const typing = view.hasFocus && pendingSave;
  const before = currentText();
  const mine = state.active;
  // A save from another tab reloads the library. The page open in this tab
  // stays as typed, and the next save folds the two copies together.
  const draft = typing && mine && mine !== "shared" ? state.texts.get(mine) : null;
  load();
  if (draft != null) state.texts.set(mine, draft);
  reconcileUnlocks();
  // Each tab keeps its own open sheet.
  if (mine === "shared" || state.order.includes(mine)) state.active = mine;
  renderSidebar();
  if (!typing && currentText() !== before) showSheet();
}
addEventListener("storage", (e) => {
  if (!e.key?.startsWith(PREFIX) || e.key === PREFIX + "migrated") return;
  onStoreWrite(e.key.slice(PREFIX.length), e.newValue);
});
try {
  const pagesChannel = new BroadcastChannel(CHANNEL);
  pagesChannel.onmessage = (e) => {
    if (!e.data || typeof e.data.key !== "string" || e.data.tab === selfTab) return;
    onStoreWrite(e.data.key, e.data.value);
  };
} catch {
  // This tab still has its own copy. Another tab will not push updates here.
}

// ---------------------------------------------------------------------------------
// The sheet on screen.

function currentText() {
  return state.active === "shared" ? state.shared?.text ?? "" : state.texts.get(state.active) ?? "";
}

let pendingSave = null;
let editedAt = 0;
let hintTimer = null;

function armHintTimer() {
  clearTimeout(hintTimer);
  hintTimer = setTimeout(() => {
    if (!view) return;
    const answers = view.state.field(sheetField).answers;
    view.dispatch({ effects: setResults.of(answers.slice()) });
  }, HINT_IDLE);
}

function hintReady(lineNo) {
  if (typeof view === "undefined" || !view) return true;
  const head = view.state.selection.main.head;
  if (view.state.doc.lineAt(head).number !== lineNo) return true;
  return performance.now() - editedAt >= HINT_IDLE;
}

function syncEmpty() {
  if (!el.empty || !view) return;
  el.empty.hidden = view.state.doc.toString().trim() !== "";
}

function editingText() {
  if (state.active === "shared") return state.shared?.text ?? "";
  if (pageConcealed(state.active)) return "";
  return shownPlain(state.active);
}

function reconcileUnlocks() {
  for (const id of [...plains.keys()]) {
    if (!state.order.includes(id)) {
      forgetPage(id);
      continue;
    }
    const stored = state.texts.get(id) ?? "";
    if (!isLock(stored) && !coveringLock(state.tree, id)) {
      plains.delete(id);
      pageKeys.delete(id);
      continue;
    }
    const key = keyFor(id);
    if (!key || (isLock(stored) && !parseLockResult(lockOpen(key, stored, id)).ok)) forgetPage(id);
  }
}

function onInput() {
  editedAt = performance.now();
  armHintTimer();
  noteActivity();
  const text = view.state.doc.toString();
  if (state.active === "shared") {
    state.shared.text = text;
  } else if (keyFor(state.active)) {
    plains.set(state.active, text);
    const sealed = reseal(state.active, text);
    if (sealed) state.texts.set(state.active, sealed);
    clearTimeout(pendingSave);
    pendingSave = setTimeout(() => {
      pendingSave = null;
      saveSheet(state.active);
    }, 300);
  } else {
    state.texts.set(state.active, text);
    clearTimeout(pendingSave);
    pendingSave = setTimeout(() => {
      pendingSave = null;
      saveSheet(state.active);
    }, 300);
  }
  evaluate();
  renderSidebar();
  syncEmpty();
}

// Each sheet keeps its own editor state while the page is open, so undo stays per sheet.
const editorStates = new Map();
let shownId = null;

function showSheet() {
  switchingPage = true;
  closeRename();
  closeFolderRename();
  closeRefPrompt();
  closeRenameMenu();
  refBase = null;
  refDismissed = false;
  closePagePreview();
  clearPreview();
  if (shownId !== null) editorStates.set(shownId, view.state);
  shownId = state.active;
  results = [];
  resultsText = "";
  resultsExact = false;
  if (state.active && state.active !== "shared" && pageConcealed(state.active)) {
    editorStates.delete(state.active);
    switchingPage = false;
    showGate();
    if (el.shared) el.shared.hidden = true;
    renderSidebar();
    syncEmpty();
    return;
  }
  hideGate();
  if (el.editor && !preview && !pagePreviewOpen) el.editor.hidden = false;
  const text = state.active === "shared" ? (state.shared?.text ?? "") : shownPlain(state.active);
  let es = editorStates.get(state.active);
  if (!es || es.doc.toString() !== text) es = EditorState.create({ doc: text, extensions: editorExtensions });
  view.setState(es);
  switchingPage = false;
  syncRenameButton();
  el.shared.hidden = EMBED || state.active !== "shared";
  showInstall();
  evaluate();
  renderSidebar();
  syncEmpty();
}

function select(id) {
  clearPreview();
  if (id !== "shared" && expandTo(state.tree, id)) saveIndex();
  if (id !== state.active) {
    flushSave();
    state.active = id;
    if (id !== "shared") saveIndex();
  }
  showSheet();
  if (!pageConcealed(id)) view.focus();
  setDrawer(false);
}

function flushSave() {
  if (pendingSave) {
    clearTimeout(pendingSave);
    pendingSave = null;
    return saveSheet(state.active);
  }
  return Promise.resolve(false);
}
addEventListener("pagehide", () => {
  flushSave();
  plains.clear();
  pageKeys.clear();
  folderKeys.clear();
});


function renderSidebar() {
  const rows = [];
  if (state.shared) rows.push(pageRow("shared", state.shared.text, true, 0));
  for (const entry of visible(state.tree)) {
    rows.push(entry.type === "folder" ? folderRow(entry) : pageRow(entry.id, shownPlain(entry.id), false, entry.depth));
  }
  const fill = document.createElement("div");
  fill.className = "side-fill";
  el.sheets.replaceChildren(...rows, fill);
  paintDrop();
  if (!preview) {
    const name = state.active === "shared" ? title(state.shared?.text ?? "") : pageConcealed(state.active) ? LOCKED_PAGE : title(shownPlain(state.active));
    el.title.textContent = name;
    document.title = `${name} · Omacus`;
  }
  renderDeleted();
}

function sideRow(kind, id, depth) {
  const row = document.createElement("div");
  row.className = "side-row";
  row.dataset.kind = kind;
  row.dataset.id = id;
  row.style.setProperty("--depth", String(depth));
  return row;
}

function svgEl(name, attrs) {
  const el = document.createElementNS("http://www.w3.org/2000/svg", name);
  for (const [key, value] of Object.entries(attrs)) el.setAttribute(key, value);
  return el;
}

function lockIcon(open) {
  const svg = svgEl("svg", {
    class: open ? "lock-ico open" : "lock-ico",
    viewBox: "0 0 24 24",
    "aria-hidden": "true",
  });
  svg.append(
    svgEl("rect", { x: "6", y: "11", width: "12", height: "9", rx: "1.5" }),
    svgEl("path", { d: open ? "M8 11V8a4 4 0 0 1 8 0" : "M8 11V8a4 4 0 0 1 8 0v3" }),
  );
  return svg;
}

function chevronIcon() {
  const svg = svgEl("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" });
  svg.append(svgEl("path", { d: "M9 6l6 6-6 6" }));
  return svg;
}

function dotsIcon() {
  const svg = svgEl("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" });
  svg.append(
    svgEl("circle", { cx: "6", cy: "12", r: "1.5" }),
    svgEl("circle", { cx: "12", cy: "12", r: "1.5" }),
    svgEl("circle", { cx: "18", cy: "12", r: "1.5" }),
  );
  return svg;
}

function shareIcon() {
  const svg = svgEl("svg", { class: "ios-share", viewBox: "0 0 24 24", "aria-label": "Share" });
  svg.append(
    svgEl("path", { d: "M12 15V3M8 7l4-4 4 4" }),
    svgEl("path", { d: "M7 10H5v11h14V10h-2" }),
  );
  return svg;
}

function moreButton(label) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "row-more";
  button.setAttribute("aria-label", label);
  button.setAttribute("aria-haspopup", "menu");
  button.append(dotsIcon());
  return button;
}

function pageRow(id, text, shared, depth) {
  const row = sideRow(shared ? "shared" : "page", id, depth);
  const name = document.createElement("button");
  name.type = "button";
  name.className = "side-name";
  const labelText = shared ? heading(text) : pageLabel(id);
  const label = document.createElement("span");
  label.textContent = labelText;
  name.title = labelText;
  name.append(label);
  if (!shared && (isLock(state.texts.get(id) ?? "") || coveringLock(state.tree, id))) {
    const icon = document.createElement("span");
    icon.className = "lock-mark";
    icon.append(lockIcon(!pageConcealed(id)));
    name.append(icon);
  }
  if (shared) {
    const tag = document.createElement("i");
    tag.className = "shared-tag";
    tag.textContent = "shared";
    name.append(tag);
  }
  row.append(name);
  if (!shared) row.append(moreButton("Page menu"));
  if (id === state.active && !preview) row.setAttribute("aria-current", "true");
  return row;
}

function folderRow(entry) {
  const row = sideRow("folder", entry.id, entry.depth);
  if (entry.collapsed) row.dataset.collapsed = "1";
  const chevron = document.createElement("button");
  chevron.type = "button";
  chevron.className = "chevron";
  chevron.setAttribute("aria-expanded", String(!entry.collapsed));
  chevron.setAttribute("aria-label", entry.collapsed ? "Expand folder" : "Collapse folder");
  chevron.append(chevronIcon());
  const name = document.createElement("button");
  name.type = "button";
  name.className = "side-name";
  const label = document.createElement("span");
  label.textContent = entry.name || "Folder";
  name.title = entry.name || "Folder";
  name.append(label);
  if (entry.locked) {
    const icon = document.createElement("span");
    icon.className = "lock-mark";
    icon.append(lockIcon(folderKeys.has(entry.id)));
    name.append(icon);
  }
  row.append(chevron, name, moreButton("Folder menu"));
  return row;
}

const TRASH_KEY = "trash";
const TRASH_CAP = 30;

function loadTrash() {
  try {
    const raw = JSON.parse(store.get(TRASH_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((t) => t && t.id && typeof t.text === "string") : [];
  } catch {
    return [];
  }
}

function saveTrash(items) {
  store.set(TRASH_KEY, JSON.stringify(items.slice(0, TRASH_CAP)));
}

function renderDeleted() {
  if (!el.deleted) return;
  const items = loadTrash();
  el.deleted.hidden = items.length === 0;
  el.deleted.replaceChildren();
  if (!items.length) return;
  const label = document.createElement("p");
  label.className = "deleted-label";
  label.textContent = "Recently deleted";
  el.deleted.append(label);
  for (const item of items) {
    const b = document.createElement("button");
    b.type = "button";
    const name = isLock(item.text) ? LOCKED_PAGE : heading(item.text);
    b.textContent = name;
    b.title = name;
    if (preview?.id === item.id) b.setAttribute("aria-current", "true");
    b.addEventListener("click", () => showPreview(item));
    el.deleted.append(b);
  }
}

let preview = null;

function showPreview(item) {
  closePagePreview();
  preview = item;
  const locked = isLock(item.text);
  const name = locked ? LOCKED_PAGE : heading(item.text);
  if (el.preview) el.preview.hidden = false;
  if (el.previewTitle) el.previewTitle.textContent = name;
  if (el.previewBody) {
    el.previewBody.hidden = false;
    el.previewBody.textContent = locked
      ? "This page is locked. Restore it and enter the password to read it."
      : item.text;
  }
  if (el.editor) el.editor.hidden = true;
  hideGate();
  if (el.title) el.title.textContent = name;
  document.title = `${locked ? LOCKED_PAGE : title(item.text)} · Omacus`;
  renderSidebar();
}

function clearPreview() {
  if (!preview) return;
  preview = null;
  if (el.preview) el.preview.hidden = true;
  if (el.previewBody) el.previewBody.hidden = true;
  if (pageConcealed(state.active)) showGate();
  else if (el.editor && !pagePreviewOpen) el.editor.hidden = false;
  renderSidebar();
  syncEmpty();
}

// Ctrl+P: the page rendered as notes, with each answer in brackets. This is what
// prints. Esc, or Edit, returns to the plain-text page.
let pagePreviewOpen = false;

function openPagePreview() {
  if (preview) clearPreview();
  pagePreviewOpen = true;
  if (el.pagePreview) el.pagePreview.hidden = false;
  if (el.editor) el.editor.hidden = true;
  refreshPagePreview();
}

function closePagePreview() {
  if (!pagePreviewOpen) return;
  pagePreviewOpen = false;
  if (el.pagePreview) el.pagePreview.hidden = true;
  if (el.editor && !preview) el.editor.hidden = false;
}

function togglePagePreview() {
  if (refuseConcealed()) return;
  if (pagePreviewOpen) {
    closePagePreview();
    view.focus();
  } else {
    openPagePreview();
  }
}

function safeLink(href) {
  return href.startsWith("https://") || href.startsWith("http://") || href.startsWith("mailto:");
}

function appendInlines(parent, inlines) {
  for (const inline of inlines || []) {
    if (inline.t === "strong" || inline.t === "em" || inline.t === "code") {
      const tag = inline.t === "strong" ? "strong" : inline.t === "em" ? "em" : "code";
      const node = document.createElement(tag);
      node.textContent = inline.s || "";
      parent.append(node);
    } else if (inline.t === "link" && safeLink(inline.href || "")) {
      const link = document.createElement("a");
      link.href = inline.href;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      link.textContent = inline.s || "";
      parent.append(link);
    } else {
      parent.append(document.createTextNode(inline.s || ""));
    }
  }
}

function appendRich(parent, rich, prefix) {
  const expr = document.createElement("span");
  expr.className = "expr";
  if (prefix) expr.append(prefix);
  const bodyEmpty = !(rich?.inlines || []).some((inline) => inline.t !== "text" || inline.s);
  if (!bodyEmpty) appendInlines(expr, rich.inlines);
  if (rich?.comment != null) {
    const note = document.createElement("span");
    note.className = "note";
    if (rich.comment) note.textContent = rich.comment;
    else note.append(document.createElement("br"));
    expr.append(note);
  }
  parent.append(expr);
  if (rich?.answer) {
    const pill = document.createElement("button");
    pill.type = "button";
    pill.className = "pill";
    pill.dataset.value = rich.answer;
    pill.textContent = `[ ${rich.answer} ]`;
    pill.title = "Click to copy";
    pill.addEventListener("click", () => {
      copy(pill.dataset.value || pill.textContent).then(() => toast(copiedMessage(pill.dataset.value)));
    });
    parent.append(pill);
  }
}

function renderPreview(host, blocks) {
  host.replaceChildren();
  const article = document.createElement("article");
  article.className = "md";
  for (const block of blocks || []) {
    if (block.t === "heading") {
      const level = Math.min(6, Math.max(1, Number(block.level) || 1));
      const heading = document.createElement(`h${level}`);
      appendRich(heading, block.rich);
      article.append(heading);
    } else if (block.t === "lines") {
      for (const rich of block.rows || []) {
        const p = document.createElement("p");
        p.className = "line";
        appendRich(p, rich);
        article.append(p);
      }
    } else if (block.t === "blank") {
      const p = document.createElement("p");
      p.className = "line blank";
      p.append(document.createElement("br"));
      article.append(p);
    } else if (block.t === "list") {
      const list = document.createElement(block.ordered ? "ol" : "ul");
      (block.items || []).forEach((item, i) => {
        const li = document.createElement("li");
        if (item.number) li.value = item.number;
        if (item.checked != null) li.className = "task";
        const mark = document.createElement("span");
        if (item.checked == null) {
          mark.className = "mark";
          mark.textContent = block.ordered ? `${item.number || i + 1}. ` : "• ";
        }
        if (item.checked != null) {
          const box = document.createElement("input");
          box.type = "checkbox";
          box.disabled = true;
          box.checked = !!item.checked;
          mark.append(box);
        }
        appendRich(li, item.rich, mark.childNodes.length || mark.textContent ? mark : null);
        list.append(li);
      });
      article.append(list);
    } else if (block.t === "code") {
      const pre = document.createElement("pre");
      pre.className = "code";
      const code = document.createElement("code");
      code.textContent = (block.lines || []).join("\n");
      pre.append(code);
      article.append(pre);
    } else if (block.t === "quote") {
      const quote = document.createElement("blockquote");
      for (const rich of block.rows || []) {
        const p = document.createElement("p");
        p.className = "line";
        appendRich(p, rich);
        quote.append(p);
      }
      article.append(quote);
    } else if (block.t === "table") {
      const table = document.createElement("table");
      (block.rows || []).forEach((row, n) => {
        const tr = document.createElement("tr");
        const cell = block.header && n === 0 ? "th" : "td";
        for (const inlines of row) {
          const td = document.createElement(cell);
          appendInlines(td, inlines);
          tr.append(td);
        }
        table.append(tr);
      });
      article.append(table);
    }
  }
  host.append(article);
}

function refreshPagePreview() {
  if (!pagePreviewOpen || !el.pagePreviewBody || !engineReady) return;
  const text = view.state.doc.toString();
  const answers = view.state.field(sheetField).answers;
  let blocks = [];
  try {
    blocks = JSON.parse(previewBlocks(text, JSON.stringify(answers)));
  } catch {
    blocks = [];
  }
  renderPreview(el.pagePreviewBody, blocks);
}

function printPagePreview() {
  if (refuseConcealed()) return;
  if (!pagePreviewOpen) openPagePreview();
  // The browser picks the folder and whether to replace a file. The title is
  // the page name, so Save as PDF suggests that instead of "Omacus".
  const previous = document.title;
  const name = heading(currentText());
  document.title = name;
  const restore = () => {
    document.title = previous;
    removeEventListener("afterprint", restore);
  };
  addEventListener("afterprint", restore);
  window.print();
}

function restoreTrashed(id) {
  const items = loadTrash();
  const item = items.find((t) => t.id === id);
  if (!item) return;
  clearPreview();
  saveTrash(items.filter((t) => t.id !== id));
  dropPlaceholder();
  state.texts.set(id, item.text);
  if (!pageIds(state.tree).includes(id)) state.tree.unshift({ type: "page", id });
  syncOrder();
  saveSheet(id);
  state.active = id;
  saveIndex();
  showSheet();
  toast(`Restored “${title(item.text)}”.`);
}

// ---------------------------------------------------------------------------------
// The editor: CodeMirror (editor.js, bundled by site/tools/editor). Lines wrap at the
// full width. Each answer floats at the right edge of its line, unless Settings hides
// that column: then the answer sits inline after the line. A long line still drops
// the answer onto the row below.

const setResults = StateEffect.define();

class Answer extends WidgetType {
  constructor(r) {
    super();
    this.r = r;
  }
  eq(other) {
    return other.r.display === this.r.display && other.r.color === this.r.color && !other.r.stale === !this.r.stale;
  }
  toDOM() {
    const span = document.createElement("span");
    // Stale: from before an edit above it, until its new answer arrives.
    span.className = this.r.stale ? "res stale" : "res";
    span.title = "Click to copy";
    const swatch = swatchOr(this.r.color);
    if (swatch) span.append(swatch);
    span.append(answerLabel(this.r.display));
    span.addEventListener("mousedown", (e) => e.preventDefault());
    span.addEventListener("click", () => copyAnswer(span, this.r.display));
    return span;
  }
  ignoreEvent() {
    return true;
  }
}

// Why a line has no answer ("Can't divide by zero"), shown quietly in its place.
class Note extends WidgetType {
  constructor(text, hint) {
    super();
    this.text = text;
    this.hint = !!hint;
  }
  eq(other) {
    return other.text === this.text && other.hint === this.hint;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = this.hint ? "res note hint" : "res note";
    if (this.hint) span.title = this.text;
    else span.textContent = this.text;
    return span;
  }
}

function copyAnswer(span, display) {
  copy(display).then(() => {
    span.classList.add("copied");
    toast(`Copied ${display}`);
    setTimeout(() => span.classList.remove("copied"), 900);
  });
}

// Colours and answers for the whole sheet. Sheets are short, so it's redone per change.
function decorate(doc, answers) {
  const out = new RangeSetBuilder();
  const vars = definedVars(doc.toString().split("\n"));
  for (let n = 1; n <= doc.lines; n++) {
    const line = doc.line(n);
    const r = answers[n - 1];
    const spans = lineSpans(line.text, vars);
    for (const [from, to, cls] of spans) {
      if (to > from) out.add(line.from + from, line.from + to, Decoration.mark({ class: cls }));
    }
    if (r?.display) out.add(line.to, line.to, Decoration.widget({ widget: new Answer(r), side: 1 }));
    else if (r?.note && !r.stale && (!r.hint || hintReady(n))) {
      out.add(line.to, line.to, Decoration.widget({ widget: new Note(r.note, r.hint), side: 1 }));
    } else if (r?.hint && r.note && !r.stale) armHintTimer();
  }
  return out.finish();
}

// Answers for one text moved to where their lines are in another, while the new
// answers are on their way (the engine's `realign`): lines unchanged from the top and
// from the bottom keep theirs, those below the edit marked stale, and the lines that
// changed have none yet. `fresh` answers (worked out for `oldLines`, maybe only the
// first lines' so far) are used where there are some, and `fallback` (already lined
// up with `newLines`) where not, stale.
function realign(oldText, answers, newText, fallback = []) {
  const map = realignMap(oldText, newText);
  return [...map].map((j, i) => {
    const from = j >= 0 ? j : j <= -2 ? -2 - j : -1;
    const r = from >= 0 ? answers[from] : undefined;
    if (r) return j <= -2 && !r.stale ? { ...r, stale: true } : r;
    const old = fallback[i];
    return old && !old.stale ? { ...old, stale: true } : old;
  });
}

const sheetField = StateField.define({
  create: (st) => ({ answers: [], deco: decorate(st.doc, []) }),
  update(value, tr) {
    let answers = value.answers;
    const fresh = tr.effects.some((e) => e.is(setResults));
    for (const e of tr.effects) if (e.is(setResults)) answers = e.value;
    if (!fresh && tr.docChanged && answers.length) {
      answers = realign(tr.startState.doc.toString(), answers, tr.state.doc.toString());
    }
    if (answers === value.answers && !tr.docChanged) return value;
    return { answers, deco: decorate(tr.state.doc, answers) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.deco),
});

// Autocomplete: the engine's suggestions for the word being typed.
function completions(ctx) {
  if (!calc) return null;
  const doc = ctx.state.doc.toString();
  const found = JSON.parse(calc.suggestAt(doc, ctx.pos, JSON.stringify(pageCatalog())));
  if (!found) return null;
  // Enter accepts the page picker and the variables that follow `@handle.`.
  const atPick = /(?:^|[^A-Za-z0-9_])@[A-Za-z0-9]*\.?[A-Za-z0-9_]*$/.test(doc.slice(0, ctx.pos));
  if (atPick) explicitCompletion = true;
  return {
    from: found.start,
    filter: false,
    options: found.items.map((item) => ({
      label: item.text,
      detail: item.kind === "page" || (atPick && item.kind === "variable")
        ? item.detail
        : `${item.kind === "zone" ? "time zone" : item.kind} · ${item.detail}`,
      type: item.kind,
      apply(view, completion, from, to) {
        const insert = completion.label;
        view.dispatch({
          changes: { from, to, insert },
          selection: { anchor: from + insert.length },
          userEvent: "input",
        });
        if (completion.type === "page") {
          explicitCompletion = true;
          requestAnimationFrame(() => startCompletion(view));
        }
      },
    })),
  };
}

// Enter accepts only after Ctrl+Space. Typing still opens the list, and Tab
// accepts it, but Enter stays a newline so "3 km in" cannot become "inch".
let explicitCompletion = false;
let armingExplicit = false;

function hoverBlocked() {
  return !!renamePop || !!renameMenu || !!refPrompt || !!(el.lock && !el.lock.hidden);
}

function hoverSource(view, pos) {
  if (!calc || hoverBlocked() || completionOpen(view.state)) return null;
  const doc = view.state.doc.toString();
  const found = JSON.parse(calc.hoverValueWith(doc, pos, currentHandle(), JSON.stringify(pageCatalog())));
  if (!found) return null;
  return {
    pos: found.start,
    end: found.end,
    above: true,
    create() {
      const dom = document.createElement("div");
      dom.className = "hover-card";
      const value = document.createElement("div");
      value.className = "hover-value";
      value.textContent = found.value;
      dom.append(value);
      if (found.sourceTitle || found.sourceHandle) {
        const src = document.createElement("div");
        src.className = "hover-source";
        const name = found.sourceTitle || found.sourceHandle;
        src.textContent = `from ${name}`;
        dom.append(src);
      }
      if (found.usedBy) {
        const used = document.createElement("div");
        used.className = "hover-source";
        used.textContent = found.usedBy === 1 ? "Used by 1 other page" : `Used by ${found.usedBy} other pages`;
        dom.append(used);
      }
      if (found.sourceHandle) {
        dom.addEventListener("mousedown", (event) => {
          event.preventDefault();
          openHandle(found.sourceHandle);
        });
      }
      return {
        dom,
        resize: false,
        positioned() {
          placeHoverCard(view, dom, found.start);
        },
      };
    },
  };
}

// One card, above the token. Below only when the window has no room above it,
// so a second line cannot spill onto the next row of the page.
function placeHoverCard(view, dom, at) {
  const host = dom.closest(".cm-tooltip");
  if (!host) return;
  const token = view.coordsAtPos(at);
  if (!token) return;
  const rect = host.getBoundingClientRect();
  if (rect.width < 1 || rect.height < 1) return;
  const gap = 6;
  const margin = 8;
  let top = token.top - rect.height - gap;
  if (top < margin) top = token.bottom + gap;
  let left = token.left;
  if (left + rect.width > window.innerWidth - margin) {
    left = Math.max(margin, window.innerWidth - margin - rect.width);
  }
  if (left < margin) left = margin;
  const fixed = getComputedStyle(host).position === "fixed";
  const origin = fixed
    ? { top: 0, left: 0 }
    : (host.offsetParent || host.parentElement)?.getBoundingClientRect() || { top: 0, left: 0 };
  const nextTop = top - origin.top;
  const nextLeft = left - origin.left;
  const curTop = parseFloat(host.style.top);
  const curLeft = parseFloat(host.style.left);
  if (!Number.isFinite(curTop) || Math.abs(curTop - nextTop) > 1) host.style.top = `${nextTop}px`;
  if (!Number.isFinite(curLeft) || Math.abs(curLeft - nextLeft) > 1) host.style.left = `${nextLeft}px`;
}

// F2 or the right-click menu: one field, Enter applies. Not a hover popup.
let renamePop = null;
let refPrompt = null;
let applyingRename = false;
// The page before a definition rename started. Typing the new name one letter
// at a time still compares against this, so the uses of the old name stay found.
let refBase = null;
let refDismissed = false;

function closeRename() {
  renamePop?.remove();
  renamePop = null;
}

function openRename(view, utf16) {
  if (!calc) return false;
  const doc = view.state.doc.toString();
  const found = JSON.parse(bindingAt(doc, utf16));
  if (!found) return false;
  closeRename();
  closeRefPrompt();
  refBase = null;
  refDismissed = false;
  const host = document.createElement("div");
  host.className = "rename-pop";
  const input = document.createElement("input");
  input.type = "text";
  input.spellcheck = false;
  input.autocapitalize = "off";
  input.autocorrect = "off";
  input.autocomplete = "off";
  input.enterKeyHint = "done";
  input.setAttribute("aria-label", "New name");
  input.value = found.name;
  const err = document.createElement("div");
  err.className = "rename-error";
  host.append(input, err);
  document.querySelector("[data-editor]").append(host);
  const place = () => placePop(view, host, found.start);
  place();
  input.focus();
  input.select();
  input.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      if (input.value.trim() === found.name) {
        closeRename();
        view.focus();
        return;
      }
      const pages = JSON.stringify(pageCatalog());
      const source = pageIndex(state.active);
      const result = JSON.parse(propagateRename(pages, source, utf16, input.value));
      if (result.error) {
        if (result.error === "none") {
          closeRename();
          view.focus();
          return;
        }
        err.textContent = result.message;
        input.focus();
        return;
      }
      applyEdits(result.edits, result.blurb, result.cursor);
      closeRename();
      view.focus();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeRename();
      view.focus();
    }
  });
  const note = document.createElement("div");
  note.className = "rename-note";
  host.append(note);
  const showBlurb = () => {
    err.textContent = "";
    note.textContent = "";
    if (!calc || input.value.trim() === found.name) return;
    try {
      const result = JSON.parse(propagateRename(JSON.stringify(pageCatalog()), pageIndex(state.active), utf16, input.value));
      if (result.error) err.textContent = result.message;
      else if (result.blurb) note.textContent = result.blurb;
    } catch {}
  };
  input.addEventListener("input", showBlurb);
  renamePop = host;
  view.dispatch({});
  return true;
}

function closeRefPrompt() {
  refPrompt?.remove();
  refPrompt = null;
}

function placePop(view, host, at) {
  const coords = view.coordsAtPos(at);
  const box = document.querySelector("[data-editor]").getBoundingClientRect();
  if (!coords) return;
  host.style.top = `${coords.bottom - box.top + 4}px`;
  const width = host.offsetWidth || 160;
  const maxLeft = Math.max(8, box.width - width - 8);
  host.style.left = `${Math.min(Math.max(8, coords.left - box.left), maxLeft)}px`;
}

function placeRefPrompt(view, host) {
  placePop(view, host, view.state.selection.main.head);
}

function showRefPrompt(view, edit) {
  const mode = edit.collision || !edit.references ? "msg" : "apply";
  if (refPrompt && refPrompt.dataset.mode === mode) {
    const label = refPrompt.querySelector(".ref-update, .ref-msg");
    if (label) label.textContent = edit.message;
    refPrompt._edit = edit;
    placeRefPrompt(view, refPrompt);
    return;
  }
  closeRefPrompt();
  const host = document.createElement("div");
  host.className = "ref-prompt";
  host.dataset.mode = mode;
  host._edit = edit;
  if (mode === "msg") {
    const msg = document.createElement("span");
    msg.className = "ref-msg";
    msg.textContent = edit.message;
    host.append(msg);
  } else {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "ref-update";
    btn.textContent = edit.message;
    btn.addEventListener("mousedown", (event) => {
      event.preventDefault();
      const current = host._edit;
      if (!current) return;
      applyDefinition(view, current);
      refBase = null;
      refDismissed = false;
      closeRefPrompt();
      view.focus();
    });
    host.append(btn);
  }
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.className = "ref-dismiss";
  dismiss.textContent = "Dismiss";
  dismiss.addEventListener("mousedown", (event) => {
    event.preventDefault();
    refDismissed = true;
    closeRefPrompt();
  });
  host.append(dismiss);
  document.querySelector("[data-editor]").append(host);
  placeRefPrompt(view, host);
  refPrompt = host;
}

// A definition's name changed. Keep the offer alive across each letter.
function noteDefinitionEdit(view, before, after) {
  if (!calc || applyingRename || renamePop) return;
  const continued = refBase != null;
  const base = refBase ?? before;
  let edit;
  try {
    edit = JSON.parse(definitionChange(base, after));
  } catch {
    return;
  }
  if (edit) {
    if (!continued) refDismissed = false;
    refBase = base;
    if (refDismissed) closeRefPrompt();
    else showRefPrompt(view, withForeign(edit));
    return;
  }
  const fresh = JSON.parse(definitionChange(before, after));
  if (fresh) {
    refDismissed = false;
    refBase = before;
    showRefPrompt(view, withForeign(fresh));
    return;
  }
  refBase = null;
  refDismissed = false;
  closeRefPrompt();
}

let renameMenu = null;
function closeRenameMenu() {
  renameMenu?.remove();
  renameMenu = null;
}

function showRenameMenu(view, x, y, utf16) {
  closeRenameMenu();
  const menu = document.createElement("div");
  menu.className = "rename-menu";
  menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - 180))}px`;
  menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - 56))}px`;
  const item = document.createElement("button");
  item.type = "button";
  const label = document.createElement("span");
  label.textContent = "Rename variable";
  const keys = document.createElement("kbd");
  keys.textContent = "F2";
  item.append(label, keys);
  item.addEventListener("mousedown", (event) => {
    event.preventDefault();
    event.stopPropagation();
    closeRenameMenu();
    openRename(view, utf16);
  });
  menu.append(item);
  document.body.append(menu);
  renameMenu = menu;
  view.dispatch({});
}

// The Rename button in the top bar, shown on a phone while the caret is on a name.
function syncRenameButton() {
  const button = document.querySelector(".rename-bar");
  if (!button) return;
  if (!calc) {
    button.hidden = true;
    return;
  }
  let found = null;
  try {
    found = JSON.parse(bindingAt(view.state.doc.toString(), view.state.selection.main.head));
  } catch {
    found = null;
  }
  button.hidden = !found;
}

function renameAtCaret() {
  if (!openRename(view, view.state.selection.main.head)) {
    toast("Put the cursor on a variable name.");
  }
}

// A finger held on a name opens the rename field. A mouse right-click opens the menu.
// contextmenu is the long-press on some phones; the timer covers the ones that never fire it.
const LONG_PRESS_MS = 550;
let pressOpenedAt = 0;

function armRenamePress(view) {
  const dom = view.contentDOM;
  let point = null;
  let pressTimer = 0;
  const stop = () => {
    clearTimeout(pressTimer);
    pressTimer = 0;
    point = null;
  };
  dom.addEventListener(
    "touchstart",
    (event) => {
      if (event.touches.length !== 1) {
        stop();
        return;
      }
      const touch = event.touches[0];
      const pos = view.posAtCoords({ x: touch.clientX, y: touch.clientY });
      stop();
      if (pos == null || !calc) return;
      let found = null;
      try {
        found = JSON.parse(bindingAt(view.state.doc.toString(), pos));
      } catch {
        return;
      }
      if (!found) return;
      point = { x: touch.clientX, y: touch.clientY, pos };
      pressTimer = setTimeout(() => {
        pressTimer = 0;
        const held = point;
        point = null;
        if (held && openRename(view, held.pos)) pressOpenedAt = performance.now();
      }, LONG_PRESS_MS);
    },
    { passive: true },
  );
  dom.addEventListener(
    "touchmove",
    (event) => {
      if (!point || !event.touches[0]) return;
      const touch = event.touches[0];
      if (Math.hypot(touch.clientX - point.x, touch.clientY - point.y) > 12) stop();
    },
    { passive: true },
  );
  dom.addEventListener(
    "touchend",
    (event) => {
      const justOpened = performance.now() - pressOpenedAt < 400;
      stop();
      if (justOpened) event.preventDefault();
    },
    { passive: false },
  );
  dom.addEventListener("touchcancel", stop);
  view.dom.addEventListener("contextmenu", (event) => {
    if (!calc) return;
    if (performance.now() - pressOpenedAt < 700) {
      event.preventDefault();
      return;
    }
    const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
    if (pos == null) return;
    let found = null;
    try {
      found = JSON.parse(bindingAt(view.state.doc.toString(), pos));
    } catch {
      return;
    }
    if (!found) return;
    event.preventDefault();
    if (point) {
      stop();
      if (openRename(view, pos)) pressOpenedAt = performance.now();
      return;
    }
    showRenameMenu(view, event.clientX, event.clientY, pos);
  });
  document.addEventListener("mousedown", (event) => {
    if (renameMenu && !renameMenu.contains(event.target)) closeRenameMenu();
  });
}

// As in the desktop app: Tab accepts a suggestion. Ctrl+Space opens the list
// and then Enter accepts it. Otherwise Enter is a new line.
const editorKeys = [
  { key: "Tab", run: acceptCompletion },
  { key: "ArrowDown", run: moveCompletionSelection(true) },
  { key: "ArrowUp", run: moveCompletionSelection(false) },
  {
    key: "Escape",
    run(view) {
      explicitCompletion = false;
      if (closeCompletion(view)) return true;
      if (renamePop) {
        closeRename();
        return true;
      }
      if (refPrompt) {
        refDismissed = true;
        closeRefPrompt();
        return true;
      }
      return false;
    },
  },
  {
    key: "Ctrl-Space",
    run(view) {
      armingExplicit = true;
      const opened = startCompletion(view);
      armingExplicit = false;
      explicitCompletion = completionOpen(view.state);
      return opened;
    },
  },
  {
    key: "Enter",
    run(view) {
      if (!explicitCompletion || !completionOpen(view.state)) return false;
      explicitCompletion = false;
      return acceptCompletion(view);
    },
  },
  {
    key: "Mod-z",
    run(view) {
      if (crossUndo && crossUndo.beforeText === crossUndo.afterText && view.state.doc.toString() === crossUndo.beforeText) {
        restoreCrossUndo();
        return true;
      }
      return false;
    },
  },
  { key: "Mod-Shift-s", run: () => (action("copy"), true) },
  { key: "Mod-Shift-m", run: () => (action("markdown"), true) },
  {
    key: "F2",
    run(view) {
      return openRename(view, view.state.selection.main.head);
    },
  },
];

// Set while a phone keyboard's composition is being copied into the document,
// so that copy is not itself treated as a definition rename.
let committingTyped = false;
let switchingPage = false;

const editorExtensions = [
  undoHistory(),
  Prec.highest(keymap.of(editorKeys)),
  keymap.of([...defaultKeymap, ...historyKeymap]),
  EditorView.domEventHandlers({
    click(event, view) {
      if (!calc) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      let ref = null;
      try {
        ref = JSON.parse(pageRefAt(view.state.doc.toString(), pos));
      } catch {
        return false;
      }
      if (!ref) return false;
      openHandle(ref.handle);
      return false;
    },
  }),
  EditorView.lineWrapping,
  sheetField,
  autocompletion({ override: [completions], defaultKeymap: false, icons: false }),
  hoverTooltip(hoverSource, {
    hoverTime: 250,
    hideOn(tr) {
      return tr.docChanged || completionOpen(tr.state) || hoverBlocked();
    },
  }),
  EditorView.updateListener.of((update) => {
    // A doc change has to recalculate even when the completion check throws.
    // completionStatus wants the state; passing the view throws, which used to
    // skip this and leave the new line blank and the lines below it dimmed.
    if (update.docChanged) {
      noteCrossUndo(update);
      if (!committingTyped && !switchingPage) {
        noteDefinitionEdit(update.view, update.startState.doc.toString(), update.state.doc.toString());
      }
      onInput();
    }
    if (update.docChanged || update.selectionSet) syncRenameButton();
    if (!armingExplicit && !completionOpen(update.state)) explicitCompletion = false;
  }),
  EditorView.contentAttributes.of({
    spellcheck: "false",
    autocapitalize: "off",
    autocorrect: "off",
    autocomplete: "off",
    "aria-label": "Page",
  }),
];

// completionStatus reads an EditorState. A throw means it isn't open, so
// Enter stays a newline.
function completionOpen(state) {
  try {
    return completionStatus(state) === "active";
  } catch {
    return false;
  }
}

const view = new EditorView({ parent: el.host, state: EditorState.create({ doc: "", extensions: editorExtensions }) });

// An iPhone keyboard can show composition or an autocorrect in the DOM before
// that text is in the document. Once the editor's own update has had a turn,
// if the DOM is still ahead, commit it so the answer column sees it.
function plainTyped(text) {
  return text.replace(/\u200b/g, "").replace(/[\u00a0\u2007\u202f\u2008\u2009\u200a\u2018\u2019\u201a\u201b\u201c\u201d\u201e\u201f\u2010\u2011\u2012\u2013\u2014\u2212\ufeff]/g, (ch) => {
    if ("\u2018\u2019\u201a\u201b".includes(ch)) return "'";
    if ("\u201c\u201d\u201e\u201f".includes(ch)) return '"';
    if ("\u2010\u2011\u2012\u2013\u2014\u2212".includes(ch)) return "-";
    if (ch === "\ufeff") return "";
    return " ";
  });
}
// The answer widget sits in the line. Reading textContent would copy "100" back
// into the page as more math.
function lineSource(el) {
  let text = "";
  const walk = (node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      text += node.nodeValue;
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const cls = node.classList;
    if (cls.contains("res") || cls.contains("cm-widget") || cls.contains("cm-widgetBuffer")) return;
    if (node.contentEditable === "false") return;
    for (const child of node.childNodes) walk(child);
  };
  walk(el);
  return plainTyped(text);
}
function typedFromDom() {
  const lines = [...view.contentDOM.querySelectorAll(".cm-line")];
  if (!lines.length) return null;
  return lines.map(lineSource).join("\n");
}
function caretInDom() {
  const sel = document.getSelection();
  if (!sel?.anchorNode || !view.contentDOM.contains(sel.anchorNode)) return null;
  let index = 0;
  for (const line of view.contentDOM.querySelectorAll(".cm-line")) {
    if (line.contains(sel.anchorNode)) {
      const range = document.createRange();
      range.setStart(line, 0);
      range.setEnd(sel.anchorNode, sel.anchorOffset);
      return index + plainTyped(range.toString()).length;
    }
    index += lineSource(line).length + 1;
  }
  return null;
}
let typedFrame = 0;
function commitTypedText() {
  if (committingTyped || !view.hasFocus) return;
  const dom = typedFromDom();
  if (dom == null) return;
  const doc = view.state.doc.toString();
  if (dom === doc) return;
  const caret = caretInDom();
  committingTyped = true;
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: dom },
    selection: { anchor: Math.max(0, Math.min(dom.length, caret ?? dom.length)) },
  });
  committingTyped = false;
}
function scheduleTypedCommit() {
  cancelAnimationFrame(typedFrame);
  typedFrame = requestAnimationFrame(() => {
    typedFrame = 0;
    commitTypedText();
  });
}
for (const type of ["input", "compositionupdate", "compositionend", "beforeinput"]) {
  view.contentDOM.addEventListener(type, scheduleTypedCommit);
}
armRenamePress(view);

// The calculation runs in a worker (worker.js), so the page never waits on it. The
// page sends each new text; the worker works on the newest, in slices, and sends
// answers back as it goes (a long page fills in from the top). Copies are worked out
// there too, never on the page while there's a worker. One that stops answering for
// HANG_MS (it can't be interrupted mid-line) is replaced and given everything again.
// Without workers, the page calculates itself.
const HANG_MS = 5000;
let worker = null;
let ratesJson = null;
let seq = 0;
let latest = null; // the newest { seq, sheet, text } sent
let answered = 0; // the seq of the newest text fully answered
let heardAt = 0; // when the worker last said anything
const copies = new Map(); // seq -> { sheet, text, resolve }, copies on their way

function scriptURL(url) {
  const make = globalThis.omacusScriptURL;
  return typeof make === "function" ? make(url) : url;
}

function startWorker() {
  worker?.terminate();
  try {
    worker = new Worker(scriptURL(WORKER), { type: "module" });
  } catch {
    worker = null;
    return;
  }
  heardAt = performance.now();
  worker.onmessage = (e) => onWorker(e.data);
  worker.onerror = () => {
    // A worker that can't start (an old browser without module workers): calculate here.
    worker?.terminate();
    worker = null;
    for (const c of copies.values()) c.resolve(pageLayout(c.text));
    copies.clear();
    evaluate();
  };
  worker.postMessage({ type: "init", wasm: new URL(WASM, location.href).href, rates: ratesJson });
  // Messages wait in the worker until its engine is ready.
  if (latest && answered < latest.seq) worker.postMessage({ type: "sheet", ...latest });
  for (const [id, c] of copies) {
    worker.postMessage({ type: "copy", seq: id, sheet: c.sheet, handle: c.handle, text: c.text, pages: c.pages });
  }
}

function onWorker(m) {
  heardAt = performance.now();
  if (m.type === "sheet") {
    if (m.done) answered = Math.max(answered, m.seq);
    if (m.sheet === (state.active ?? "")) show(m.results, m.text, m.done);
  } else if (m.type === "copy") {
    copies.get(m.seq)?.resolve(m.results);
    copies.delete(m.seq);
  } else if (m.type === "error") {
    console.error("Omacus worker:", m.message);
  }
}

// A worker with work outstanding that has gone quiet is replaced.
setInterval(() => {
  const waiting = copies.size || (latest && answered < latest.seq);
  if (worker && waiting && performance.now() - heardAt > HANG_MS) startWorker();
}, 1000);

// Answers worked out for `text` (all, or the first lines' so far), lined up with what
// the editor shows now; lines without one yet keep the answer they show, dimmed.
function show(answers, text, done) {
  const now = view.state.doc.toString();
  const shown = view.state.field(sheetField).answers;
  results = realign(text, answers, now, shown);
  resultsText = now;
  resultsExact = done && text === now;
  view.dispatch({ effects: setResults.of(results) });
  syncEmpty();
  if (pagePreviewOpen) refreshPagePreview();
}

let frame = 0;
function evaluate() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => {
    const text = view.state.doc.toString();
    const sheet = state.active ?? "";
    const pages = pageCatalog();
    const handle = currentHandle();
    if (worker) {
      latest = { seq: ++seq, sheet, handle, text, pages };
      worker.postMessage({ type: "sheet", ...latest });
    } else if (calc) {
      show(JSON.parse(calc.evaluateSheetWith(sheet, handle, text, JSON.stringify(pages))), text, true);
    }
  });
}

// The sheet with its answers, for the clipboard: the answers on screen when they're
// all fresh for this exact text, otherwise worked out (in the worker, if there is one).
function laidOut(text, markdown) {
  const lay = (json) => layoutAnswers(text, json, markdown);
  if (resultsExact && resultsText === text) return Promise.resolve(lay(JSON.stringify(results)));
  if (!worker) return Promise.resolve(lay(pageLayout(text)));
  const id = ++seq;
  const handle = currentHandle();
  const pages = pageCatalog();
  worker.postMessage({ type: "copy", seq: id, sheet: state.active ?? "", handle, text, pages });
  return new Promise((resolve) => copies.set(id, { sheet: state.active ?? "", handle, text, pages, resolve })).then(lay);
}

// Without a worker: the answers worked out here.
function pageLayout(text) {
  return calc.evaluateSheetWith(state.active ?? "", currentHandle(), text, JSON.stringify(pageCatalog()));
}

let varCache = { text: "", vars: new Set() };
function definedVars(lines) {
  const text = lines.join("\n");
  if (engineReady) {
    if (varCache.text !== text) {
      varCache = { text, vars: new Set(JSON.parse(variables(text))) };
    }
    return varCache.vars;
  }
  const vars = new Set();
  for (const l of lines) {
    const m = l.match(/^\s*([A-Za-z_]\w*)\s*=[^=]/);
    if (m) vars.add(m[1]);
  }
  return vars;
}

// Light styling for edit mode: a heading is larger, **bold** and *italic* show,
// and a mid-line # (or //) is a comment. Offsets are UTF-16, like the editor.
function lineSpans(line, vars) {
  const marks = marksFor(line);
  const cls = new Array(line.length).fill("");
  const paint = (from, to, name) => {
    for (let i = Math.max(0, from); i < Math.min(line.length, to); i++) cls[i] = name;
  };
  const add = (from, to, name) => {
    for (let i = Math.max(0, from); i < Math.min(line.length, to); i++) {
      if (cls[i] && cls[i] !== "t-com") cls[i] += " " + name;
      else if (!cls[i]) cls[i] = name;
    }
  };
  if (marks.heading) paint(0, line.length, `t-h${marks.heading}`);
  else {
    const code = line.slice(0, marks.comment ?? line.length);
    for (const [from, to, name] of tokens(code, vars, marks.define)) {
      paint(from, to, name);
    }
    for (const [from, to, name] of pageRefSpans(code)) paint(from, to, name);
  }
  if (marks.comment != null) paint(marks.comment, line.length, "t-com");
  for (const [from, to] of marks.bold) add(from, to, "t-bold");
  for (const [from, to] of marks.italic) add(from, to, "t-em");
  for (const [from, to] of marks.markers) paint(from, to, "t-com");
  const out = [];
  let i = 0;
  while (i < line.length) {
    const name = cls[i];
    let j = i + 1;
    while (j < line.length && cls[j] === name) j++;
    if (name) out.push([i, j, name]);
    i = j;
  }
  return out;
}

function marksFor(line) {
  if (engineReady) return JSON.parse(pageMarks(line));
  const comment = line.indexOf("//");
  const heading = /^(#{1,6})(?:\s|$)/.exec(line);
  return {
    heading: heading ? Math.min(6, heading[1].length) : null,
    comment: comment >= 0 ? comment : null,
    bold: [],
    italic: [],
    markers: [],
  };
}

const kindCache = new Map();
function isKeywordOrUnit(word) {
  const key = word.toLowerCase();
  if (kindCache.has(key)) return kindCache.get(key);
  const yes = engineReady ? wordKind(word) !== "" : KEYWORDS.has(key);
  kindCache.set(key, yes);
  return yes;
}

// The same colours as the desktop app, as [from, to, class] spans.
// `@handle` is its own colour. A bare `@name` with no dot is left alone so it
// can later be a person mention. `mail@trip.total` is not a reference.
function pageRefSpans(line) {
  const spans = [];
  let quoted = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      quoted = !quoted;
      continue;
    }
    if (quoted || ch !== "@") continue;
    if (i > 0 && /[A-Za-z0-9_]/.test(line[i - 1])) continue;
    const match = /^@([A-Za-z][A-Za-z0-9]*)\.([A-Za-z_][A-Za-z0-9_]*)/.exec(line.slice(i));
    if (!match) continue;
    const handleEnd = i + 1 + match[1].length;
    spans.push([i, handleEnd, "t-ref"]);
    const nameAt = handleEnd + 1;
    spans.push([nameAt, nameAt + match[2].length, "t-var"]);
    i = nameAt + match[2].length - 1;
  }
  return spans;
}

function tokens(line, vars, define) {
  const out = [];
  const code = line;
  let at = 0;
  const label = code.match(/^(\s*[A-Za-z][A-Za-z0-9 '\-_&/]*:)(?= )/);
  if (label) {
    out.push([0, label[1].length, "t-label"]);
    at = label[1].length;
  }
  const re = /("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹₩₽₺%#])|(\s+)|(.)/gu;
  re.lastIndex = at;
  let m;
  while ((m = re.exec(code))) {
    const [tok, str, num, word, sym, ws] = m;
    const span = (cls) => out.push([m.index, m.index + tok.length, cls]);
    if (str) span("t-com");
    else if (num || sym) span("t-num");
    else if (word) {
      const defined = define && m.index === define[0];
      const cls = defined ? "t-def" : vars.has(word) ? "t-var" : isKeywordOrUnit(word) ? "t-kw" : "";
      if (cls) span(cls);
    } else if (!ws) span("t-op");
  }
  return out.sort((x, y) => x[0] - y[0]);
}

// ---------------------------------------------------------------------------------
// Share links: "#v1." then the sheet, deflated and base64url-encoded, by the engine
// (the desktop app makes and reads exactly the same links).

function sharePageUrl(payload) {
  return `https://app.omacus.com#${payload}`;
}

async function share() {
  if (refuseConcealed()) return;
  const text = editingText();
  const url = sharePageUrl(shareEncode(text));
  await copy(url);
  const n = url.length.toLocaleString();
  if (url.length > 8000) toast(linkCopied(n, "Long links can be cut off by some apps; for a long page, Copy as Markdown table is safer."));
  else if (url.length > 2000) toast(linkCopied(n, "Fine for chat apps; some email and SMS may cut it."));
  else toast(linkCopied(null, "Anyone with it can open this page."));
}

async function openShared() {
  const hash = decodeURIComponent(location.hash.slice(1));
  if (!hash) return;
  const text = shareDecode(hash);
  history.replaceState(null, "", location.pathname + location.search);
  if (text === undefined) return toast("That link couldn't be opened: it may be cut off or damaged.");
  // Already saved? Open that copy instead of a duplicate.
  const same = state.order.find((id) => state.texts.get(id) === text);
  if (same) {
    state.shared = null;
    select(same);
    return toast("This page is already in your pages.");
  }
  state.shared = { text, link: hash };
  state.active = "shared";
  showSheet();
}

// "Open in Omacus": hand the sheet to the desktop app through its omacus:// link.
// If nothing takes it (no app, or one older than 0.1.10), say where to get it.
function openInApp() {
  if (refuseConcealed()) return;
  const link = `omacus://open#${shareEncode(editingText())}`;
  let left = false;
  const away = () => (left = true);
  addEventListener("blur", away, { once: true });
  document.addEventListener("visibilitychange", away, { once: true });
  location.href = link;
  setTimeout(() => {
    removeEventListener("blur", away);
    if (!left) {
      toast(desktopMissing());
    }
  }, 1600);
}

function saveShared() {
  if (!state.shared) return;
  const id = addSheet(state.shared.text, { select: false });
  state.shared = null;
  state.active = id;
  saveIndex();
  showSheet();
  toast("Saved to your pages.");
}

addEventListener("hashchange", openShared);

// ---------------------------------------------------------------------------------
// Exchange rates: the same source as the desktop app, cached for 12 hours.

// Rates for both engines: the page's (for copies without a worker) and the worker's.
function setRates(json) {
  if (!calc.setRates(json)) return false;
  ratesJson = json;
  worker?.postMessage({ type: "rates", rates: json });
  return true;
}

// "fresh" hides the note. "stale" shows it: a refresh was due and did not succeed,
// so the cached rates (if any) are still what currency math uses.
function setRatesNote(state) {
  const note = $("[data-rates-note]");
  if (!note) return;
  note.dataset.ratesState = state;
  note.hidden = state !== "stale";
}

async function loadRates() {
  let cached = null;
  try {
    cached = JSON.parse(store.get("rates") ?? "null");
  } catch {}
  const have = !!(cached?.rates && setRates(JSON.stringify(cached.rates)));
  const age = Number.isFinite(cached?.at) ? Date.now() - cached.at : Infinity;
  if (have && age < RATES_MAX_AGE) {
    setRatesNote("fresh");
    return;
  }
  try {
    const res = await fetch(RATES_URL);
    if (!res.ok) throw new Error(String(res.status));
    const { rates } = await res.json();
    if (!rates || !setRates(JSON.stringify(rates))) throw new Error("no rates");
    store.set("rates", JSON.stringify({ at: Date.now(), rates }));
    setRatesNote("fresh");
    evaluate();
  } catch {
    // A refresh was due and failed. Cached rates, when we have them, stay in use.
    setRatesNote("stale");
  }
}

// ---------------------------------------------------------------------------------
// Menu actions.

async function action(name) {
  closeMenu();
  closeRowMenu();
  const text = editingText();
  if (name === "rename") {
    if (refuseConcealed()) return;
    renameAtCaret();
  } else if (name === "handle") {
    openHandleDialog();
  } else if (name === "copy") {
    if (refuseConcealed()) return;
    await copyLater(laidOut(text, false));
    toast("Page copied with answers.");
  } else if (name === "markdown") {
    if (refuseConcealed()) return;
    await copyLater(laidOut(text, true));
    toast("Page copied as a Markdown table.");
  } else if (name === "preview") {
    togglePagePreview();
  } else if (name === "close-preview") {
    closePagePreview();
    view.focus();
  } else if (name === "print-preview") {
    printPagePreview();
  } else if (name === "export-preview") {
    toast("In the print dialog, choose Save as PDF.");
    printPagePreview();
  } else if (name === "new") {
    flushSave();
    if (addSheet("")) {
      showSheet();
      view.focus();
      // The phone drawer was covering the blank page, and the empty hint stays
      // hidden while that drawer is open.
      setDrawer(false);
    }
  } else if (name === "lock" || name === "lock-now") {
    const id = state.active;
    const choice = pageLockChoice(id);
    if (choice === "create") openLock("create-page", id);
    else if (choice === "relock") lockNow();
    else if (choice === "unlock") {
      showSheet();
      el.gatePassword?.focus();
    } else toast("Open a page to lock it.");
  } else if (name === "remove-lock") {
    if (state.active && state.active !== "shared") openLock("remove-page", state.active);
  } else if (name === "close-lock") {
    closeLock();
  } else if (name === "lock-submit") {
    submitLock();
  } else if (name === "gate-unlock") {
    submitGate();
  } else if (name === "new-folder") {
    addFolder(null);
  } else if (name === "move-folder") {
    if (state.active && state.active !== "shared" && pageIds(state.tree).includes(state.active)) openMove(state.active);
    else toast("Open a page to move it.");
  } else if (name === "close-move") {
    closeMove();
  } else if (name === "close-rename") {
    closeFolderRename();
  } else if (name === "rename-commit") {
    commitFolderRename();
  } else if (name === "close-switcher") {
    closeSwitcher();
  } else if (name === "delete") {
    deleteSheet();
  } else if (name === "export") {
    exportZip();
  } else if (name === "import") {
    el.importInput.click();
  } else if (name === "save-shared") {
    saveShared();
  } else if (name === "share") {
    share();
  } else if (name === "open-app") {
    openInApp();
  } else if (name === "install") {
    installPrompt?.prompt();
  } else if (name === "dismiss-install") {
    dismissInstall();
  } else if (name === "apply-update") {
    applyUpdate();
  } else if (name === "restore-trash") {
    if (preview) restoreTrashed(preview.id);
  } else if (name === "purge-trash") {
    if (!preview || !el.confirm) return;
  const purgeName = isLock(preview.text) ? LOCKED_PAGE : heading(preview.text);
  el.confirmText.textContent = `Delete “${purgeName}” forever? This can't be undone.`;
  el.confirm.hidden = false;
  const yes = $("[data-confirm-yes]");
  if (yes) yes.textContent = "Delete forever";
  el.confirm.dataset.mode = "purge";
    el.confirm.dataset.id = preview.id;
  } else if (name === "collapse-side") {
    setSide(false);
    setDrawer(false);
  } else if (name === "expand-side") {
    setSide(true);
  } else if (name === "settings") {
    openSettings("settings");
  } else if (name === "about") {
    openSettings("about");
  } else if (name === "close-settings") {
    closeSettings();
  } else if (name === "privacy") {
    toast(privacyNote(), null, 7000);
  }
}

function deleteSheet() {
  if (state.active === "shared") {
    state.shared = null;
    state.active = state.order[0] ?? addSheet("");
    return showSheet();
  }
  deletePage(state.active);
}

function deletePage(id) {
  if (!pageIds(state.tree).includes(id) || !el.confirm) return;
  el.confirmText.textContent = `Delete “${pageLabel(id)}”? You can restore it from Recently deleted.`;
  el.confirm.hidden = false;
  const yes = $("[data-confirm-yes]");
  if (yes) yes.textContent = "Delete";
  el.confirm.dataset.mode = "sheet";
  el.confirm.dataset.id = id;
}

function deleteFolder(id) {
  const name = folders(state.tree).find((folder) => folder.id === id)?.name || "Folder";
  if (!el.confirm) return;
  el.confirmText.textContent = `Delete “${name}”? Pages inside move to Recently deleted, where they can be restored.`;
  el.confirm.hidden = false;
  const yes = $("[data-confirm-yes]");
  if (yes) yes.textContent = "Delete";
  el.confirm.dataset.mode = "folder";
  el.confirm.dataset.id = id;
}

function commitDelete() {
  const id = el.confirm?.dataset.id;
  if (!id) return;
  if (el.confirm.dataset.mode === "purge") {
    el.confirm.hidden = true;
    const items = loadTrash();
    const item = items.find((t) => t.id === id);
    saveTrash(items.filter((t) => t.id !== id));
    state.handles.delete(id);
    saveIndex();
    if (preview?.id === id) clearPreview();
    renderDeleted();
    if (item) toast(`Deleted “${heading(item.text)}” forever.`);
    return;
  }
  el.confirm.hidden = true;
  if (el.confirm.dataset.mode === "folder") commitDeleteFolder(id);
  else commitDeletePage(id);
}

function dropPlaceholder() {
  if (state.order.length === 1 && (state.texts.get(state.order[0]) ?? "") === "") {
    const blank = state.order[0];
    removeNode(state.tree, blank);
    state.texts.delete(blank);
    store.remove("page:" + blank);
    syncOrder();
  }
}

function commitDeletePage(id) {
  const text = state.texts.get(id);
  if (text == null) return;
  const label = isLock(text) ? LOCKED_PAGE : title(shownPlain(id) || text);
  forgetPage(id);
  const wasActive = state.active === id;
  const oldAt = state.order.indexOf(id);
  const taken = removeNode(state.tree, id);
  const parent = taken?.parent ?? null;
  const at = taken?.index ?? 0;
  const trash = loadTrash().filter((t) => t.id !== id);
  trash.unshift({ id, text, handle: state.handles.get(id) || "" });
  saveTrash(trash);
  const pending = flushSave();
  state.texts.delete(id);
  store.remove("page:" + id);
  store.remove("sheet:" + id);
  Promise.resolve(pending).then(() => dropDocPage(id));
  syncOrder();
  if (wasActive) {
    state.active = state.order[Math.min(Math.max(oldAt, 0), state.order.length - 1)] ?? null;
    if (!state.active) addSheet("");
  }
  saveIndex();
  showSheet();
  toast(`Deleted “${label}”.`, {
    label: "Undo",
    run() {
      dropPlaceholder();
      saveTrash(loadTrash().filter((t) => t.id !== id));
      state.texts.set(id, text);
      insertNode(state.tree, parent, at, { type: "page", id });
      syncOrder();
      saveSheet(id);
      state.active = id;
      saveIndex();
      showSheet();
    },
  });
}

function commitDeleteFolder(id) {
  const taken = removeNode(state.tree, id);
  if (!taken || taken.node.type !== "folder") {
    if (taken) insertNode(state.tree, taken.parent, taken.index, taken.node);
    return;
  }
  const name = taken.node.name || "Folder";
  const ids = collectPageIds(taken.node);
  const pages = ids.map((pid) => ({ id: pid, text: state.texts.get(pid) ?? "" }));
  const trash = loadTrash().filter((t) => !ids.includes(t.id));
  for (const page of [...pages].reverse()) trash.unshift({ id: page.id, text: page.text });
  saveTrash(trash);
  const pending = flushSave();
  for (const pid of ids) {
    forgetPage(pid);
    state.texts.delete(pid);
    store.remove("page:" + pid);
    store.remove("sheet:" + pid);
  }
  Promise.resolve(pending).then(() => {
    for (const pid of ids) dropDocPage(pid);
  });
  folderKeys.delete(id);
  const active = state.active;
  syncOrder();
  if (!state.order.includes(active)) {
    state.active = state.order[0] ?? null;
    if (!state.active) addSheet("");
  }
  saveIndex();
  showSheet();
  toast(`Deleted “${name}”.`, {
    label: "Undo",
    run() {
      dropPlaceholder();
      saveTrash(loadTrash().filter((t) => !ids.includes(t.id)));
      for (const page of pages) {
        state.texts.set(page.id, page.text);
        saveSheet(page.id);
      }
      insertNode(state.tree, taken.parent, taken.index, taken.node);
      syncOrder();
      if (pages[0]) state.active = pages[0].id;
      saveIndex();
      showSheet();
    },
  });
}

function setDrawer(open) {
  el.page.classList.toggle("drawer", open);
  $(".sheets-toggle")?.setAttribute("aria-expanded", String(open));
}

const SIDE_MIN = 168;
const SIDE_MAX = 480;

function setSide(open) {
  store.set("side", open ? "1" : "0");
  applySide();
}

function applySide() {
  if (!el.workspace) return;
  const open = store.get("side") !== "0";
  el.workspace.classList.toggle("collapsed", !open);
  // An inline width beats the collapsed rail's 52px, so drop it while closed.
  if (!open) {
    el.workspace.style.removeProperty("--side-w");
    return;
  }
  const width = Number(store.get("sidew"));
  if (width >= SIDE_MIN && width <= SIDE_MAX) {
    el.workspace.style.setProperty("--side-w", `${width}px`);
  } else {
    el.workspace.style.removeProperty("--side-w");
  }
}

applySide();

const resize = $("[data-resize]");
if (resize) {
  resize.addEventListener("pointerdown", (e) => {
    if (!el.workspace || el.workspace.classList.contains("collapsed")) return;
    if (matchMedia("(max-width: 760px)").matches) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = el.workspace.getBoundingClientRect
      ? document.getElementById("side").getBoundingClientRect().width
      : SIDE_MIN;
    const move = (ev) => {
      const next = Math.min(SIDE_MAX, Math.max(SIDE_MIN, startW + ev.clientX - startX));
      el.workspace.style.setProperty("--side-w", `${next}px`);
    };
    const up = (ev) => {
      resize.releasePointerCapture?.(ev.pointerId);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const next = Math.round(document.getElementById("side").getBoundingClientRect().width);
      store.set("sidew", String(Math.min(SIDE_MAX, Math.max(SIDE_MIN, next))));
    };
    resize.setPointerCapture?.(e.pointerId);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  });
}

// ---------------------------------------------------------------------------------
// Folders: the same tree the desktop app stores. Drag reorders; Move to folder
// and Ctrl+J are the keyboard path, including on a phone.

function newFolderCommand(query) {
  const q = query.trim().toLowerCase();
  return (q.length >= 2 && "new folder".startsWith(q)) || (q.length >= 3 && "folder".startsWith(q));
}

function moveFolderCommand(query) {
  const q = query.trim().toLowerCase();
  return q.length >= 2 && "move to folder".startsWith(q);
}

function renameCommand(query) {
  const q = query.trim().toLowerCase();
  return q.length >= 2 && ("rename".startsWith(q) || "rename variable".startsWith(q));
}

function addFolder(parent) {
  const id = createFolder(state.tree, parent);
  if (!id) {
    toast("A folder can only sit inside one other folder.");
    return;
  }
  if (el.workspace?.classList.contains("collapsed")) setSide(true);
  saveIndex();
  renderSidebar();
  openFolderRename(id);
}

function openFolderRename(id) {
  const folder = folders(state.tree).find((item) => item.id === id);
  if (!folder || !el.rename) return;
  closeRename();
  el.rename.dataset.id = id;
  el.renameInput.value = folder.name;
  el.rename.hidden = false;
  el.renameInput.focus();
  el.renameInput.select();
}

function closeFolderRename() {
  if (el.rename) el.rename.hidden = true;
}

function commitFolderRename() {
  const id = el.rename?.dataset.id;
  if (!id || !renameFolder(state.tree, id, el.renameInput.value)) return;
  closeFolderRename();
  saveIndex();
  renderSidebar();
}

function closeMove() {
  if (el.move) el.move.hidden = true;
}

function openMove(pageId) {
  if (!el.move || !pageIds(state.tree).includes(pageId)) return;
  closeRowMenu();
  el.move.dataset.id = pageId;
  const current = folderOf(state.tree, pageId);
  el.moveList.replaceChildren();
  const addChoice = (label, parent, depth) => {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.style.paddingLeft = `${8 + depth * 14}px`;
    button.addEventListener("click", () => {
      if (current !== parent) {
        applyPlace(pageId, parent ? { kind: "into", id: parent } : { kind: "root" });
      }
      closeMove();
      renderSidebar();
    });
    el.moveList.append(button);
  };
  addChoice(current == null ? "No folder  ·  current" : "No folder", null, 0);
  const list = folders(state.tree);
  if (!list.length) {
    const note = document.createElement("p");
    note.className = "confirm-note";
    note.textContent = "No folders yet. Create one from the sidebar.";
    el.moveList.append(note);
  }
  for (const folder of list) {
    const here = current === folder.id;
    addChoice(here ? `${folder.name}  ·  current` : folder.name, folder.id, Math.max(0, folder.depth - 1));
  }
  el.move.hidden = false;
}

let switcherIndex = 0;

function switcherItems(query) {
  const hits = [];
  if (newFolderCommand(query)) hits.push({ kind: "new-folder", label: "New folder" });
  if (moveFolderCommand(query) && state.active && state.active !== "shared" && pageIds(state.tree).includes(state.active)) {
    hits.push({ kind: "move", label: "Move to folder…" });
  }
  if (renameCommand(query) && state.active && state.active !== "shared") {
    hits.push({ kind: "rename", label: "Rename variable", shortcut: "F2" });
  }
  const q = query.trim().toLowerCase();
  const addPage = (id, text, concealed) => {
    const label = concealed ? LOCKED_PAGE : heading(text);
    const hay = concealed ? label : `${label}\n${text}`;
    if (!q || hay.toLowerCase().includes(q)) hits.push({ kind: "page", id, label });
  };
  if (state.shared) addPage("shared", state.shared.text, false);
  for (const id of state.order) addPage(id, pageConcealed(id) ? "" : shownPlain(id), pageConcealed(id));
  return hits;
}

function renderSwitcher() {
  if (!el.switcherHits) return;
  const hits = switcherItems(el.switcherQuery.value);
  if (switcherIndex >= hits.length) switcherIndex = Math.max(0, hits.length - 1);
  el.switcherHits.replaceChildren();
  if (!hits.length) {
    const empty = document.createElement("p");
    empty.className = "confirm-note";
    empty.textContent = "No matching page";
    el.switcherHits.append(empty);
    return;
  }
  hits.forEach((hit, i) => {
    const button = document.createElement("button");
    button.type = "button";
    const label = document.createElement("span");
    label.textContent = hit.label;
    button.append(label);
    if (hit.shortcut) {
      const keys = document.createElement("kbd");
      keys.textContent = hit.shortcut;
      button.append(keys);
    }
    if (i === switcherIndex) button.setAttribute("aria-selected", "true");
    button.addEventListener("click", () => followSwitcher(hit));
    el.switcherHits.append(button);
  });
}

function openSwitcher() {
  if (EMBED || !el.switcher) return;
  closeMenu();
  closeRowMenu();
  closeMove();
  switcherIndex = 0;
  el.switcherQuery.value = "";
  el.switcher.hidden = false;
  renderSwitcher();
  el.switcherQuery.focus();
}

function closeSwitcher() {
  if (el.switcher) el.switcher.hidden = true;
}

function followSwitcher(hit) {
  closeSwitcher();
  if (hit.kind === "new-folder") addFolder(null);
  else if (hit.kind === "move") openMove(state.active);
  else if (hit.kind === "rename") renameAtCaret();
  else if (hit.id) select(hit.id);
}

function closeRowMenu() {
  if (el.rowMenu) el.rowMenu.hidden = true;
}

function openMenu(x, y, items) {
  if (!el.rowMenu) return;
  el.rowMenu.replaceChildren();
  for (const item of items) {
    if (item === "sep") {
      el.rowMenu.append(document.createElement("hr"));
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.setAttribute("role", "menuitem");
    button.textContent = item.label;
    if (item.danger) button.className = "danger";
    button.addEventListener("click", (event) => {
      event.stopPropagation();
      closeRowMenu();
      item.run();
    });
    el.rowMenu.append(button);
  }
  el.rowMenu.hidden = false;
  el.rowMenu.style.left = "0px";
  el.rowMenu.style.top = "0px";
  const rect = el.rowMenu.getBoundingClientRect();
  el.rowMenu.style.left = `${Math.min(Math.max(8, x), window.innerWidth - rect.width - 8)}px`;
  el.rowMenu.style.top = `${Math.min(Math.max(8, y), window.innerHeight - rect.height - 8)}px`;
}

function openBlankMenu(x, y) {
  openMenu(x, y, [
    { label: "New page", run: () => action("new") },
    { label: "New folder", run: () => addFolder(null) },
  ]);
}

function nudge(id, step) {
  if (!moveSibling(state.tree, id, step)) return;
  syncOrder();
  saveIndex();
  renderSidebar();
}

function openRowMenu(row, x, y) {
  if (!row) return;
  const id = row.dataset.id;
  if (row.dataset.kind === "folder") {
    const items = [{ label: "New page", run: () => { flushSave(); if (addSheet("", { parent: id })) { showSheet(); view?.focus(); } } }];
    if (canNest(state.tree, id)) items.push({ label: "New folder", run: () => addFolder(id) });
    items.push({ label: "Rename", run: () => openFolderRename(id) });
    items.push("sep");
    const flock = folderLockState(id);
    if (flock === "none") items.push({ label: "Lock…", run: () => openLock("create-folder", id) });
    else if (flock === "closed") items.push({ label: "Unlock…", run: () => openLock("unlock-folder", id) });
    else if (flock === "open") {
      items.push({ label: "Change password…", run: () => openLock("change-folder", id) });
      items.push({ label: "Remove lock…", run: () => openLock("remove-folder", id) });
    }
    if (anyUnlocked()) items.push({ label: "Lock now", run: () => lockNow() });
    items.push("sep", { label: "Delete folder", danger: true, run: () => deleteFolder(id) });
    openMenu(x, y, items);
    return;
  }
  const ends = siblingEnds(state.tree, id);
  const items = [];
  if (!ends.first) items.push({ label: "Move up", run: () => nudge(id, -1) });
  if (!ends.last) items.push({ label: "Move down", run: () => nudge(id, 1) });
  items.push({ label: "Move to folder…", run: () => openMove(id) });
  items.push("sep");
  const covered = !!(coveringLock(state.tree, id) && !individualLock(id));
  const choice = pageLockChoice(id);
  if (choice === "unlock") items.push({ label: "Unlock…", run: () => select(id) });
  else if (choice === "create") items.push({ label: "Lock…", run: () => openLock("create-page", id) });
  else if (choice === "relock" && !covered) {
    items.push({ label: "Lock now", run: () => lockNow() });
    items.push({ label: "Change password…", run: () => openLock("change-page", id) });
  } else if (choice === "relock") items.push({ label: "Lock now", run: () => lockNow() });
  if (ownPageLock(id)) items.push({ label: "Remove lock…", run: () => openLock("remove-page", id) });
  items.push("sep", { label: "Delete page", danger: true, run: () => deletePage(id) });
  openMenu(x, y, items);
}

let drag = null;
let ignoreClick = false;
let expandHover = null;

function ensureChip() {
  let chip = document.querySelector(".drag-chip");
  if (!chip) {
    chip = document.createElement("div");
    chip.className = "drag-chip";
    chip.hidden = true;
    document.body.append(chip);
  }
  return chip;
}

function showChip() {
  const chip = ensureChip();
  chip.textContent = drag.kind === "folder"
    ? folders(state.tree).find((folder) => folder.id === drag.id)?.name || "Folder"
    : pageLabel(drag.id);
  chip.hidden = false;
}

function moveChip(x, y) {
  const chip = ensureChip();
  const width = chip.offsetWidth || 120;
  chip.style.left = `${Math.max(8, Math.min(x + 12, window.innerWidth - width - 8))}px`;
  chip.style.top = `${Math.max(8, Math.min(y + 14, window.innerHeight - 36))}px`;
}

function hideChip() {
  const chip = document.querySelector(".drag-chip");
  if (chip) chip.hidden = true;
}

function paintDrop() {
  if (!el.sheets) return;
  el.sheets.classList.remove("drop-end");
  for (const row of el.sheets.querySelectorAll(".side-row")) {
    row.classList.remove("drop-before", "drop-after", "drop-into");
  }
  if (!drag?.active || !drag.spot) return;
  if (drag.spot.kind === "root") {
    el.sheets.classList.add("drop-end");
    return;
  }
  const row = [...el.sheets.querySelectorAll(".side-row")].find((item) => item.dataset.id === drag.spot.id);
  if (!row) return;
  row.classList.add(drag.spot.kind === "before" ? "drop-before" : drag.spot.kind === "after" ? "drop-after" : "drop-into");
}

function chooseSpot(row, clientY) {
  const id = row.dataset.id;
  if (id === drag.id) return null;
  const rect = row.getBoundingClientRect();
  const ratio = rect.height ? (clientY - rect.top) / rect.height : 0.5;
  let spot = row.dataset.kind === "folder" && ratio > 0.28 && ratio < 0.72
    ? { kind: "into", id }
    : { kind: ratio < 0.5 ? "before" : "after", id };
  if (spot.kind === "into" && !validDrop(state.tree, drag.id, spot)) {
    spot = { kind: ratio < 0.5 ? "before" : "after", id };
  }
  return validDrop(state.tree, drag.id, spot) ? spot : null;
}

function spotAt(x, y) {
  const hit = document.elementFromPoint(x, y);
  const row = hit?.closest?.(".side-row");
  if (row && el.sheets.contains(row) && row.dataset.kind !== "shared") return chooseSpot(row, y);
  if (hit?.closest?.("[data-sheets]") && validDrop(state.tree, drag.id, { kind: "root" })) return { kind: "root" };
  return null;
}

function noteExpand(x, y) {
  const row = document.elementFromPoint(x, y)?.closest?.(".side-row");
  if (!row || row.dataset.kind !== "folder" || row.dataset.collapsed !== "1" || row.dataset.id === drag.id) {
    expandHover = null;
    return;
  }
  const id = row.dataset.id;
  const now = performance.now();
  if (expandHover?.id !== id) expandHover = { id, at: now };
  else if (now - expandHover.at >= 450 && expandFolder(state.tree, id)) {
    saveIndex();
    expandHover = { id, at: Infinity };
    renderSidebar();
  }
}

function trackDrag(on) {
  const mode = on ? "addEventListener" : "removeEventListener";
  window[mode]("pointermove", onSidePointerMove, { passive: false });
  window[mode]("pointerup", onSidePointerUp);
  window[mode]("pointercancel", onSidePointerUp);
}

function onSidePointerDown(e) {
  if (e.button !== 0 || drag) return;
  const row = e.target.closest?.(".side-row");
  if (!row || (row.dataset.kind !== "page" && row.dataset.kind !== "folder")) return;
  if (e.target.closest(".row-more, .chevron")) return;
  drag = { id: row.dataset.id, kind: row.dataset.kind, pointerId: e.pointerId, x: e.clientX, y: e.clientY, active: false, spot: null };
  trackDrag(true);
}

function onSidePointerMove(e) {
  if (!drag || e.pointerId !== drag.pointerId) return;
  const dx = e.clientX - drag.x;
  const dy = e.clientY - drag.y;
  if (!drag.active) {
    if (Math.hypot(dx, dy) < 8) return;
    if (e.pointerType === "touch" && Math.abs(dy) > Math.abs(dx)) {
      trackDrag(false);
      drag = null;
      return;
    }
    drag.active = true;
    el.sheets.setPointerCapture?.(e.pointerId);
    document.body.classList.add("dragging-side");
    showChip();
  }
  e.preventDefault();
  moveChip(e.clientX, e.clientY);
  const spot = spotAt(e.clientX, e.clientY);
  if (JSON.stringify(spot) !== JSON.stringify(drag.spot)) {
    drag.spot = spot;
    paintDrop();
  }
  noteExpand(e.clientX, e.clientY);
}

function onSidePointerUp(e) {
  if (!drag || e.pointerId !== drag.pointerId) return;
  const finished = drag;
  drag = null;
  expandHover = null;
  trackDrag(false);
  hideChip();
  document.body.classList.remove("dragging-side");
  if (e.type === "pointercancel" || !finished.active) return;
  ignoreClick = true;
  if (finished.spot) applyPlace(finished.id, finished.spot);
  renderSidebar();
}

function onSideClick(e) {
  if (ignoreClick) {
    ignoreClick = false;
    return;
  }
  const more = e.target.closest?.(".row-more");
  if (more) {
    e.stopPropagation();
    e.preventDefault();
    const row = more.closest(".side-row");
    const rect = more.getBoundingClientRect();
    openRowMenu(row, rect.left, rect.bottom + 4);
    return;
  }
  const row = e.target.closest?.(".side-row");
  if (!row) return;
  if (row.dataset.kind === "folder") {
    toggleFolder(state.tree, row.dataset.id);
    saveIndex();
    renderSidebar();
    return;
  }
  select(row.dataset.id);
}

function onSideContext(e) {
  if (!e.target.closest?.(".side-row, .side-fill, .sheet-list")) return;
  e.preventDefault();
  const row = e.target.closest?.(".side-row");
  if (!row || row.dataset.kind === "shared") openBlankMenu(e.clientX, e.clientY);
  else openRowMenu(row, e.clientX, e.clientY);
}

if (el.sheets) {
  el.sheets.addEventListener("pointerdown", onSidePointerDown);
  el.sheets.addEventListener("pointermove", onSidePointerMove, { passive: false });
  el.sheets.addEventListener("pointerup", onSidePointerUp);
  el.sheets.addEventListener("pointercancel", onSidePointerUp);
  el.sheets.addEventListener("click", onSideClick);
  el.sheets.addEventListener("contextmenu", onSideContext);
}

el.renameInput?.addEventListener("keydown", (e) => {
  if (e.key === "Enter") {
    e.preventDefault();
    commitFolderRename();
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    closeFolderRename();
  }
});

el.switcherQuery?.addEventListener("input", () => {
  switcherIndex = 0;
  renderSwitcher();
});
el.switcherQuery?.addEventListener("keydown", (e) => {
  const hits = switcherItems(el.switcherQuery.value);
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    if (!hits.length) return;
    switcherIndex = (switcherIndex + (e.key === "ArrowDown" ? 1 : hits.length - 1)) % hits.length;
    renderSwitcher();
  } else if (e.key === "Enter") {
    e.preventDefault();
    if (hits[switcherIndex]) followSwitcher(hits[switcherIndex]);
  } else if (e.key === "Escape") {
    e.preventDefault();
    e.stopPropagation();
    closeSwitcher();
  }
});

document.addEventListener("click", (e) => {
  if (!e.target.closest?.("[data-row-menu]")) closeRowMenu();
  const button = e.target.closest?.("[data-action]");
  const tab = e.target.closest?.("[data-settings-tab]");
  if (tab) showSettingsTab(tab.dataset.settingsTab);
  if (button) action(button.dataset.action);
  const submenu = e.target.closest?.("[data-menu-open]");
  if (submenu && el.menu?.contains(submenu)) {
    showMenuPanel(submenu.dataset.menuOpen);
    queueMicrotask(() => focusMenuItem(0));
  }
  const back = e.target.closest?.("[data-menu-back]");
  if (back && el.menu?.contains(back)) {
    showMenuPanel("page");
    queueMicrotask(() => focusMenuItem(0));
  }
  if (e.target.closest?.("[data-menu-button]")) {
    const willOpen = el.menu.hidden;
    el.menu.hidden = !willOpen;
    el.menuButton.setAttribute("aria-expanded", String(willOpen));
    if (willOpen) {
      showMenuPanel("page");
      syncLockMenu();
      queueMicrotask(() => focusMenuItem(0));
    }
  } else if (!e.target.closest?.("[data-menu]")) closeMenu();
  if (e.target.closest?.(".sheets-toggle")) setDrawer(!el.page.classList.contains("drawer"));
  else if (e.target.closest?.("[data-scrim]")) setDrawer(false);
});

// A tap on the sheet while the phone drawer is open dismisses it, and does not
// also move the cursor in the editor underneath.
document.addEventListener(
  "pointerdown",
  (e) => {
    if (!el.page.classList.contains("drawer")) return;
    if (!matchMedia("(max-width: 760px)").matches) return;
    const target = e.target instanceof Element ? e.target : e.target.parentElement;
    if (target?.closest(".side, .sheets-toggle, .bar")) return;
    setDrawer(false);
    e.preventDefault();
    e.stopPropagation();
  },
  { capture: true, passive: false },
);

function showMenuPanel(name) {
  if (!el.menu) return;
  for (const panel of el.menu.querySelectorAll("[data-menu-panel]")) {
    panel.hidden = panel.dataset.menuPanel !== name;
  }
}

function activeMenuItems() {
  const panel = el.menu?.querySelector("[data-menu-panel]:not([hidden])");
  if (!panel) return [];
  return [...panel.querySelectorAll('[role="menuitem"]')].filter((node) => !node.hidden);
}

function focusMenuItem(index) {
  const items = activeMenuItems();
  if (!items.length) return;
  items[(index + items.length) % items.length].focus();
}

function syncLockMenu() {
  const choice = pageLockChoice(state.active);
  const lockBtn = el.menu?.querySelector('[data-action="lock"]');
  const removeBtn = el.menu?.querySelector('[data-action="remove-lock"]');
  const labels = { create: "Lock…", relock: "Lock now", unlock: "Unlock…", none: "Lock…" };
  if (lockBtn) {
    lockBtn.textContent = labels[choice] || "Lock…";
    lockBtn.hidden = choice === "none";
  }
  if (removeBtn) removeBtn.hidden = !ownPageLock(state.active);
}

function closeMenu() {
  if (!el.menu) return;
  el.menu.hidden = true;
  el.menuButton.setAttribute("aria-expanded", "false");
  showMenuPanel("page");
}

function inNameField(target) {
  const node = target instanceof Element ? target : target?.parentElement;
  return !!node?.closest("input[data-rename-input], .rename-pop input, input[data-handle-input]");
}

addEventListener(
  "keydown",
  (e) => {
    // A folder name, a variable rename, or a handle is a normal text field.
    if (inNameField(e.target)) return;
    if (el.menu && !el.menu.hidden) {
      const items = activeMenuItems();
      const index = items.indexOf(document.activeElement);
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        const delta = e.key === "ArrowDown" ? 1 : -1;
        focusMenuItem(index < 0 ? 0 : index + delta);
        return;
      }
      if (e.key === "ArrowRight" && document.activeElement?.dataset?.menuOpen) {
        e.preventDefault();
        e.stopPropagation();
        showMenuPanel(document.activeElement.dataset.menuOpen);
        focusMenuItem(0);
        return;
      }
      if (e.key === "ArrowLeft" && el.menu.querySelector('[data-menu-panel="copy"]:not([hidden])')) {
        e.preventDefault();
        e.stopPropagation();
        showMenuPanel("page");
        focusMenuItem(0);
        return;
      }
    }
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "b") {
      e.preventDefault();
      if (matchMedia("(max-width: 760px)").matches) {
        setDrawer(!el.page.classList.contains("drawer"));
      } else if (el.workspace) {
        setSide(el.workspace.classList.contains("collapsed"));
      }
    }
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "j") {
      e.preventDefault();
      e.stopPropagation();
      if (el.switcher && !el.switcher.hidden) closeSwitcher();
      else openSwitcher();
    }
    if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      const tag = document.activeElement?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if ((el.confirm && !el.confirm.hidden) || (el.pick && !el.pick.hidden) || (el.rename && !el.rename.hidden) || (el.move && !el.move.hidden) || (el.switcher && !el.switcher.hidden)) return;
      if (state.active && state.active !== "shared") {
        e.preventDefault();
        e.stopPropagation();
        nudge(state.active, e.key === "ArrowUp" ? -1 : 1);
      }
    }
    // Ctrl+Shift+L copies a share link, as in the desktop app.
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.shiftKey && e.key.toLowerCase() === "l") {
      e.preventDefault();
      action("share");
    }
    // Ctrl+, opens settings. Ctrl+\ hides the answers column. Ctrl+P previews
    // the page (and must not open the browser's print dialog).
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && (e.key === "," || e.key === "\\" || e.key.toLowerCase() === "p")) {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === ",") openSettings();
      else if (e.key === "\\") toggleAnswers();
      else if (!e.repeat) togglePagePreview();
    }
  },
  true,
);
addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    if (el.switcher && !el.switcher.hidden) {
      closeSwitcher();
      return;
    }
    if (el.lock && !el.lock.hidden) {
      closeLock();
      return;
    }
    if (el.move && !el.move.hidden) {
      closeMove();
      return;
    }
    if (el.rename && !el.rename.hidden) {
      closeFolderRename();
      return;
    }
    if (el.rowMenu && !el.rowMenu.hidden) {
      closeRowMenu();
      return;
    }
    if (el.handleDialog && !el.handleDialog.hidden) {
      closeHandleDialog();
      return;
    }
    if (el.settings && !el.settings.hidden) {
      closeSettings();
      return;
    }
    if (el.pick && !el.pick.hidden) {
      choosePick(false);
      return;
    }
    if (el.confirm && !el.confirm.hidden) {
      el.confirm.hidden = true;
      return;
    }
    if (pagePreviewOpen) {
      closePagePreview();
      view.focus();
      return;
    }
    if (el.menu && !el.menu.hidden && el.menu.querySelector('[data-menu-panel="copy"]:not([hidden])')) {
      showMenuPanel("page");
      focusMenuItem(0);
      return;
    }
    closeMenu();
    setDrawer(false);
  }
});
function openHandleDialog() {
  if (!el.handleDialog || state.active === "shared" || !state.active) return;
  const handle = currentHandle();
  el.handleTitle.textContent = `Handle for “${title(currentText())}”`;
  el.handleInput.value = handle;
  el.handleNote.textContent = "";
  el.handleNote.classList.remove("handle-error");
  el.handleDialog.hidden = false;
  el.handleInput.focus();
  el.handleInput.select();
}

function closeHandleDialog() {
  if (el.handleDialog) el.handleDialog.hidden = true;
  view?.focus();
}

function previewHandle(next) {
  const current = currentHandle();
  if (!next || next.toLowerCase() === current.toLowerCase()) {
    el.handleNote.textContent = "";
    el.handleNote.classList.remove("handle-error");
    return;
  }
  const source = pageIndex(state.active);
  let result;
  try {
    result = JSON.parse(propagateHandle(JSON.stringify(pageCatalog()), source, next));
  } catch {
    return;
  }
  el.handleNote.textContent = result.message || result.blurb || "";
  el.handleNote.classList.toggle("handle-error", !!result.error);
}

function commitHandle() {
  const next = el.handleInput.value;
  const source = pageIndex(state.active);
  const result = JSON.parse(propagateHandle(JSON.stringify(pageCatalog()), source, next));
  if (result.error) {
    el.handleNote.textContent = result.message;
    el.handleNote.classList.add("handle-error");
    el.handleInput.focus();
    return;
  }
  closeHandleDialog();
  applyEdits(result.edits, result.blurb);
}

el.handleInput?.addEventListener("input", () => previewHandle(el.handleInput.value.trim()));
el.handleInput?.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    commitHandle();
  } else if (event.key === "Escape") {
    event.preventDefault();
    closeHandleDialog();
  }
});
$("[data-handle-save]")?.addEventListener("click", commitHandle);
$("[data-handle-cancel]")?.addEventListener("click", closeHandleDialog);

$("[data-confirm-yes]")?.addEventListener("click", commitDelete);
$("[data-confirm-no]")?.addEventListener("click", () => {
  if (el.confirm) el.confirm.hidden = true;
});
$("[data-pick-keep]")?.addEventListener("click", () => choosePick(false));
$("[data-pick-other]")?.addEventListener("click", () => choosePick(true));

// ---------------------------------------------------------------------------------
// Export and import: plain .txt files in a .zip, like the desktop app's sheets folder.

function folderLockState(id) {
  const cover = coveringLock(state.tree, id);
  if (!cover) return "none";
  if (cover.id !== id) return "covered";
  return folderKeys.has(id) ? "open" : "closed";
}

function exportZip() {
  flushSave();
  const version = document.querySelector('meta[name="omacus-version"]')?.content || "";
  const pages = [];
  for (const id of state.order) {
    pages.push({ id, text: state.texts.get(id) ?? "" });
  }
  const bytes = zipBytes(backupEntries({
    version,
    library: {
      order: state.order,
      active: state.active,
      tree: state.tree,
      handles: Object.fromEntries(state.handles),
    },
    settings,
    trash: loadTrash(),
    pages,
  }));
  const blob = new Blob([bytes], { type: "application/zip" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "omacus-backup.zip";
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  const locked = pages.filter((page) => isLock(page.text)).length;
  let note = `Exported everything (${pages.length} page${pages.length === 1 ? "" : "s"}).`;
  if (locked) note += " Locked pages stay locked in the file.";
  toast(note);
}

function applyMerged(merged) {
  for (const page of merged.pages) {
    if (!state.texts.has(page.id)) {
      state.texts.set(page.id, page.text);
      store.set("page:" + page.id, page.text);
    }
  }
  state.tree = merged.tree;
  syncOrder();
  if (!state.order.includes(state.active)) state.active = state.order[0] ?? null;
  saveIndex();
  if (merged.trash?.length && !loadTrash().length) saveTrash(merged.trash);
  if (merged.settings) {
    const before = JSON.stringify(settings);
    if (merged.settings.theme && settings.theme === "auto") settings.theme = merged.settings.theme;
    if (settings.hideAnswers == null && merged.settings.hideAnswers != null) settings.hideAnswers = !!merged.settings.hideAnswers;
    if (settings.lockMinutes == null && merged.settings.lockMinutes != null) settings.lockMinutes = merged.settings.lockMinutes;
    if (JSON.stringify(settings) !== before) {
      saveSettings();
      applySettings();
    }
  }
}

el.importInput.addEventListener("change", async () => {
  let count = 0;
  for (const file of el.importInput.files) {
    if (file.name.toLowerCase().endsWith(".zip")) {
      const entries = await unzipNamed(file);
      const backup = parseBackup(entries);
      if (backup) {
        const merged = mergeBackup({
          pages: [...state.texts].map(([id, text]) => ({ id, text })),
          tree: state.tree,
          trash: loadTrash(),
          settings,
        }, backup);
        applyMerged(merged);
        count += merged.added.length;
        continue;
      }
      for (const entry of entries) {
        if (!entry.name.toLowerCase().endsWith(".txt")) continue;
        const text = entry.text;
        if (!state.order.some((id) => state.texts.get(id) === text)) {
          addSheet(text, { select: count === 0 });
          count++;
        }
      }
    } else {
      const text = await file.text();
      if (!state.order.some((id) => state.texts.get(id) === text)) {
        addSheet(text, { select: count === 0 });
        count++;
      }
    }
  }
  el.importInput.value = "";
  showSheet();
  toast(count ? `Imported ${count} page${count === 1 ? "" : "s"}.` : "Nothing new to import.");
});

// ---------------------------------------------------------------------------------
// Small things.

// Copy text that may still be on its way. Browsers (Safari above all) allow the
// clipboard only within the click, so the write starts now with the text to come.
async function copyLater(pending) {
  if (typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
    try {
      const blob = pending.then((t) => new Blob([t], { type: "text/plain" }));
      await navigator.clipboard.write([new ClipboardItem({ "text/plain": blob })]);
      return;
    } catch {}
  }
  await copy(await pending);
}

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const t = document.createElement("textarea");
    t.value = text;
    document.body.append(t);
    t.select();
    document.execCommand("copy");
    t.remove();
  }
}

function swatchOr(color) {
  if (!color) return null;
  const swatch = document.createElement("span");
  swatch.className = "swatch";
  swatch.style.background = color;
  return swatch;
}

function answerLabel(display) {
  const b = document.createElement("b");
  b.textContent = display;
  return b;
}

function copiedMessage(value) {
  const span = document.createElement("span");
  span.append("Copied ");
  span.append(answerLabel(value));
  return span;
}

function linkCopied(count, note) {
  const span = document.createElement("span");
  span.append(count ? `Link copied (${count} characters). ` : "Link copied. ");
  const small = document.createElement("small");
  small.textContent = note;
  span.append(small);
  return span;
}

function desktopMissing() {
  const span = document.createElement("span");
  span.append("Nothing opened it. ");
  const link = document.createElement("a");
  link.href = "https://omacus.com/download/";
  link.textContent = "Get Omacus";
  span.append(link);
  const small = document.createElement("small");
  small.textContent = " (0.1.10 or newer opens links)";
  span.append(small);
  return span;
}

function privacyNote() {
  const span = document.createElement("span");
  const title = document.createElement("b");
  title.textContent = "Private. ";
  span.append(title, "Pages are saved in this browser, on this device, and never uploaded. A share link carries the page after the #, which browsers don't send to any server. ");
  const link = document.createElement("a");
  link.href = "https://omacus.com/privacy/";
  link.target = "_blank";
  link.rel = "noopener";
  link.textContent = "Privacy";
  span.append(link);
  return span;
}

let toastTimer = 0;
function toast(message, undo, ms) {
  const span = document.createElement("span");
  if (typeof message === "string") span.textContent = message;
  else span.append(message);
  el.toast.replaceChildren(span);
  if (undo) {
    const b = document.createElement("button");
    b.textContent = undo.label;
    b.addEventListener("click", () => {
      el.toast.hidden = true;
      undo.run();
    });
    el.toast.append(b);
  }
  el.toast.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (el.toast.hidden = true), ms ?? (undo ? 6000 : 3200));
}

// ---------------------------------------------------------------------------------
// Offline and installable: a service worker caches the app. The first time, a banner
// says it can be installed: an Install button where the browser offers one, and the
// Add to Home Screen steps on iPhone and iPad, which have no install prompt.

const INSTALL_DISMISSED = "install-dismissed";
let installPrompt = null;

function ios() {
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}

function showInstall() {
  const banner = $("[data-install-banner]");
  const dismissed = store.get(INSTALL_DISMISSED) === "1";
  const host = $("[data-install-text]");
  host.replaceChildren();
  let text = false;
  if (installPrompt) {
    const b = document.createElement("b");
    b.textContent = "Install Omacus. ";
    host.append(b, "It opens in its own window and works offline.");
    text = true;
  } else if (ios()) {
    const lead = document.createElement("b");
    lead.textContent = "Install Omacus: ";
    const step = document.createElement("b");
    step.textContent = "Add to Home Screen";
    host.append(lead, "tap ", shareIcon(), " then ", step, ". It works offline, like an app, and Safari keeps your pages.");
    text = true;
  }
  banner.hidden = EMBED || dismissed || standalone() || !text || state.active === "shared";
  $("[data-action=install]", banner).hidden = !installPrompt;
}

function dismissInstall() {
  store.set(INSTALL_DISMISSED, "1");
  $("[data-install-banner]").hidden = true;
}

addEventListener("beforeinstallprompt", (e) => {
  e.preventDefault();
  installPrompt = e;
  $("[data-install]").hidden = false;
  showInstall();
});
addEventListener("appinstalled", () => {
  installPrompt = null;
  $("[data-install]").hidden = true;
  $("[data-install-banner]").hidden = true;
  toast("Installed. Omacus now opens like an app and works offline.");
});

function standalone() {
  return matchMedia("(display-mode: standalone)").matches || navigator.standalone === true;
}

let applyingUpdate = false;
let waitingUpdate = null;

function offerUpdate(reg) {
  if (!navigator.serviceWorker.controller || !reg?.waiting) return;
  waitingUpdate = reg;
  const banner = $("[data-update-banner]");
  if (banner) banner.hidden = false;
}

function watchUpdate(reg) {
  const installed = (worker) => {
    worker?.addEventListener("statechange", () => {
      if (worker.state === "installed") offerUpdate(reg);
    });
  };
  reg.addEventListener("updatefound", () => installed(reg.installing));
  if (reg.waiting) offerUpdate(reg);
}

async function applyUpdate() {
  applyingUpdate = true;
  await flushSave();
  await storeDrained();
  const reg = waitingUpdate || (await navigator.serviceWorker.getRegistration());
  const waiting = reg?.waiting;
  if (!waiting) {
    location.reload();
    return;
  }
  waiting.postMessage({ type: "skip-waiting" });
  setTimeout(() => location.reload(), 1500);
}

async function goOffline() {
  if (!("serviceWorker" in navigator)) return;
  // Local previews serve files at fixed URLs, which the worker would keep serving
  // from its copy. There, remove any worker instead. `?sw=1` registers one anyway,
  // which is how an offline open is checked.
  const local = ["localhost", "127.0.0.1"].includes(location.hostname);
  if (local && !new URLSearchParams(location.search).has("sw")) {
    for (const reg of await navigator.serviceWorker.getRegistrations()) await reg.unregister();
    for (const key of await caches.keys()) await caches.delete(key);
    return;
  }
  try {
    const reg = await navigator.serviceWorker.register(scriptURL(new URL("sw.js", import.meta.url).href), { scope: "./" });
    navigator.serviceWorker.addEventListener("controllerchange", () => {
      if (applyingUpdate) location.reload();
    });
    // The first install takes over so this visit is enough for a later offline reload.
    // An update waits and offers Reload, after the open page has been saved.
    if (!navigator.serviceWorker.controller) {
      (reg.installing || reg.waiting)?.postMessage({ type: "skip-waiting" });
    }
    await navigator.serviceWorker.ready;
    watchUpdate(reg);
    const base = new URL("./", import.meta.url).href;
    const urls = [
      ...new Set([
        base,
        WORKER,
        ...performance
          .getEntriesByType("resource")
          .map((r) => r.name)
          .filter((u) => u.startsWith(location.origin)),
      ]),
    ];
    (reg.active ?? navigator.serviceWorker.controller)?.postMessage({ type: "cache", urls });
  } catch {
    // No offline support here (a private window, say); the app still works online.
  }
  showInstall();
}

// ---------------------------------------------------------------------------------
// Start.

function syncSnapshotPages() {
  const pages = [];
  for (const [id, text] of state.texts) {
    if (!id || id === "shared") continue;
    pages.push([id, text ?? ""]);
  }
  return pages;
}

async function takeSyncSnapshot() {
  if (pendingSave) {
    clearTimeout(pendingSave);
    pendingSave = null;
    applyingRemote = true;
    try {
      await saveSheet(state.active);
    } finally {
      applyingRemote = false;
    }
  }
  return {
    pages: syncSnapshotPages(),
    doc: docBytes && docBytes.length ? new Uint8Array(docBytes) : new Uint8Array(),
  };
}

async function backupBeforeRemote() {
  const version = document.querySelector('meta[name="omacus-version"]')?.content || "";
  let index = {};
  try {
    index = JSON.parse(store.get("index") || "{}");
  } catch {
    index = {};
  }
  const zip = zipBytes(backupEntries({
    version,
    library: index,
    settings,
    trash: loadTrash(),
    pages: libraryPages(),
  }));
  const backup = bytesToB64(zip);
  await store.set("sync-remote-backup", backup);
  if (store.get("sync-remote-backup") !== backup) throw new Error("The backup could not be saved.");
}

async function applyRemoteLibrary(nextDoc, pages) {
  await backupBeforeRemote();
  applyingRemote = true;
  try {
    // `nextDoc` is already the merged document. Folding it into the copy
    // already on this device would put a deleted line back.
    let doc = nextDoc && nextDoc.length ? nextDoc : (docBytes && docBytes.length ? docBytes : null);
    for (const [id, text] of pages) {
      if (!id || id === "shared") continue;
      const body = typeof text === "string" ? text : "";
      if (doc && doc.length && !isLock(body)) {
        let shown = "";
        try {
          shown = docSnapshot(doc, id) || "";
        } catch {
          shown = "";
        }
        if (shown !== body) {
          try {
            doc = docApply(doc, id, body);
          } catch {
            // The page text is still kept below.
          }
        }
      }
      if (!state.texts.has(id)) {
        state.texts.set(id, body);
        if (!pageIds(state.tree).includes(id)) state.tree.push({ type: "page", id });
        const taken = new Set([...state.handles.values()].map((handle) => handle.toLowerCase()));
        if (!state.handles.has(id)) state.handles.set(id, allocateHandle(heading(body), taken));
      } else if ((state.texts.get(id) ?? "") !== body) {
        state.texts.set(id, body);
        if (plains.has(id) && !isLock(body)) plains.set(id, body);
      }
    }
    if (doc && doc.length) {
      docBytes = doc;
      const encoded = bytesToB64(doc);
      docSaved = encoded;
      await store.set("library-doc", encoded);
    }
    for (const [id, text] of state.texts) {
      if (id && id !== "shared") await store.set("page:" + id, text ?? "");
    }
    syncOrder();
    if (state.active !== "shared" && !state.order.includes(state.active)) state.active = state.order[0] ?? state.active;
    saveIndex();
    renderSidebar();
    if (state.active && state.active !== "shared") showSheet();
    showDocConflicts();
  } finally {
    applyingRemote = false;
  }
}

async function start() {
  await storeReady();
  settings = loadSettings();
  applySettings();
  const persistNote = $("[data-persist-note]");
  if (persistNote) persistNote.hidden = EMBED || persistGranted();
  showBuildHash();
  if (!EMBED) showInstall();
  if (EMBED) {
    el.page.classList.add("embed");
    const open = $("[data-embed-open]");
    open.hidden = false;
    // Carry the edits along to the full app.
    open.addEventListener("click", () => (open.href = sharePageUrl(shareEncode(currentText()))));
  } else {
    load();
    const action = seedAction({
      pages: state.order.length,
      seeded: librarySeeded,
      trash: loadTrash().length,
    });
    if (action === "welcome-mark" || action === "mark") librarySeeded = true;
    if (action === "examples") installLibrary(freshLibrary(WELCOME));
    else if (action === "welcome" || action === "welcome-mark") addSheet(WELCOME);
    else if (action === "mark") saveIndex();
  }
  await init({ module_or_path: WASM });
  engineReady = true;
  calc = new Calc();
  if (!EMBED) await ensureLibraryDoc();
  if (typeof Worker !== "undefined") startWorker();
  el.loading.remove();
  await openShared();
  if (state.active !== "shared") showSheet();
  loadRates();
  if (!EMBED) goOffline();
  if (!EMBED) {
    const syncTest = location.hostname === "sync-test.omacus.com" || document.documentElement.dataset.syncTest === "1";
    await mountSync({
      syncTest,
      openSettings: (tab) => openSettings(tab),
      snapshot: takeSyncSnapshot,
      apply: applyRemoteLibrary,
      bytesToB64,
      b64ToBytes,
      docUnion,
      storeGet: (key) => store.get(key),
      storeSet: (key, value) => store.set(key, value),
      storeRemove: (key) => store.remove(key),
      onEdit: (fn) => {
        syncTouched = fn;
      },
    });
  }
  if (matchMedia("(min-width: 761px)").matches) view.focus();
}

start().catch((e) => {
  console.error(e);
  if (typeof globalThis.omacusShowStored === "function") globalThis.omacusShowStored();
  else if (el.loading) el.loading.textContent = "Omacus couldn't start. Your pages are still in this browser.";
});
