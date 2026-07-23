//! `WalletRuntimeCore` — the worker-only Fedimint client. Builds/joins the
//! client (mintv2 + usdt modules), reads the USDT balance, drives on-chain
//! peg-in/peg-out, and sends/receives ecash. See docs/FEDIMINT_BRANCH.md for
//! the backend API this maps onto.

// `ClientHandleArc` is `Arc<ClientHandle>`, and on wasm `ClientHandle` is not
// `Send + Sync`. That is fine here: everything runs on the single worker thread.
#![allow(clippy::arc_with_non_send_sync)]

use std::cell::RefCell;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use fedimint_bip39::Mnemonic;
use fedimint_client::secret::RootSecretStrategy;
use fedimint_client::{Client, ClientBuilder, ClientHandleArc, RootSecret};
use fedimint_connectors::ConnectorRegistry;
use fedimint_core::Amount;
use fedimint_core::base32::{self, FEDIMINT_PREFIX};
use fedimint_core::config::FederationId;
use fedimint_core::db::{Database, IDatabaseTransactionOpsCoreTyped};
use fedimint_core::encoding::{Decodable, Encodable};
use fedimint_core::impl_db_record;
use fedimint_core::invite_code::InviteCode;
use fedimint_bip39::Bip39RootSecretStrategy;
use fedimint_cursed_redb::MemAndRedb;
use fedimint_derive_secret::{ChildId, DerivableSecret};
use fedimint_mintv2_client::{ECash, MintClientInit, MintClientModule, MintOperationMeta};
use fedimint_usdt_client::{UsdtClientInit, UsdtClientModule, UsdtOperationMeta};
use fedimint_usdt_common::{EvmAddress, USDT_UNIT, UsdtAmount};

use crate::browser;
use crate::wallet_runtime::{HistoryItem, WorkerEvent, emit_event};

const DB_FILE: &str = "wallet.redb";

/// DB key-prefixes in the single OPFS-backed database. The Fedimint client owns
/// the `ClientDatabase` sub-prefix; the wallet mnemonic lives under its own.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum DbKeyPrefix {
    ClientDatabase = 0x00,
    Mnemonic = 0x01,
}

#[derive(Debug, Clone, Encodable, Decodable, Eq, PartialEq, Hash)]
struct MnemonicKey;

impl_db_record!(
    key = MnemonicKey,
    value = Vec<u8>,
    db_prefix = DbKeyPrefix::Mnemonic,
);

pub struct WalletRuntimeCore {
    base_db: Database,
    connectors: ConnectorRegistry,
    client: RefCell<Option<ClientHandleArc>>,
    pub storage_notice: Option<String>,
}

impl WalletRuntimeCore {
    /// Open the OPFS DB, ensure a mnemonic exists, build the connectors, and —
    /// if this browser has already joined — reopen the client.
    pub async fn connect() -> anyhow::Result<Self> {
        let storage_notice = (!browser::storage_supported()).then(|| {
            "This wallet needs a recent Chromium-based browser (OPFS Sync Access \
             Handles are required for storage)."
                .to_string()
        });

        let handle = browser::open_wallet_db(DB_FILE)
            .await
            .map_err(|e| anyhow::anyhow!("failed to open OPFS database: {e}"))?;
        let cursed = MemAndRedb::new(handle)
            .map_err(|e| anyhow::anyhow!("failed to open redb: {e:?}"))?;
        let base_db = Database::new(cursed, Default::default());

        ensure_mnemonic(&base_db).await?;

        let connectors = ConnectorRegistry::build_from_client_defaults().bind().await?;

        let core = Self {
            base_db,
            connectors,
            client: RefCell::new(None),
            storage_notice,
        };

        if Client::is_initialized(&core.client_db()).await {
            core.open().await?;
        }

        Ok(core)
    }

    fn client_db(&self) -> Database {
        self.base_db
            .with_prefix(vec![DbKeyPrefix::ClientDatabase as u8])
    }

    fn client(&self) -> anyhow::Result<ClientHandleArc> {
        self.client
            .borrow()
            .clone()
            .context("not joined to a federation")
    }

