// omacus.com/app: the notepad calculator in the browser. The math is the desktop
// app's own engine, compiled to WebAssembly (pkg/, built from web/ in the repo).
// Sheets live in this browser's localStorage; a share link carries a sheet in the
// part after "#", which browsers never send to the server.

import init, { Calc, suggest, shareEncode, shareDecode, layoutAnswers } from "./pkg/omacus_web.js";
import {
  EditorState, StateField, StateEffect, RangeSetBuilder, Prec, EditorView, Decoration, WidgetType, keymap,
  defaultKeymap, history as undoHistory, historyKeymap, autocompletion, acceptCompletion, closeCompletion, moveCompletionSelection,
} from "./editor.js";

const WASM = "/app/pkg/omacus_web_bg.wasm";
const WORKER = new URL("./worker.js", import.meta.url).href;
const PREFIX = "omacus:";
const RATES_URL = "https://open.er-api.com/v6/latest/USD";
const RATES_MAX_AGE = 12 * 60 * 60 * 1000;
const KEYWORDS = new Set(["prev", "sum", "total", "avg", "average", "mean", "today", "now",
  "tomorrow", "yesterday", "plus", "minus", "times", "mod"]);

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

// Click an answer to copy it. Share link sends this sheet to anyone.
// Private: your sheets are saved in this browser only. Nothing is uploaded.
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
};

let calc;
let results = [];
// The text `results` were worked out for (the editor may be ahead of it).
let resultsText = "";
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

function load() {
  let index = {};
  try {
    index = JSON.parse(store.get("index") ?? "{}");
  } catch {}
  state.order = Array.isArray(index.order) ? index.order.filter((id) => store.get("sheet:" + id) !== null) : [];
  state.texts = new Map(state.order.map((id) => [id, store.get("sheet:" + id)]));
  state.active = state.order.includes(index.active) ? index.active : state.order[0] ?? null;
}

function saveIndex() {
  store.set("index", JSON.stringify({ order: state.order, active: state.active }));
}

