// omacus.com/app: the notepad calculator in the browser. The math is the desktop
// app's own engine, compiled to WebAssembly (pkg/, built from web/ in the repo).
// Sheets live in this browser's localStorage; a share link carries a sheet in the
// part after "#", which browsers never send to the server.

import init, {
  Calc, suggest, shareEncode, shareDecode, layoutAnswers, realignMap, pageMarks, previewHtml, variables, wordKind,
} from "./pkg/omacus_web.js";
import {
  EditorState, StateField, StateEffect, RangeSetBuilder, Prec, EditorView, Decoration, WidgetType, keymap, hoverTooltip,
  defaultKeymap, history as undoHistory, historyKeymap, autocompletion, acceptCompletion, closeCompletion,
  moveCompletionSelection, startCompletion, completionStatus,
} from "./editor.js";

const WASM = "/app/pkg/omacus_web_bg.wasm";
const WORKER = new URL("./worker.js", import.meta.url).href;
const PREFIX = "omacus:";
const RATES_URL = "https://open.er-api.com/v6/latest/USD";
const RATES_MAX_AGE = 12 * 60 * 60 * 1000;
const KEYWORDS = new Set(["prev", "sum", "total", "avg", "average", "mean", "min", "max", "today", "now",
  "tomorrow", "yesterday", "plus", "minus", "times", "mod"]);
const HINT_IDLE = 600;