    /// Join a federation from an invite code (idempotent).
    pub async fn join(&self, invite_code: &str) -> anyhow::Result<()> {
        if self.client.borrow().is_some() {
            return Ok(());
        }
        let client_db = self.client_db();
        if Client::is_initialized(&client_db).await {
            return self.open().await;
        }

        let mnemonic = self.get_mnemonic().await?;
        let invite = InviteCode::from_str(invite_code)?;
        let federation_id = invite.federation_id();
        let secret = derive_federation_secret(&mnemonic, &federation_id);

        let builder = build_client_builder().await?;
        let preview = builder.preview(self.connectors.clone(), &invite).await?;
        let client = Arc::new(
            preview
                .join(client_db, RootSecret::StandardDoubleDerive(secret))
                .await?,
        );

        *self.client.borrow_mut() = Some(client);
        Ok(())
    }

    /// Reopen an already-joined client from the persisted config.
    async fn open(&self) -> anyhow::Result<()> {
        let client_db = self.client_db();
        let config = Client::get_config_from_db(&client_db)
            .await
            .context("client config not found in database")?;
        let federation_id = config.calculate_federation_id();

        let mnemonic = self.get_mnemonic().await?;
        let secret = derive_federation_secret(&mnemonic, &federation_id);

        let builder = build_client_builder().await?;
        let client = Arc::new(
            builder
                .open(
                    self.connectors.clone(),
                    client_db,
                    RootSecret::StandardDoubleDerive(secret),
                )
                .await?,
        );

        *self.client.borrow_mut() = Some(client);
        Ok(())
    }

    pub async fn is_joined(&self) -> bool {
        self.client.borrow().is_some() || Client::is_initialized(&self.client_db()).await
    }

    async fn get_mnemonic(&self) -> anyhow::Result<Mnemonic> {
        let mut dbtx = self.base_db.begin_transaction_nc().await;
        let entropy = dbtx
            .get_value(&MnemonicKey)
            .await
            .context("wallet mnemonic missing")?;
        Ok(Mnemonic::from_entropy(&entropy)?)
    }

    // --- balance ---

    /// Raw USDT balance (smallest unit). The USDT-denominated mintv2 instance is
    /// the primary module for `USDT_UNIT`, so this reads the spendable balance.
    pub async fn get_balance(&self) -> anyhow::Result<u64> {
        let client = self.client()?;
        let amount = client.get_balance_for_unit(USDT_UNIT).await?;
        Ok(amount.msats)
    }

    // --- peg-in (deposit) ---

