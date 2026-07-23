//! USDT Demo Wallet — single wasm binary with two entrypoints.
//!
//! Loaded both as the page script (main thread → mount the Leptos UI) and as
//! the worker script (`wallet-worker.js` → install the message loop). `main()`
//! branches on the runtime context. See docs/ARCHITECTURE.md.

mod app;
mod browser;
mod fedimint;
mod wallet_runtime;

fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();

    // In the worker context this installs the request loop and returns true; we
    // must NOT mount the UI there.
    if wallet_runtime::run_worker_entrypoint() {
        return;
    }

    leptos::mount::mount_to_body(app::App);
}
