//! Leptos (CSR) UI. Talks only to the `WalletRuntime` proxy — it never touches
//! the Fedimint client, which lives in the worker.

use leptos::prelude::*;

use crate::wallet_runtime::{HistoryItem, QuoteInfo, WalletRuntime};

/// Which top-level screen is showing.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Loading,
    Onboard,
    Home,
}

/// A shared "refresh the balance/history" trigger cards can bump.
#[derive(Clone, Copy)]
struct Refresh(RwSignal<u32>);

#[component]
pub fn App() -> impl IntoView {
    let runtime = WalletRuntime::new();
    provide_context(runtime.clone());
    provide_context(Refresh(RwSignal::new(0)));

    let phase = RwSignal::new(Phase::Loading);
    let notice = RwSignal::new(None::<String>);

    let boot_rt = runtime.clone();
    wasm_bindgen_futures::spawn_local(async move {
        match boot_rt.connect().await {
            Ok(info) => {
                if !info.storage_ok {
                    notice.set(info.storage_notice);
                }
                match boot_rt.is_joined().await {
                    Ok(true) => phase.set(Phase::Home),
                    Ok(false) => phase.set(Phase::Onboard),
                    Err(e) => {
                        notice.set(Some(format!("Could not read wallet state: {e}")));
                        phase.set(Phase::Onboard);
                    }
                }
            }
            Err(e) => {
                notice.set(Some(format!("Failed to start wallet: {e}")));
                phase.set(Phase::Onboard);
            }
        }
    });

    view! {
        <div class="app">
            {move || match phase.get() {
                Phase::Loading => view! { <div class="center dim">"Loading…"</div> }.into_any(),
                Phase::Onboard => view! { <Onboarding phase=phase /> }.into_any(),
                Phase::Home => view! { <Home /> }.into_any(),
            }}
            {move || notice.get().map(|n| view! { <div class="status error">{n}</div> })}
        </div>
    }
}

#[component]
fn Onboarding(phase: RwSignal<Phase>) -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let invite = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let err = RwSignal::new(None::<String>);

    let on_join = move |_| {
        let code = invite.get().trim().to_string();
        if code.is_empty() {
            err.set(Some("Enter an invite code.".to_string()));
            return;
        }
        busy.set(true);
        err.set(None);
        let runtime = runtime.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match runtime.join(code).await {
                Ok(()) => phase.set(Phase::Home),
                Err(e) => err.set(Some(format!("{e}"))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="onboard">
            <h1>"USDT Wallet"</h1>
            <p>"Paste a federation invite code to join. Your wallet key is generated
                and stored locally in this browser."</p>
            <textarea
                placeholder="fed11..."
                prop:value=move || invite.get()
                on:input=move |ev| invite.set(event_target_value(&ev))
            />
            <button on:click=on_join disabled=move || busy.get()>
                {move || if busy.get() { "Joining…" } else { "Join federation" }}
            </button>
            {move || err.get().map(|e| view! { <div class="status error">{e}</div> })}
        </div>
    }
}

#[component]
fn Home() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let refresh = expect_context::<Refresh>();
    let balance = RwSignal::new(None::<u64>);

    // Refresh the balance whenever the trigger bumps (and once on mount).
    let bal_rt = runtime.clone();
    Effect::new(move |_| {
        refresh.0.get();
        let bal_rt = bal_rt.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(b) = bal_rt.get_balance().await {
                balance.set(Some(b));
            }
        });
    });

    // Worker events (e.g. a credited deposit) bump the same trigger.
    runtime.set_event_listener(move |_ev| {
        refresh.0.update(|n| *n += 1);
    });

    view! {
        <div class="brand"><span class="dot">"₮"</span> "USDT Wallet"</div>

        <div class="balance-card">
            <div class="balance-label">"Balance"</div>
            <div class="balance-value">
                {move || match balance.get() {
                    Some(b) => format_usdt(b),
                    None => "—".to_string(),
                }}
                <span class="unit">"USDT"</span>
            </div>
        </div>

        <ReceiveCard />
        <SendCard />
        <EcashSendCard />
        <EcashReceiveCard />
        <HistoryCard />
    }
}

