//! Chart panel: the custom K-line view of one followed symbol, persisted per
//! panel instance. The symbol is picked through the search dialog; history
//! comes from the Bars RPC and live updates from the shared stream — daily
//! bars on the 1D timeframe, minute bars aggregated client-side into the
//! chosen intraday bucket (see `widgets/kline`).

use std::collections::BTreeSet;

use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EntityId, EventEmitter,
    FocusHandle, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Task,
    WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::button::Button,
    div,
};
use protocol::Bar;

use anysdk::alpaca_stream::StockMessage;

use crate::panels::{Panel, panel_title, refresh_button, reload_on_connect};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::stream_dispatch::{SharedSpec, StreamSpec, spawn_filter_stage};
use crate::utils::state::{AppState, HubEvent, ServerStatus, SymbolWish};
use crate::widgets::datatable;
use crate::widgets::dialog::open_search_symbol_dialog;
use crate::widgets::icon_button;
use crate::widgets::kline::{Kline, KlineState, Timeframe};

pub struct ChartPanel {
    focus_handle: FocusHandle,
    kline: Entity<KlineState>,
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

impl Panel for ChartPanel {
    type DataEntity = KlineState;
    const NAME: &'static str = "ChartPanel";
    const ICON: AppIcon = AppIcon::Chart;
    const TITLE_KEY: &'static str = "app.panels.chart";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let persisted_symbol = info
            .get("symbol")
            .and_then(|value| value.as_str())
            .map(String::from);
        let persisted_timeframe = info
            .get("timeframe")
            .and_then(|value| value.as_str())
            .and_then(Timeframe::parse)
            .unwrap_or_default();

        let hub = cx.global::<AppState>().hub.clone();
        let kline = cx.new(|_| KlineState::new(persisted_timeframe));

        // Connection status drives the message pump: connected attaches a
        // fresh broadcast receiver, any other status cancels it. History is
        // RPC (reloaded via `reload_on_connect`); the pump only feeds live
        // bars into the current tail.
        cx.subscribe(&hub, |this, hub, _: &HubEvent, cx| match hub.read(cx).status() {
            ServerStatus::Connected => this.refresh_pump(cx),
            _ => this.pump = None,
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

        // The element paints from the state entity; viewport/interaction
        // changes notify it and this observer keeps the panel repainting.
        cx.observe(&kline, |_, _, cx| cx.notify()).detach();

        // Follow settings changes (e.g. a language switch) to retranslate
        // the title.
        cx.observe_global_in::<AppState>(window, |_, _, cx| cx.notify())
            .detach();

        let mut this = Self {
            focus_handle: cx.focus_handle(),
            kline,
            active: true,
            subscriber,
            spec: SharedSpec::default(),
            pump: None,
        };
        this.kline
            .update(cx, |state, _| state.set_symbol(persisted_symbol));
        this.sync_subscription(cx);
        // The stream may already be up: the panel can open after connect.
        this.refresh_pump(cx);
        // Startup load + auto-load once the connection lands.
        Self::refresh(this.kline.downgrade(), cx);
        reload_on_connect::<Self>(this.kline.downgrade(), cx);
        this
    }

    /// Reload the chart history for the current symbol + timeframe from the
    /// Bars RPC.
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        crate::widgets::kline::load_history(entity, cx);
    }
}

impl ChartPanel {
    fn symbol(&self, cx: &App) -> Option<String> {
        self.kline.read(cx).symbol().map(str::to_owned)
    }

    /// Follow `symbol` and reload its history; a switch starts a fresh chart.
    fn select_symbol(&mut self, symbol: String, handle: AnyWindowHandle, cx: &mut Context<Self>) {
        let changed = self
            .kline
            .update(cx, |state, _| state.set_symbol(Some(symbol)));
        if changed {
            self.sync_subscription(cx);
            Self::refresh(self.kline.downgrade(), cx);
            // 标的/周期是面板 dump 的一部分,但它们的变更不触发
            // DockEvent::LayoutChanged——显式补一次落盘。必须 defer:此刻正在
            // ChartPanel 自己的 update 里,dock dump 会逐个 update 面板实体,
            // 同步调用就撞上"already being updated"重入崩溃(陷阱 1 同款)。
            cx.defer(move |cx| crate::windows::persist_window_dock(handle, cx));
            cx.notify();
        }
    }

    /// Switch the timeframe: resubscribe (the minute-bars wish follows) and
    /// load the new history. The old chart stays visible until the new bars
    /// land.
    fn select_timeframe(
        &mut self,
        timeframe: Timeframe,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let changed = self
            .kline
            .update(cx, |state, _| state.set_timeframe(timeframe));
        if changed {
            self.sync_subscription(cx);
            Self::refresh(self.kline.downgrade(), cx);
            // 同 select_symbol:落盘必须 defer 出本轮 ChartPanel update。
            cx.defer(move |cx| crate::windows::persist_window_dock(handle, cx));
            cx.notify();
        }
    }

