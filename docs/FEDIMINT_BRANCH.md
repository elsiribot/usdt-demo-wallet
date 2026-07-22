# Fedimint `usdt-wallet` branch — clone & API reference

Self-contained reference for building against the USDT wallet backend. Assumes
**no access to any local checkout** — everything here is enough to clone the
branch into a temp dir and call its API.

## 1. Where the code lives

| | |
|---|---|
| **Remote (HTTPS, public)** | `https://github.com/elsiribot/fedimint.git` |
| **Remote (SSH)** | `git@github.com:elsiribot/fedimint.git` |
| **Branch** | `2026-07-usdt-wallet` |
| **Pinned commit** | `ecc17458da1c470b2a984cf63a7b09337ad3c232` |

> The branch is **only** on the `elsiribot/fedimint` fork. It is **not** on
> `github.com/fedimint/fedimint` (upstream) nor on `github.com/elsirion/minimint`.
> Verified reachable over anonymous HTTPS.

### Clone into a temp dir

```bash
# Shallow clone of just the pinned commit into a temp dir
TMP="$(mktemp -d)"
git clone --depth 1 --branch 2026-07-usdt-wallet \
  https://github.com/elsiribot/fedimint.git "$TMP/fedimint"
cd "$TMP/fedimint"
git checkout ecc17458da1c470b2a984cf63a7b09337ad3c232   # exact pin (may be branch HEAD already)
```

If you only need to *read* the source (not build), a sparse/blob-filter clone is
cheaper:

```bash
git clone --filter=blob:none --no-checkout --branch 2026-07-usdt-wallet \
  https://github.com/elsiribot/fedimint.git "$TMP/fedimint"
```

### Use it as a Cargo git dependency

Pin every fedimint crate to the same commit:

```toml
[dependencies]
fedimint-core          = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-client        = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-client-module = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-api-client    = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-connectors    = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-derive-secret = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-cursed-redb   = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-bip39         = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-eventlog      = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-mintv2-client = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-usdt-client   = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
fedimint-usdt-common   = { git = "https://github.com/elsiribot/fedimint.git", rev = "ecc17458da1c470b2a984cf63a7b09337ad3c232" }
```

## 2. Crate paths within the repo

| Crate | Path |
|---|---|
| `fedimint-usdt-client` | `modules/fedimint-usdt-client/` |
| `fedimint-usdt-common` | `modules/fedimint-usdt-common/` |
| `fedimint-usdt-server` | `modules/fedimint-usdt-server/` (guardian side; not needed by the wallet) |
| `fedimint-usdt-tests` | `modules/fedimint-usdt-tests/` (**best usage examples** — see below) |
| `fedimint-mintv2-client` | `modules/fedimint-mintv2-client/` |
| `fedimint-mintv2-common` | `modules/fedimint-mintv2-common/` |
| `fedimint-client` (per-unit balance) | `fedimint-client/src/client.rs` |

The single most useful file to read for end-to-end flows is
`modules/fedimint-usdt-tests/tests/tests.rs` and its siblings
(`withdraw_e2e.rs`, `recovery_e2e.rs`).

## 3. Core constant

```rust
// fedimint_usdt_common
pub const USDT_UNIT: AmountUnit = AmountUnit::new_custom(1);
pub const KIND: ModuleKind = ModuleKind::from_static_str("usdt");
```

All USDT ecash is denominated in `USDT_UNIT`. `UsdtAmount(pub u64)` and
`Amount::from_msats(x)` both carry the raw integer USDT amount (the module treats
`UsdtAmount.0` and the mintv2 `Amount` msat value as the same integer). Format an
amount with `UsdtAmount`'s `Display` (raw integer).

## 4. `UsdtClientModule` API (client)

Obtain it with `client.get_first_module::<UsdtClientModule>()?`.

```rust
// ---- Peg-IN (deposit) ----

/// Reserve the next deposit index, derive its claim keypair (seed-recoverable),
/// and return the EVM address the user must send USDT to. Gated on the
/// federation reporting BootstrapState::Ready.
pub async fn allocate_deposit(&self) -> anyhow::Result<(Keypair, EvmAddress)>;

/// The EVM deposit address for a given claim pubkey (deterministic).
pub fn deposit_address(&self, claim_pubkey: &secp256k1::PublicKey) -> EvmAddress;

/// Ask the federation to watch `claim_keypair`'s address, poll until a credit
/// is claimable (exponential backoff, capped 5s), then submit the claim tx that
/// mints USDT_UNIT ecash. This is the deposit "auto-watch" driver.
pub async fn check_and_claim(
    &self,
    claim_keypair: &Keypair,
    deadline: Duration,
) -> anyhow::Result<()>;

/// One-shot claim of whatever is already claimable for `claim_pk`.
pub async fn claim(&self, claim_pk: secp256k1::PublicKey) -> anyhow::Result<UsdtAmount>;

/// credited / claimed / claimable for a deposit account.
pub async fn deposit_status(&self, claim_pk: secp256k1::PublicKey)
    -> anyhow::Result<DepositStatusResponse>;

/// Seed-only rescan to rediscover credited deposits (recovery flow).
pub async fn recover_deposits(&self, gap_limit: u64) -> anyhow::Result<RecoverySummary>;

// ---- Peg-OUT (withdrawal) ----

/// Minimum fee a withdrawal of `amount` must currently offer, plus advisory
/// validity window. { max_fee: UsdtAmount, valid_blocks: u64 }.
pub async fn withdraw_fee_quote(&self, amount: UsdtAmount)
    -> anyhow::Result<WithdrawFeeQuoteResponse>;

/// Burn `amount + max_fee` of USDT_UNIT ecash and enqueue an on-chain payout of
/// `amount` to `recipient`. Awaits consensus acceptance (a stale max_fee below
/// the live quote is surfaced as Err). Returns the change OutPointRange; the
/// withdrawal output is at out_idx 0.
pub async fn withdraw(
    &self,
    recipient: EvmAddress,
    amount: UsdtAmount,
    max_fee: UsdtAmount,
) -> anyhow::Result<OutPointRange>;

/// OutPoint of the withdrawal output from `withdraw`'s returned range.
pub fn withdrawal_out_point(range: &OutPointRange) -> OutPoint; // { txid: range.txid(), out_idx: 0 }

/// Consensus-agreed lifecycle stage of a withdrawal.
pub async fn withdrawal_status(&self, out_point: OutPoint)
    -> anyhow::Result<WithdrawalStatusResponse>;

/// Poll withdrawal_status until Confirmed{block} (Ok) / Failed (Err) / deadline.
pub async fn await_withdrawal_confirmed(&self, out_point: OutPoint, deadline: Duration)
    -> anyhow::Result<u64>;

/// Federation bootstrap/health (must be Ready to allocate deposits).
pub async fn status(&self) -> anyhow::Result<StatusResponse>;
```

