//! Live quote panel: the top-of-book view of one followed symbol,
//! persisted per panel instance. The symbol is picked through the search
//! dialog; both sides of the spread are shown — price, size and exchange —
//! straight from the shared Alpaca quote stream (this panel opts into
//! quotes). A one-sided quote is legal (e.g. at the opening auction): the
//! missing side renders as dashes.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use gpui_kit::{
    App, Context, Div, EntityId, EventEmitter, FocusHandle, Hsla, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    base::{h_flex, v_flex},
    component::{ActiveTheme as _, button::Button},
    div,
};

use crate::panels::{Panel, panel_title};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{RelayHubEvent, RelayStatus, AppState, SymbolWish};
use crate::widgets::dialog::open_search_symbol_dialog;
use crate::widgets::icon_button;

/// One side of the top of book: price, size and the exchange it rests on.
#[derive(Clone, Default)]
struct Side {
    price: Option<f64>,
    size: Option<i64>,
    exchange: Option<String>,
}

impl Side {
    /// Take a side from a quote; the price must be positive for the side to
    /// mean anything (a zero price is the stream's "no quote on this side").
    fn take(price: f64, size: i64, exchange: &str) -> Self {
        let (price, size) = if price > 0.0 {
            (Some(price), Some(size))
        } else {
            (None, None)
        };
        Self {
            price,
            size,
            exchange: (price.is_some()).then(|| exchange.to_string()),
        }
    }
}

/// The full top of book of the followed symbol.
#[derive(Clone, Default)]
struct Quote {
    bid: Side,
    ask: Side,
}

pub struct QuotePanel {
    focus_handle: FocusHandle,
    /// The followed symbol (uppercased); `None` = nothing followed yet.
    symbol: Option<String>,
    /// The latest top of book; `None` while no quote has arrived.
    quote: Option<Quote>,
    /// Whether this tab is the active one in its tab group. A hidden panel
    /// unsubscribes from the stream and resubscribes when shown again.
    active: bool,
    subscriber: EntityId,
    /// The filter the background drain stage applies, kept in step with
    /// the selection and rules.
    spec: SharedSpec,
    /// Live message pump for the current connection.
    pump: Option<Task<()>>,
}

impl Panel for QuotePanel {
    type DataEntity = Self;
    const NAME: &'static str = "QuotePanel";
    const ICON: AppIcon = AppIcon::Quote;
    const TITLE_KEY: &'static str = "app.panels.quote";

    fn new(_window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(|s| s.to_ascii_uppercase());

        let hub = cx.global::<AppState>().relay_hub.clone();

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it and drops
        // the quote (the last values are stale by then).
        cx.subscribe(&hub, |this, hub, _: &RelayHubEvent, cx| {
            match hub.read(cx).status() {
                RelayStatus::Connected => {
                    this.set_connected(true, cx);
                    this.refresh_pump(cx);
                }
                _ => {
                    this.set_connected(false, cx);
                    this.pump = None;
                }
            }
        })
        .detach();

        // Release the stream subscription no matter how this panel dies.
        let subscriber = cx.entity_id();
        let release_hub = hub.clone();
        cx.on_release(move |_, cx| {
            release_hub.update(cx, |hub, _| {
                hub.set_symbols(subscriber, SymbolWish::default())
            });
        })
        .detach();

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            symbol,
            quote: None,
            active: true,
            subscriber,
            spec: SharedSpec::default(),
            pump: None,
        };
        this.sync_subscription(cx);
        // The stream may already be up: the panel can open after connect.
        this.refresh_pump(cx);
        this
    }

    fn refresh(_entity: WeakEntity<Self>, _cx: &mut App) {}
}

