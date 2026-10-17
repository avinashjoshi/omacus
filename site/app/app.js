// omacus.com/app: the notepad calculator in the browser. The math is the desktop
// app's own engine, compiled to WebAssembly (pkg/, built from web/ in the repo).
// Sheets live in this browser's localStorage; a share link carries a sheet in the
// part after "#", which browsers never send to the server.

import init, { Calc, suggest } from "./pkg/omacus_web.js";

const WASM = "/app/pkg/omacus_web_bg.wasm";
const PREFIX = "omacus:";
const RATES_URL = "https://open.er-api.com/v6/latest/USD";
const RATES_MAX_AGE = 12 * 60 * 60 * 1000;
const SHARE_VERSION = "v1.";
const SHARE_MAX_BYTES = 1 << 20; // a shared sheet expands to at most 1 MB
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

// Click an answer to copy it. Share link (top right) sends this sheet to anyone.
`;

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const el = {
  page: document.body,
  text: $("[data-text]"),
  rows: $("[data-rows]"),
  editor: $("[data-editor]"),
  sheets: $("[data-sheets]"),
  title: $("[data-title]"),
  shared: $("[data-shared]"),
  suggest: $("[data-suggest]"),
  toast: $("[data-toast]"),
  menu: $("[data-menu]"),
  menuButton: $("[data-menu-button]"),
  importInput: $("[data-import]"),
  loading: $("[data-loading]"),
};

let calc;
let results = [];
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
  if (!e.key?.startsWith(PREFIX)) return;
  const typing = document.activeElement === el.text && pendingSave;
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
  const text = el.text.value;
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
  updateSuggest();
}

function showSheet() {
  el.text.value = currentText();
  el.shared.hidden = state.active !== "shared";
  closeSuggest();
  evaluate();
  renderSidebar();
}

function select(id) {
  if (id === state.active) return;
  flushSave();
  state.active = id;
  if (id !== "shared") saveIndex();
  showSheet();
  el.text.focus({ preventScroll: true });
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
// Answers and highlighting: the layer over the textarea, one row per line.

let frame = 0;
function evaluate() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(() => {
    const text = el.text.value;
    results = calc ? JSON.parse(calc.evaluate(text)) : [];
    render(text);
  });
}

function render(text) {
  const lines = text.split("\n");
  const vars = definedVars(lines);
  const widest = results.reduce((w, r) => Math.max(w, [...(r.display ?? "")].length + (r.color ? 2 : 0)), 0);
  // Up to 45% of the width for answers (55% on phones); a longer one ends in "…".
  const share = el.editor.clientWidth < 600 ? 0.55 : 0.45;
  const room = Math.max(8, Math.min(widest + 1, Math.floor((el.editor.clientWidth / charWidth()) * share)));
  el.editor.style.setProperty("--res-w", `${room}ch`);
  el.page.style.setProperty("--res-w", `${room}ch`);

  el.rows.innerHTML = lines
    .map((line, i) => {
      const r = results[i];
      const src = r?.kind === "header" ? `<span class="t-head">${esc(line)}</span>` : highlight(line, vars);
      let res = "";
      if (r?.display) {
        const swatch = r.color ? `<span class="swatch" style="background:${r.color}"></span>` : "";
        res = `<span class="res" data-line="${i}" title="${esc(r.display)}">${swatch}<b>${esc(r.display)}</b></span>`;
      }
      // A trailing space keeps an empty line one line tall, as in the textarea.
      return `<div class="row">${src || " "}${res}</div>`;
    })
    .join("");
  syncScroll();
}

let measured = 0;
function charWidth() {
  if (!measured) {
    const probe = document.createElement("span");
    probe.textContent = "0".repeat(20);
    probe.style.cssText = "position:absolute;visibility:hidden;font:inherit;white-space:pre";
    el.editor.append(probe);
    measured = probe.getBoundingClientRect().width / 20 || 9;
    probe.remove();
  }
  return measured;
}
addEventListener("resize", () => {
  measured = 0;
  evaluate();
});

function syncScroll() {
  el.rows.style.transform = `translateY(${-el.text.scrollTop}px)`;
}

function definedVars(lines) {
  const vars = new Set();
  for (const l of lines) {
    const m = l.match(/^\s*([A-Za-z_]\w*)\s*=[^=]/);
    if (m) vars.add(m[1]);
  }
  return vars;
}

// The same colours as the desktop app's matte-black theme.
function highlight(line, vars) {
  let code = line;
  let comment = "";
  const c = line.indexOf("//");
  if (c >= 0) {
    code = line.slice(0, c);
    comment = line.slice(c);
  }
  let out = "";
  const label = code.match(/^(\s*[A-Za-z][A-Za-z0-9 '\-_&/]*:)(?= )/);
  if (label) {
    out += `<span class="t-label">${esc(label[1])}</span>`;
    code = code.slice(label[1].length);
  }
  const re = /("[^"]*"?)|(\d[\w.,]*)|([A-Za-z_]\w*)|([$£€¥₹₩₽₺%#])|(\s+)|(.)/gu;
  let m;
  while ((m = re.exec(code))) {
    const [tok, str, num, word, sym, ws] = m;
    if (str) out += `<span class="t-com">${esc(tok)}</span>`;
    else if (num || sym) out += `<span class="t-num">${esc(tok)}</span>`;
    else if (word) {
      const cls = vars.has(word) ? "t-var" : KEYWORDS.has(word.toLowerCase()) ? "t-kw" : "";
      out += cls ? `<span class="${cls}">${esc(tok)}</span>` : esc(tok);
    } else if (ws) out += tok;
    else out += `<span class="t-op">${esc(tok)}</span>`;
  }
  if (comment) out += `<span class="t-com">${esc(comment)}</span>`;
  return out;
}

function esc(s) {
  return s.replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);
}

// Click an answer to copy it.
el.editor.addEventListener("click", (e) => {
  const res = e.target.closest?.(".res");
  if (!res) return;
  const r = results[Number(res.dataset.line)];
  if (!r) return;
  copy(r.display).then(() => {
    res.classList.add("copied");
    toast(`Copied <b>${esc(r.display)}</b>`);
    setTimeout(() => res.classList.remove("copied"), 900);
  });
});

el.text.addEventListener("input", onInput);
el.text.addEventListener("scroll", syncScroll, { passive: true });

// ---------------------------------------------------------------------------------
// Autocomplete: the engine's suggestions, in a list at the caret.

let suggestion = null;

function updateSuggest() {
  const before = el.text.value.slice(0, el.text.selectionStart);
  if (el.text.selectionStart !== el.text.selectionEnd || !calc) return closeSuggest();
  const found = JSON.parse(suggest(before, el.text.value));
  if (!found) return closeSuggest();
  suggestion = { ...found, index: 0 };
  el.suggest.replaceChildren(
    ...found.items.map((item, i) => {
      const li = document.createElement("li");
      li.textContent = item;
      li.setAttribute("role", "option");
      li.addEventListener("mousedown", (e) => {
        e.preventDefault();
        accept(i);
      });
      return li;
    }),
  );
  markSuggestion();
  const at = caretPoint(found.start);
  el.suggest.style.left = `${Math.min(at.x, el.editor.clientWidth - 160)}px`;
  el.suggest.style.top = `${at.y + at.h}px`;
  el.suggest.hidden = false;
}

function markSuggestion() {
  $$("li", el.suggest).forEach((li, i) => li.setAttribute("aria-selected", String(i === suggestion?.index)));
}

function closeSuggest() {
  suggestion = null;
  el.suggest.hidden = true;
}

function accept(i = suggestion?.index ?? 0) {
  if (!suggestion) return;
  const word = suggestion.items[i];
  const end = el.text.selectionStart;
  el.text.setRangeText(word, suggestion.start, end, "end");
  closeSuggest();
  onInput();
  closeSuggest();
}

// Where a character offset sits, using an invisible copy of the textarea.
function caretPoint(offset) {
  const mirror = document.createElement("div");
  const style = getComputedStyle(el.text);
  for (const p of ["paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "fontFamily", "fontSize",
    "lineHeight", "letterSpacing", "whiteSpace", "overflowWrap", "tabSize", "boxSizing"]) mirror.style[p] = style[p];
  mirror.style.cssText += `position:absolute;visibility:hidden;top:0;left:0;width:${el.text.clientWidth}px;`;
  mirror.textContent = el.text.value.slice(0, offset);
  const mark = document.createElement("span");
  mark.textContent = "​";
  mirror.append(mark);
  el.editor.append(mirror);
  const x = mark.offsetLeft;
  const y = mark.offsetTop - el.text.scrollTop;
  const h = mark.offsetHeight;
  mirror.remove();
  return { x, y, h };
}

el.text.addEventListener("keydown", (e) => {
  if (suggestion && !el.suggest.hidden) {
    const n = suggestion.items.length;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      suggestion.index = (suggestion.index + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      return markSuggestion();
    }
    if (e.key === "Tab" || e.key === "Enter") {
      e.preventDefault();
      return accept();
    }
    if (e.key === "Escape") return closeSuggest();
  }
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.shiftKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    action("copy");
  } else if (mod && e.shiftKey && e.key.toLowerCase() === "m") {
    e.preventDefault();
    action("markdown");
  }
});
el.text.addEventListener("click", closeSuggest);
el.text.addEventListener("blur", () => setTimeout(closeSuggest, 150));

// ---------------------------------------------------------------------------------
// Share links: "#v1." then the sheet, deflated and base64url-encoded.

async function encodeShare(text) {
  const stream = new Blob([new TextEncoder().encode(text)]).stream().pipeThrough(new CompressionStream("deflate-raw"));
  const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return SHARE_VERSION + btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function decodeShare(hash) {
  if (!hash.startsWith(SHARE_VERSION)) return null;
  const b64 = hash.slice(SHARE_VERSION.length).replace(/-/g, "+").replace(/_/g, "/");
  let bin;
  try {
    bin = atob(b64 + "===".slice((b64.length + 3) % 4));
  } catch {
    return null;
  }
  const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
  const reader = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw")).getReader();
  const chunks = [];
  let size = 0;
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      // A link can't make the browser unpack gigabytes.
      if (size > SHARE_MAX_BYTES) return null;
      chunks.push(value);
    }
  } catch {
    return null;
  }
  return new TextDecoder().decode(await new Blob(chunks).arrayBuffer());
}

async function share() {
  const text = currentText();
  const url = `${location.origin}/app#${await encodeShare(text)}`;
  await copy(url);
  const n = url.length.toLocaleString();
  if (url.length > 8000) toast(`Link copied (${n} characters). <small>Long links can be cut off by some apps; for a long sheet, Copy as Markdown table is safer.</small>`);
  else if (url.length > 2000) toast(`Link copied (${n} characters). <small>Fine for chat apps; some email and SMS may cut it.</small>`);
  else toast("Link copied. <small>Anyone with it can open this sheet.</small>");
}

