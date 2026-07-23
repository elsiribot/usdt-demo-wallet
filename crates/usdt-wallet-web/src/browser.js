// Browser glue, inlined into the wasm bindgen bundle via
// `#[wasm_bindgen(module = "/src/browser.js")]`. The same module is loaded on
// both the main thread and inside the worker, so every function here must be
// safe to *import* in either context; each is only *called* from the context
// where its APIs exist (worker spawn on main, OPFS on the worker).

// ---- main thread: worker lifecycle ----

let WORKER = null;

// Spawn the wallet worker. `onMessage` is a Rust callback invoked with each
// string the worker posts back. The worker boots the same wasm binary.
export function createWalletWorker(onMessage) {
  const url = new URL("./wallet-worker.js", document.baseURI);
  WORKER = new Worker(url, { type: "module" });
  WORKER.onmessage = (ev) => onMessage(ev.data);
  WORKER.onerror = (ev) => {
    console.error("wallet worker error", ev);
  };
  return true;
}

// Post a request string to the worker.
export function workerPostMessage(msg) {
  if (!WORKER) throw new Error("worker not started");
  WORKER.postMessage(msg);
}

// ---- worker thread: message loop wiring ----

// Register the worker's onmessage handler (Rust installs its dispatcher here).
export function setWorkerOnMessage(handler) {
  self.onmessage = (ev) => handler(ev.data);
}

// Post a message from the worker back to the main thread.
export function workerPostToMain(msg) {
  self.postMessage(msg);
}

// ---- worker thread: OPFS-backed database ----

// True iff the runtime supports OPFS Sync Access Handles (worker-only, recent
// Chromium). The Fedimint DB requires these.
export function storageSupported() {
  return (
    typeof FileSystemFileHandle !== "undefined" &&
    typeof FileSystemFileHandle.prototype.createSyncAccessHandle === "function"
  );
}

// Open (creating if needed) an OPFS file and return its SyncAccessHandle. This
// throws on browsers without sync access handles — surfaced to the user as a
// "use a recent Chromium-based browser" notice.
export async function openWalletDb(fileName) {
  const root = await navigator.storage.getDirectory();
  const handle = await root.getFileHandle(fileName, { create: true });
  return await handle.createSyncAccessHandle();
}

// ---- main thread: clipboard ----

export async function clipboardWrite(text) {
  await navigator.clipboard.writeText(text);
}