    /// Allocate an EVM deposit address and spawn the auto-watch task that claims
    /// the deposit into ecash once credited.
    pub async fn receive_onchain(&self) -> anyhow::Result<String> {
        let client = self.client()?;
        let usdt = client.get_first_module::<UsdtClientModule>()?;
        let (claim_keypair, address) = usdt.allocate_deposit().await?;
        let address_str = address.to_string();

        // Background auto-watch: ask the federation to watch the address, poll
        // until claimable, submit the claim tx, then notify the UI.
        let watch_client = client.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(usdt) = watch_client.get_first_module::<UsdtClientModule>() else {
                return;
            };
            match usdt
                .check_and_claim(&claim_keypair, Duration::from_secs(3600))
                .await
            {
                Ok(()) => emit_event(WorkerEvent::DepositCredited),
                Err(e) => tracing::warn!("deposit auto-watch ended: {e:#}"),
            }
        });

        Ok(address_str)
    }

    // --- peg-out (withdrawal) ---

    pub async fn withdraw_quote(&self, amount: u64) -> anyhow::Result<(u64, u64)> {
        let client = self.client()?;
        let usdt = client.get_first_module::<UsdtClientModule>()?;
        let quote = usdt.withdraw_fee_quote(UsdtAmount(amount)).await?;
        Ok((quote.max_fee.0, quote.valid_blocks))
    }

    pub async fn withdraw_onchain(
        &self,
        recipient: &str,
        amount: u64,
        max_fee: u64,
    ) -> anyhow::Result<String> {
        let client = self.client()?;
        let usdt = client.get_first_module::<UsdtClientModule>()?;
        let recipient = EvmAddress::from_str(recipient).context("invalid EVM address")?;

        let range = usdt
            .withdraw(recipient, UsdtAmount(amount), UsdtAmount(max_fee))
            .await?;
        let out_point = UsdtClientModule::withdrawal_out_point(&range);
        let block = usdt
            .await_withdrawal_confirmed(out_point, Duration::from_secs(600))
            .await?;

        Ok(format!("Confirmed on-chain at block {block}"))
    }

    // --- ecash (offline P2P) ---

    pub async fn ecash_send(&self, amount: u64) -> anyhow::Result<String> {
        let client = self.client()?;
        let mint = client.get_first_module::<MintClientModule>()?;
        let (_op, ecash) = mint
            .send(Amount::from_msats(amount), serde_json::Value::Null, true)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(base32::encode_prefixed(FEDIMINT_PREFIX, &ecash))
    }

    pub async fn ecash_receive(&self, ecash: &str) -> anyhow::Result<()> {
        let client = self.client()?;
        let mint = client.get_first_module::<MintClientModule>()?;
        let ecash: ECash = base32::decode_prefixed(FEDIMINT_PREFIX, ecash)
            .context("could not parse ecash string")?;
        mint.receive(ecash, serde_json::Value::Null)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(())
    }

    // --- history ---

    pub async fn list_operations(&self, limit: usize) -> anyhow::Result<Vec<HistoryItem>> {
        let client = self.client()?;
        let entries = client
            .operation_log()
            .paginate_operations_rev(limit, None)
            .await;

        Ok(entries
            .into_iter()
            .map(|(_key, entry)| {
                let module = entry.operation_module_kind().to_string();
                let (summary, amount, incoming) = match module.as_str() {
                    "usdt" => match entry.try_meta::<UsdtOperationMeta>() {
                        Ok(UsdtOperationMeta::Claim { amount, .. }) => {
                            ("Deposit", Some(amount.0), Some(true))
                        }
                        Ok(UsdtOperationMeta::Withdraw { amount, .. }) => {
                            ("Withdrawal", Some(amount.0), Some(false))
                        }
                        Err(_) => ("On-chain USDT", None, None),
                    },
                    "mintv2" => match entry.try_meta::<MintOperationMeta>() {
                        Ok(MintOperationMeta::Send { ecash, .. }) => {
                            ("Ecash sent", ecash_amount(&ecash), Some(false))
                        }
                        Ok(MintOperationMeta::Receive { ecash, .. }) => {
                            ("Ecash received", ecash_amount(&ecash), Some(true))
                        }
                        Ok(MintOperationMeta::Reissue { amount, .. }) => {
                            ("Reissue", Some(amount.msats), None)
                        }
                        Err(_) => ("Ecash transfer", None, None),
                    },
                    _ => ("Transaction", None, None),
                };
                HistoryItem {
                    module,
                    summary: summary.to_string(),
                    amount,
                    incoming,
                }
            })
            .collect())
    }
}

/// Decode the raw USDT amount carried inside an ecash string.
fn ecash_amount(ecash: &str) -> Option<u64> {
    base32::decode_prefixed::<ECash>(FEDIMINT_PREFIX, ecash)
        .ok()
        .map(|e| e.amount().msats)
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

async fn build_client_builder() -> anyhow::Result<ClientBuilder> {
    let mut builder = Client::builder().await?;
    builder.with_module(MintClientInit); // mintv2 — primary for USDT_UNIT
    builder.with_module(UsdtClientInit); // on-chain peg-in/peg-out bridge
    Ok(builder)
}

/// Generate + persist a 12-word mnemonic on first run.
async fn ensure_mnemonic(db: &Database) -> anyhow::Result<()> {
    let mut dbtx = db.begin_transaction().await;
    if dbtx.get_value(&MnemonicKey).await.is_none() {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy).map_err(|e| anyhow::anyhow!("getrandom failed: {e}"))?;
        let mnemonic = Mnemonic::from_entropy(&entropy)?;
        dbtx.insert_new_entry(&MnemonicKey, &mnemonic.to_entropy()).await;
    }
    dbtx.commit_tx().await;
    Ok(())
}

/// Same derivation scheme as fedimint-client-rpc: global root → per-federation
/// wallet key, double-derived.
fn derive_federation_secret(mnemonic: &Mnemonic, federation_id: &FederationId) -> DerivableSecret {
    let global_root_secret = Bip39RootSecretStrategy::<12>::to_root_secret(mnemonic);
    let multi_federation_root_secret = global_root_secret.child_key(ChildId(0));
    let federation_root_secret = multi_federation_root_secret.federation_key(federation_id);
    let federation_wallet_root_secret = federation_root_secret.child_key(ChildId(0));
    federation_wallet_root_secret.child_key(ChildId(0))
}