async function openShared() {
  const hash = decodeURIComponent(location.hash.slice(1));
  if (!hash) return;
  const text = await decodeShare(hash);
  history.replaceState(null, "", location.pathname);
  if (text === null) return toast("That link couldn't be opened: it may be cut off or damaged.");
  // Already saved? Open that copy instead of a duplicate.
  const same = state.order.find((id) => state.texts.get(id) === text);
  if (same) {
    state.shared = null;
    select(same);
    return toast("This sheet is already in your sheets.");
  }
  state.shared = { text };
  state.active = "shared";
  showSheet();
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

async function loadRates() {
  let cached = null;
  try {
    cached = JSON.parse(store.get("rates") ?? "null");
  } catch {}
  if (cached?.rates) calc.setRates(JSON.stringify(cached.rates));
  if (cached && Date.now() - cached.at < RATES_MAX_AGE) return;
  try {
    const res = await fetch(RATES_URL);
    const { rates } = await res.json();
    if (rates && calc.setRates(JSON.stringify(rates))) {
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
    await copy(calc.withAnswers(text));
    toast("Sheet copied with answers.");
  } else if (name === "markdown") {
    await copy(calc.markdownTable(text));
    toast("Sheet copied as a Markdown table.");
  } else if (name === "new") {
    flushSave();
    addSheet("");
    showSheet();
    el.text.focus();
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
function toast(html, undo) {
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
  toastTimer = setTimeout(() => (el.toast.hidden = true), undo ? 6000 : 3200);
}

// ---------------------------------------------------------------------------------
// Start.

async function start() {
  load();
  if (!state.order.length) addSheet(WELCOME);
  await init({ module_or_path: WASM });
  calc = new Calc();
  el.loading.remove();
  await openShared();
  if (state.active !== "shared") showSheet();
  loadRates();
  if (matchMedia("(min-width: 761px)").matches) el.text.focus({ preventScroll: true });
}

start().catch((e) => {
  el.loading.textContent = "Omacus couldn't start in this browser. Try a current Chrome, Firefox, Safari or Edge.";
  console.error(e);
});