#[component]
fn ReceiveCard() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let address = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let err = RwSignal::new(None::<String>);

    let on_reveal = move |_| {
        busy.set(true);
        err.set(None);
        let runtime = runtime.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match runtime.receive_onchain().await {
                Ok(addr) => address.set(Some(addr)),
                Err(e) => err.set(Some(format!("{e}"))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="card">
            <h3>"Receive on-chain"</h3>
            <p class="hint">"Get an EVM deposit address. USDT sent to it is minted into
                your ecash balance automatically."</p>
            {move || match address.get() {
                None => view! {
                    <button on:click=on_reveal.clone() disabled=move || busy.get()>
                        {move || if busy.get() { "Allocating…" } else { "Show deposit address" }}
                    </button>
                }.into_any(),
                Some(addr) => view! {
                    <div class="qr" inner_html=qr_svg(&addr)></div>
                    <div class="address-box mono">{addr.clone()}</div>
                    <CopyButton text=addr />
                    <div class="status waiting"><span class="spin"></span>"Waiting for deposit…"</div>
                }.into_any(),
            }}
            {move || err.get().map(|e| view! { <div class="status error">{e}</div> })}
        </div>
    }
}

#[component]
fn SendCard() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let refresh = expect_context::<Refresh>();
    let recipient = RwSignal::new(String::new());
    let amount = RwSignal::new(String::new());
    let quote = RwSignal::new(None::<QuoteInfo>);
    let status = RwSignal::new(None::<String>);
    let err = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let get_quote = {
        let runtime = runtime.clone();
        move |_| {
            let Some(raw) = parse_usdt(&amount.get()) else {
                err.set(Some("Enter a valid amount.".to_string()));
                return;
            };
            err.set(None);
            status.set(None);
            busy.set(true);
            let runtime = runtime.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match runtime.withdraw_quote(raw).await {
                    Ok(q) => quote.set(Some(q)),
                    Err(e) => err.set(Some(format!("{e}"))),
                }
                busy.set(false);
            });
        }
    };

    let confirm = {
        let runtime = runtime.clone();
        move |_| {
            let (Some(raw), Some(q)) = (parse_usdt(&amount.get()), quote.get()) else {
                return;
            };
            let to = recipient.get().trim().to_string();
            if to.is_empty() {
                err.set(Some("Enter a recipient address.".to_string()));
                return;
            }
            busy.set(true);
            err.set(None);
            let runtime = runtime.clone();
            wasm_bindgen_futures::spawn_local(async move {
                match runtime.withdraw_onchain(to, raw, q.max_fee).await {
                    Ok(s) => {
                        status.set(Some(s));
                        quote.set(None);
                        refresh.0.update(|n| *n += 1);
                    }
                    Err(e) => err.set(Some(format!("{e}"))),
                }
                busy.set(false);
            });
        }
    };

    view! {
        <div class="card">
            <h3>"Send on-chain"</h3>
            <label>"Recipient EVM address"</label>
            <input
                placeholder="0x…"
                prop:value=move || recipient.get()
                on:input=move |ev| recipient.set(event_target_value(&ev))
            />
            <label>"Amount (USDT)"</label>
            <input
                placeholder="0.00"
                prop:value=move || amount.get()
                on:input=move |ev| amount.set(event_target_value(&ev))
            />
            {move || match quote.get() {
                None => view! {
                    <button on:click=get_quote.clone() disabled=move || busy.get()>
                        {move || if busy.get() { "Fetching quote…" } else { "Get fee quote" }}
                    </button>
                }.into_any(),
                Some(q) => view! {
                    <div class="status info">
                        {format!("Max fee {} USDT — valid ~{} blocks", format_usdt(q.max_fee), q.valid_blocks)}
                    </div>
                    <button on:click=confirm.clone() disabled=move || busy.get()>
                        {move || if busy.get() { "Withdrawing…" } else { "Confirm withdrawal" }}
                    </button>
                }.into_any(),
            }}
            {move || status.get().map(|s| view! { <div class="status info">{s}</div> })}
            {move || err.get().map(|e| view! { <div class="status error">{e}</div> })}
        </div>
    }
}

