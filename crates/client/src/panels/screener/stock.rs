//! Screener panel: one saved Finviz preset's export as a table. The preset
//! picker (with add/delete in its dropdown footer) rides the tab title bar;
//! the selection is persisted per panel instance. A second view renders the
//! result's tickers as lazily-loaded K-line charts (virtual list — only the
//! visible range's charts exist and fetch data), gated on the export having
//! a Ticker column.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::{
    App, AppContext as _, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString, Size,
    Stateful, StatefulInteractiveElement as _, Styled, WeakEntity, Window,
    assets::IconName,
    base::dock::{PanelEvent, PanelInfo, PanelState},
    base::{h_flex, v_flex},
    component::{
        ActiveTheme as _, VirtualListScrollHandle, button::Button, combobox::ComboboxEvent,
        scroll::ScrollableElement as _, searchable_list::SearchableVec, table::TableState,
        v_virtual_list,
    },
    div, px, size,
};

use crate::panels::{Panel, panel_title, refresh_button};
use crate::tr;
use crate::utils::AppIcon;
use crate::utils::state::AppState;
use crate::widgets::datatable::{self, StockScreenerDelegate};
use crate::widgets::dialog::{open_confirm_dialog, open_screener_preset_dialog};
use crate::widgets::kline::{
    self, KlineState, Timeframe,
    card::{CARD_H, kline_card},
};
use crate::widgets::picker::{PickerAction, StringPicker};

/// The panel's content view.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Table,
    Chart,
}

pub struct StockScreenerPanel {
    focus_handle: FocusHandle,
    picker: StringPicker,
    table: Entity<TableState<StockScreenerDelegate>>,
    /// Content view: the table or the chart list.
    view_mode: ViewMode,
    /// The chart view's K-line timeframe; switching clears every chart state
    /// and the visible range re-fetches.
    chart_timeframe: Timeframe,
    /// Per-ticker K-line states, created lazily as the virtual list reveals
    /// them; cleared whenever the result set or the timeframe changes.
    charts: BTreeMap<String, Entity<KlineState>>,
    /// The chart list's scroll handle: tracking it is what makes the wheel
    /// scroll the list (the cards themselves pass wheel events through).
    chart_scroll: VirtualListScrollHandle,
}

impl Panel for StockScreenerPanel {
    type DataEntity = TableState<StockScreenerDelegate>;
    const NAME: &'static str = "StockScreenerPanel";
    const ICON: AppIcon = AppIcon::Filter;
    const TITLE_KEY: &'static str = "app.panels.stock_screener";

