// Service worker for the Omacus web app. After the first visit the shell is cached,
// so a reload — including one that bypasses the HTTP cache — still opens offline.
// A new version waits. The page saves the open edit, then asks this worker to take over.

const CACHE = "omacus-app-28";
const CACHE_NAME = CACHE;

const APP_FILES = [
  "./",
  "index.html",
  "app.js",
  "tree.js",
  "layer.js",
  "seed.js",
  "examples.js",
  "app.css",
  "editor.js",
  "worker.js",
  "store.js",
  "trust.js",
  "frame.js",
  "recover.js",
  "backup.js",
  "sync.js",
  "sync-config.js",
  "sync-link.mjs",
  "device-name.js",
  "sync-save.mjs",
  "sync-vaults.mjs",
  "sync-leader.mjs",
  "migrate.js",
  "migrate-client.js",
  "migrate-protocol.js",
  "migrate.html",
  "manifest.webmanifest",
  "pkg/omacus_web.js",
  "pkg/omacus_web_bg.wasm",
];

const SHARED = [
  "/css/site.css",
  "/css/tokens.css",
  "/fonts/jetbrains-mono-latin.woff2",
  "/fonts/inter-latin.woff2",
  "/img/icon.svg",
  "/img/icon-192.png",
  "/img/icon-512.png",
  "/img/apple-touch-icon.png",
];

function shellUrls() {
  const base = new URL("./", self.location);
  return [
    ...APP_FILES.map((path) => new URL(path, base).href),
    ...SHARED.map((path) => new URL(path, self.location.origin).href),
  ];
}

self.addEventListener("install", (e) => {
  e.waitUntil(precache());
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((key) => key !== CACHE_NAME).map((key) => caches.delete(key)));
      await self.clients.claim();
    })(),
  );
});

function isScript(req) {
  const path = new URL(req.url).pathname;
  return path.endsWith(".js") || path.endsWith(".mjs") || path.endsWith(".wasm");
}

// A missing file on the host comes back as the app page. That is not a script,
// and caching it would leave the next launch stuck on the loading line.
function honest(res, req) {
  if (!res || !res.ok) return false;
  if (!isScript(req)) return true;
  return !(res.headers.get("content-type") || "").includes("text/html");
}

// One cache operation at a time. A write that is still reading a socket holds
// WebKit's cache lock, and the next match never answers, so a reload stays on
// the loading line.
let cacheQueue = Promise.resolve();

function enqueue(task) {
  const run = cacheQueue.then(task, task);
  cacheQueue = run.then(() => {}, () => {});
  return run;
}

// Read the whole body before anything is stored. A socket the server dropped
// must not be the body of a cache write.
function fetchFresh(url, ms) {
  const ctrl = new AbortController();
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => {
      ctrl.abort();
      reject(new Error("timed out"));
    }, ms);
  });
  const done = (async () => {
    const res = await fetch(url, { signal: ctrl.signal });
    const body = await res.arrayBuffer();
    // fetch already decoded the bytes. Keeping the encoded length or encoding
    // would make the stored body unread, and the next launch would sit on the
    // loading line.
    const skip = new Set(["content-encoding", "content-length", "transfer-encoding", "connection", "keep-alive"]);
    const headers = new Headers();
    res.headers.forEach((value, name) => {
      if (skip.has(name)) return;
      try { headers.set(name, value); } catch { /* A hop-by-hop header cannot be stored. */ }
    });
    return new Response(body, { status: res.status, statusText: res.statusText, headers });
  })();
  done.catch(() => {});
  return Promise.race([done, timeout]).finally(() => clearTimeout(timer));
}

async function matchAny(cache, req) {
  return enqueue(async () => (await cache.match(req)) || (await cache.match(req, { ignoreSearch: true })));
}

async function keep(cache, url) {
  const req = new Request(url);
  if (await matchAny(cache, req).then((hit) => hit && honest(hit, req))) return;
  try {
    const res = await fetchFresh(req, 15000);
    if (honest(res, req)) await enqueue(() => cache.put(req, res));
  } catch {
    // The shell can be short a file. Pages in the browser are a different store.
  }
}

async function precache(extra = []) {
  const cache = await caches.open(CACHE_NAME);
  const urls = [...new Set([...shellUrls(), ...extra])];
  await Promise.all(urls.map((url) => keep(cache, url)));
  return cache;
}

function offline() {
  return new Response("Offline", {
    status: 503,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}

function isAppNavigation(pathname) {
  const base = new URL("./", self.location);
  const root = base.pathname;
  const bare = root.length > 1 ? root.replace(/\/$/, "") : root;
  const index = new URL("index.html", base).pathname;
  return pathname === root || pathname === bare || pathname === index;
}

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  if (url.pathname === "/v1" || url.pathname.startsWith("/v1/")) return;
  if (req.mode === "navigate") {
    if (isAppNavigation(url.pathname)) e.respondWith(networkFirstPage());
    return;
  }
  const cachePromise = caches.open(CACHE_NAME);
  // A cached file is the answer. The fresh copy is stored only after its body
  // has arrived, so a dropped connection cannot sit in the cache.
  const refresh = cachePromise.then(async (cache) => {
    try {
      const res = await fetchFresh(req, 15000);
      if (honest(res, req)) await enqueue(() => cache.put(req, res.clone()));
      return res;
    } catch {
      return null;
    }
  });
  e.waitUntil(refresh.then(() => {}));
  e.respondWith(
    (async () => {
      const cache = await cachePromise;
      const hit = await matchAny(cache, req);
      if (hit && honest(hit, req)) return hit;
      return (await refresh) || offline();
    })(),
  );
});

// Fresh when the network answers; the cached shell when it does not. A reload
// that skips the HTTP cache still comes through here, including with no connection.
async function networkFirstPage() {
  const key = new URL("./", self.location).href;
  const cache = await caches.open(CACHE_NAME);
  if (self.navigator.onLine !== false) {
    try {
      const res = await fetchFresh(key, 2500);
      if (res.ok) {
        await enqueue(() => cache.put(key, res.clone()));
        return res;
      }
    } catch {
      // Offline, or the reload could not reach the network.
    }
  }
  const cached = (await matchAny(cache, key)) || (await matchAny(cache, new URL("index.html", self.location).href));
  if (!cached) return offline();
  const headers = new Headers(cached.headers);
  headers.set("X-Omacus-Offline", "1");
  return new Response(cached.body, { status: cached.status, statusText: cached.statusText, headers });
}

self.addEventListener("message", (e) => {
  if (e.data?.type === "skip-waiting") {
    self.skipWaiting();
    return;
  }
  if (e.data?.type === "shell-status") {
    const source = e.source;
    const port = e.ports[0];
    e.waitUntil(
      (async () => {
        const cache = await caches.open(CACHE_NAME);
        const keys = new Set((await enqueue(() => cache.keys())).map((req) => req.url));
        const missing = shellUrls().filter((url) => !keys.has(url));
        const reply = { type: "shell-status", ready: missing.length === 0, missing };
        if (port) port.postMessage(reply);
        else source?.postMessage(reply);
      })(),
    );
    return;
  }
  if (e.data?.type !== "cache") return;
  const extra = Array.isArray(e.data.urls) ? e.data.urls : [];
  const keep = new Set([...shellUrls(), ...extra]);
  e.waitUntil(
    (async () => {
      const cache = await precache(extra);
      const base = new URL("./", self.location).pathname;
      for (const req of await enqueue(() => cache.keys())) {
        const path = new URL(req.url).pathname;
        if (path !== base && !keep.has(req.url)) await enqueue(() => cache.delete(req));
      }
    })(),
  );
});
