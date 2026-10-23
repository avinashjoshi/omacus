// The service worker for omacus.com/app: it keeps the app working offline and lets
// it install to a Home Screen. The page is fetched fresh when there's a connection
// and falls back to the cached copy offline. The shell (script, styles, engine,
// fonts) is served from cache immediately and refreshed in the background, so an
// open app starts offline and the next visit picks up a new version.
//
// A new cache name drops everything older workers saved, so no page runs on a mix
// of versions. Local previews unregister the worker from the page; this file does
// not, or a `?sw=1` check could not register one.

const CACHE = "omacus-app-4";

// What the app needs in order to open. The page also sends the URLs it loaded;
// both lists are kept, and anything else from an older version is dropped.
const SHELL = [
  "/app/",
  "/app/app.js",
  "/app/tree.js",
  "/app/app.css",
  "/app/editor.js",
  "/app/worker.js",
  "/app/manifest.webmanifest",
  "/app/pkg/omacus_web.js",
  "/app/pkg/omacus_web_bg.wasm",
  "/css/site.css",
  "/fonts/jetbrains-mono-latin.woff2",
  "/fonts/inter-latin.woff2",
  "/img/icon.svg",
  "/img/icon-192.png",
  "/img/icon-512.png",
  "/img/apple-touch-icon.png",
];

self.addEventListener("install", () => self.skipWaiting());

self.addEventListener("activate", (e) => {
  e.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key)));
      await precache();
      await self.clients.claim();
    })(),
  );
});

function shellUrls() {
  return SHELL.map((path) => new URL(path, self.location.origin).href);
}

async function precache(extra = []) {
  const cache = await caches.open(CACHE);
  const urls = [...new Set([...shellUrls(), ...extra])];
  await Promise.all(urls.map((url) => cache.match(url).then((hit) => hit || cache.add(url).catch(() => {}))));
  return cache;
}

function offline() {
  return new Response("Offline", {
    status: 503,
    headers: { "content-type": "text/plain; charset=utf-8" },
  });
}

self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return;
  // Only the app page. A link out to the rest of the site must not be rewritten
  // into the cached app.
  if (req.mode === "navigate") {
    if (url.pathname === "/app" || url.pathname === "/app/" || url.pathname === "/app/index.html") {
      e.respondWith(networkFirstPage());
    }
    return;
  }
  // waitUntil has to be called before the first await, or the browser drops the
  // background refresh. The same fetch fills the cache and, on a miss, the page.
  const cachePromise = caches.open(CACHE);
  const fetched = fetch(req)
    .then(async (res) => {
      if (res.ok) {
        const cache = await cachePromise;
        await cache.put(req, res.clone());
      }
      return res;
    })
    .catch(() => null);
  e.waitUntil(fetched.then(() => {}));
  e.respondWith(
    (async () => {
      const cache = await cachePromise;
      const hit = await cache.match(req);
      if (hit) return hit;
      return (await fetched) || offline();
    })(),
  );
});

// The page: fresh if possible, so a new version shows up; the cached copy when offline.
// Only a successful response is stored. The cache key ignores the query string.
async function networkFirstPage() {
  const key = new URL("/app/", self.location.origin).href;
  try {
    const res = await fetch(key);
    if (res.ok) {
      const cache = await caches.open(CACHE);
      await cache.put(key, res.clone());
    }
    return res;
  } catch {
    return (await caches.match(key)) || offline();
  }
}

// The page lists what it loaded before this worker was in charge; cache those too,
// and drop files from older versions. The shell list is always kept.
self.addEventListener("message", (e) => {
  if (e.data?.type !== "cache") return;
  const extra = Array.isArray(e.data.urls) ? e.data.urls : [];
  const keep = new Set([...shellUrls(), ...extra]);
  e.waitUntil(
    (async () => {
      const cache = await precache(extra);
      for (const req of await cache.keys()) {
        const path = new URL(req.url).pathname;
        if (path !== "/app/" && !keep.has(req.url)) await cache.delete(req);
      }
    })(),
  );
});
