//! Chart panel: the custom K-line view of one followed symbol, persisted per
//! panel instance. The symbol is picked through the search dialog; history
//! comes from the Bars RPC and live updates from the shared stream — daily
//! bars on the 1D timeframe, minute bars aggregated client-side into the
//! chosen intraday bucket (see `widgets/kline`).

use std::collections::BTreeSet;

use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    component::button::Button,
    div,
};

use anysdk::alpaca_stream::StockMessage;

use crate::data::stream::{StreamConsumer, StreamHandle, Wish};
use crate::panels::{Panel, panel_title, refresh_button, reload_on_connect};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
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
    /// The panel's stream subscription (wish, filter, pump, lifecycle).
    stream: StreamHandle<Self>,
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

        let kline = cx.new(|_| KlineState::new(persisted_timeframe));

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
            stream: StreamHandle::new(cx),
        };
        this.kline
            .update(cx, |state, _| state.set_symbol(persisted_symbol));
        this.sync_subscription(cx);
        // The stream may already be up: the panel can open after connect.
        this.stream.attach_pump(cx);
        // Startup load + auto-load once the connection lands.
        Self::refresh(this.kline.downgrade(), cx);
        reload_on_connect::<Self>(this.kline.downgrade(), cx);
        // 桶收盘倒计时:每秒 notify 一次 K 线状态,倒计时在 paint 时按
        // 当前时钟现算。实体随窗口关闭释放,循环自然结束。
        let kline = this.kline.downgrade();
        cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                if kline.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    /// Reload the chart history for the current symbol + timeframe from the
    /// Bars RPC.
    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        crate::widgets::kline::load_history(entity, cx);
    }
}

impl StreamConsumer for ChartPanel {
    const STAGE: &'static str = "chart";

    fn stream_handle(&mut self) -> &mut StreamHandle<Self> {
        &mut self.stream
    }

    /// 实时蜡烛的唯一来源 = trade 流:每笔成交按当前周期归桶喂给
    /// `apply_trade`(同桶增量、越桶开新根),K 线逐笔跟手。bar 推送
    /// (Daily/Minute/Updated)不再消费。每批一次实体 update,由 GPUI
    /// 合并重绘。
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
        self.kline.update(cx, |state, cx| {
            let mut changed = false;
            for trade in &trades {
                changed |=
                    state.apply_trade(trade.time.timestamp_millis(), trade.price, trade.size);
            }
            if changed {
                cx.notify();
            }
        });
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

    /// Publish the followed symbol to the hub: trades 是实时蜡烛的唯一
    /// 数据源(bar 推送已不再消费)。While the tab is not the active one
    /// the set is empty, unsubscribing its share of the union.
    fn sync_subscription(&mut self, cx: &mut Context<Self>) {
        let wanted: BTreeSet<String> = if self.active {
            self.symbol(cx)
                .map(|symbol| BTreeSet::from([symbol]))
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        self.stream.set_wish(
            Wish {
                symbols: wanted,
                trades: true,
                ..Default::default()
            },
            cx,
        );
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
            self.stream.attach_pump(cx);
        } else {
            self.stream.stop_pump();
        }
    }

    fn on_removed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.stream.unsubscribe(cx);
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
        let connected = crate::data::stream::connected(cx);

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
