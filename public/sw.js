// Minimal no-op service worker. Present so the app can be installed as a PWA;
// it deliberately does not cache the wasm bundle (the OPFS-backed client state
// is the source of truth and stale wasm caching would complicate upgrades).
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
self.addEventListener("fetch", () => {});