    /// Publish the followed symbol to the hub. Trades + daily bars always
    /// ride along (daily tail updates); the minute-bars wish is only set
    /// while an intraday timeframe is on.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.symbol(cx)
                .map(|symbol| BTreeSet::from([symbol]))
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        let bars = self.kline.read(cx).timeframe().is_intraday();
        *self.spec.write().expect("spec lock poisoned") = StreamSpec {
            symbols: wanted.clone(),
            trades: true,
            ..Default::default()
        };
        let hub = cx.global::<AppState>().hub.clone();
        hub.update(cx, |hub, _| {
            hub.set_symbols(
                self.subscriber,
                SymbolWish {
                    symbols: wanted,
                    trades: true,
                    bars,
                    quotes: false,
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
            .spawn(spawn_filter_stage("chart", rx, spec, tx, clock))
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

    /// Feed live bars into the chart tail: daily bars on 1D, minute/updated
    /// bars aggregated into the chosen bucket on intraday timeframes. Each
    /// change notifies the state entity (the panel renders from it, so GPUI
    /// invalidates the view).
    fn apply_batch(&mut self, messages: Vec<StockMessage>, cx: &mut Context<Self>) {
        let timeframe = self.kline.read(cx).timeframe();
        for message in messages {
            match message {
                StockMessage::DailyBar(bar) if timeframe == Timeframe::OneDay => {
                    self.kline.update(cx, |state, cx| {
                        if state.apply_daily_bar(Bar {
                            time_ms: bar.time.timestamp_millis(),
                            open: bar.open,
                            high: bar.high,
                            low: bar.low,
                            close: bar.close,
                            volume: bar.volume,
                            vwap: bar.vwap,
                            trade_count: bar.trade_count,
                        }) {
                            cx.notify();
                        }
                    });
                }
                StockMessage::MinuteBar(bar) | StockMessage::UpdatedBar(bar)
                    if timeframe.is_intraday() =>
                {
                    self.kline.update(cx, |state, cx| {
                        if state.apply_minute_bar(
                            bar.time.timestamp_millis(),
                            bar.open,
                            bar.high,
                            bar.low,
                            bar.close,
                            bar.volume,
                            bar.vwap,
                            bar.trade_count,
                        ) {
                            cx.notify();
                        }
                    });
                }
                _ => {}
            }
        }
    }
}

impl gpui_kit::base::dock::Panel for ChartPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the followed symbol + timeframe per instance; `new` reads
    /// them back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({
            "symbol": self.symbol(cx),
            "timeframe": self.kline.read(cx).timeframe().as_str(),
        }));
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

impl gpui_kit::component::dock::Panel for ChartPanel {
    /// The followed symbol as the tab label; the panel name until one is set.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .symbol(cx)
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    /// Title bar: the timeframe dropdown sits left of the symbol search and
    /// history reload.
    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![
            icon_button(
                "chart-pick-symbol",
                AppIcon::Search,
                tr!("app.chart.search_title"),
                false,
                {
                    let weak = cx.weak_entity();
                    move |_, window, cx| {
                        let handle = window.window_handle();
                        open_search_symbol_dialog(window, cx, {
                            let weak = weak.clone();
                            move |item, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.select_symbol(item.symbol.clone(), handle, cx)
                                });
                            }
                        });
                    }
                },
            ),
            refresh_button::<Self>("chart-refresh", self.kline.downgrade()),
        ])
    }

    /// Pinned to the trailing end of the title bar: the timeframe dropdown
    /// (the `toolbar_buttons` contract is `Vec<Button>`, which cannot carry
    /// a popup-anchored element, so the selector lives here instead).
    fn title_suffix(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        use gpui_kit::component::{
            IconName, Sizable as _, button::ButtonVariants as _, menu::DropdownMenu as _,
        };

        let weak = cx.weak_entity();
        let current = self.kline.read(cx).timeframe();
        let handle = window.window_handle();

        let trigger = Button::new("chart-timeframe")
            .ghost()
            .small()
            .label(current.label())
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _window, _cx| {
                crate::widgets::kline::timeframe_menu(
                    current,
                    {
                        let weak = weak.clone();
                        move |timeframe, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.select_timeframe(timeframe, handle, cx)
                            });
                        }
                    },
                    menu,
                )
            });

        Some(trigger)
    }
}

impl EventEmitter<PanelEvent> for ChartPanel {}

impl gpui_kit::Focusable for ChartPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChartPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.kline.read(cx);
        let loading = state.is_loading();
        let has_symbol = state.symbol().is_some();
        let has_bars = !state.bars().is_empty();
        let loaded = state.has_loaded();
        let connected = matches!(
            cx.global::<AppState>().hub.read(cx).status(),
            ServerStatus::Connected
        );

        // The chart element paints the canvas itself; the panel owns the
        // loading/empty overlays floating above the axes.
        fn overlay(content: impl IntoElement) -> AnyElement {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(content)
                .into_any_element()
        }

        div()
            .size_full()
            .child(Kline::new(self.kline.clone()))
            .children((has_symbol && loading).then(|| overlay(datatable::loading_view())))
            .children((has_symbol && !has_bars && !loading).then(|| {
                let text = if loaded {
                    tr!("app.chart.no_data")
                } else if connected {
                    tr!("app.chart.waiting")
                } else {
                    tr!("app.chart.not_connected")
                };
                overlay(datatable::empty_view(text, cx))
            }))
            .children(
                (!has_symbol)
                    .then(|| overlay(datatable::empty_view(tr!("app.chart.no_symbol"), cx))),
            )
    }
}
