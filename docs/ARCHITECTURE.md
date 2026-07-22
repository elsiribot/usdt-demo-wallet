# Architecture — worker + message-passing pattern

How the Leptos UI and the Fedimint client cooperate in the browser. This is the
`fedigents` pattern, distilled to what this wallet needs. It exists because
**the Fedimint DB requires OPFS Sync Access Handles, which only exist inside a
Web Worker.** The UI thread must therefore never construct or touch the client.

## Component map

| File | Thread | Responsibility |
|---|---|---|
| `main.rs` | both | Entry. Branch: worker context → run message loop; else → mount Leptos UI. |
| `app.rs` | main | Leptos components; calls the `WalletRuntime` proxy; never sees the client. |
| `wallet_runtime.rs` | main + worker | `WalletRuntime` proxy (main), `Command`/`ResponseEnvelope` types, and `run_worker_entrypoint()` + `handle_request` (worker). |
| `fedimint.rs` | worker only | `WalletRuntimeCore`: build/join client, balance, deposit, withdraw, ecash. |
| `browser.rs` / `browser.js` | main + worker | wasm-bindgen glue: spawn worker, open OPFS DB, clipboard, QR. |
| `wallet-worker.js` | worker | Boots the wasm module inside the worker; posts `__ready__`. |

## The single-binary two-entrypoint trick

One wasm binary is loaded both as the page script and as the worker script.
`main.rs` decides which role to play:

```rust
fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    if wallet_runtime::run_worker_entrypoint() {
        return; // we are the worker: message loop installed, do not mount UI
    }
    leptos::mount::mount_to_body(app::App);
}
```

`run_worker_entrypoint()` returns `false` on the main thread (detected via
`browser::is_worker_context()` — there is no `Window`, there is a
`DedicatedWorkerGlobalScope`). On the worker it installs an `onmessage` handler
and returns `true`.

## Request/response protocol

Main thread → worker: a JSON-serialized `RequestEnvelope { id, command }`.
Worker → main thread: `ResponseEnvelope { id, payload }` where `payload` is
`Ok(serde_json::Value)` or `Err(String)`. The proxy keeps a map of in-flight
`id → oneshot::Sender` and resolves the matching future when a reply arrives.

```rust
// wallet_runtime.rs (main-thread proxy), shape only
pub enum Command {
    Connect,
    Join { invite_code: String },
    IsJoined,
    GetBalance,
    ReceiveOnchain,
    WithdrawQuote { amount: u64 },
    WithdrawOnchain { recipient: String, amount: u64, max_fee: u64 },
    EcashSend { amount: u64 },
    EcashReceive { ecash: String },
    ListOperations { limit: usize },
}

impl WalletRuntime {
    async fn request<T: DeserializeOwned>(&self, cmd: Command) -> anyhow::Result<T> {
        // assign id, register oneshot, postMessage(JSON), await reply, deserialize
    }
}
```

The worker side dispatches with a `match request.command { … }`, calling into
`WalletRuntimeCore` and serializing the result. Keep each arm small; put real
logic in `fedimint.rs`.

## Pushed events (worker → main, unsolicited)

Some flows are not simple request/reply:

- **Deposit auto-watch**: after `ReceiveOnchain`, a spawned worker task runs
  `check_and_claim` and, on success, posts a `WorkerEvent::DepositCredited`
  (separate envelope, not tied to a request `id`). The UI listens and refreshes
  the balance.
- **Bootstrap progress** (optional): stream join progress notes to the UI.

Model these as a second envelope type the proxy routes to registered listeners
(`set_operation_listener` in `fedigents`), distinct from request replies.

## DB / storage

- `browser.js::openWalletDb(fileName)` gets the OPFS root
  (`navigator.storage.getDirectory()`), opens a file handle, and returns a
  `createSyncAccessHandle()`. This throws on browsers without sync access
  handles — surface a clear "use a recent Chromium-based browser" notice.
- `fedimint-cursed-redb` wraps that handle as the redb backend for the client DB.
- `Connect` opens the DB and reports a `storage_notice` if handles are
  unsupported; the UI can warn but still render.

## What to copy vs. change from `fedigents`

**Copy structurally:** `main.rs` branch, the `WalletRuntime`/worker envelope
plumbing in `wallet_runtime.rs`, `browser.rs`/`browser.js` (worker spawn + OPFS +
clipboard + QR), `wallet-worker.js`, the mnemonic/secret derivation, the OPFS DB
open.

**Change:** the module set (mintv2 + usdt, not mint/ln/wallet/meta), the
`Command` variants (USDT deposit/withdraw/ecash, not invoice/pay), the balance
call (`get_balance_for_unit(USDT_UNIT)`), and delete everything agent/PPQ/LLM/
voice/image (`agent.rs`, `ppq.rs`, `calc.rs`, chat UI in `app.rs`, STT/recording
in `browser.js`).

See [`FEDIMINT_BRANCH.md`](./FEDIMINT_BRANCH.md) for the exact backend API each
`Command` maps onto.