    fn new(window: &mut Window, cx: &mut Context<Self>, info: &serde_json::Value) -> Self {
        let focus_handle = cx.focus_handle();
        let persisted_preset = info
            .get("preset")
            .and_then(|value| value.as_str())
            .map(String::from);

        let delegate = StockScreenerDelegate::new(persisted_preset, cx);
        let picker = StringPicker::new(
            StockScreenerDelegate::preset_names(cx),
            delegate.preset.clone(),
            window,
            cx,
        );
        let table = datatable::new_table_state(cx, window, move |_| delegate);

        // Preset lists are shared settings; follow edits made elsewhere
        // (settings Finviz page or other Screener instances).
        cx.observe_global_in::<AppState>(window, |this, window, cx| {
            let changed = this.table.update(cx, |state, cx| {
                let changed = state.delegate_mut().sync_with_settings(cx);
                if changed {
                    cx.notify();
                }
                changed
            });
            if changed {
                Self::fetch(this.table.downgrade(), cx);
            }
            // The picker mirrors the delegate once the sync has settled.
            let current = this.preset(cx);
            this.picker
                .set_items(StockScreenerDelegate::preset_names(cx), window, cx);
            this.picker.set_selected(current, window, cx);
            cx.notify();
        })
        .detach();

        // Picking a preset in the picker loads its data.
        cx.subscribe(
            picker.state(),
            |this, _, event: &ComboboxEvent<SearchableVec<String>>, cx| {
                if let ComboboxEvent::Change(values) = event
                    && let Some(name) = values.last()
                {
                    this.select_preset(name.clone(), cx);
                }
            },
        )
        .detach();

        let this = Self {
            focus_handle,
            picker,
            table,
            view_mode: ViewMode::Table,
            chart_timeframe: Timeframe::default(),
            charts: BTreeMap::new(),
            chart_scroll: VirtualListScrollHandle::new(),
        };
        // Auto-load once when a preset is selected.
        if this.preset(cx).is_some() {
            Self::fetch(this.table.downgrade(), cx);
        }
        // Offline at startup: the fetch above was skipped silently; load
        // once the connection lands (refresh itself re-checks the preset).
        crate::panels::reload_on_connect::<Self>(this.table.downgrade(), cx);
        // Follow the table's data changes: the chart view's per-ticker
        // states are invalidated whenever the result set changes, and the
        // Chart tab falls back to Table when the export has no Ticker.
        cx.observe(&this.table, |this, _, cx| this.sync_chart_set(cx))
            .detach();
        // 离线期间创建的图表状态永远拉不到历史(拉取被静默跳过):连接
        // 建立即清空,可见范围随之重拉。
        crate::data::stream::on_status_change(cx, |this, _, cx| {
            if this.charts.is_empty() {
                return;
            }
            this.charts.clear();
            cx.notify();
        });
        this
    }

    fn refresh(entity: WeakEntity<Self::DataEntity>, cx: &mut App) {
        Self::fetch(entity, cx)
    }
}

impl StockScreenerPanel {
    fn preset(&self, cx: &App) -> Option<String> {
        self.table.read(cx).delegate().preset.clone()
    }

    /// Switch the content view (0 = Table, 1 = Chart; the Chart tab only
    /// exists when the result has a Ticker column).
    fn set_view_mode(&mut self, ix: usize, cx: &mut Context<Self>) {
        let mode = if ix == 1 {
            ViewMode::Chart
        } else {
            ViewMode::Table
        };
        if self.view_mode != mode {
            self.view_mode = mode;
            cx.notify();
        }
    }

    /// Switch the chart view's timeframe: every chart state is invalidated;
    /// the visible range re-fetches on the next virtual-list frame.
    fn set_chart_timeframe(&mut self, timeframe: Timeframe, cx: &mut Context<Self>) {
        if self.chart_timeframe == timeframe {
            return;
        }
        self.chart_timeframe = timeframe;
        self.charts.clear();
        cx.notify();
    }

    /// Follow the table's result set: whenever the tickers change, every
    /// chart state is invalidated (the visible range re-fetches); without a
    /// Ticker column the panel falls back to the Table view.
    fn sync_chart_set(&mut self, cx: &mut Context<Self>) {
        let tickers = self.table.read(cx).delegate().tickers();
        let same = match &tickers {
            Some(list) => {
                list.len() == self.charts.len()
                    && list.iter().all(|ticker| self.charts.contains_key(ticker))
            }
            None => self.charts.is_empty(),
        };
        if !same {
            self.charts.clear();
            if tickers.is_none() {
                self.view_mode = ViewMode::Table;
            }
            cx.notify();
        }
    }

    /// Lazily create (and fetch history for) the K-line states of the
    /// tickers currently visible in the virtual list. Runs deferred — never
    /// inside the render pass.
    fn ensure_charts(&mut self, tickers: Vec<String>, cx: &mut Context<Self>) {
        let mut created = false;
        for ticker in tickers {
            if self.charts.contains_key(&ticker) {
                continue;
            }
            let entity = cx.new(|_| {
                let mut state = KlineState::new(self.chart_timeframe);
                state.set_symbol(Some(ticker.clone()));
                state
            });
            kline::load_history(entity.downgrade(), cx);
            self.charts.insert(ticker, entity);
            created = true;
        }
        if created {
            cx.notify();
        }
    }

