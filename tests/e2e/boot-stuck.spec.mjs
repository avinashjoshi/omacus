// A published copy that answers seed.js with the app page stays on
// "Loading the engine…", including after a cold start. The pages in IndexedDB
// stay put, and the fixed copy opens them.

import { test, expect, devices } from "@playwright/test";
import { createServer } from "node:http";
import { readFileSync, existsSync, mkdirSync } from "node:fs";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(fileURLToPath(new URL("../../site/", import.meta.url)));
const headers = readFileSync(join(site, "app/_headers"), "utf8");
const csp = headers.match(/Content-Security-Policy:\s*(.+)/)[1].trim();
const shots = "/opt/cursor/artifacts/screenshots";
const PAGE_ID = "ferry";
const PAGE_TEXT = "# Trip Budget\nferry = 40\nmilk $3\n";
const PORT = 4183;

const types = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
  ".woff2": "font/woff2",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".webmanifest": "application/manifest+json",
};

// The worker that shipped with the broken app: it stores a 200 response even
// when that response is the app page, and a later request ignores the hash.
const OLD_SW = `const CACHE = "omacus-app-8";
const FILES = ["./","index.html","app.js","seed.js","examples.js","app.css","editor.js","worker.js","store.js","trust.js","backup.js","tree.js","pkg/omacus_web.js","pkg/omacus_web_bg.wasm"];
self.addEventListener("install", (e) => {
  e.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    await cache.addAll(FILES.map((path) => new URL(path, self.location).href));
  })());
});
self.addEventListener("activate", (e) => {
  e.waitUntil((async () => {
    const keys = await caches.keys();
    await Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)));
    await self.clients.claim();
  })());
});
self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  if (new URL(req.url).origin !== self.location.origin) return;
  if (req.mode === "navigate") {
    e.respondWith((async () => {
      try { return await fetch(new URL("./", self.location).href); }
      catch { return (await caches.match(new URL("./", self.location).href)) || new Response("Offline", { status: 503 }); }
    })());
    return;
  }
  e.respondWith((async () => {
    const cache = await caches.open(CACHE);
    const hit = (await cache.match(req)) || (await cache.match(req, { ignoreSearch: true }));
    if (hit) return hit;
    const res = await fetch(req);
    if (res.ok) await cache.put(req, res.clone());
    return res;
  })());
});
self.addEventListener("message", (e) => {
  if (e.data && e.data.type === "skip-waiting") self.skipWaiting();
});
`;

const state = { brokenScripts: false, shell: "new" };
let server;

function currentIndex() {
  return readFileSync(join(site, "app/index.html"));
}

function oldIndex() {
  return Buffer.from(
    currentIndex()
      .toString("utf8")
      .replace(/<div class="boot-fail"[\s\S]*?<\/div>\s*/, "")
      .replace(/<script src="recover\.js"><\/script>\s*/, ""),
  );
}

function startServer() {
  server = createServer((req, res) => {
    const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
    let path = decodeURIComponent(url.pathname);
    if (path.endsWith("/")) path += "index.html";
    const common = {
      "content-security-policy": csp,
      "x-content-type-options": "nosniff",
      "referrer-policy": "strict-origin-when-cross-origin",
      "cache-control": "no-cache, no-store",
    };
    const script = path.endsWith("/seed.js") || path.endsWith("/examples.js");
    if (script && state.brokenScripts) {
      res.writeHead(200, { ...common, "content-type": "text/html; charset=utf-8" });
      res.end(state.shell === "old" ? oldIndex() : currentIndex());
      return;
    }
    if (path === "/app/fixture.html") {
      res.writeHead(200, { ...common, "content-type": "text/html; charset=utf-8" });
      res.end("<!doctype html><title>fixture</title><script src=\"trust.js\"></script>");
      return;
    }
    if (path === "/app/sw.js" && state.shell === "old") {
      res.writeHead(200, { ...common, "content-type": "text/javascript; charset=utf-8" });
      res.end(OLD_SW);
      return;
    }
    const file = normalize(join(site, path));
    if (!file.startsWith(site) || !existsSync(file)) {
      res.writeHead(404, common);
      res.end("not found");
      return;
    }
    let body = readFileSync(file);
    if (path === "/app/index.html" && state.shell === "old") body = oldIndex();
    res.writeHead(200, { ...common, "content-type": types[extname(file)] || "application/octet-stream" });
    res.end(body);
  });
  return new Promise((resolve) => server.listen(PORT, "127.0.0.1", resolve));
}

