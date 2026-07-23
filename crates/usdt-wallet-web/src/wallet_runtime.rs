//! The main-thread `WalletRuntime` proxy, the `Command` protocol, and the
//! worker-side message loop. See docs/ARCHITECTURE.md.
//!
//! Main → worker: a JSON `RequestEnvelope { id, command }`.
//! Worker → main: a JSON `WorkerOut` — a `Response { id, .. }` correlated by
//! `id`, an unsolicited `Event { .. }`, or `Ready`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use anyhow::Context as _;
use futures::channel::oneshot;
use send_wrapper::SendWrapper;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::browser;
use crate::fedimint::WalletRuntimeCore;

// ---------------------------------------------------------------------------
// Protocol
// ---------------------------------------------------------------------------

/// Commands sent main → worker. Each maps to one backend flow.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "command", rename_all = "snake_case")]
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

#[derive(Serialize, Deserialize)]
struct RequestEnvelope {
    id: u64,
    command: Command,
}

/// Unsolicited worker → main events (not tied to a request id).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkerEvent {
    /// A watched deposit was credited and claimed into ecash.
    DepositCredited,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum RpcResult {
    Ok { data: serde_json::Value },
    Err { error: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WorkerOut {
    Ready,
    // NB: `result` is a nested field, NOT `#[serde(flatten)]`. Flattening an
    // internally-tagged enum breaks serde deserialization, which would leave the
    // main thread unable to parse responses (and every request hanging forever).
    Response { id: u64, result: RpcResult },
    Event { event: WorkerEvent },
}

// ---------------------------------------------------------------------------
// Typed response DTOs (main thread)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ConnectInfo {
    pub storage_ok: bool,
    pub storage_notice: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct QuoteInfo {
    pub max_fee: u64,
    pub valid_blocks: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HistoryItem {
    pub module: String,
    pub summary: String,
    /// Raw USDT amount (smallest unit), if known.
    pub amount: Option<u64>,
    /// `Some(true)` incoming (+), `Some(false)` outgoing (−), `None` neutral.
    pub incoming: Option<bool>,
}

// ---------------------------------------------------------------------------
// Main-thread proxy
// ---------------------------------------------------------------------------

type EventListener = Box<dyn Fn(WorkerEvent)>;

struct Inner {
    next_id: Cell<u64>,
    pending: RefCell<HashMap<u64, oneshot::Sender<RpcResult>>>,
    event_listener: RefCell<Option<EventListener>>,
    /// Set once the worker posts `Ready`. Requests wait for this — a message
    /// posted before the worker installs its `onmessage` handler is dropped.
    ready: Cell<bool>,
    ready_waiters: RefCell<Vec<oneshot::Sender<()>>>,
}

impl Inner {
    fn on_message(&self, raw: &str) {
        match serde_json::from_str::<WorkerOut>(raw) {
            Ok(WorkerOut::Ready) => {
                browser::log("[main] worker ready");
                self.ready.set(true);
                for tx in self.ready_waiters.borrow_mut().drain(..) {
                    let _ = tx.send(());
                }
            }
            Ok(WorkerOut::Response { id, result }) => {
                if let Some(tx) = self.pending.borrow_mut().remove(&id) {
                    let _ = tx.send(result);
                }
            }
            Ok(WorkerOut::Event { event }) => {
                if let Some(listener) = self.event_listener.borrow().as_ref() {
                    listener(event);
                }
            }
            Err(e) => {
                web_sys::console::error_1(&format!("bad worker message: {e}: {raw}").into());
            }
        }
    }
}

/// Cheap-to-clone handle to the worker. Shared across the UI via context.
///
/// The inner state is `Rc`-based (single-threaded wasm); `SendWrapper` makes the
/// handle `Send + Sync` so it can live in Leptos signals/context, which require
/// those bounds. It only ever runs on the one browser thread, so the wrapper's
/// same-thread guarantee always holds.
#[derive(Clone)]
pub struct WalletRuntime {
    inner: SendWrapper<Rc<Inner>>,
}

impl WalletRuntime {
    /// Spawn the worker and wire up its message channel.
    pub fn new() -> Self {
        let inner = Rc::new(Inner {
            next_id: Cell::new(1),
            pending: RefCell::new(HashMap::new()),
            event_listener: RefCell::new(None),
            ready: Cell::new(false),
            ready_waiters: RefCell::new(Vec::new()),
        });

        let inner_cb = inner.clone();
        browser::create_wallet_worker(move |data: wasm_bindgen::JsValue| {
            if let Some(s) = data.as_string() {
                inner_cb.on_message(&s);
            }
        });

        Self {
            inner: SendWrapper::new(inner),
        }
    }

    /// Register a single listener for unsolicited worker events.
    pub fn set_event_listener(&self, listener: impl Fn(WorkerEvent) + 'static) {
        *self.inner.event_listener.borrow_mut() = Some(Box::new(listener));
    }

    async fn request<T: DeserializeOwned>(&self, command: Command) -> anyhow::Result<T> {
        // Wait for the worker to signal it's listening before posting, or the
        // message is dropped (posted before its `onmessage` was installed).
        if !self.inner.ready.get() {
            let (tx, rx) = oneshot::channel();
            self.inner.ready_waiters.borrow_mut().push(tx);
            let _ = rx.await;
        }

        let id = self.inner.next_id.get();
        self.inner.next_id.set(id + 1);

        let (tx, rx) = oneshot::channel();
        self.inner.pending.borrow_mut().insert(id, tx);

        let envelope = RequestEnvelope { id, command };
        browser::log(&format!("[main] sending request id={id}"));
        browser::worker_post_message(&serde_json::to_string(&envelope)?);

        let result = rx.await.context("worker channel closed")?;
        match result {
            RpcResult::Ok { data } => Ok(serde_json::from_value(data)?),
            RpcResult::Err { error } => Err(anyhow::anyhow!(error)),
        }
    }

    // --- typed command helpers ---

    pub async fn connect(&self) -> anyhow::Result<ConnectInfo> {
        self.request(Command::Connect).await
    }

    pub async fn join(&self, invite_code: String) -> anyhow::Result<()> {
        self.request::<serde_json::Value>(Command::Join { invite_code })
            .await
            .map(|_| ())
    }

    pub async fn is_joined(&self) -> anyhow::Result<bool> {
        self.request(Command::IsJoined).await
    }

    /// Raw USDT balance (smallest unit, 10^-6 USDT).
    pub async fn get_balance(&self) -> anyhow::Result<u64> {
        let v: String = self
            .request::<StringWrap>(Command::GetBalance)
            .await?
            .value;
        Ok(v.parse()?)
    }

    /// EVM deposit address; kicks off the worker's auto-watch.
    pub async fn receive_onchain(&self) -> anyhow::Result<String> {
        Ok(self
            .request::<StringWrap>(Command::ReceiveOnchain)
            .await?
            .value)
    }

    pub async fn withdraw_quote(&self, amount: u64) -> anyhow::Result<QuoteInfo> {
        self.request(Command::WithdrawQuote { amount }).await
    }

    pub async fn withdraw_onchain(
        &self,
        recipient: String,
        amount: u64,
        max_fee: u64,
    ) -> anyhow::Result<String> {
        Ok(self
            .request::<StringWrap>(Command::WithdrawOnchain {
                recipient,
                amount,
                max_fee,
            })
            .await?
            .value)
    }

    pub async fn ecash_send(&self, amount: u64) -> anyhow::Result<String> {
        Ok(self
            .request::<StringWrap>(Command::EcashSend { amount })
            .await?
            .value)
    }

    pub async fn ecash_receive(&self, ecash: String) -> anyhow::Result<()> {
        self.request::<serde_json::Value>(Command::EcashReceive { ecash })
            .await
            .map(|_| ())
    }

    pub async fn list_operations(&self, limit: usize) -> anyhow::Result<Vec<HistoryItem>> {
        self.request(Command::ListOperations { limit }).await
    }
}

/// `{ "value": "…" }` — the shape single-string command results use.
#[derive(Deserialize)]
struct StringWrap {
    value: String,
}

// ---------------------------------------------------------------------------
// Worker-side loop
// ---------------------------------------------------------------------------

thread_local! {
    static CORE: RefCell<Option<Rc<WalletRuntimeCore>>> = const { RefCell::new(None) };
}

/// Detect the worker context; if present, install the request loop and return
/// true so `main()` skips mounting the UI.
pub fn run_worker_entrypoint() -> bool {
    if !browser::is_worker_context() {
        return false;
    }

    browser::set_worker_on_message(|data: wasm_bindgen::JsValue| {
        if let Some(raw) = data.as_string() {
            wasm_bindgen_futures::spawn_local(async move {
                handle_incoming(&raw).await;
            });
        }
    });

    browser::log("[worker] entrypoint installed, message loop ready");
    emit(&WorkerOut::Ready);
    true
}

/// Emit an unsolicited event from a worker background task.
pub fn emit_event(event: WorkerEvent) {
    emit(&WorkerOut::Event { event });
}

fn emit(out: &WorkerOut) {
    if let Ok(s) = serde_json::to_string(out) {
        browser::worker_post_to_main(&s);
    }
}

async fn handle_incoming(raw: &str) {
    let envelope: RequestEnvelope = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            web_sys::console::error_1(&format!("bad request: {e}: {raw}").into());
            return;
        }
    };
    let id = envelope.id;
    browser::log(&format!("[worker] handling request id={id}"));
    let result = match dispatch(envelope.command).await {
        Ok(data) => RpcResult::Ok { data },
        Err(e) => RpcResult::Err {
            error: format!("{e:#}"),
        },
    };
    browser::log(&format!("[worker] responding to id={id}"));
    emit(&WorkerOut::Response { id, result });
}

fn core() -> anyhow::Result<Rc<WalletRuntimeCore>> {
    CORE.with(|c| c.borrow().clone())
        .context("wallet not connected yet")
}

async fn dispatch(command: Command) -> anyhow::Result<serde_json::Value> {
    use serde_json::json;

    match command {
        Command::Connect => {
            let core = WalletRuntimeCore::connect().await?;
            let notice = core.storage_notice.clone();
            CORE.with(|c| *c.borrow_mut() = Some(Rc::new(core)));
            Ok(json!({ "storage_ok": notice.is_none(), "storage_notice": notice }))
        }
        Command::Join { invite_code } => {
            core()?.join(&invite_code).await?;
            Ok(json!(null))
        }
        Command::IsJoined => Ok(json!(core()?.is_joined().await)),
        Command::GetBalance => {
            let amount = core()?.get_balance().await?;
            Ok(json!({ "value": amount.to_string() }))
        }
        Command::ReceiveOnchain => {
            let address = core()?.receive_onchain().await?;
            Ok(json!({ "value": address }))
        }
        Command::WithdrawQuote { amount } => {
            let (max_fee, valid_blocks) = core()?.withdraw_quote(amount).await?;
            Ok(json!({ "max_fee": max_fee, "valid_blocks": valid_blocks }))
        }
        Command::WithdrawOnchain {
            recipient,
            amount,
            max_fee,
        } => {
            let status = core()?.withdraw_onchain(&recipient, amount, max_fee).await?;
            Ok(json!({ "value": status }))
        }
        Command::EcashSend { amount } => {
            let ecash = core()?.ecash_send(amount).await?;
            Ok(json!({ "value": ecash }))
        }
        Command::EcashReceive { ecash } => {
            core()?.ecash_receive(&ecash).await?;
            Ok(json!(null))
        }
        Command::ListOperations { limit } => {
            let items = core()?.list_operations(limit).await?;
            Ok(serde_json::to_value(items)?)
        }
    }
}
