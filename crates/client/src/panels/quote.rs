//! Live quote panel: the top-of-book view of one followed symbol,
//! persisted per panel instance. The symbol is picked through the search
//! dialog; both sides of the spread are shown as two three-column tables
//! side by side — Bid on the left, Ask on the right (price / size / MPID) —
//! straight from the shared Alpaca quote stream (this panel opts into
//! quotes). A one-sided quote is legal (e.g. at the opening auction): the
//! missing side renders as dashes.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use gpui_kit::{
    App, Context, Entity, EntityId, EventEmitter, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    base::h_flex,
    component::button::Button,
    div,
};

use crate::panels::{Panel, panel_title};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{HubEvent, ServerStatus, AppState, SymbolWish};
use crate::widgets::datatable::{self, QuoteSide, QuoteSideDelegate, table_render};
use gpui_kit::component::table::TableState as DataTableState;
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

pub struct QuotePanel {
    focus_handle: FocusHandle,
    /// The followed symbol (uppercased); `None` = nothing followed yet.
    symbol: Option<String>,
    /// The two top-of-book tables — Bid on the left, Ask on the right —
    /// sharing the same three-column structure.
    bid_table: Entity<DataTableState<QuoteSideDelegate>>,
    ask_table: Entity<DataTableState<QuoteSideDelegate>>,
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

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(|s| s.to_ascii_uppercase());

        let hub = cx.global::<AppState>().hub.clone();

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it and drops
        // the quote (the last values are stale by then).
        cx.subscribe(&hub, |this, hub, _: &HubEvent, cx| {
            match hub.read(cx).status() {
                ServerStatus::Connected => {
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
            bid_table: datatable::new_table_state(cx, window, |_| {
                QuoteSideDelegate::new(QuoteSide::Bid)
            }),
            ask_table: datatable::new_table_state(cx, window, |_| {
                QuoteSideDelegate::new(QuoteSide::Ask)
            }),
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
        let hub = cx.global::<AppState>().hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                self.subscriber,
                SymbolWish {
                    symbols: wanted,
                    trades: false,
                    bars: false,
                    quotes: true,
                },
            )
        });
    }

    /// (Re)attach the broadcast-receiver pump, draining in batches: one
    /// entity update per batch keeps the main thread cheap while other
    /// panels' high-rate subscriptions fill the channel.
    fn refresh_pump(&mut self, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().hub.clone();
        let clock = hub.read(cx).clock();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        let spec = self.spec.clone();
        let (tx, mut rx_out) = tokio::sync::mpsc::channel(8);
        cx.background_executor()
            .spawn(spawn_filter_stage("quote", rx, spec, tx, clock))
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
            let Side { price, size, exchange } =
                Side::take(quote.bid_price, quote.bid_size, &quote.bid_exchange);
            self.bid_table.update(cx, |table, cx| {
                if table.delegate_mut().set_row(price, size, exchange) {
                    cx.notify();
                }
            });
            let Side { price, size, exchange } =
                Side::take(quote.ask_price, quote.ask_size, &quote.ask_exchange);
            self.ask_table.update(cx, |table, cx| {
                if table.delegate_mut().set_row(price, size, exchange) {
                    cx.notify();
                }
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
        if connected {
            return;
        }
        // 清空两侧表格(最后值已过期),回退到空态破折号。
        for table in [&self.bid_table, &self.ask_table] {
            table.update(cx, |table, cx| {
                if table.delegate_mut().set_row(None, None, None) {
                    cx.notify();
                }
            });
        }
    }

    /// Follow `symbol` and resubscribe; a switch drops the old top of book.
    fn select_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        let symbol = symbol.to_ascii_uppercase();
        if self.symbol.as_deref() == Some(symbol.as_str()) {
            return;
        }
        self.symbol = Some(symbol);
        self.set_connected(false, cx);
        self.sync_subscription(cx);
        cx.notify();
    }
}

impl Render for QuotePanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Two three-column tables side by side — Bid on the left, Ask on the
        // right — the standard quote-ladder arrangement. items_stretch:
        // h_flex 默认交叉轴居中会让表格垂直悬浮,这里必须顶满。
        h_flex()
            .size_full()
            .items_stretch()
            .gap_1()
            .p_1()
            .child(div().flex_1().child(table_render(&self.bid_table)))
            .child(div().flex_1().child(table_render(&self.ask_table)))
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
        let hub = cx.global::<AppState>().hub.clone();
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
