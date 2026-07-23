//! wasm-bindgen glue to `browser.js`. The JS module is inlined into the bundle
//! and loaded in both the main thread and the worker; each function is only
//! *called* from the context where its browser APIs exist.

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::FileSystemSyncAccessHandle;

#[wasm_bindgen(module = "/src/browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = createWalletWorker)]
    fn create_wallet_worker_js(on_message: &js_sys::Function) -> bool;

    #[wasm_bindgen(js_name = workerPostMessage)]
    fn worker_post_message_js(msg: &str);

    #[wasm_bindgen(js_name = setWorkerOnMessage)]
    fn set_worker_on_message_js(handler: &js_sys::Function);

    #[wasm_bindgen(js_name = workerPostToMain)]
    fn worker_post_to_main_js(msg: &str);

    #[wasm_bindgen(js_name = storageSupported)]
    fn storage_supported_js() -> bool;

    #[wasm_bindgen(js_name = openWalletDb, catch)]
    async fn open_wallet_db_js(file_name: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_name = clipboardWrite, catch)]
    async fn clipboard_write_js(text: &str) -> Result<JsValue, JsValue>;
}

/// True when running inside the dedicated worker (no `Window`, a
/// `DedicatedWorkerGlobalScope`). `instanceof` against an undefined constructor
/// on the main thread simply returns false.
pub fn is_worker_context() -> bool {
    js_sys::global()
        .dyn_ref::<web_sys::DedicatedWorkerGlobalScope>()
        .is_some()
}

/// Spawn the worker. The closure is leaked (lives for the app lifetime).
pub fn create_wallet_worker(on_message: impl FnMut(JsValue) + 'static) -> bool {
    let cb = Closure::wrap(Box::new(on_message) as Box<dyn FnMut(JsValue)>);
    let ok = create_wallet_worker_js(cb.as_ref().unchecked_ref());
    cb.forget();
    ok
}

pub fn worker_post_message(msg: &str) {
    worker_post_message_js(msg);
}

/// Install the worker's onmessage handler. The closure is leaked.
pub fn set_worker_on_message(handler: impl FnMut(JsValue) + 'static) {
    let cb = Closure::wrap(Box::new(handler) as Box<dyn FnMut(JsValue)>);
    set_worker_on_message_js(cb.as_ref().unchecked_ref());
    cb.forget();
}

pub fn worker_post_to_main(msg: &str) {
    worker_post_to_main_js(msg);
}

pub fn storage_supported() -> bool {
    storage_supported_js()
}

/// Log to the browser console (visible from both main thread and worker).
pub fn log(msg: &str) {
    web_sys::console::log_1(&msg.into());
}

/// Open the OPFS-backed database file and return its SyncAccessHandle
/// (worker-only). Errors on browsers without sync access handles.
pub async fn open_wallet_db(file_name: &str) -> Result<FileSystemSyncAccessHandle, String> {
    let value = open_wallet_db_js(file_name)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok(value.unchecked_into::<FileSystemSyncAccessHandle>())
}

pub async fn clipboard_write(text: &str) -> Result<(), String> {
    clipboard_write_js(text).await.map(|_| ()).map_err(|e| format!("{e:?}"))
}