#[component]
fn EcashSendCard() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let refresh = expect_context::<Refresh>();
    let amount = RwSignal::new(String::new());
    let ecash = RwSignal::new(None::<String>);
    let err = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let on_send = move |_| {
        let Some(raw) = parse_usdt(&amount.get()) else {
            err.set(Some("Enter a valid amount.".to_string()));
            return;
        };
        err.set(None);
        busy.set(true);
        let runtime = runtime.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match runtime.ecash_send(raw).await {
                Ok(s) => {
                    ecash.set(Some(s));
                    refresh.0.update(|n| *n += 1);
                }
                Err(e) => err.set(Some(format!("{e}"))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="card">
            <h3>"Send ecash"</h3>
            <p class="hint">"Spend to an offline ecash string you can hand to anyone."</p>
            {move || match ecash.get() {
                None => view! {
                    <input
                                placeholder="Amount (USDT)"
                        prop:value=move || amount.get()
                        on:input=move |ev| amount.set(event_target_value(&ev))
                    />
                    <button on:click=on_send.clone() disabled=move || busy.get()>
                        {move || if busy.get() { "Creating…" } else { "Create ecash" }}
                    </button>
                }.into_any(),
                Some(s) => view! {
                    <div class="qr" inner_html=qr_svg(&s)></div>
                    <div class="address-box mono">{truncate_middle(&s)}</div>
                    <CopyButton text=s />
                    <button class="secondary" on:click=move |_| ecash.set(None)>"Done"</button>
                }.into_any(),
            }}
            {move || err.get().map(|e| view! { <div class="status error">{e}</div> })}
        </div>
    }
}

#[component]
fn EcashReceiveCard() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let refresh = expect_context::<Refresh>();
    let input = RwSignal::new(String::new());
    let status = RwSignal::new(None::<String>);
    let err = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let on_receive = move |_| {
        let s = input.get().trim().to_string();
        if s.is_empty() {
            err.set(Some("Paste an ecash string.".to_string()));
            return;
        }
        err.set(None);
        status.set(None);
        busy.set(true);
        let runtime = runtime.clone();
        wasm_bindgen_futures::spawn_local(async move {
            match runtime.ecash_receive(s).await {
                Ok(()) => {
                    status.set(Some("Redeemed into your balance.".to_string()));
                    input.set(String::new());
                    refresh.0.update(|n| *n += 1);
                }
                Err(e) => err.set(Some(format!("{e}"))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="card">
            <h3>"Receive ecash"</h3>
            <textarea
                placeholder="fedimint1…"
                prop:value=move || input.get()
                on:input=move |ev| input.set(event_target_value(&ev))
            />
            <button on:click=on_receive disabled=move || busy.get()>
                {move || if busy.get() { "Redeeming…" } else { "Redeem" }}
            </button>
            {move || status.get().map(|s| view! { <div class="status info">{s}</div> })}
            {move || err.get().map(|e| view! { <div class="status error">{e}</div> })}
        </div>
    }
}

#[component]
fn HistoryCard() -> impl IntoView {
    let runtime = expect_context::<WalletRuntime>();
    let refresh = expect_context::<Refresh>();
    let items = RwSignal::new(Vec::<HistoryItem>::new());

    let load_rt = runtime.clone();
    Effect::new(move |_| {
        refresh.0.get();
        let load_rt = load_rt.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok(list) = load_rt.list_operations(20).await {
                items.set(list);
            }
        });
    });

    view! {
        <div class="card">
            <h3>"History"</h3>
            {move || {
                let list = items.get();
                if list.is_empty() {
                    view! { <p class="hint">"No transactions yet."</p> }.into_any()
                } else {
                    list.into_iter()
                        .map(|it| {
                            let (amount_text, amount_class) = match (it.amount, it.incoming) {
                                (Some(raw), Some(true)) => (format!("+{} USDT", format_usdt(raw)), "amt pos"),
                                (Some(raw), Some(false)) => (format!("-{} USDT", format_usdt(raw)), "amt neg"),
                                (Some(raw), None) => (format!("{} USDT", format_usdt(raw)), "amt"),
                                (None, _) => (String::new(), "amt"),
                            };
                            view! {
                                <div class="history-item">
                                    <span class="kind">{it.summary}</span>
                                    <span class=amount_class>{amount_text}</span>
                                </div>
                            }
                        })
                        .collect_view()
                        .into_any()
                }
            }}
        </div>
    }
}

#[component]
fn CopyButton(text: String) -> impl IntoView {
    let copied = RwSignal::new(false);
    let on_click = move |_| {
        let text = text.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = crate::browser::clipboard_write(&text).await;
        });
        copied.set(true);
    };
    view! {
        <button class="secondary" on:click=on_click>
            {move || if copied.get() { "Copied!" } else { "Copy" }}
        </button>
    }
}

// ---------------------------------------------------------------------------
// formatting helpers
// ---------------------------------------------------------------------------

/// Render a raw USDT amount (10^-6 units) as a trimmed decimal string.
fn format_usdt(raw: u64) -> String {
    let whole = raw / 1_000_000;
    let frac = raw % 1_000_000;
    if frac == 0 {
        whole.to_string()
    } else {
        let s = format!("{whole}.{frac:06}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Parse a user-entered USDT decimal (≤6 places) into raw units.
fn parse_usdt(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (whole_str, frac_str) = s.split_once('.').unwrap_or((s, ""));
    if frac_str.len() > 6 || !frac_str.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: u64 = whole_str.parse().ok()?;
    let frac: u64 = if frac_str.is_empty() {
        0
    } else {
        format!("{frac_str:0<6}").parse().ok()?
    };
    whole.checked_mul(1_000_000)?.checked_add(frac)
}

fn truncate_middle(s: &str) -> String {
    if s.len() <= 24 {
        s.to_string()
    } else {
        format!("{}…{}", &s[..12], &s[s.len() - 8..])
    }
}

/// Render `data` as an inline SVG QR code.
fn qr_svg(data: &str) -> String {
    use qrcode::render::svg;
    match qrcode::QrCode::new(data.as_bytes()) {
        Ok(code) => code
            .render::<svg::Color>()
            .min_dimensions(180, 180)
            .quiet_zone(false)
            .dark_color(svg::Color("#0b1220"))
            .light_color(svg::Color("#ffffff"))
            .build(),
        Err(_) => String::new(),
    }
}
