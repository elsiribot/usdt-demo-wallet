# USDT Demo Wallet — Design

A minimal browser wallet for **USDT-denominated ecash** backed by a Fedimint
federation, with **on-chain USDT peg-in / peg-out**. It has exactly two client
modules and no Lightning, no Bitcoin on-chain, and none of the agent / LLM /
voice machinery of the app it is modelled on (`fedigents`).

- **What it does:** hold USDT ecash, receive USDT on-chain (peg-in), send USDT
  on-chain (peg-out), and send/receive ecash strings peer-to-peer (offline).
- **What it is:** a Leptos (CSR/wasm) single-page app built with Trunk, packaged
  as a git + Nix project.

> This document is the design spec. The self-contained reference an isolated
> agent needs to clone the backend and call its API lives in
> [`FEDIMINT_BRANCH.md`](./FEDIMINT_BRANCH.md).

## 1. Backend: the Fedimint `usdt-wallet` branch

The wallet is a client for a federation running the **`usdt` module** plus a
**`mintv2` instance denominated in `USDT_UNIT`**. Neither is published to
crates.io; both live on a branch. See [`FEDIMINT_BRANCH.md`](./FEDIMINT_BRANCH.md)
for the remote, branch, pinned commit, and full API surface.

Two concepts to internalize:

- **`USDT_UNIT`** (`fedimint_usdt_common::USDT_UNIT`, `AmountUnit::new_custom(1)`)
  is the `AmountUnit` all USDT ecash is denominated in. The `mintv2` instance
  that issues USDT notes is config-gen'd with this unit, and the `usdt` module
  credits/debits in this unit. The client routes balance per unit via
  `Client::primary_module_for_unit`, so the mintv2 primary module registered
  for `USDT_UNIT` is what holds the wallet's spendable balance.
- **The `usdt` module** does not itself hold a balance the user spends. It is the
  bridge: a **deposit** credits a per-user EVM account, which the user *claims*
  into `USDT_UNIT` ecash; a **withdrawal** burns `USDT_UNIT` ecash and queues an
  on-chain payout signed by the federation's MPC.

## 2. Architecture: why a web worker

This is the load-bearing lesson copied from `fedigents`.

The Fedimint client persists to a database. In the browser the only workable
backend is **OPFS Sync Access Handles** (`fedimint-cursed-redb` over
`FileSystemSyncAccessHandle`). Sync access handles are **only available inside a
Web Worker**, never on the main thread. Therefore:

```
┌──────────────── main thread ────────────────┐      ┌───────── Web Worker ──────────┐
│  Leptos UI (app.rs)                          │      │  WalletRuntimeCore (fedimint.rs)│
│    │                                         │ JSON │    - fedimint Client            │
│  WalletRuntime proxy (wallet_runtime.rs) ────┼─────▶│    - mintv2 + usdt modules      │
│    - serialize Command enum                  │ msgs │    - OPFS / cursed-redb DB      │
│    - await ResponseEnvelope                  │◀─────┼──── postMessage(ResponseEnvelope)│
└──────────────────────────────────────────────┘      └────────────────────────────────┘
```

- `wallet-worker.js` boots the same wasm binary; `run_worker_entrypoint()`
  detects the worker context and installs the message loop instead of mounting
  the UI (`main.rs` branches on this).
- The main thread never touches the DB or the client. It sends a `Command`, the
  worker runs it against `WalletRuntimeCore`, and replies with a
  `ResponseEnvelope { id, payload }`. Requests are correlated by `id`.
- Long-running / streaming work (deposit auto-watch, payment-received
  notifications) is pushed from worker → main thread as separate event messages,
  not as request replies.

The single wasm binary therefore has two entrypoints (UI mount vs. worker loop),
exactly as in `fedigents/crates/fedigents-web/src/main.rs`.

## 3. Client module set

Only two modules are registered on the `ClientBuilder`:

```rust
builder.with_module(fedimint_mintv2_client::MintClientInit); // USDT ecash (primary for USDT_UNIT)
builder.with_module(fedimint_usdt_client::UsdtClientInit);   // on-chain peg-in/peg-out
```

No `LightningClientInit`, no `WalletClientInit`, no `MetaClientInit`. The primary
module for `USDT_UNIT` is the mintv2 instance; the usdt module funds claims into
it and burns from it on withdrawal.

## 4. Federation onboarding

The user pastes an **invite code** on first launch (no hardcoded federation).
The worker builds the client, derives the federation secret from a locally
generated/stored mnemonic (same derivation scheme as `fedigents`:
`bip39` mnemonic → global root secret → per-federation key), joins, and persists
to OPFS. Subsequent launches reopen the existing DB and reconnect.

## 5. Worker command surface

`Command` is a serde enum sent main→worker; each returns a typed
`ResponsePayload::Ok(json)` or `Err(message)`.

