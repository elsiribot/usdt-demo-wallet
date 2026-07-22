# USDT Demo Wallet

A minimal browser wallet for **USDT-denominated Fedimint ecash** with **on-chain
USDT peg-in / peg-out**. Two client modules only — `mintv2` (USDT ecash) and
`usdt` (on-chain bridge) — built as a Leptos (CSR/wasm) app with Trunk, packaged
git + Nix.

Modelled on [`fedigents`](../fedigents) for the browser/worker/OPFS plumbing,
with all of its agent / LLM / voice / Lightning / Bitcoin code removed.

## Features

- Hold a USDT ecash balance.
- **Receive on-chain**: get an EVM deposit address; the wallet auto-watches it
  and mints ecash once the deposit is credited.
- **Send on-chain**: withdraw USDT to any EVM address (with a fee quote).
- **Ecash send/receive**: spend to / redeem from an offline ecash string (P2P).

## Documentation

| Doc | Purpose |
|---|---|
| [`docs/DESIGN.md`](docs/DESIGN.md) | Full design spec: architecture, modules, commands, UI, layout, non-goals. |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | The web-worker + message-passing + OPFS pattern and how to implement it. |
| [`docs/FEDIMINT_BRANCH.md`](docs/FEDIMINT_BRANCH.md) | **Self-contained** backend reference: how to clone the `usdt-wallet` branch into a temp dir, the pinned commit, git-dependency pins, and the full client API surface. |

`FEDIMINT_BRANCH.md` is written so an isolated agent with only this repo's docs
can clone the backend and call its API without any local checkout.

## Build

Requires Nix (flakes). The dev shell provides the Rust wasm toolchain, Trunk,
and cargo-leptos.

```bash
nix develop        # or: direnv allow
trunk build        # produce a wasm bundle in dist/
trunk serve        # local dev server
```

The wallet must run in a **recent Chromium-based browser** (OPFS Sync Access
Handles are required for storage).

## Running against a federation

On first launch, paste an **invite code** to a federation that runs the `usdt`
module and a `mintv2` instance denominated in `USDT_UNIT`. Standing up such a
federation is out of scope for this repo — see the backend branch's
`modules/fedimint-usdt-tests/` and its deployment runbook.

## Backend pin

All fedimint crates are git-pinned to
`https://github.com/elsiribot/fedimint.git`, branch `2026-07-usdt-wallet`, commit
`ecc17458da1c470b2a984cf63a7b09337ad3c232`. See
[`docs/FEDIMINT_BRANCH.md`](docs/FEDIMINT_BRANCH.md).
