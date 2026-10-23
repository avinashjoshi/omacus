// If the app module never starts, the pages are still in this browser.
// Show them and offer a copy. This file does not write to either store.

(function () {
  const DB_NAME = "omacus";
  const STORE = "kv";
  const PREFIX = "omacus:";
  const RELOAD = "omacus-boot-reload";
  let rows = null;
  let showing = false;

  function $(sel) {
    return document.querySelector(sel);
  }

  function booted() {
    return globalThis.__omacusBooted === true;
  }

  function readLocal() {
    const out = [];
    try {
      for (let i = 0; i < localStorage.length; i++) {
        const full = localStorage.key(i);
        if (!full || !full.startsWith(PREFIX) || full === PREFIX + "migrated") continue;
        const value = localStorage.getItem(full);
        if (typeof value === "string") out.push([full.slice(PREFIX.length), value]);
      }
    } catch {
      // Private mode can refuse the mirror. IndexedDB may still hold the pages.
    }
    return out;
  }

  function readDb() {
    return new Promise((resolve) => {
      let req;
      try {
        req = indexedDB.open(DB_NAME);
      } catch {
        resolve([]);
        return;
      }
      req.onupgradeneeded = () => {
        try { req.transaction.abort(); } catch { /* no store yet, and we will not create one */ }
      };
      req.onerror = () => resolve([]);
      req.onsuccess = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains(STORE)) {
          db.close();
          resolve([]);
          return;
        }
        const found = [];
        const tx = db.transaction(STORE, "readonly");
        const cur = tx.objectStore(STORE).openCursor();
        cur.onsuccess = () => {
          const row = cur.result;
          if (!row) return;
          found.push([String(row.key), typeof row.value === "string" ? row.value : String(row.value)]);
          row.continue();
        };
        tx.oncomplete = () => {
          db.close();
          resolve(found);
        };
        tx.onerror = () => {
          db.close();
          resolve(found);
        };
      };
    });
  }

  async function stored() {
    if (rows) return rows;
    const merged = new Map(readLocal());
    for (const [key, value] of await readDb()) merged.set(key, value);
    rows = [...merged.entries()];
    return rows;
  }

  function pageTitle(text) {
    if (typeof text === "string" && text.startsWith("OMACUS-LOCK/1\n")) return "Locked page";
    const line = String(text ?? "").split("\n").map((part) => part.trim()).find(Boolean) ?? "";
    const name = line.replace(/^[#/\s]+/, "").trim();
    return name || "Untitled";
  }

  function pagesFrom(entries) {
    const map = new Map(entries);
    let index = {};
    try {
      index = JSON.parse(map.get("index") || "{}");
    } catch {
      index = {};
    }
    const order = Array.isArray(index.order) ? index.order.filter((id) => map.has("page:" + id)) : [];
    const ids = [...order];
    for (const key of map.keys()) {
      if (key.startsWith("page:") && !ids.includes(key.slice(5))) ids.push(key.slice(5));
    }
    return ids.map((id) => ({ id, text: map.get("page:" + id) ?? "" }));
  }

  function showPage(page) {
    const body = $("[data-boot-body]");
    const title = $("[data-title]");
    if (title) title.textContent = pageTitle(page.text);
    if (!body) return;
    const locked = page.text.startsWith("OMACUS-LOCK/1\n");
    body.hidden = false;
    body.textContent = locked ? "Locked page. The words stay sealed." : page.text;
  }

  function editorUp() {
    return !!document.querySelector(".cm-content");
  }

  // force is the engine itself giving up. A slow start must not cover the page,
  // and a start that already drew the editor must not be covered either.
  async function show(force) {
    if (showing) return;
    if (!force && booted()) return;
    if (force && editorUp()) return;
    showing = true;
    const entries = await stored();
    if ((!force && booted()) || (force && editorUp())) {
      showing = false;
      return;
    }
    const panel = $("[data-boot-fail]");
    const list = $("[data-boot-pages]");
    const loading = $("[data-loading]");
    if (loading) loading.hidden = true;
    if (!panel || !list) return;
    list.replaceChildren();
    const pages = pagesFrom(entries);
    for (const page of pages) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "boot-page";
      button.textContent = pageTitle(page.text);
      button.addEventListener("click", () => showPage(page));
      list.append(button);
    }
    if (!pages.length) {
      const empty = document.createElement("p");
      empty.textContent = "No pages were found in this browser.";
      list.append(empty);
    } else {
      showPage(pages[0]);
    }
    panel.hidden = false;
    const exportButton = $("[data-boot-export]");
    if (exportButton && !exportButton.dataset.wired) {
      exportButton.dataset.wired = "1";
      exportButton.addEventListener("click", () => download(entries, pages));
    }
    wakeWorker();
  }

  async function download(entries, pages) {
    const map = new Map(entries);
    let library = {};
    let settings = {};
    let trash = [];
    try { library = JSON.parse(map.get("index") || "{}"); } catch { library = {}; }
    try { settings = JSON.parse(map.get("settings") || "{}"); } catch { settings = {}; }
    try { trash = JSON.parse(map.get("trash") || "[]"); } catch { trash = []; }
    const version = document.querySelector('meta[name="omacus-version"]')?.content || "";
    try {
      const mod = await import("./backup.js");
      const bytes = mod.zipBytes(mod.backupEntries({ version, library, settings, trash, pages }));
      saveBlob(new Blob([bytes], { type: "application/zip" }), "omacus-backup.zip");
      return;
    } catch {
      // The zip helper did not load. A plain copy still leaves with the reader.
    }
    const plain = JSON.stringify({ version, library, settings, trash, pages }, null, 2);
    saveBlob(new Blob([plain], { type: "application/json" }), "omacus-backup.json");
  }

  function saveBlob(blob, name) {
    const link = document.createElement("a");
    link.href = URL.createObjectURL(blob);
    link.download = name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }

  function wakeWorker() {
    if (!("serviceWorker" in navigator)) return;
    const href = new URL("sw.js", location.href).href;
    const url = globalThis.omacusScriptURL ? globalThis.omacusScriptURL(href) : href;
    navigator.serviceWorker.register(url).then((reg) => {
      const nudge = (worker) => {
        if (!worker) return;
        const go = () => {
          if (worker.state === "installed" && navigator.serviceWorker.controller) {
            worker.postMessage({ type: "skip-waiting" });
          }
        };
        go();
        worker.addEventListener("statechange", go);
      };
      nudge(reg.installing);
      nudge(reg.waiting);
      reg.addEventListener("updatefound", () => nudge(reg.installing));
    }).catch(() => {});
    navigator.serviceWorker.addEventListener("controllerchange", () => {
      if (booted()) return;
      try {
        if (sessionStorage.getItem(RELOAD)) return;
        sessionStorage.setItem(RELOAD, "1");
      } catch {
        return;
      }
      location.reload();
    });
  }

  globalThis.omacusShowStored = () => show(true);

  function consider() {
    if (!booted()) show(false);
  }

  for (const script of document.scripts) {
    if (script.type === "module") script.addEventListener("error", consider);
  }
  window.addEventListener("error", (event) => {
    const text = String(event.message || "");
    if (text.includes("module") || text.includes("seed") || text.includes("Import")) consider();
  });
  setTimeout(consider, 8000);
})();