| Command | Backend call | Notes |
|---|---|---|
| `Connect` | open DB / detect storage | returns storage capability notice |
| `Join { invite_code }` | build + join client | idempotent if already joined |
| `IsJoined` | check DB for joined federation | drives onboarding vs. home screen |
| `GetBalance` | `Client::get_balance_for_unit(USDT_UNIT)` | returns raw USDT amount (e6) |
| `ReceiveOnchain` | `UsdtClientModule::allocate_deposit()` | returns `(index, EvmAddress)`; kicks off auto-watch |
| `WithdrawQuote { amount }` | `withdraw_fee_quote(amount)` | returns `{ max_fee, valid_blocks }` |
| `WithdrawOnchain { recipient, amount, max_fee }` | `withdraw()` then `await_withdrawal_confirmed()` | recipient parsed as `EvmAddress` |
| `EcashSend { amount }` | mintv2 `send(amount, meta, include_invite)` | returns the ecash string |
| `EcashReceive { ecash }` | mintv2 `receive(ecash, meta)` | parses ecash string, redeems |
| `ListOperations { limit }` | operation log dump | history feed |

### Deposit auto-watch

After `ReceiveOnchain` returns the EVM address, the worker spawns a background
task that calls `UsdtClientModule::check_and_claim(claim_keypair, deadline)`.
That call asks the federation to watch the address, polls `deposit_status` with
exponential backoff until a credit becomes claimable, then submits the claim
transaction that mints `USDT_UNIT` ecash. When it completes, the worker pushes a
"deposit credited" event to the main thread and the balance refreshes
automatically. The UI shows the address plus a "waiting for deposit…" state; no
manual claim button.

## 6. UI (Leptos, CSR)

- **Onboarding**: paste invite code → Join. Show progress / errors.
- **Home**: large USDT balance, refreshed on events and on demand. Cards:
  - **Receive on-chain** — reveal EVM deposit address (QR + copy), auto-watch
    status.
  - **Send on-chain** — recipient EVM address + amount → fetch quote → confirm
    (amount + max_fee) → withdraw → show confirmation status.
  - **Ecash send** — amount → produce ecash string (QR + copy).
  - **Ecash receive** — paste ecash string → redeem.
  - **History** — operation log entries.

Styling is a stripped-down adaptation of `fedigents` `styles.css` (no chat UI).

## 7. Project layout

```
usdt-demo-wallet/
├── flake.nix                 # Nix dev shell: rust wasm toolchain, trunk, cargo-leptos, etc.
├── .envrc                    # use flake
├── .cargo/config.toml        # wasm getrandom backend flag
├── Cargo.toml                # workspace
├── Trunk.toml
├── index.html                # trunk entry; loads wasm + styles + worker glue
├── rust-toolchain.toml       # pin stable + wasm32 target
├── crates/usdt-wallet-web/
│   ├── Cargo.toml            # git deps pinned to the usdt-wallet branch commit
│   └── src/
│       ├── main.rs           # UI-mount vs worker-entrypoint branch
│       ├── app.rs            # Leptos components
│       ├── fedimint.rs       # WalletRuntimeCore: join, balance, deposit, withdraw, ecash
│       ├── wallet_runtime.rs # main-thread proxy + Command enum + worker message loop
│       ├── browser.rs        # wasm-bindgen glue to browser.js
│       ├── browser.js        # worker spawn, OPFS open, clipboard, QR
│       ├── wallet-worker.js  # worker bootstrap
│       └── styles.css
├── public/{manifest.webmanifest, sw.js, icon.svg}
└── docs/{DESIGN.md, ARCHITECTURE.md, FEDIMINT_BRANCH.md}
```

## 8. Dependencies

All fedimint crates are **git dependencies pinned to a single commit** on the
`usdt-wallet` branch (see [`FEDIMINT_BRANCH.md`](./FEDIMINT_BRANCH.md) for the
exact URL/rev). At minimum:

```
fedimint-client, fedimint-client-module, fedimint-core, fedimint-api-client,
fedimint-connectors, fedimint-derive-secret, fedimint-cursed-redb,
fedimint-bip39, fedimint-eventlog,
fedimint-mintv2-client, fedimint-usdt-client, fedimint-usdt-common
```

Same wasm build flags as `fedigents`: `getrandom_backend="wasm_js"`, unwrapped
clang for the wasm target, `console_error_panic_hook`, `tracing-wasm`.

## 9. Verification target

Success for the initial build = **compiles to `wasm32-unknown-unknown` and
`trunk build` produces a bundle** (all git deps resolve, no type errors). Live
runtime testing (deposit → claim → withdraw → ecash) requires a running
federation with the usdt + USDT-`mintv2` modules and testnet USDT, provided out
of band via an invite code; it is out of scope for the initial scaffold.

## 10. Explicit non-goals

- No Lightning, no Bitcoin on-chain wallet, no meta module.
- No AI agent, PPQ billing, chat UI, voice/STT, or image generation.
- No multi-federation support (one federation at a time).
- No custom fee-market UI beyond surfacing the quoted `max_fee`.