impl QuotePanel {
    /// Publish the followed symbol to the hub with quotes opted in. While
    /// the tab is not the active one the set is empty, unsubscribing its
    /// share of the union.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.symbol
                .as_ref()
                .map(|symbol| BTreeSet::from([symbol.clone()]))
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        *self.spec.write().expect("spec lock poisoned") = StreamSpec {
            symbols: wanted.clone(),
            quotes: true,
            ..Default::default()
        };
        *self.spec.write().expect("spec lock poisoned") = StreamSpec {
            symbols: wanted.clone(),
            quotes: true,
            ..Default::default()
        };
        let hub = cx.global::<AppState>().relay_hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                self.subscriber,
                SymbolWish {
                    symbols: wanted,
                    trades: false,
                    quotes: true,
                },
            )
        });
    }

    /// (Re)attach the broadcast-receiver pump, draining in batches: one
    /// entity update per batch keeps the main thread cheap while other
    /// panels' high-rate subscriptions fill the channel.
    fn refresh_pump(&mut self, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().relay_hub.clone();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        let spec = self.spec.clone();
        let (tx, mut rx_out) = tokio::sync::mpsc::channel(8);
        cx.background_executor()
            .spawn(spawn_filter_stage("quote", rx, spec, tx))
            .detach();
        self.pump = Some(cx.spawn(async move |this, cx| {
            while let Some(batch) = rx_out.recv().await {
                if this
                    .update(cx, |this, cx| this.apply_batch(batch, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn apply_batch(&mut self, messages: Vec<StockMessage>, cx: &mut Context<Self>) {
        let Some(symbol) = &self.symbol else {
            return;
        };
        let mut applied = false;
        for message in messages {
            let StockMessage::Quote(quote) = message else {
                continue;
            };
            if !quote.symbol.eq_ignore_ascii_case(symbol) {
                continue;
            }
            self.quote = Some(Quote {
                bid: Side::take(quote.bid_price, quote.bid_size, &quote.bid_exchange),
                ask: Side::take(quote.ask_price, quote.ask_size, &quote.ask_exchange),
            });
            applied = true;
        }
        if applied {
            cx.notify();
        }
    }

    /// Mirror the stream status into the top of book: with the stream down
    /// the last values are stale, and a quote view must not show them.
    fn set_connected(&mut self, connected: bool, cx: &mut Context<Self>) {
        if !connected && self.quote.is_some() {
            self.quote = None;
            cx.notify();
        }
    }

    /// Follow `symbol` and resubscribe; a switch drops the old top of book.
    fn select_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        let symbol = symbol.to_ascii_uppercase();
        if self.symbol.as_deref() == Some(symbol.as_str()) {
            return;
        }
        self.symbol = Some(symbol);
        self.quote = None;
        self.sync_subscription(cx);
        cx.notify();
    }
}

/// A flat chip (native rectangle, no rounding) that scales with the window:
/// every chip in a row takes an equal share of the available width, text
/// centered. Same shape as the ETF link calculator's chips.
fn pill(text: impl Into<SharedString>, bg: Hsla, fg: Hsla) -> Div {
    div()
        .flex()
        .flex_1()
        .justify_center()
        .py_2()
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .child(text.into())
}

/// A muted column label, same footprint as a chip.
fn column_label(text: &str, cx: &Context<QuotePanel>) -> Div {
    div()
        .flex()
        .flex_1()
        .justify_center()
        .py_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.to_string())
}

/// `1,234,567` — sizes read better grouped.
fn format_size(size: i64) -> String {
    let digits = size.abs().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    if size < 0 {
        out.insert(0, '-');
    }
    out
}

impl Render for QuotePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(symbol) = &self.symbol else {
            // Nothing followed: the toolbar search is the way in.
            return div()
                .size_full()
                .flex()
                .justify_center()
                .items_center()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("app.quote.no_symbol")),
                )
                .into_any_element();
        };
        let quote = self.quote.clone().unwrap_or_default();
        let theme = cx.theme();

        // Header: the symbol chip followed by the three column labels.
        let header = h_flex()
            .items_center()
            .gap_2()
            .child(pill(
                symbol.clone(),
                theme.secondary,
                theme.secondary_foreground,
            ))
            .child(column_label(&tr!("app.quote.price"), cx))
            .child(column_label(&tr!("app.quote.size"), cx))
            .child(column_label(&tr!("app.quote.exchange"), cx));

        // One row per side of the spread: label, price, size, exchange. Bid
        // takes the user's bullish color (red in the default theme), ask the
        // bearish one — the same assignment as the quote rows everywhere. A
        // missing side (one-sided quotes are legal) renders as dashes.
        let side_row = |label: &str, side: &Side, price_bg: Hsla| {
            let dash = || pill("—", theme.secondary, theme.secondary_foreground);
            h_flex()
                .items_center()
                .gap_2()
                .child(pill(label, theme.secondary, theme.secondary_foreground))
                .child(match side.price {
                    Some(price) => pill(format!("{price:.2}"), price_bg, theme.background),
                    None => dash(),
                })
                .child(match side.size {
                    Some(size) => pill(
                        format_size(size),
                        theme.secondary,
                        theme.secondary_foreground,
                    ),
                    None => dash(),
                })
                .child(match &side.exchange {
                    Some(exchange) => pill(
                        exchange.clone(),
                        theme.secondary,
                        theme.secondary_foreground,
                    ),
                    None => dash(),
                })
        };

        v_flex()
            .size_full()
            .justify_center()
            .p_4()
            .gap_3()
            .child(header)
            .child(side_row(
                &tr!("app.quote.bid"),
                &quote.bid,
                theme.chart_bullish,
            ))
            .child(side_row(
                &tr!("app.quote.ask"),
                &quote.ask,
                theme.chart_bearish,
            ))
            .into_any_element()
    }
}

impl gpui_kit::base::dock::Panel for QuotePanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the followed symbol per instance; `new` reads it back.
    fn dump(&self, _cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "symbol": self.symbol }));
        state
    }

    fn set_active(&mut self, active: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.active = active;
        self.sync_subscription(cx);
        // A backgrounded panel has no subscription to feed its pump: stop
        // it, so it neither wakes for other panels' messages nor burns
        // cycles while hidden. Reactivating (or a later `Connected` event)
        // reattaches it.
        if active {
            self.refresh_pump(cx);
        } else {
            self.pump = None;
        }
    }

    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().relay_hub.clone();
        let subscriber = self.subscriber;
        hub.update(cx, |hub, _| {
            hub.set_symbols(subscriber, SymbolWish::default())
        });
        self.pump = None;
    }
}

impl EventEmitter<PanelEvent> for QuotePanel {}

impl gpui_kit::Focusable for QuotePanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for QuotePanel {
    /// The followed symbol as the tab label; the panel name until one is
    /// set.
    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .symbol
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    /// Title-bar button: search a symbol and follow its top of book.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        let weak = cx.weak_entity();
        Some(vec![icon_button(
            "quote-pick-symbol",
            AppIcon::Search,
            tr!("app.quote.search_title"),
            false,
            move |_, window, cx| {
                open_search_symbol_dialog(window, cx, {
                    let weak = weak.clone();
                    move |item, cx| {
                        let _ =
                            weak.update(cx, |this, cx| this.select_symbol(item.symbol.clone(), cx));
                    }
                });
            },
        )])
    }
}