const WELCOME = `# Welcome to Omacus
// Write math the way you'd write a note. Answers appear on the right.

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
  workspace: $("[data-workspace]"),
  editor: $("[data-editor]"),
  preview: $("[data-preview]"),
  previewTitle: $("[data-preview-title]"),
  previewBody: $("[data-preview-body]"),
  settings: $("[data-settings]"),
  pagePreview: $("[data-page-preview]"),
  pagePreviewBody: $("[data-page-preview-body]"),
};

// pageMarks and variables throw until init() finishes. The editor is created first.
let engineReady = false;

let calc;
let results = [];
// The text `results` were worked out for (the editor may be ahead of it), and whether
// they're all of that text's answers, fresh (not some still on their way).
let resultsText = "";
let resultsExact = false;
const state = { order: [], active: null, texts: new Map(), shared: null };

// ---------------------------------------------------------------------------------
// Storage: one localStorage entry per sheet, plus the order and the active one.

const store = {
  get(key) {
    try {
      return localStorage.getItem(PREFIX + key);
    } catch {
      return null;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(PREFIX + key, value);
      return true;
    } catch {
      toast("Couldn't save: this browser's storage is full or turned off.");
      return false;
    }
  },
  remove(key) {
    try {
      localStorage.removeItem(PREFIX + key);
    } catch {}
  },
};

// Same built-in colors as the desktop app. Auto follows this device.
const THEMES = {
  light: { dark: false, bg: "#ffffff", side: "#f6f8fa", fg: "#24292f", dim: "#6e7781", comment: "#8c959f", accent: "#0969da", variable: "#cf222e", defined: "#1a7f37", keyword: "#8250df", number: "#1f2328", sel: "#ddf4ff", hover: "#eaeef2", line: "#d0d7de", line2: "#afb8c1" },
  dark: { dark: true, bg: "#1a1b26", side: "#16161e", fg: "#c0caf5", dim: "#787c99", comment: "#565f89", accent: "#7aa2f7", variable: "#bb9af7", defined: "#9ece6a", keyword: "#7dcfff", number: "#e0e6ff", sel: "#33467c", hover: "#24283b", line: "#2a2e45", line2: "#3b4261" },
  paper: { dark: false, bg: "#f6f1e7", side: "#efe6d6", fg: "#3c3226", dim: "#8a7b68", comment: "#a89880", accent: "#b45309", variable: "#9f1239", defined: "#1d4ed8", keyword: "#0f766e", number: "#2a2118", sel: "#eadcc4", hover: "#e7dcc8", line: "#e0d3bc", line2: "#c9b89a" },
  dusk: { dark: true, bg: "#1c2433", side: "#151b28", fg: "#d7e0f0", dim: "#8b9bb3", comment: "#64748b", accent: "#8eb4ff", variable: "#f0a27a", defined: "#c4b5fd", keyword: "#7ddea0", number: "#f1f5ff", sel: "#2c3d5c", hover: "#243044", line: "#2c3a50", line2: "#3d4e6b" },
};
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
    return { theme, hideAnswers: !!saved?.hideAnswers };
  } catch {
    return { theme: "auto", hideAnswers: false };
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

function applySettings() {
  const t = THEMES[themeName()];
  const root = document.documentElement;
  const set = (name, value) => root.style.setProperty(name, value);
  set("--app-bg", t.bg);
  set("--app-side", t.side);
  set("--app-fg", t.fg);
  set("--app-dim", t.dim);
  set("--app-comment", t.comment);
  set("--app-accent", t.accent);
  set("--app-var", t.variable);
  set("--app-def", t.defined);
  set("--app-kw", t.keyword);
  set("--app-num", t.number);
  set("--app-sel", t.sel);
  set("--app-hover", t.hover);
  set("--text", t.fg);
  set("--text-2", t.dim);
  set("--line", t.line);
  set("--line-2", t.line2);
  set("--panel", t.side);
  set("--panel-2", t.hover);
  set("--bg", t.bg);
  root.style.colorScheme = t.dark ? "dark" : "light";
  document.body.classList.toggle("answers-inline", settings.hideAnswers);
  const meta = document.querySelector('meta[name="theme-color"]');
  if (meta) meta.content = t.bg;
}

function updateThemeNote() {
  const note = $("[data-theme-note]");
  if (note) note.textContent = THEME_NOTES[settings.theme] || THEME_NOTES.auto;
}

function showSettingsTab(name) {
  const tab = name === "about" ? "about" : "settings";
  for (const button of document.querySelectorAll("[data-settings-tab]")) {
    button.setAttribute("aria-selected", String(button.dataset.settingsTab === tab));
  }
  for (const panel of document.querySelectorAll("[data-settings-panel]")) {
    panel.hidden = panel.dataset.settingsPanel !== tab;
  }
  const title = document.getElementById("settings-title");
  if (title) title.textContent = tab === "about" ? "About" : "Settings";
}

function openSettings(tab = "settings") {
  closeMenu();
  if (!el.settings) return;
  const select = $("[data-setting=theme]", el.settings);
  const hide = $("[data-setting=hide-answers]", el.settings);
  if (select) select.value = settings.theme;
  if (hide) hide.checked = settings.hideAnswers;
  updateThemeNote();
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

// Older builds stored each page under `sheet:`. Read those once and write `page:`.
function migratePageKeys() {
  let legacy = [];
  try {
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i);
      if (key?.startsWith(PREFIX + "sheet:")) legacy.push(key);
    }
  } catch {
    return;
  }
  for (const key of legacy) {
    const id = key.slice((PREFIX + "sheet:").length);
    if (store.get("page:" + id) == null) {
      const value = localStorage.getItem(key);
      if (value != null) store.set("page:" + id, value);
    }
    store.remove("sheet:" + id);
  }
}

function load() {
  migratePageKeys();
  let index = {};
  try {
    index = JSON.parse(store.get("index") ?? "{}");
  } catch {}
  state.order = Array.isArray(index.order) ? index.order.filter((id) => store.get("page:" + id) !== null) : [];
  state.texts = new Map(state.order.map((id) => [id, store.get("page:" + id)]));
  state.active = state.order.includes(index.active) ? index.active : state.order[0] ?? null;
}

function saveIndex() {
  store.set("index", JSON.stringify({ order: state.order, active: state.active }));
}

let persistAsked = false;
function saveSheet(id) {
  if (id === "shared") return;
  store.set("page:" + id, state.texts.get(id) ?? "");
  // Ask the browser to keep this data rather than clearing it to free space.
  if (!persistAsked) {
    persistAsked = true;
    navigator.storage?.persist?.().catch(() => {});
  }
}

function newId() {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
}

function addSheet(text, { select = true } = {}) {
  const id = newId();
  state.texts.set(id, text);
  state.order.unshift(id);
  saveSheet(id);
  if (select) state.active = id;
  saveIndex();
  return id;
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
addEventListener("storage", (e) => {
  if (EMBED || !e.key?.startsWith(PREFIX)) return;
  const typing = view.hasFocus && pendingSave;
  const before = currentText();
  const mine = state.active;
  load();
  // Each tab keeps its own open sheet.
  if (mine === "shared" || state.order.includes(mine)) state.active = mine;
  renderSidebar();
  if (!typing && currentText() !== before) showSheet();
});

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

function onInput() {
  editedAt = performance.now();
  armHintTimer();
  const text = view.state.doc.toString();
  if (state.active === "shared") {
    state.shared.text = text;
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
  closePagePreview();
  clearPreview();
  if (shownId !== null) editorStates.set(shownId, view.state);
  shownId = state.active;
  results = [];
  resultsText = "";
  resultsExact = false;
  const text = currentText();
  let es = editorStates.get(state.active);
  if (!es || es.doc.toString() !== text) es = EditorState.create({ doc: text, extensions: editorExtensions });
  view.setState(es);
  el.shared.hidden = EMBED || state.active !== "shared";
  showInstall();
  evaluate();
  renderSidebar();
  syncEmpty();
}

function select(id) {
  clearPreview();
  if (id === state.active) {
    showSheet();
    view.focus();
    setDrawer(false);
    return;
  }
  flushSave();
  state.active = id;
  if (id !== "shared") saveIndex();
  showSheet();
  view.focus();
  setDrawer(false);
}

function flushSave() {
  if (pendingSave) {
    clearTimeout(pendingSave);
    pendingSave = null;
    saveSheet(state.active);
  }
}
addEventListener("pagehide", flushSave);

function renderSidebar() {
  const items = [];
  if (state.shared) items.push(["shared", state.shared.text, true]);
  for (const id of state.order) items.push([id, state.texts.get(id), false]);
  el.sheets.replaceChildren(
    ...items.map(([id, text, shared]) => {
      const b = document.createElement("button");
      b.type = "button";
      b.dataset.id = id;
      const name = document.createElement("span");
      name.textContent = heading(text);
      b.title = heading(text);
      if (id === state.active && !preview) b.setAttribute("aria-current", "true");
      b.append(name);
      if (shared) {
        const tag = document.createElement("i");
        tag.className = "shared-tag";
        tag.textContent = "shared";
        b.append(tag);
      }
      b.addEventListener("click", () => select(id));
      return b;
    }),
  );
  if (!preview) {
    el.title.textContent = title(currentText());
    document.title = `${title(currentText())} · Omacus`;
  }
  renderDeleted();
}

const TRASH_KEY = PREFIX + "trash";
const TRASH_CAP = 30;

function loadTrash() {
  try {
    const raw = JSON.parse(localStorage.getItem(TRASH_KEY) || "[]");
    return Array.isArray(raw) ? raw.filter((t) => t && t.id && typeof t.text === "string") : [];
  } catch {
    return [];
  }
}

function saveTrash(items) {
  localStorage.setItem(TRASH_KEY, JSON.stringify(items.slice(0, TRASH_CAP)));
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
    b.textContent = heading(item.text);
    b.title = heading(item.text);
    if (preview?.id === item.id) b.setAttribute("aria-current", "true");
    b.addEventListener("click", () => showPreview(item));
    el.deleted.append(b);
  }
}

let preview = null;

function showPreview(item) {
  closePagePreview();
  preview = item;
  if (el.preview) el.preview.hidden = false;
  if (el.previewTitle) el.previewTitle.textContent = heading(item.text);
  if (el.previewBody) {
    el.previewBody.hidden = false;
    el.previewBody.textContent = item.text;
  }
  if (el.editor) el.editor.hidden = true;
  if (el.title) el.title.textContent = heading(item.text);
  document.title = `${title(item.text)} · Omacus`;
  renderSidebar();
}

function clearPreview() {
  if (!preview) return;
  preview = null;
  if (el.preview) el.preview.hidden = true;
  if (el.previewBody) el.previewBody.hidden = true;
  if (el.editor && !pagePreviewOpen) el.editor.hidden = false;
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
  if (pagePreviewOpen) {
    closePagePreview();
    view.focus();
  } else {
    openPagePreview();
  }
}

function refreshPagePreview() {
  if (!pagePreviewOpen || !el.pagePreviewBody || !engineReady) return;
  const text = view.state.doc.toString();
  const answers = view.state.field(sheetField).answers;
  el.pagePreviewBody.innerHTML = previewHtml(text, JSON.stringify(answers));
  for (const pill of el.pagePreviewBody.querySelectorAll(".pill")) {
    pill.title = "Click to copy";
    pill.addEventListener("click", () => {
      const value = pill.dataset.value || pill.textContent;
      copy(value).then(() => toast(`Copied <b>${esc(value)}</b>`));
    });
  }
}

function printPagePreview() {
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
  state.texts.set(id, item.text);
  state.order.unshift(id);
  saveSheet(id);
  state.active = id;
  saveIndex();
  showSheet();
  toast(`Restored “${esc(title(item.text))}”.`);
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
    span.innerHTML =
      (this.r.color ? `<span class="swatch" style="background:${esc(this.r.color)}"></span>` : "") +
      `<b>${esc(this.r.display)}</b>`;
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
    toast(`Copied <b>${esc(display)}</b>`);
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
  const found = JSON.parse(suggest(doc.slice(0, ctx.pos), doc));
  if (!found) return null;
  return {
    from: found.start,
    filter: false,
    options: found.items.map((item) => ({
      label: item.text,
      detail: `${item.kind === "zone" ? "time zone" : item.kind} · ${item.detail}`,
      type: item.kind,
    })),
  };
}

// Enter accepts only after Ctrl+Space. Typing still opens the list, and Tab
// accepts it, but Enter stays a newline so "3 km in" cannot become "inch".
let explicitCompletion = false;
let armingExplicit = false;

function hoverSource(view, pos) {
  if (!calc) return null;
  const found = JSON.parse(calc.hoverValue(view.state.doc.toString(), pos));
  if (!found) return null;
  return {
    pos: found.start,
    end: found.end,
    above: true,
    create() {
      const dom = document.createElement("div");
      dom.className = "hover-value";
      dom.textContent = found.value;
      return { dom };
    },
  };
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
      return closeCompletion(view);
    },
  },
  {
    key: "Ctrl-Space",
    run(view) {
      armingExplicit = true;
      const opened = startCompletion(view);
      armingExplicit = false;
      explicitCompletion = completionStatus(view) === "active";
      return opened;
    },
  },
  {
    key: "Enter",
    run(view) {
      if (!explicitCompletion || completionStatus(view) !== "active") return false;
      explicitCompletion = false;
      return acceptCompletion(view);
    },
  },
  { key: "Mod-Shift-s", run: () => (action("copy"), true) },
  { key: "Mod-Shift-m", run: () => (action("markdown"), true) },
];

const editorExtensions = [
  undoHistory(),
  Prec.highest(keymap.of(editorKeys)),
  keymap.of([...defaultKeymap, ...historyKeymap]),
  EditorView.lineWrapping,
  sheetField,
  autocompletion({ override: [completions], defaultKeymap: false, icons: false }),
  hoverTooltip(hoverSource, { hoverTime: 250 }),
  EditorView.updateListener.of((update) => {
    if (!armingExplicit && completionStatus(update.view) !== "active") explicitCompletion = false;
    if (update.docChanged) onInput();
  }),
  EditorView.contentAttributes.of({ spellcheck: "false", autocapitalize: "off", autocorrect: "off", "aria-label": "Page" }),
];

const view = new EditorView({ parent: el.host, state: EditorState.create({ doc: "", extensions: editorExtensions }) });

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

function startWorker() {
  worker?.terminate();
  try {
    worker = new Worker(WORKER, { type: "module" });
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
  for (const [id, c] of copies) worker.postMessage({ type: "copy", seq: id, sheet: c.sheet, text: c.text });
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
    if (worker) {
      latest = { seq: ++seq, sheet, text };
      worker.postMessage({ type: "sheet", ...latest });
    } else if (calc) {
      show(JSON.parse(calc.evaluateSheet(sheet, text)), text, true);
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
  worker.postMessage({ type: "copy", seq: id, sheet: state.active ?? "", text });
  return new Promise((resolve) => copies.set(id, { sheet: state.active ?? "", text, resolve })).then(lay);
}

// Without a worker: the answers worked out here.
function pageLayout(text) {
  return calc.evaluate(text);
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
    for (const [from, to, name] of tokens(line.slice(0, marks.comment ?? line.length), vars, marks.define)) {
      paint(from, to, name);
    }
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

function esc(s) {
  return s.replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);
}


// ---------------------------------------------------------------------------------
// Share links: "#v1." then the sheet, deflated and base64url-encoded, by the engine
// (the desktop app makes and reads exactly the same links).

async function share() {
  const text = currentText();
  const url = `${location.origin}/app#${shareEncode(text)}`;
  await copy(url);
  const n = url.length.toLocaleString();
  if (url.length > 8000) toast(`Link copied (${n} characters). <small>Long links can be cut off by some apps; for a long page, Copy as Markdown table is safer.</small>`);
  else if (url.length > 2000) toast(`Link copied (${n} characters). <small>Fine for chat apps; some email and SMS may cut it.</small>`);
  else toast("Link copied. <small>Anyone with it can open this page.</small>");
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
  const link = `omacus://open#${shareEncode(currentText())}`;
  let left = false;
  const away = () => (left = true);
  addEventListener("blur", away, { once: true });
  document.addEventListener("visibilitychange", away, { once: true });
  location.href = link;
  setTimeout(() => {
    removeEventListener("blur", away);
    if (!left) {
      toast('Nothing opened it. <a href="/download/">Get Omacus</a> <small>(0.1.10 or newer opens links)</small>');
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
  const text = currentText();
  if (name === "copy") {
    await copyLater(laidOut(text, false));
    toast("Page copied with answers.");
  } else if (name === "markdown") {
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
    toast("In the print dialog, choose <b>Save as PDF</b>.");
    printPagePreview();
  } else if (name === "new") {
    flushSave();
    addSheet("");
    showSheet();
    view.focus();
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
  } else if (name === "restore-trash") {
    if (preview) restoreTrashed(preview.id);
  } else if (name === "purge-trash") {
    if (!preview || !el.confirm) return;
  el.confirmText.textContent = `Delete “${heading(preview.text)}” forever? This can't be undone.`;
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
    toast(
      "<b>Private.</b> Pages are saved in this browser, on this device, and never uploaded. " +
        "A share link carries the page after the #, which browsers don't send to any server. " +
        '<a href="/privacy/" target="_blank" rel="noopener">Privacy</a>',
      null,
      7000,
    );
  }
}

function deleteSheet() {
  if (state.active === "shared") {
    state.shared = null;
    state.active = state.order[0] ?? addSheet("");
    return showSheet();
  }
  const id = state.active;
  const at = state.order.indexOf(id);
  const text = state.texts.get(id);
  const name = title(text);
  if (!el.confirm) return;
  el.confirmText.textContent = `Delete “${name}”? You can restore it from Recently deleted.`;
  el.confirm.hidden = false;
  const yes = $("[data-confirm-yes]");
  if (yes) yes.textContent = "Delete";
  el.confirm.dataset.mode = "sheet";
  el.confirm.dataset.id = id;
  el.confirm.dataset.at = String(at);
}

function commitDelete() {
  const id = el.confirm?.dataset.id;
  if (!id) return;
  if (el.confirm.dataset.mode === "purge") {
    el.confirm.hidden = true;
    const items = loadTrash();
    const item = items.find((t) => t.id === id);
    saveTrash(items.filter((t) => t.id !== id));
    if (preview?.id === id) clearPreview();
    renderDeleted();
    if (item) toast(`Deleted “${esc(heading(item.text))}” forever.`);
    return;
  }
  el.confirm.hidden = true;
  const at = Number(el.confirm.dataset.at);
  const text = state.texts.get(id);
  if (text == null) return;
  const trash = loadTrash().filter((t) => t.id !== id);
  trash.unshift({ id, text });
  saveTrash(trash);
  flushSave();
  state.order.splice(at, 1);
  state.texts.delete(id);
  store.remove("page:" + id);
  store.remove("sheet:" + id);
  state.active = state.order[Math.min(at, state.order.length - 1)] ?? addSheet("");
  saveIndex();
  showSheet();
  toast(`Deleted “${esc(title(text))}”.`, {
    label: "Undo",
    run() {
      saveTrash(loadTrash().filter((t) => t.id !== id));
      state.texts.set(id, text);
      state.order.splice(at, 0, id);
      saveSheet(id);
      state.active = id;
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

document.addEventListener("click", (e) => {
  const button = e.target.closest?.("[data-action]");
  const tab = e.target.closest?.("[data-settings-tab]");
  if (tab) showSettingsTab(tab.dataset.settingsTab);
  if (button) action(button.dataset.action);
  if (e.target.closest?.("[data-menu-button]")) {
    el.menu.hidden = !el.menu.hidden;
    el.menuButton.setAttribute("aria-expanded", String(!el.menu.hidden));
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

function closeMenu() {
  el.menu.hidden = true;
  el.menuButton.setAttribute("aria-expanded", "false");
}

addEventListener(
  "keydown",
  (e) => {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === "b") {
      e.preventDefault();
      if (matchMedia("(max-width: 760px)").matches) {
        setDrawer(!el.page.classList.contains("drawer"));
      } else if (el.workspace) {
        setSide(el.workspace.classList.contains("collapsed"));
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
    if (el.settings && !el.settings.hidden) {
      closeSettings();
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
    closeMenu();
    setDrawer(false);
  }
});
$("[data-confirm-yes]")?.addEventListener("click", commitDelete);
$("[data-confirm-no]")?.addEventListener("click", () => {
  if (el.confirm) el.confirm.hidden = true;
});

// ---------------------------------------------------------------------------------
// Export and import: plain .txt files in a .zip, like the desktop app's sheets folder.

function exportZip() {
  flushSave();
  const used = new Set();
  const files = state.order.map((id) => {
    let name = title(state.texts.get(id)).replace(/[\\/:*?"<>|…]/g, "").trim() || "Untitled";
    let unique = name;
    for (let n = 2; used.has(unique.toLowerCase()); n++) unique = `${name} ${n}`;
    used.add(unique.toLowerCase());
    return { name: `${unique}.txt`, data: new TextEncoder().encode(state.texts.get(id)) };
  });
  const blob = zip(files);
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = `omacus-pages-${new Date().toISOString().slice(0, 10)}.zip`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  toast(`Exported ${files.length} page${files.length === 1 ? "" : "s"}.`);
}

el.importInput.addEventListener("change", async () => {
  let count = 0;
  for (const file of el.importInput.files) {
    const texts = file.name.toLowerCase().endsWith(".zip") ? await unzip(file) : [await file.text()];
    for (const text of texts) {
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

const CRC = new Uint32Array(256).map((_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

// A zip of stored (uncompressed) files: small sheets need nothing more.
function zip(files) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const f of files) {
    const name = new TextEncoder().encode(f.name);
    const crc = crc32(f.data);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, 20, true);
    local.setUint16(6, 0x0800, true); // UTF-8 names
    local.setUint32(14, crc, true);
    local.setUint32(18, f.data.length, true);
    local.setUint32(22, f.data.length, true);
    local.setUint16(26, name.length, true);
    parts.push(local, name, f.data);
    const entry = new DataView(new ArrayBuffer(46));
    entry.setUint32(0, 0x02014b50, true);
    entry.setUint16(4, 20, true);
    entry.setUint16(6, 20, true);
    entry.setUint16(8, 0x0800, true);
    entry.setUint32(16, crc, true);
    entry.setUint32(20, f.data.length, true);
    entry.setUint32(24, f.data.length, true);
    entry.setUint16(28, name.length, true);
    entry.setUint32(42, offset, true);
    central.push(entry, name);
    offset += 30 + name.length + f.data.length;
  }
  const size = central.reduce((n, p) => n + p.byteLength, 0);
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, files.length, true);
  end.setUint16(10, files.length, true);
  end.setUint32(12, size, true);
  end.setUint32(16, offset, true);
  return new Blob([...parts, ...central, end], { type: "application/zip" });
}

// The .txt files in a zip, stored or deflated.
async function unzip(file) {
  const buf = new Uint8Array(await file.arrayBuffer());
  const view = new DataView(buf.buffer);
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65557); i--) {
    if (view.getUint32(i, true) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) return [];
  const texts = [];
  let p = view.getUint32(eocd + 16, true);
  for (let n = view.getUint16(eocd + 10, true); n > 0 && p + 46 <= buf.length; n--) {
    if (view.getUint32(p, true) !== 0x02014b50) break;
    const method = view.getUint16(p + 10, true);
    const size = view.getUint32(p + 20, true);
    const nameLen = view.getUint16(p + 28, true);
    const extra = view.getUint16(p + 30, true);
    const note = view.getUint16(p + 32, true);
    const local = view.getUint32(p + 42, true);
    const name = new TextDecoder().decode(buf.subarray(p + 46, p + 46 + nameLen));
    p += 46 + nameLen + extra + note;
    if (!name.toLowerCase().endsWith(".txt") || name.split("/").pop().startsWith(".")) continue;
    const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true);
    const data = buf.subarray(start, start + size);
    if (method === 0) texts.push(new TextDecoder().decode(data));
    else if (method === 8) {
      const out = new Blob([data]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
      texts.push(await new Response(out).text());
    }
  }
  return texts;
}

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

let toastTimer = 0;
function toast(html, undo, ms) {
  el.toast.innerHTML = `<span>${html}</span>`;
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

const INSTALL_DISMISSED = "omacus:install-dismissed";
const SHARE_ICON =
  '<svg class="ios-share" viewBox="0 0 24 24" aria-label="Share"><path d="M12 15V3M8 7l4-4 4 4"/><path d="M7 10H5v11h14V10h-2"/></svg>';
let installPrompt = null;

function ios() {
  return /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
}

function showInstall() {
  const banner = $("[data-install-banner]");
  let dismissed = false;
  try {
    dismissed = localStorage.getItem(INSTALL_DISMISSED) === "1";
  } catch {}
  const text = installPrompt
    ? "<b>Install Omacus.</b> It opens in its own window and works offline."
    : ios()
      ? `<b>Install Omacus:</b> tap ${SHARE_ICON} then <b>Add to Home Screen</b>. It works offline, like an app, and Safari keeps your pages.`
      : null;
  banner.hidden = EMBED || dismissed || standalone() || !text || state.active === "shared";
  if (text) $("[data-install-text]").innerHTML = text;
  $("[data-action=install]", banner).hidden = !installPrompt;
}

function dismissInstall() {
  try {
    localStorage.setItem(INSTALL_DISMISSED, "1");
  } catch {}
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
    const reg = await navigator.serviceWorker.register("/app/sw.js", { scope: "/app/" });
    await navigator.serviceWorker.ready;
    // What this page loaded before the worker took over, for offline use.
    const urls = [
      ...new Set([
        `${location.origin}/app/`,
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

async function start() {
  if (EMBED) {
    el.page.classList.add("embed");
    const open = $("[data-embed-open]");
    open.hidden = false;
    // Carry the edits along to the full app.
    open.addEventListener("click", () => (open.href = `/app#${shareEncode(currentText())}`));
  } else {
    load();
    if (!state.order.length) addSheet(WELCOME);
  }
  await init({ module_or_path: WASM });
  engineReady = true;
  calc = new Calc();
  if (typeof Worker !== "undefined") startWorker();
  el.loading.remove();
  await openShared();
  if (state.active !== "shared") showSheet();
  loadRates();
  if (!EMBED) goOffline();
  if (matchMedia("(min-width: 761px)").matches) view.focus();
}

start().catch((e) => {
  el.loading.textContent = "Omacus couldn't start in this browser. Try a current Chrome, Firefox, Safari or Edge.";
  console.error(e);
});