async function seedLibrary(page) {
  await page.goto(`http://127.0.0.1:${PORT}/app/fixture.html`);
  await page.evaluate(async ({ id, text }) => {
    const index = JSON.stringify({
      order: [id],
      active: id,
      tree: [{ type: "page", id }],
      handles: { [id]: "trip" },
      seeded: true,
    });
    await new Promise((resolve, reject) => {
      const req = indexedDB.open("omacus", 1);
      req.onupgradeneeded = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains("kv")) db.createObjectStore("kv");
      };
      req.onerror = () => reject(req.error);
      req.onsuccess = () => {
        const db = req.result;
        const tx = db.transaction("kv", "readwrite");
        tx.objectStore("kv").put(text, "page:" + id);
        tx.objectStore("kv").put(index, "index");
        tx.oncomplete = () => { db.close(); resolve(); };
        tx.onerror = () => reject(tx.error);
      };
    });
  }, { id: PAGE_ID, text: PAGE_TEXT });
}

async function storedPages(page) {
  // WebKit can reload while a service worker takes over. Read again after that.
  let last;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      return await readStoredPages(page);
    } catch (error) {
      last = error;
      const text = String(error && error.message ? error.message : error);
      if (attempt === 2 || !/destroyed|navigation/i.test(text)) throw error;
      await page.waitForLoadState("domcontentloaded").catch(() => {});
    }
  }
  throw last;
}

function readStoredPages(page) {
  return page.evaluate(() => new Promise((resolve) => {
    const req = indexedDB.open("omacus");
    req.onerror = () => resolve([]);
    req.onsuccess = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains("kv")) {
        db.close();
        resolve([]);
        return;
      }
      const rows = [];
      const tx = db.transaction("kv", "readonly");
      const cur = tx.objectStore("kv").openCursor();
      cur.onsuccess = () => {
        const row = cur.result;
        if (!row) return;
        rows.push([String(row.key), typeof row.value === "string" ? row.value : ""]);
        row.continue();
      };
      tx.oncomplete = () => { db.close(); resolve(rows); };
      tx.onerror = () => { db.close(); resolve(rows); };
    };
  }));
}

function pageText(rows) {
  return rows.find(([key]) => key === `page:${PAGE_ID}`)?.[1] ?? "";
}

const { defaultBrowserType, ...iphone } = devices["iPhone 13"];
void defaultBrowserType;
test.use({
  ...iphone,
  baseURL: `http://127.0.0.1:${PORT}`,
});

test.beforeAll(async () => {
  mkdirSync(shots, { recursive: true });
  await startServer();
});

test.afterAll(async () => {
  await new Promise((resolve) => server.close(resolve));
});

test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    const orig = window.matchMedia.bind(window);
    window.matchMedia = (query) => {
      if (String(query).includes("display-mode: standalone")) {
        return {
          matches: true,
          media: String(query),
          addEventListener() {},
          removeEventListener() {},
          addListener() {},
          removeListener() {},
          dispatchEvent() { return false; },
          onchange: null,
        };
      }
      return orig(query);
    };
    try {
      Object.defineProperty(navigator, "standalone", { configurable: true, get: () => true });
    } catch {
      // The browser already exposes it.
    }
  });
});