    /// Switch to `name` and load it. Re-selecting the current preset is a
    /// no-op; the toolbar refresh button is the explicit way to refetch.
    fn select_preset(&mut self, name: String, cx: &mut Context<Self>) {
        let changed = self.table.update(cx, |state, cx| {
            let changed = state.delegate_mut().select(name);
            if changed {
                cx.notify();
            }
            changed
        });
        if changed {
            Self::fetch(self.table.downgrade(), cx);
            cx.notify();
        }
    }

    fn add_preset(
        &mut self,
        name: String,
        parameter: anysdk::finviz_sdk::screener::Parameter,
        cx: &mut Context<Self>,
    ) {
        StockScreenerDelegate::add_preset(name.clone(), parameter, cx);
        self.select_preset(name, cx);
    }

    /// Fetch the selected preset's export into the table through the app's
    /// Finviz client. Without an API key nothing is sent: the user gets a
    /// message and the table stays idle.
    fn fetch(table: WeakEntity<TableState<StockScreenerDelegate>>, cx: &mut App) {
        let Some(preset) = table
            .upgrade()
            .and_then(|table| table.read(cx).delegate().preset.clone())
        else {
            return;
        };
        let Some(parameter) = cx
            .global::<AppState>()
            .settings
            .finviz
            .screener_parameter
            .get(&preset)
            .cloned()
        else {
            return;
        };
        // Finviz token 已下沉服务端;唯一的前置条件是 server 在线。
        // 离线时静默跳过:启动自动加载等 Connected 后由 reload_on_connect 补,
        // 手动刷新则等连接后自然生效。
        crate::data::DataRequest::new(table, tr!("app.stock_screener.failed"))
            .run(
                cx,
                |state, cx| {
                    let delegate = state.delegate_mut();
                    delegate.page.is_loading = true;
                    delegate.page.fetch_generation += 1;
                    cx.notify();
                    Some(delegate.page.fetch_generation)
                },
                // A preset switch or settings sync while this request is in
                // flight invalidates its outcome.
                |state, generation| state.delegate_mut().page.fetch_generation == generation,
                move |server| async move {
                    crate::data::api::screener::export(&server, &parameter).await
                },
                |state, cx, _generation, export| {
                    let delegate = state.delegate_mut();
                    delegate.set_data(export.headers, export.rows);
                    // The columns follow the export's headers: rebuild the
                    // table's column-group snapshot.
                    state.refresh(cx);
                    cx.notify();
                },
                |state, _cx, _generation, _err| {
                    state.delegate_mut().page.is_loading = false;
                },
            );
    }
}

impl gpui_kit::base::dock::Panel for StockScreenerPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    /// Persist the selected preset per instance; `new` reads it back.
    fn dump(&self, cx: &App) -> PanelState {
        let mut state = PanelState::new(Self::NAME);
        state.info = PanelInfo::Panel(serde_json::json!({ "preset": self.preset(cx) }));
        state
    }
}

impl EventEmitter<PanelEvent> for StockScreenerPanel {}

