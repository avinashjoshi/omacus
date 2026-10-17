// The service worker for omacus.com/app: it keeps the app working offline and lets
// it install to a Home Screen. Pages load fresh when there's a connection; the
// engine, styles and fonts (whose URLs change with every version) come from cache.

const CACHE = "omacus-app";

self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));

self.addEventListener("fetch", (e) => {
  const req = e.request;
  const url = new URL(req.url);
  if (req.method !== "GET" || url.origin !== location.origin) return;
  if (req.mode === "navigate") {
    // The page: fresh if possible, so a new version shows up; cached when offline.
    e.respondWith(
      fetch(req)
        .then((res) => {
          const copy = res.clone();
          caches.open(CACHE).then((c) => c.put("/app/", copy));
          return res;
        })
        .catch(() => caches.match("/app/")),
    );
    return;
  }
  // Everything else: cached first (versioned URLs never change), stored on first use.
  e.respondWith(
    caches.match(req).then(
      (hit) =>
        hit ||
        fetch(req).then((res) => {
          if (res.ok) {
            const copy = res.clone();
            caches.open(CACHE).then((c) => c.put(req, copy));
          }
          return res;
        }),
    ),
  );
});

// The page lists what it loaded before this worker was in charge; cache those too,
// and drop files from older versions.
self.addEventListener("message", (e) => {
  if (e.data?.type !== "cache") return;
  const keep = new Set(e.data.urls);
  e.waitUntil(
    caches.open(CACHE).then(async (c) => {
      await Promise.all(e.data.urls.map((u) => c.match(u).then((hit) => hit || c.add(u).catch(() => {}))));
      for (const req of await c.keys()) {
        const path = new URL(req.url).pathname;
        if (path !== "/app/" && !keep.has(req.url)) await c.delete(req);
      }
    }),
  );
});