test("a cold start stays on the loading line until the real scripts are served", async ({ page }) => {
  state.brokenScripts = true;
  state.shell = "old";
  await page.emulateMedia({ colorScheme: "dark" });
  await seedLibrary(page);
  await page.goto(`http://127.0.0.1:${PORT}/app/`);
  await page.waitForTimeout(1500);
  await expect(page.locator("[data-loading]")).toHaveText("Loading the engine…");
  await expect(page.locator(".cm-content")).toHaveCount(0);
  await expect(page.locator("[data-title]")).toHaveText("");
  const stuck = await storedPages(page);
  expect(pageText(stuck)).toBe(PAGE_TEXT);
  expect(stuck.some(([key]) => key === "library-doc")).toBe(false);
  await page.screenshot({ path: `${shots}/iphone-loading-before-dark.png` });

  state.brokenScripts = false;
  state.shell = "new";
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForSelector(".cm-content", { timeout: 45000 });
  await expect(page.locator(".cm-content")).toContainText("ferry = 40");
  await expect(page.locator("[data-title]")).toHaveText("Trip Budget");
  await expect(page.locator("[data-boot-fail]")).toBeHidden();
  const restored = await storedPages(page);
  expect(pageText(restored)).toBe(PAGE_TEXT);
  await page.screenshot({ path: `${shots}/iphone-restored-after-dark.png` });
  await page.locator(".sheets-toggle").click();
  await expect(page.locator("#side")).toContainText("Trip Budget");
  await page.screenshot({ path: `${shots}/iphone-restored-drawer-dark.png` });
  await page.evaluate(() => { document.documentElement.dataset.appTheme = "light"; });
  await page.screenshot({ path: `${shots}/iphone-restored-drawer-light.png` });
  await page.locator(".sheets-toggle").click();
  await expect(page.locator(".cm-content")).toContainText("ferry = 40");
  await page.screenshot({ path: `${shots}/iphone-restored-after-light.png` });
});

test("a cached app page for seed.js still opens the saved pages", async ({ page }) => {
  state.brokenScripts = true;
  state.shell = "old";
  await page.emulateMedia({ colorScheme: "dark" });
  await seedLibrary(page);
  await page.evaluate(async () => {
    const href = new URL("sw.js", location.href).href;
    const reg = await navigator.serviceWorker.register(globalThis.omacusScriptURL(href));
    const worker = reg.installing || reg.waiting || reg.active;
    if (worker && worker.state !== "activated") {
      await new Promise((resolve) => {
        worker.addEventListener("statechange", () => {
          if (worker.state === "activated") resolve();
        });
      });
    }
    await navigator.serviceWorker.ready;
  });
  const cachedType = await page.evaluate(async () => {
    const name = (await caches.keys()).find((key) => key === "omacus-app-8");
    if (!name) return "missing-cache";
    const hit = await (await caches.open(name)).match(new URL("seed.js", location.href).href);
    if (!hit) return "missing-seed";
    return hit.headers.get("content-type") || "";
  });
  expect(cachedType).toContain("text/html");

  await page.goto(`http://127.0.0.1:${PORT}/app/?sw=1`);
  await page.waitForTimeout(1500);
  await expect(page.locator("[data-loading]")).toHaveText("Loading the engine…");
  expect(pageText(await storedPages(page))).toBe(PAGE_TEXT);

  state.brokenScripts = false;
  state.shell = "new";
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect.poll(async () => {
    const editor = page.locator(".cm-content");
    if (await editor.count()) {
      const text = await editor.innerText().catch(() => "");
      if (text.includes("ferry = 40")) return "editor";
    }
    const boot = page.locator("[data-boot-body]");
    if (await boot.count() && await boot.isVisible().catch(() => false)) {
      const text = await boot.innerText().catch(() => "");
      if (text.includes("ferry = 40")) return "boot";
    }
    return "";
  }, { timeout: 45000 }).not.toBe("");
  expect(pageText(await storedPages(page))).toBe(PAGE_TEXT);
  await page.screenshot({ path: `${shots}/iphone-cache-restored-dark.png` });
});