### Response types (`fedimint_usdt_common`)

```rust
pub struct EvmAddress(pub [u8; 20]);   // Display => "0x…40hex", FromStr accepts optional 0x
pub struct UsdtAmount(pub u64);        // Display => raw integer

pub struct DepositStatusResponse {
    pub account: EvmAddress,
    pub credited: UsdtAmount,
    pub claimed: UsdtAmount,
    pub claimable: UsdtAmount,          // credited − claimed (saturating)
}

pub struct WithdrawFeeQuoteResponse { pub max_fee: UsdtAmount, pub valid_blocks: u64 }

pub enum WithdrawalStatus {
    Unknown, Queued,
    Signing { op_hash: [u8; 32] },
    Submitted { op_hash: [u8; 32] },
    Confirmed { block: u64 },           // terminal, success
    Failed { reason: String },          // terminal, failure
}
pub struct WithdrawalStatusResponse { pub status: WithdrawalStatus }

pub enum BootstrapState { /* … */ Ready /* … */ }
pub struct StatusResponse {
    pub state: BootstrapState,
    pub entry_point_ok: bool, pub factory_ok: bool, pub impl_ok: bool,
    pub funded_guardians: u16, pub healthy_guardians: u16, pub threshold: u16,
}
```

## 5. mintv2 API (USDT ecash) — `fedimint_mintv2_client::MintClientModule`

Obtain with `client.get_first_module::<MintClientModule>()?` (the instance
registered for `USDT_UNIT`).

```rust
/// Spend `amount` into an offline ecash string (String via ECash Display /
/// base32 "fedimint" prefix). include_invite embeds the federation invite.
pub async fn send(&self, amount: Amount, custom_meta: Value, include_invite: bool)
    -> Result<(OperationId, ECash), SendECashError>;

/// Redeem a received ecash string back into the wallet.
pub async fn receive(&self, ecash: ECash, custom_meta: Value)
    -> Result<OperationId, ReceiveECashError>;

pub async fn send_fee_quote(&self, amount: Amount) -> anyhow::Result<FeeQuote>;
pub async fn receive_fee_quote(&self, ecash: &ECash) -> anyhow::Result<FeeQuote>;
```

`ECash` is base32-encodable/decodable with the `fedimint` prefix — parse a pasted
string with its `FromStr`, render with `Display`.

## 6. Balance (per-unit) — `fedimint_client::Client`

```rust
// USDT spendable balance = mintv2(USDT_UNIT) primary-module balance
pub async fn get_balance_for_unit(&self, unit: AmountUnit) -> anyhow::Result<Amount>;
// call as: client.get_balance_for_unit(fedimint_usdt_common::USDT_UNIT).await?
```

## 7. Registering the modules on the client

```rust
use fedimint_mintv2_client::MintClientInit as Mintv2ClientInit;
use fedimint_usdt_client::UsdtClientInit;

let mut builder = Client::builder(db).await?;
builder.with_module(Mintv2ClientInit); // primary module for USDT_UNIT
builder.with_module(UsdtClientInit);
// … set primary module / connectors, then preview(invite).join(db, root_secret)
```

For the exact join + secret-derivation + OPFS-DB-open sequence in a wasm/worker
context, mirror `fedigents/crates/fedigents-web/src/fedimint.rs`
(`WalletRuntimeCore::connect`) and `browser.js` (`openWalletDb`,
`createWalletWorker`), swapping its Mint(v1)/Lightning/Wallet module set for the
two modules above.

## 8. Federation requirements (for whoever runs the demo)

The wallet needs an invite code to a federation configured with:

- a **`mintv2`** instance whose config-gen `amount_unit == USDT_UNIT`, serving as
  the primary module for that unit, and
- the **`usdt`** module (guardians running `fedimint-usdt-server`, configured with
  the EVM chain / USDT contract / ERC-4337 entrypoint + factory addresses).

`fedimint_usdt_common::UsdtGenParams::default()` targets a local `anvil` dev chain
(chain id 31337). See `modules/fedimint-usdt-tests/` (anvil e2e, the Sepolia
runbook in the branch's docs) for how a real federation is stood up. Standing up
that federation is out of scope for the wallet itself.
