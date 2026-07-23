// Worker bootstrap. Boots the *same* wasm binary as the page; `main()` detects
// the worker context, installs the message loop (via browser.js
// `setWorkerOnMessage`), and posts `__ready__` back to the main thread. It does
// NOT mount the Leptos UI. See main.rs for the entrypoint branch.
import init from "./usdt-wallet-web.js";

init().catch((e) => {
  console.error("worker wasm init failed", e);
  self.postMessage(JSON.stringify({ kind: "fatal", error: String(e) }));
});
