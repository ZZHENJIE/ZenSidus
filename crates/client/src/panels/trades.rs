//! Time & sales panel: the live trade tape of one followed symbol,
//! persisted per panel instance. The symbol is picked through the search
//! dialog; the tape is a hit log (newest first, capped) recorded from the
//! shared Alpaca stream — trades are always subscribed for every wanted
//! symbol, so this panel opts into no quotes.

use std::collections::BTreeSet;

use anysdk::alpaca_stream::StockMessage;
use gpui_kit::{
    App, Context, Entity, EntityId, EventEmitter, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::table::TableState,
    div,
};

use crate::panels::{Panel, panel_title};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{RelayHubEvent, RelayStatus, AppState, SymbolWish};
use crate::widgets::datatable::{self, TradesDelegate};
use crate::widgets::dialog::open_search_symbol_dialog;
use crate::widgets::icon_button;

pub struct TradesPanel {
    focus_handle: FocusHandle,
    table: Entity<TableState<TradesDelegate>>,
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

impl Panel for TradesPanel {
    type DataEntity = TableState<TradesDelegate>;
    const NAME: &'static str = "TradesPanel";
    const ICON: AppIcon = AppIcon::Trades;
    const TITLE_KEY: &'static str = "app.panels.trades";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let persisted_symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(String::from);

        let hub = cx.global::<AppState>().relay_hub.clone();
        let connected = matches!(hub.read(cx).status(), RelayStatus::Connected);

        let table = datatable::new_table_state(cx, window, |_| {
            TradesDelegate::new(persisted_symbol.clone(), connected)
        });

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it. The tape
        // survives a disconnect — it is history, not a snapshot.
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

        // Follow settings changes (e.g. a language switch) to retranslate
        // the column headers.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            table,
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

    /// Resubscribe is a no-op for the tape (trades ride along with every
    /// wanted symbol); the hook exists for the toolbar refresh semantics.
    fn refresh(_entity: WeakEntity<Self::DataEntity>, _cx: &mut App) {}
}

impl TradesPanel {
    fn symbol(&self, cx: &App) -> Option<String> {
        self.table.read(cx).delegate().symbol.clone()
    }

    /// Publish the followed symbol to the hub (trades only). While the tab
    /// is not the active one the set is empty, unsubscribing its share of
    /// the union.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.symbol(cx)
                .map(|symbol| BTreeSet::from([symbol]))
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        *self.spec.write().expect("spec lock poisoned") = StreamSpec {
            symbols: wanted.clone(),
            trades: true,
            ..Default::default()
        };
        *self.spec.write().expect("spec lock poisoned") = StreamSpec {
            symbols: wanted.clone(),
            trades: true,
            ..Default::default()
        };
        let hub = cx.global::<AppState>().relay_hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                self.subscriber,
                SymbolWish {
                    symbols: wanted,
                    trades: true,
                    quotes: false,
                },
            )
        });
    }

    /// (Re)attach the broadcast-receiver pump, draining in batches: one
    /// entity update per batch keeps the main thread cheap while other
    /// panels' high-rate subscriptions fill the channel.
    fn refresh_pump(&mut self, cx: &mut Context<Self>) {
        let hub = cx.global::<AppState>().relay_hub.clone();
        let clock = hub.read(cx).clock();
        let Some(rx) = hub.read(cx).stock_messages() else {
            return;
        };
        let spec = self.spec.clone();
        let (tx, mut rx_out) = tokio::sync::mpsc::channel(8);
        cx.background_executor()
            .spawn(spawn_filter_stage("trades", rx, spec, tx, clock))
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
        let trades: Vec<_> = messages
            .into_iter()
            .filter_map(|message| match message {
                StockMessage::Trade(trade) => Some(trade),
                _ => None,
            })
            .collect();
        if trades.is_empty() {
            return;
        }
        for trade in &trades {
            self.table.update(cx, |state, cx| {
                if state.delegate_mut().apply_trade(trade) {
                    cx.notify();
                }
            });
        }
    }

    /// Mirror the stream status into the tables' empty-state hint.
    fn set_connected(&mut self, connected: bool, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            if delegate.connected == connected {
                return;
            }
            delegate.connected = connected;
            cx.notify();
        });
    }

    /// Follow `symbol` and resubscribe; a switch starts a fresh tape.
    fn select_symbol(&mut self, symbol: String, cx: &mut Context<Self>) {
        let changed = self
            .table
            .update(cx, |state, _| state.delegate_mut().set_symbol(Some(symbol)));
        if changed {
            self.sync_subscription(cx);
            cx.notify();
        }
    }
}

impl gpui_kit::base::dock::Panel for TradesPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the followed symbol per instance; `new` reads it back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "symbol": self.symbol(cx) }));
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

impl EventEmitter<PanelEvent> for TradesPanel {}

impl gpui_kit::Focusable for TradesPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for TradesPanel {
    /// The followed symbol as the tab label; the panel name until one is
    /// set.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .symbol(cx)
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    /// Title-bar button: search a symbol and follow its tape.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<gpui_kit::component::button::Button>> {
        let weak = cx.weak_entity();
        Some(vec![icon_button(
            "trades-pick-symbol",
            AppIcon::Search,
            tr!("app.trades.search_title"),
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

impl Render for TradesPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(datatable::table_render(&self.table))
    }
}