impl gpui_kit::Focusable for StockScreenerPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui_kit::component::dock::Panel for StockScreenerPanel {
    /// The selected preset as the tab label; the panel name until one exists.
    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self
            .preset(cx)
            .map(SharedString::from)
            .unwrap_or_else(|| tr!(Self::TITLE_KEY));
        panel_title(Self::ICON, label)
    }

    fn toolbar_buttons(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Vec<Button>> {
        Some(vec![refresh_button::<Self>(
            "screener-refresh",
            self.table.downgrade(),
        )])
    }

    /// The preset picker rides the tab title bar, next to the toolbar
    /// buttons. It is pinned to a fixed width: the combobox inside is
    /// `w_full`, and unbounded it would eat the whole tab row and push the
    /// toolbar out of view (the DatePicker trap). Preset management
    /// (new/edit/delete) lives in the shared picker footer: edit reopens the
    /// preset form seeded with the selected preset.
    fn title_suffix(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let weak = cx.weak_entity();

        Some(
            div()
                .flex_shrink_0()
                .w(px(100.))
                .child(self.picker.managed_element(
                    tr!("app.stock_screener.pick_preset"),
                    {
                        let weak = weak.clone();
                        move |cx| weak.upgrade().and_then(|p| p.read(cx).preset(cx))
                    },
                    Some(PickerAction::new(
                        "screener-add-preset",
                        IconName::Plus,
                        tr!("app.stock_screener.add_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, _| {
                                open_screener_preset_dialog(window, cx, None, {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let (name, parameter) = draft;
                                        let _ = weak.update(cx, |this, cx| {
                                            this.add_preset(name, parameter, cx)
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "screener-edit-preset",
                        AppIcon::Pencil,
                        tr!("app.stock_screener.edit_preset"),
                        {
                            let weak = weak.clone();
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                let parameter = cx
                                    .global::<AppState>()
                                    .settings
                                    .finviz
                                    .screener_parameter
                                    .get(&name)
                                    .cloned()
                                    .unwrap_or_default();
                                let previous = name.clone();
                                open_screener_preset_dialog(window, cx, Some((name, parameter)), {
                                    let weak = weak.clone();
                                    move |draft, cx| {
                                        let (name, parameter) = draft;
                                        let _ = weak.update(cx, |_, cx| {
                                            StockScreenerDelegate::upsert_preset(
                                                Some(&previous),
                                                name,
                                                parameter,
                                                cx,
                                            )
                                        });
                                    }
                                });
                            }
                        },
                    )),
                    Some(PickerAction::new(
                        "screener-delete-preset",
                        IconName::Trash,
                        tr!("app.stock_screener.delete_preset"),
                        {
                            move |window, cx, selected| {
                                let Some(name) = selected else {
                                    return;
                                };
                                open_confirm_dialog(
                                    window,
                                    cx,
                                    tr!("app.stock_screener.delete_preset"),
                                    tr!("app.stock_screener.delete_confirm", name = name.clone()),
                                    move |cx| {
                                        StockScreenerDelegate::remove_preset(&name, cx);
                                    },
                                );
                            }
                        },
                    )),
                )),
        )
    }
}

impl Render for StockScreenerPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(self.render_view_tabs(window, cx))
            .child(match self.view_mode {
                ViewMode::Table => div()
                    .flex_1()
                    .min_h_0()
                    .child(datatable::table_render(&self.table))
                    .into_any_element(),
                ViewMode::Chart => self.render_chart_view(cx).into_any_element(),
            })
    }
}

impl StockScreenerPanel {
    /// The content view's segmented switch (Table | Chart), drawn with the
    /// TabBar segmented variant's theme tokens but as a full row: each
    /// option takes the same width, and in chart mode the timeframe
    /// dropdown lives **inside** the Chart segment's rectangle. The Chart
    /// segment only renders when the export has a Ticker column.
    fn render_view_tabs(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use gpui_kit::component::{
            Sizable as _, button::ButtonVariants as _, menu::DropdownMenu as _,
        };

        let weak = cx.weak_entity();
        let has_ticker = self.table.read(cx).delegate().ticker_column().is_some();
        let chart_mode = self.view_mode == ViewMode::Chart && has_ticker;
        let timeframe = self.chart_timeframe;

        let mut track = h_flex()
            .w_full()
            .flex_shrink_0()
            .mx_1()
            .mt_1()
            .h(px(32.))
            .p(px(2.))
            .gap(px(2.))
            .rounded_lg()
            .bg(cx.theme().tokens.tab_bar_segmented)
            .child(segment_cell(
                "screener-tab-table",
                tr!("app.stock_screener.tab_table"),
                !chart_mode,
                {
                    let weak = weak.clone();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.set_view_mode(0, cx));
                    }
                },
                cx,
            ));

        if has_ticker {
            let chart_cell = segment_cell(
                "screener-tab-chart",
                tr!("app.stock_screener.tab_chart"),
                chart_mode,
                {
                    let weak = weak.clone();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |this, cx| this.set_view_mode(1, cx));
                    }
                },
                cx,
            );
            track = track.child(if chart_mode {
                // Chart 激活段:选项文字与周期下拉同处一个矩形。
                chart_cell.child(
                    Button::new("screener-timeframe")
                        .ghost()
                        .small()
                        .label(timeframe.label())
                        .icon(IconName::ChevronDown)
                        .dropdown_menu({
                            let weak = weak.clone();
                            move |menu, _, _| {
                                kline::timeframe_menu(
                                    timeframe,
                                    {
                                        let weak = weak.clone();
                                        move |tf, cx| {
                                            let _ = weak.update(cx, |this, cx| {
                                                this.set_chart_timeframe(tf, cx)
                                            });
                                        }
                                    },
                                    menu,
                                )
                            }
                        }),
                )
            } else {
                chart_cell
            });
        }
        track
    }

    /// The chart view: a virtual list over the result's tickers — only the
    /// visible range's cards exist, and each card's K-line state (bars) is
    /// created and fetched lazily as it scrolls into view.
    fn render_chart_view(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let delegate = self.table.read(cx).delegate();
        if delegate.page.is_loading {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(datatable::loading_view())
                .into_any_element();
        }
        let Some(tickers) = delegate.tickers() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(datatable::empty_view(
                    tr!("app.stock_screener.no_ticker"),
                    cx,
                ))
                .into_any_element();
        };
        if tickers.is_empty() {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(datatable::empty_view(tr!("app.stock_screener.no_data"), cx))
                .into_any_element();
        }

        let sizes: Rc<Vec<Size<Pixels>>> =
            Rc::new(tickers.iter().map(|_| size(px(0.), px(CARD_H))).collect());
        let view = cx.entity();
        div()
            .flex_1()
            .min_h_0()
            .child(
                v_virtual_list(
                    view,
                    "screener-charts",
                    sizes,
                    move |this, range, window, cx| {
                        let visible: Vec<String> = range
                            .clone()
                            .filter_map(|ix| tickers.get(ix).cloned())
                            .collect();
                        if !visible.is_empty() {
                            // 懒加载出帧外执行:渲染回调里不做状态变更。
                            cx.defer_in(window, move |this, _, cx| this.ensure_charts(visible, cx));
                        }
                        range
                            .map(|ix| {
                                let ticker = &tickers[ix];
                                kline_card(
                                    ticker.clone().into(),
                                    this.charts.get(ticker).cloned(),
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .size_full()
                .track_scroll(&self.chart_scroll),
            )
            // 虚拟列表自身不带滚动条(表格是 DataTable 自带):挂一个竖向
            // overlay 滚动条,长列表往返不再靠滚轮硬滚。
            .vertical_scrollbar(&self.chart_scroll)
            .into_any_element()
    }
}

/// 分段开关的一个单元格(TabBar segmented 同款令牌):激活 = 背景 pill +
/// 高亮文字,未激活 = 透明底 muted 文字、hover 提亮。`flex_1` 使各段同宽。
fn segment_cell(
    id: &'static str,
    label: SharedString,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let cell = div()
        .id(id)
        .flex_1()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .px_2()
        .text_sm()
        .cursor_pointer()
        .rounded_lg()
        .on_click(on_click);
    if active {
        cell.bg(cx.theme().secondary)
            .text_color(cx.theme().secondary_foreground)
            .child(label)
    } else {
        cell.text_color(cx.theme().muted_foreground)
            .hover(|style| style.bg(cx.theme().secondary_hover))
            .child(label)
    }
}