let persistAsked = false;
function saveSheet(id) {
  if (id === "shared") return;
  store.set("sheet:" + id, state.texts.get(id) ?? "");
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

function title(text) {
  const line = (text ?? "").split("\n").map((l) => l.trim()).find((l) => l) ?? "";
  const t = line.replace(/^[#/\s]+/, "").trim();
  if (!t) return "Untitled";
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
function onInput() {
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
}

// Each sheet keeps its own editor state while the page is open, so undo stays per sheet.
const editorStates = new Map();
let shownId = null;

function showSheet() {
  if (shownId !== null) editorStates.set(shownId, view.state);
  shownId = state.active;
  results = [];
  resultsText = "";
  const text = currentText();
  let es = editorStates.get(state.active);
  if (!es || es.doc.toString() !== text) es = EditorState.create({ doc: text, extensions: editorExtensions });
  view.setState(es);
  el.shared.hidden = EMBED || state.active !== "shared";
  showInstall();
  evaluate();
  renderSidebar();
}

function select(id) {
  if (id === state.active) return;
  flushSave();
  state.active = id;
  if (id !== "shared") saveIndex();
  showSheet();
  view.focus();
  el.page.classList.remove("drawer");
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
      if (id === state.active) b.setAttribute("aria-current", "true");
      const name = document.createElement("span");
      name.textContent = title(text);
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
  el.title.textContent = title(currentText());
  document.title = `${title(currentText())} · Omacus`;
}

// ---------------------------------------------------------------------------------
// The editor: CodeMirror (editor.js, bundled by site/tools/editor). Lines wrap at the
// full width. Each answer floats right at the end of its line, so it sits beside a
// short line and drops to its own row, still right-aligned, below a long one.

const setResults = StateEffect.define();

class Answer extends WidgetType {
  constructor(r) {
    super();
    this.r = r;
  }
  eq(other) {
    return other.r.display === this.r.display && other.r.color === this.r.color;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "res";
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
  constructor(text) {
    super();
    this.text = text;
  }
  eq(other) {
    return other.text === this.text;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "res note";
    span.textContent = this.text;
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
    const spans = r?.kind === "header" ? [[0, line.length, "t-head"]] : tokens(line.text, vars);
    for (const [from, to, cls] of spans) {
      if (to > from) out.add(line.from + from, line.from + to, Decoration.mark({ class: cls }));
    }
    if (r?.display) out.add(line.to, line.to, Decoration.widget({ widget: new Answer(r), side: 1 }));
    else if (r?.note) out.add(line.to, line.to, Decoration.widget({ widget: new Note(r.note), side: 1 }));
  }
  return out.finish();
}

// Answers for one text moved to where their lines are in another, while the new
// answers are on their way: lines unchanged from the top and from the bottom keep
// theirs, and the lines that changed have none yet (by position alone, inserting a
// line showed every answer below it one line off). The engine's `realign`, in JS.
function realign(oldLines, answers, newLines) {
  let top = 0;
  while (top < oldLines.length && top < newLines.length && oldLines[top] === newLines[top]) top++;
  let bottom = 0;
  while (
    bottom < oldLines.length - top &&
    bottom < newLines.length - top &&
    oldLines[oldLines.length - 1 - bottom] === newLines[newLines.length - 1 - bottom]
  )
    bottom++;
  return newLines.map((_, i) =>
    i < top ? answers[i] : i >= newLines.length - bottom ? answers[i + oldLines.length - newLines.length] : undefined,
  );
}

const sheetField = StateField.define({
  create: (st) => ({ answers: [], deco: decorate(st.doc, []) }),
  update(value, tr) {
    let answers = value.answers;
    const fresh = tr.effects.some((e) => e.is(setResults));
    for (const e of tr.effects) if (e.is(setResults)) answers = e.value;
    if (!fresh && tr.docChanged) {
      answers = realign(tr.startState.doc.toString().split("\n"), answers, tr.state.doc.toString().split("\n"));
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
  return { from: found.start, options: found.items.map((label) => ({ label })), filter: false };
}

// As in the desktop app: Tab accepts a suggestion, Enter is always a new line.
const editorKeys = [
  { key: "Tab", run: acceptCompletion },
  { key: "ArrowDown", run: moveCompletionSelection(true) },
  { key: "ArrowUp", run: moveCompletionSelection(false) },
  { key: "Escape", run: closeCompletion },
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
  EditorView.updateListener.of((u) => u.docChanged && onInput()),
  EditorView.contentAttributes.of({ spellcheck: "false", autocapitalize: "off", autocorrect: "off", "aria-label": "Sheet" }),
];

const view = new EditorView({ parent: el.host, state: EditorState.create({ doc: "", extensions: editorExtensions }) });

// The calculation runs in a worker (worker.js), so the page never waits on it. Only
// the newest text matters: while one sheet is being worked out, later edits wait and
// the latest is sent next. One that's taken long by the time newer text arrives is
// abandoned: the worker is replaced and starts on the newest text (it can't be
// interrupted mid-calculation). Without workers, the page calculates itself.
const RESTART_MS = 1000;
let worker = null;
let workerReady = false;
let ratesJson = null;
let busy = null; // { seq, at } of what the worker is working on
let waiting = null; // the newest text, not yet sent
let seq = 0;
const layouts = new Map(); // seq -> resolve, for copies worked out in the worker

function startWorker() {
  worker?.terminate();
  workerReady = false;
  busy = null;
  for (const resolve of layouts.values()) resolve(null);
  layouts.clear();
  try {
    worker = new Worker(WORKER, { type: "module" });
  } catch {
    worker = null;
    return;
  }
  worker.onmessage = (e) => onWorker(e.data);
  worker.onerror = () => {
    // A worker that can't start (an old browser without module workers): calculate here.
    worker?.terminate();
    worker = null;
    workerReady = false;
    evaluate();
  };
  worker.postMessage({ type: "init", wasm: new URL(WASM, location.href).href, rates: ratesJson });
}

function onWorker(m) {
  if (m.type === "ready") {
    workerReady = true;
    send();
  } else if (m.type === "sheet") {
    busy = null;
    if (m.sheet === (state.active ?? "")) show(JSON.parse(m.results), m.text);
    send();
  } else if (m.type === "layout") {
    layouts.get(m.seq)?.(m);
    layouts.delete(m.seq);
  } else if (m.type === "error") {
    console.error("Omacus worker:", m.message);
    busy = null;
    send();
  }
}

function send() {
  if (!worker || !workerReady || busy || !waiting) return;
  busy = { seq: waiting.seq, at: performance.now() };
  worker.postMessage({ type: "sheet", ...waiting });
  waiting = null;
}

// Answers worked out for `text`, lined up with what the editor shows now.
function show(answers, text) {
  const now = view.state.doc.toString();
  results = text === now ? answers : realign(text.split("\n"), answers, now.split("\n"));
  resultsText = text;
  view.dispatch({ effects: setResults.of(results) });
}

let frame = 0;
function evaluate() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => {
    const text = view.state.doc.toString();
    const sheet = state.active ?? "";
    if (worker) {
      waiting = { seq: ++seq, sheet, text };
      if (busy && performance.now() - busy.at > RESTART_MS) startWorker();
      else send();
    } else if (calc) {
      show(JSON.parse(calc.evaluateSheet(sheet, text)), text);
    }
  });
}

// The sheet with its answers, for the clipboard: the answers on screen when they're
// for this exact text, otherwise worked out (in the worker, if there is one).
function laidOut(text, markdown) {
  if (resultsText === text && results.length) return Promise.resolve(layoutAnswers(text, JSON.stringify(results), markdown));
  if (!worker || !workerReady) {
    return Promise.resolve(markdown ? calc.markdownTable(text) : calc.withAnswers(text));
  }
  const id = ++seq;
  worker.postMessage({ type: "layout", seq: id, text });
  return new Promise((resolve) => layouts.set(id, resolve)).then((m) =>
    m ? layoutAnswers(text, m.results, markdown) : markdown ? calc.markdownTable(text) : calc.withAnswers(text),
  );
}

function definedVars(lines) {
  const vars = new Set();
  for (const l of lines) {
    const m = l.match(/^\s*([A-Za-z_]\w*)\s*=[^=]/);
    if (m) vars.add(m[1]);
  }
  return vars;
}

// The same colours as the desktop app's matte-black theme, as [from, to, class] spans.
function tokens(line, vars) {
  const out = [];
  let code = line;
  const c = line.indexOf("//");
  if (c >= 0) {
    out.push([c, line.length, "t-com"]);
    code = line.slice(0, c);
  }
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
      const cls = vars.has(word) ? "t-var" : KEYWORDS.has(word.toLowerCase()) ? "t-kw" : "";
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
  if (url.length > 8000) toast(`Link copied (${n} characters). <small>Long links can be cut off by some apps; for a long sheet, Copy as Markdown table is safer.</small>`);
  else if (url.length > 2000) toast(`Link copied (${n} characters). <small>Fine for chat apps; some email and SMS may cut it.</small>`);
  else toast("Link copied. <small>Anyone with it can open this sheet.</small>");
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
    return toast("This sheet is already in your sheets.");
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
  toast("Saved to your sheets.");
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

async function loadRates() {
  let cached = null;
  try {
    cached = JSON.parse(store.get("rates") ?? "null");
  } catch {}
  if (cached?.rates) setRates(JSON.stringify(cached.rates));
  if (cached && Date.now() - cached.at < RATES_MAX_AGE) return;
  try {
    const res = await fetch(RATES_URL);
    const { rates } = await res.json();
    if (rates && setRates(JSON.stringify(rates))) {
      store.set("rates", JSON.stringify({ at: Date.now(), rates }));
      evaluate();
    }
  } catch {
    // Offline: the cached rates (if any) stay in use.
  }
}

// ---------------------------------------------------------------------------------
// Menu actions.

async function action(name) {
  closeMenu();
  const text = currentText();
  if (name === "copy") {
    await copyLater(laidOut(text, false));
    toast("Sheet copied with answers.");
  } else if (name === "markdown") {
    await copyLater(laidOut(text, true));
    toast("Sheet copied as a Markdown table.");
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
  } else if (name === "privacy") {
    toast(
      "<b>Private.</b> Sheets are saved in this browser, on this device, and never uploaded. " +
        "A share link carries the sheet after the #, which browsers don't send to any server. " +
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
  flushSave();
  state.order.splice(at, 1);
  state.texts.delete(id);
  store.remove("sheet:" + id);
  state.active = state.order[Math.min(at, state.order.length - 1)] ?? addSheet("");
  saveIndex();
  showSheet();
  toast(`Deleted “${esc(title(text))}”.`, {
    label: "Undo",
    run() {
      state.texts.set(id, text);
      state.order.splice(at, 0, id);
      saveSheet(id);
      state.active = id;
      saveIndex();
      showSheet();
    },
  });
}

document.addEventListener("click", (e) => {
  const button = e.target.closest?.("[data-action]");
  if (button) action(button.dataset.action);
  if (e.target.closest?.("[data-share]")) action("share");
  if (e.target.closest?.("[data-menu-button]")) {
    el.menu.hidden = !el.menu.hidden;
    el.menuButton.setAttribute("aria-expanded", String(!el.menu.hidden));
  } else if (!e.target.closest?.("[data-menu]")) closeMenu();
  if (e.target.closest?.(".sheets-toggle")) el.page.classList.toggle("drawer");
});

function closeMenu() {
  el.menu.hidden = true;
  el.menuButton.setAttribute("aria-expanded", "false");
}

addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    closeMenu();
    el.page.classList.remove("drawer");
  }
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
  a.download = `omacus-sheets-${new Date().toISOString().slice(0, 10)}.zip`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  toast(`Exported ${files.length} sheet${files.length === 1 ? "" : "s"}.`);
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
  toast(count ? `Imported ${count} sheet${count === 1 ? "" : "s"}.` : "Nothing new to import.");
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
      ? `<b>Install Omacus:</b> tap ${SHARE_ICON} then <b>Add to Home Screen</b>. It works offline, like an app, and Safari keeps your sheets.`
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
  // stale; there, remove any worker instead.
  if (["localhost", "127.0.0.1"].includes(location.hostname)) {
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
